#!/usr/bin/env python3
"""Fill the library's release columns from Discogs.

The library index stores a label, a catalog number, a release title, and a
track number for each track, and none of them come from the file's tags. The
tags name the pressing a file was ripped from, and when that pressing is a
reissue the tag names the wrong release. Discogs is the authority instead.

Matching runs in two steps so nothing writes to the library until you have read
what it would write. `match` resolves each release folder to a Discogs release
and writes a CSV of what it found. `apply` reads that CSV and writes the
release columns into the library index.

`match` reads the Discogs collection export named by `--export` first,
because it lists the releases you own and costs no requests. A folder whose
name states a catalog number is matched on that catalog number. A folder that
states none is matched on its release title, and then only when the artist
agrees, so that a title two acts have both used cannot match the wrong one.

A folder that states a catalog number the export does not list matches on its
title too, but only when the two catalog numbers name the one release: their
numbers agree once leading zeros are dropped, and neither says CD where the
other says LP. `SZ051` and `SPIRIT ZONE 051` are the one release that way.
`[TIPCD10]` and the LP `TIP LP 10` are not, and neither are `[PHNKL2080-2]` and
`BALLLP01` on another label, so those folders go to the API instead.

`match` reaches the Discogs API only for a folder the export cannot resolve,
and only when you pass `--api`. Requests are spaced a second apart, which stays
inside the authenticated limit of sixty a minute, and a rate-limit response
stops the run rather than retrying into it. `--api-limit N` stops asking after
N requests, which is how a trial run over a few folders reads what the search
comes back with before the rest of them spend anything.

A track number comes from the leading number in the file's name, not from
Discogs, so on a release spread over two discs it is the position on the disc
rather than the position on the release.

Examples:
    ```
    python3 tools/discogs_release.py match --export collection.csv --out proposed.csv
    python3 tools/discogs_release.py match --export collection.csv --out proposed.csv --api --api-limit 10
    python3 tools/discogs_release.py apply proposed.csv
    ```
"""

import argparse
import csv
import json
import os
import re
import sqlite3
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

# The Discogs API, as the address every request in this module is built
# from. `discogs_year.py` builds its own requests from the same constant.
API = "https://api.discogs.com"

# Where the library index sits by default. The Discogs collection export has
# no default, because `match` takes it as a required argument.
LIBRARY = os.path.expanduser("~/.cache/dermixen/library/library.sqlite")

# The layout version this tool writes. `SCHEMA_VERSION` in
# `crates/library/src/index.rs` has to agree with it.
SCHEMA_VERSION = 5

# Folder names that sit between a track and the folder naming its release.
# `parse_filename` in `crates/library/src/metadata.rs` skips the same ones.
PASSED_THROUGH = re.compile(r"^(cd\s*\d+|disc\s*\d+|wav|flac|mp3)$", re.IGNORECASE)

# A catalog number in square brackets at the end of a folder name, as in
# `Pleiadians - I.F.O. [DRAGCD01]`.
BRACKETED = re.compile(r"\[([^\[\]]+)\]\s*$")

# A track number at the front of a file name: one to three digits followed by a
# space, a dot, or a hyphen.
LEADING_NUMBER = re.compile(r"^(\d{1,3})\s*[-.\s]")

# One request a second stays inside the authenticated ceiling of sixty a
# minute with room to spare.
SECONDS_BETWEEN_REQUESTS = 1.0

# Discogs refuses a request that does not name the program making it.
USER_AGENT = "DermixenReleaseMatcher/1.0 +https://github.com/mmacy/dermixen"


class _RefuseCrossHostRedirect(urllib.request.HTTPRedirectHandler):
    """Refuses a redirect that points at a different host, port, or scheme.

    A request built here carries the Discogs `Authorization` header, and
    `urllib` forwards every header, that one included, when it follows a
    redirect on its own. Refusing a redirect that changes the host, the
    port, or the scheme keeps the header from ever reaching a host it was
    not made for, and keeps it from ever crossing from `https` to `http`,
    where it would travel in the clear.
    """

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        before = urllib.parse.urlsplit(req.full_url)
        after = urllib.parse.urlsplit(newurl)
        if (before.scheme, before.hostname, before.port) != (after.scheme, after.hostname, after.port):
            return None
        return super().redirect_request(req, fp, code, msg, headers, newurl)


_OPENER = urllib.request.build_opener(_RefuseCrossHostRedirect)


def open_request(request: urllib.request.Request, timeout: float = 30):
    """Send `request` through the opener that refuses a cross-host redirect.

    Both Discogs tools send every request through this function rather than
    through `urllib.request.urlopen` directly, so that neither can be made to
    hand its credentials to a host Discogs never named.
    """
    return _OPENER.open(request, timeout=timeout)


def as_text(value) -> str:
    """`value`, taken as text, or the empty string when it is not text.

    A Discogs answer sometimes puts a number, a list, or an object where the
    format calls for a name or a title. None of those state text this
    function will guess at, so each becomes the empty string instead of
    crashing whatever reads the result as a string.
    """
    return value if isinstance(value, str) else ""


def as_label_list(value) -> list[str]:
    """Every label name a Discogs `label` field gives, as a list of text.

    The field is normally a list of names. Some answers give a single string
    instead, and that whole string is the one label name, never split into
    its characters. A value that is neither becomes an empty list.
    """
    if isinstance(value, str):
        return [value]
    if isinstance(value, list):
        return [item for item in value if isinstance(item, str)]
    return []


def as_year_or(value, default: int) -> int:
    """The year a Discogs `year` field names, or `default` when it names none.

    A field names no year plainly when it holds a roman numeral, a list, or
    nothing at all.
    """
    if isinstance(value, bool):
        return default
    if isinstance(value, int):
        return value
    if isinstance(value, str) and value.isdigit():
        return int(value)
    return default


def escaped_like(fragment: str) -> str:
    """`fragment` with SQLite's `LIKE` wildcards escaped.

    A folder name that happens to contain `%` or `_` is matched literally
    rather than as a wildcard once escaped this way. Pass the result to a
    query that also gives `LIKE` the clause `ESCAPE '\\'`.
    """
    return fragment.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")


def cell(value):
    """`value`, with a quote added only if a spreadsheet might read it as a
    formula.

    A text value that begins with `=`, `+`, `-`, `@`, a tab, or a carriage
    return is a formula to a spreadsheet that opens the CSV this module
    writes, and this returns it with a single quote in front instead, which
    every common spreadsheet program reads back as the original text. Any
    other value, text or not, is returned unchanged.
    """
    if isinstance(value, str) and value[:1] in ("=", "+", "-", "@", "\t", "\r"):
        return f"'{value}"
    return value


def uncell(value):
    """The inverse of [`cell`][tools.discogs_release.cell].

    A value that begins with a single quote followed by one of `=`, `+`,
    `-`, `@`, a tab, or a carriage return loses that quote, undoing what
    `cell` added when the CSV was written. Every other value is returned
    unchanged.
    """
    if (
        isinstance(value, str)
        and len(value) >= 2
        and value[0] == "'"
        and value[1] in ("=", "+", "-", "@", "\t", "\r")
    ):
        return value[1:]
    return value


def normalized(catalog_number: str) -> str:
    """A catalog number with its spacing and punctuation dropped, so that
    `TO3 CD 002`, `TO3-CD002`, and `TO3CD002` all compare equal."""
    return re.sub(r"[^A-Z0-9]", "", catalog_number.upper())


def release_folder(path: str) -> str:
    """The folder naming the release a track belongs to.

    Walks up from the file past any `CD1`, `CD2`, or format folder, and stops
    at the first folder that does not look like one of those.
    """
    folder = os.path.dirname(path)
    while PASSED_THROUGH.match(os.path.basename(folder)):
        parent = os.path.dirname(folder)
        if parent == folder:
            break
        folder = parent
    return folder


def folder_catalog_number(folder: str) -> str | None:
    """The catalog number in a folder's name, or `None` when it has none."""
    found = BRACKETED.search(os.path.basename(folder))
    return found.group(1).strip() if found else None


def folder_artist_and_title(folder: str) -> tuple[str | None, str]:
    """The artist and the release title a folder's name states.

    A folder is named `Artist - Album`, optionally with a catalog number in
    square brackets after it. A folder whose name has no separator names only
    the release, and the artist comes back as `None`.
    """
    name = BRACKETED.sub("", os.path.basename(folder)).strip()
    artist, separator, title = name.partition(" - ")
    if not separator:
        return None, name
    return artist.strip(), title.strip()


def comparable(value: str) -> str:
    """A name with its case, spacing, and punctuation dropped, so that two
    spellings of one title compare equal."""
    return re.sub(r"[^a-z0-9]", "", value.lower())


def track_number(path: str) -> int | None:
    """The track's position on its disc, from the leading number in the file's
    name, or `None` when the name does not begin with one."""
    found = LEADING_NUMBER.match(os.path.basename(path))
    if not found:
        return None
    number = int(found.group(1))
    # A three-digit number is a disc number and a track number, as in `203`,
    # so the last two digits are the position on the disc.
    if number > 99:
        number %= 100
    return number or None


def load_export(path: str) -> tuple[dict[str, list[dict]], dict[str, list[dict]]]:
    """The Discogs collection export, indexed two ways.

    The first index is keyed by every normalized catalog number each release
    lists, so a release that lists several gets an entry under each. The second
    is keyed by the release title, for a folder whose name states no catalog
    number.
    """
    by_catalog_number: dict[str, list[dict]] = {}
    by_title: dict[str, list[dict]] = {}
    with open(path, newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            labels = [part.strip() for part in row["Label"].split(",")]
            for index, catalog_number in enumerate(row["Catalog#"].split(",")):
                key = normalized(catalog_number)
                if not key or key == "NONE":
                    continue
                by_catalog_number.setdefault(key, []).append(
                    {
                        "label": labels[index] if index < len(labels) else labels[0],
                        "catalog_number": catalog_number.strip(),
                        "title": row["Title"].strip(),
                        "artist": row["Artist"].strip(),
                        "release_id": row["release_id"].strip(),
                    }
                )
            first = [part.strip() for part in row["Catalog#"].split(",")][0]
            by_title.setdefault(comparable(row["Title"]), []).append(
                {
                    "label": labels[0],
                    "catalog_number": "" if normalized(first) == "NONE" else first,
                    "title": row["Title"].strip(),
                    "artist": row["Artist"].strip(),
                    "release_id": row["release_id"].strip(),
                }
            )
    return by_catalog_number, by_title


def credentials() -> tuple[str, str] | None:
    """The Discogs consumer key and secret from the repository's `.env` file,
    or `None` when the file does not name both.

    The file is the one Discogs offers on an application's settings page, with
    a name and a value on each line separated by a tab.
    """
    path = os.path.join(os.path.dirname(os.path.dirname(__file__)), ".env")
    if not os.path.exists(path):
        return None
    found: dict[str, str] = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            parts = re.split(r"\t+|  +", line.strip(), maxsplit=1)
            if len(parts) == 2:
                found[parts[0].strip().lower()] = parts[1].strip()
    key = found.get("consumer key")
    secret = found.get("consumer secret")
    return (key, secret) if key and secret else None


class RateLimited(Exception):
    """Discogs answered that too many requests have been made."""


def search_releases(catalog_number: str, auth: tuple[str, str]) -> list[dict]:
    """Every release Discogs lists under a catalog number.

    Raises [`RateLimited`][tools.discogs_release.RateLimited] when Discogs
    answers 429, so that a run stops rather than pushing at a closed door.

    Returns:
        Every result the answer's `results` field gives, keeping only the
        entries that are themselves objects. An answer of the wrong shape,
        such as `results` holding text instead of a list, gives no results
        rather than raising.
    """
    query = urllib.parse.urlencode({"catno": catalog_number, "type": "release"})
    request = urllib.request.Request(
        f"{API}/database/search?{query}",
        headers={
            "User-Agent": USER_AGENT,
            "Authorization": f"Discogs key={auth[0]}, secret={auth[1]}",
        },
    )
    try:
        with open_request(request) as answer:
            remaining = answer.headers.get("X-Discogs-Ratelimit-Remaining")
            body = json.load(answer)
    except urllib.error.HTTPError as problem:
        if problem.code == 429:
            raise RateLimited(catalog_number) from problem
        raise
    # Discogs counts down the requests left in the current window. Stopping
    # while a few remain leaves room for whatever else uses this account. A
    # header that does not name a number is treated as not counting down.
    try:
        low_on_requests = remaining is not None and int(remaining) < 5
    except ValueError:
        low_on_requests = False
    if low_on_requests:
        raise RateLimited(catalog_number)
    results = body.get("results") if isinstance(body, dict) else None
    return [result for result in results if isinstance(result, dict)] if isinstance(results, list) else []


def result_artist_and_title(result: dict) -> tuple[str, str]:
    """The artist and the release title a search result names.

    A result writes both into one field as `Artist - Title`. Discogs tells two
    acts of the same name apart by a number after the name, as in `Brass (8)`,
    and that number is dropped here so the name compares against a folder's.
    A `title` field that is not text gives an empty artist and an empty title.
    """
    artist, separator, title = as_text(result.get("title")).partition(" - ")
    if not separator:
        return "", artist.strip()
    return re.sub(r"\s*\(\d+\)$", "", artist.strip()), title.strip()


def names_the_same_act(result: dict, wanted: str) -> bool:
    """Whether a search result names the act a folder names, treating every
    spelling of a compilation as one act."""
    compilations = {"va", "various", "variousartists"}
    theirs = comparable(result_artist_and_title(result)[0])
    if wanted in compilations:
        return theirs in compilations
    return theirs == wanted or theirs == f"the{wanted}" or f"the{theirs}" == wanted


def from_search(
    results: list[dict], catalog_number: str, artist: str | None
) -> dict | None:
    """The one release a search settled on, or `None` when the search found
    nothing that fits or could not choose between what it found.

    Discogs matches a catalog number loosely, so a search for `EOR003` answers
    with six releases by four acts and a search for `SZ051` answers with
    twenty-eight releases that merely contain those characters. Only a result
    whose catalog number matches exactly, once spacing and punctuation are
    dropped, is a candidate, and when the folder names an act only that act's
    releases are. What is left is one release in one or more pressings, and the
    earliest pressing is the answer. Candidates that still disagree on the
    title are an ambiguity this tool does not resolve.
    """
    wanted = normalized(catalog_number)
    exact = [
        result
        for result in results
        if normalized(as_text(result.get("catno"))) == wanted
    ]
    if not exact:
        return None
    if artist:
        agreeing = [
            result for result in exact if names_the_same_act(result, comparable(artist))
        ]
        if not agreeing:
            return None
        exact = agreeing
    titles = {comparable(result_artist_and_title(result)[1]) for result in exact}
    if len(titles) > 1:
        return None
    # A year that does not name a plain number, such as a roman numeral, sorts
    # last rather than first, so it never wins a release that has a real year.
    earliest = min(exact, key=lambda result: as_year_or(result.get("year"), 9999))
    labels = as_label_list(earliest.get("label"))
    return {
        "label": labels[0] if labels else "",
        "catalog_number": as_text(earliest.get("catno")).strip(),
        "title": result_artist_and_title(earliest)[1],
        "release_id": str(earliest.get("id", "")),
    }


def digit_runs(catalog_number: str) -> list[int]:
    """The numbers in a catalog number, in order, with leading zeros dropped,
    so that `BMPHQCD01` and `bmphqcd001` both give `[1]`."""
    return [int(run) for run in re.findall(r"\d+", catalog_number)]


def pressing_format(catalog_number: str) -> str:
    """`CD` or `LP` when a catalog number says which pressing it is, and the
    empty string when it says neither."""
    letters = normalized(catalog_number)
    if "LP" in letters:
        return "LP"
    return "CD" if "CD" in letters else ""


def names_the_same_release(mine: str, theirs: str) -> bool:
    """Whether two catalog numbers name the one release.

    A folder writes a catalog number the way its owner files it and Discogs
    writes it the way the label printed it, so `SZ051` and `SPIRIT ZONE 051`
    are the same release, as are `TRANR604CD` and `TRANRCD604`. The numbers in
    the two have to agree for that, which is what separates those from
    `PHNKL2080-2` and `BALLLP01`. Where one says CD and the other says LP they
    are two pressings of one release rather than one release, and the catalog
    number of the pressing on disk is not the other one's.
    """
    if digit_runs(mine) != digit_runs(theirs) or not digit_runs(mine):
        return False
    mine_format, theirs_format = pressing_format(mine), pressing_format(theirs)
    return not (mine_format and theirs_format and mine_format != theirs_format)


def by_title_match(folder: str, by_title: dict[str, list[dict]]) -> list[dict] | None:
    """The export entries whose title the folder's name states, or `None` when
    the export has no entry under that title.

    A title on its own can name two different acts' releases, so when the
    folder's name states an artist, only an entry naming that artist or a
    compilation counts. The answer is a list so that the caller checks it for
    an ambiguity the same way it checks a catalog number match.
    """
    artist, title = folder_artist_and_title(folder)
    entries = by_title.get(comparable(title))
    if not entries:
        return None
    if artist is None:
        return entries
    wanted = comparable(artist)
    agreeing = [
        entry
        for entry in entries
        if comparable(entry["artist"]) in (wanted, "various", "variousartists")
    ]
    return agreeing or None


def library_folders(path: str) -> dict[str, list[str]]:
    """Every release folder in the library index and the file paths under it."""
    connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    try:
        version = connection.execute("PRAGMA user_version").fetchone()[0]
        if version != SCHEMA_VERSION:
            raise SystemExit(
                f"{path} is a version {version} library file, and this tool writes "
                f"version {SCHEMA_VERSION}. Scan again into a fresh library file."
            )
        folders: dict[str, list[str]] = {}
        for (file_path,) in connection.execute("SELECT path FROM tracks"):
            folders.setdefault(release_folder(file_path), []).append(file_path)
    finally:
        connection.close()
    return folders


FIELDS = [
    "folder",
    "tracks",
    "folder_catalog_number",
    "label",
    "catalog_number",
    "release_title",
    "data_source",
    "note",
]


def run_match(args: argparse.Namespace) -> int:
    """Resolves every release folder and writes what it found as a CSV."""
    by_catalog_number, by_title = load_export(args.export)
    folders = library_folders(args.library)
    auth = credentials() if args.api else None
    if args.api and not auth:
        print(
            "no Discogs consumer key and secret in .env, so no API lookups",
            file=sys.stderr,
        )
    counts = {"export": 0, "title": 0, "api": 0, "unresolved": 0}
    asked = 0
    rows = []
    stopped = None
    for folder in sorted(folders):
        paths = folders[folder]
        in_folder = folder_catalog_number(folder)
        row = {
            "folder": folder,
            "tracks": len(paths),
            "folder_catalog_number": in_folder or "",
            "label": "",
            "catalog_number": "",
            "release_title": "",
            "data_source": "",
            "note": "",
        }
        found = by_catalog_number.get(normalized(in_folder)) if in_folder else None
        if not found:
            # A title match for a folder that states a catalog number is taken
            # only when the two numbers name the one release. Otherwise the
            # match answers with whichever pressing the collection happens to
            # contain, which is how the Doof folder `[TIPCD10]` matches the LP
            # `TIP LP 10`. Those folders go to the API instead.
            titled = by_title_match(folder, by_title)
            if titled and in_folder:
                titled = [
                    entry
                    for entry in titled
                    if names_the_same_release(in_folder, entry["catalog_number"])
                ] or None
            if titled:
                found = titled
                counts["title"] += 1
                counts["export"] -= 1
        if found:
            titles = {entry["title"].lower() for entry in found}
            if len(titles) == 1:
                row["label"] = found[0]["label"]
                row["catalog_number"] = found[0]["catalog_number"]
                row["release_title"] = found[0]["title"]
                row["data_source"] = "discogs_export"
                counts["export"] += 1
            else:
                row["note"] = (
                    f"the export lists {len(found)} releases under this catalog "
                    f"number and they disagree on the title"
                )
                counts["unresolved"] += 1
        elif auth and in_folder and not stopped and asked != args.api_limit:
            asked += 1
            time.sleep(SECONDS_BETWEEN_REQUESTS)
            try:
                chosen = from_search(
                    search_releases(in_folder, auth),
                    in_folder,
                    folder_artist_and_title(folder)[0],
                )
            except RateLimited:
                stopped = folder
                row["note"] = "the run stopped here, because Discogs rate-limited it"
                counts["unresolved"] += 1
                chosen = None
            if chosen:
                row["label"] = chosen["label"]
                row["catalog_number"] = chosen["catalog_number"]
                row["release_title"] = chosen["title"]
                row["data_source"] = "discogs_api"
                counts["api"] += 1
            elif not stopped:
                row["note"] = (
                    "the Discogs API found no one release whose catalog number "
                    "matches exactly and whose act the folder names"
                )
                counts["unresolved"] += 1
        else:
            if not in_folder:
                row["note"] = (
                    "the folder name states no catalog number, and the export "
                    "lists no release under its title"
                )
            elif auth and asked == args.api_limit:
                row["note"] = "the run had spent the requests --api-limit allows"
            else:
                row["note"] = "the export lists no release under this catalog number"
            counts["unresolved"] += 1
        rows.append(row)
    with open(args.out, "w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows({key: cell(value) for key, value in row.items()} for row in rows)
    resolved = counts["export"] + counts["title"] + counts["api"]
    tracks = sum(row["tracks"] for row in rows if row["data_source"])
    print(
        f"{resolved} of {len(rows)} folders resolved, covering {tracks} tracks: "
        f"{counts['export']} matched in the export by catalog number, "
        f"{counts['title']} by title, {counts['api']} from the API. "
        f"{counts['unresolved']} unresolved."
    )
    print(f"wrote {args.out}")
    if stopped:
        print(f"Discogs rate-limited the run at {stopped}", file=sys.stderr)
        return 1
    return 0


def run_apply(args: argparse.Namespace) -> int:
    """Writes the release columns into the library index from a match CSV."""
    with open(args.proposed, newline="", encoding="utf-8") as handle:
        rows = [
            {key: uncell(value) for key, value in row.items()}
            for row in csv.DictReader(handle)
            if row["data_source"]
        ]
    connection = sqlite3.connect(args.library)
    try:
        version = connection.execute("PRAGMA user_version").fetchone()[0]
        if version != SCHEMA_VERSION:
            raise SystemExit(
                f"{args.library} is a version {version} library file, and this tool "
                f"writes version {SCHEMA_VERSION}."
            )
        written = 0
        for row in rows:
            paths = [
                path
                for (path,) in connection.execute(
                    "SELECT path FROM tracks WHERE path LIKE ? ESCAPE '\\'",
                    (escaped_like(row["folder"]) + "/%",),
                )
                if release_folder(path) == row["folder"]
            ]
            for path in paths:
                connection.execute(
                    "UPDATE tracks SET label = ?, catalog_number = ?, "
                    "release_title = ?, track_number = ?, release_data_source = ? "
                    "WHERE path = ?",
                    (
                        row["label"] or None,
                        row["catalog_number"] or None,
                        row["release_title"] or None,
                        track_number(path),
                        row["data_source"],
                        path,
                    ),
                )
                written += 1
        connection.commit()
    finally:
        connection.close()
    print(f"wrote the release columns of {written} tracks in {len(rows)} folders")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--library", default=LIBRARY, help="the library index")
    commands = parser.add_subparsers(dest="command", required=True)

    matcher = commands.add_parser("match", help="resolve release folders to releases")
    matcher.add_argument("--export", required=True, help="the Discogs collection export CSV")
    matcher.add_argument("--out", required=True, help="the CSV to write")
    matcher.add_argument(
        "--api",
        action="store_true",
        help="ask the Discogs API about a folder the export cannot resolve",
    )
    matcher.add_argument(
        "--api-limit",
        type=int,
        default=None,
        metavar="N",
        help="ask the Discogs API at most N times, for a trial run over a few folders",
    )
    matcher.set_defaults(run=run_match)

    applier = commands.add_parser("apply", help="write a match CSV into the library")
    applier.add_argument("proposed", help="the CSV `match` wrote")
    applier.set_defaults(run=run_apply)

    args = parser.parse_args()
    return args.run(args)


if __name__ == "__main__":
    sys.exit(main())
