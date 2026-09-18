#!/usr/bin/env python3
"""Correct the library's year column, which many tags get wrong.

The `year` column means the year the music was first released or produced, not
the year of the pressing on disk. A reissue's tag names the year of the
pressing, so a track on `Digitalis - The Early Years 1995-2000` is tagged 2020,
and a 1997 recording on a 2013 DAT Records pressing is tagged 2013.

Matching runs in two steps so nothing writes to the library until you have read
what it would write. `match` proposes a year for every suspect row and writes a
CSV of what it found and why. `apply` reads that CSV and writes the years.

The sources answer in this order, and the first that answers wins. Another
copy of the track that the library dates before the suspect year, a year the
collection export named by `--export` gives the release, another cut of a track
the library already dates, and a year the title states outright cost no
requests. Then, with `--api`, the catalog number in the folder name is looked
up, which dates every track on the release in one request, and a track with no
catalog number goes to the Discogs track search, which answers with the
releases containing that track, and the earliest of those is the year.

A release on an archival label is not evidence of a year. DAT Records presses
1990s material that circulated only on DAT tapes and dates it to the pressing,
and the labels in `ARCHIVAL_LABELS` do the same. A release whose title says it
collects older music is not evidence either. Where every release the search
finds is one of those, the search has found no original year, and that absence
is the finding. The row's year is cleared rather than guessed, because an empty
year is better than a wrong one.

Examples:
    ```
    python3 tools/discogs_year.py match --export collection.csv --out years.csv
    python3 tools/discogs_year.py match --export collection.csv --out years.csv --api --api-limit 20
    python3 tools/discogs_year.py apply years.csv
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

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from discogs_release import (  # noqa: E402
    API,
    LIBRARY,
    folder_artist_and_title,
    folder_catalog_number,
    SCHEMA_VERSION,
    USER_AGENT,
    RateLimited,
    as_label_list,
    as_text,
    cell,
    comparable,
    credentials,
    escaped_like,
    load_export,
    names_the_same_act,
    normalized,
    open_request,
    release_folder,
    search_releases,
    uncell,
)

# A year at or after this one is a suspect. Goa trance that says 2005 or later
# is nearly always a reissue of older music. `--suspect-from` changes it.
SUSPECT_FROM = 2005

# Goa trance has no releases before this year, so a search result older than
# it is a different recording that happens to share a name.
EARLIEST_PLAUSIBLE = 1988

# Labels that press older material and date it to the pressing. A release on
# one of these says nothing about when the music was made. The names are
# matched as fragments, because Discogs writes a label several ways.
ARCHIVAL_LABELS = [
    "dat records",
    "dat mafia",
    "dat universe",
    "unreleased goa",
    "memo604",
    "digital reprints",
]

# Words a release puts in its title when it collects older music. A release
# named this way dates the collection rather than the music, so it is no
# evidence of when a track was made. This catches the self-released
# retrospectives that carry no label at all, such as Nervasystem's
# `Early Daze Compilation`.
RETROSPECTIVE_TITLES = [
    "early years",
    "early daze",
    "the early",
    "singles collection",
    "single collection",
    "singles from",
    "tracks collection",
    "goa years",
    " years (",
    "retrospective",
    "anthology",
    "archive",
    "rewind",
    "remaster",
    "re-master",
    "reissue",
    "spirit of the 90",
    "best of",
    "discography",
]

# A run of requests is spaced this far apart, which stays under the
# authenticated ceiling of sixty a minute over a long pass.
SECONDS_BETWEEN_REQUESTS = 1.2


def is_archival(label: str) -> bool:
    """Whether a label presses older material and dates it to the pressing.

    Suntrip, Zion 604 and Anjuna are not on this list even though they press
    older material, because each also releases new music, so the label alone
    does not settle a release. Their reissues are caught by the release title
    instead, or decided one release at a time.
    """
    lowered = label.lower()
    return any(fragment in lowered for fragment in ARCHIVAL_LABELS)


def stated_year(title: str) -> int | None:
    """The recording year a title states, or `None` when it states none.

    A release names the year it was recorded when that is the point of it, as
    `Live In Athens 1996` does, and so does a track called
    `(Summer 1996 Live Mix)`. A title naming two years states a span rather
    than a year, so `The Early Years 1995-2000` gives nothing.
    """
    years = [
        int(found)
        for found in re.findall(r"(?<!\d)(\d{4})(?!\d)", title)
        if EARLIEST_PLAUSIBLE <= int(found) <= 2004
    ]
    return years[0] if len(years) == 1 else None


def is_retrospective(title: str) -> bool:
    """Whether a release title says the release collects older music."""
    lowered = title.lower()
    return any(fragment in lowered for fragment in RETROSPECTIVE_TITLES)


def search_tracks(artist: str, title: str, auth: tuple[str, str]) -> list[dict]:
    """Every release Discogs lists that contains this artist's track.

    Returns:
        Every result the answer's `results` field gives, keeping only the
        entries that are themselves objects. An answer of the wrong shape
        gives no results rather than raising.
    """
    query = urllib.parse.urlencode(
        {"artist": artist, "track": title, "type": "release"}
    )
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
            raise RateLimited(f"{artist} - {title}") from problem
        raise
    # A header that does not name a number is treated as not counting down.
    try:
        low_on_requests = remaining is not None and int(remaining) < 5
    except ValueError:
        low_on_requests = False
    if low_on_requests:
        raise RateLimited(f"{artist} - {title}")
    results = body.get("results") if isinstance(body, dict) else None
    return [result for result in results if isinstance(result, dict)] if isinstance(results, list) else []


def earliest_year(results: list[dict], suspect_from: int) -> tuple[int | None, str]:
    """The year the music was first released, and what says so.

    A result on an archival label is passed over, because that label's date is
    the pressing rather than the recording. When every result is archival, or
    every result is itself in the suspect era, nothing here is evidence of an
    original year and the answer is `None`.
    """
    dated = []
    for result in results:
        year = result.get("year")
        if not year or not str(year).isdigit() or int(year) < EARLIEST_PLAUSIBLE:
            continue
        dated.append((int(year), result))
    if not dated:
        return None, "Discogs lists no dated release containing this track"
    year, result = min(dated, key=lambda pair: pair[0])
    labels = as_label_list(result.get("label"))
    title = as_text(result.get("title"))
    label = labels[0] if labels else "no label"
    # The earliest release is the answer only when it is the music's own
    # release. An archival pressing or a retrospective collection is dated
    # when it was assembled, so the music is older than it by an unknown
    # amount, and an unknown year is left empty rather than guessed.
    # One archival label among a release's several is enough. Discogs lists a
    # recording studio and an artist's own imprint alongside the label, so the
    # DAT reissue of the Crop Circles EP reads as DAT Records, Soundbusters
    # Recording Studios, and Etnicanet, and only the first of those says what
    # kind of release it is.
    # An archival pressing can still name the year it was recorded, and
    # `Etnica - Live In Athens 1996` is the recording rather than the pressing
    # whoever put it out. The stated year wins over the rejection.
    spoken = stated_year(title)
    if any(is_archival(one) for one in labels):
        if spoken:
            return spoken, f"{title} states the year {spoken}"
        return None, f"the earliest is {title} on {label}, {year}, an archival pressing"
    if is_retrospective(title):
        if spoken:
            return spoken, f"{title} states the year {spoken}"
        return None, f"the earliest is {title}, {year}, a retrospective collection"
    return year, f"{title} on {label}, {year}"


FIELDS = [
    "hash",
    "folder",
    "artist",
    "title",
    "release_title",
    "catalog_number",
    "label",
    "current_year",
    "proposed_year",
    "source",
    "evidence",
]


def suspects(
    connection: sqlite3.Connection, suspect_from: int, include_undated: bool
) -> list[dict]:
    """Every row whose year needs work, with its artist and title.

    A year in the suspect era is one the tags probably got wrong. A row with no
    year at all needs the same lookups, and gets them when `include_undated`
    says so.
    """
    where = "year >= ?" if not include_undated else "(year >= ? OR year IS NULL)"
    rows = connection.execute(
        "SELECT hash, path, artist, title, COALESCE(release_title, ''), year, "
        f"COALESCE(catalog_number, ''), COALESCE(label, '') FROM tracks WHERE {where} "
        "ORDER BY artist, title",
        (suspect_from,),
    )
    return [
        {
            "hash": hash_text,
            "folder": release_folder(path),
            "artist": artist or "",
            "title": title or "",
            "release_title": release_title,
            "current_year": year,
            "catalog_number": catalog_number,
            "label": label,
        }
        for hash_text, path, artist, title, release_title, year, catalog_number, label in rows
    ]


def release_year(
    catalog_number: str, artist: str | None, auth: tuple[str, str]
) -> tuple[int | None, str]:
    """The year Discogs gives the release with this catalog number.

    A catalog number identifies a release far better than a track title does,
    and one lookup dates every track on the release. Discogs matches a catalog
    number loosely, so `BR013CD` answers with thirteen releases of which two
    carry that exact number, and naming the act settles which. The archival
    and retrospective rules apply here as they do to a track search, since a
    reissue's release date is the pressing rather than the music.
    """
    results = search_releases(catalog_number, auth)
    wanted = normalized(catalog_number)
    exact = [
        result
        for result in results
        if normalized(as_text(result.get("catno"))) == wanted
    ]
    if artist:
        named = [result for result in exact if names_the_same_act(result, comparable(artist))]
        if named:
            exact = named
    return earliest_year(exact, SUSPECT_FROM)


def export_years(path: str) -> dict[str, int]:
    """The year the collection export gives each release, by catalog number.

    The export dates the pressing that is owned, so this year is worth taking
    only where it is earlier than the year already stored. Where it is later,
    the pressing is the reissue and the tag is the older record of the two.
    """
    by_catalog_number, _ = load_export(path)
    found: dict[str, int] = {}
    with open(path, newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            year = row["Released"].strip()[:4]
            if not year.isdigit():
                continue
            for catalog_number in row["Catalog#"].split(","):
                key = normalized(catalog_number)
                if key and key != "NONE":
                    found[key] = min(int(year), found.get(key, 9999))
    # Only a catalog number the export actually indexes counts, so a release
    # the match could not resolve cannot pick one up by accident.
    return {key: year for key, year in found.items() if key in by_catalog_number}


def agreed_by_release(rows: list[dict]) -> dict[str, int]:
    """The year a release settles on, for each release where its tracks agree.

    A track Discogs does not index sits on a release whose other tracks it
    does, and those tracks date the release. Two tracks have to agree before
    the release counts as dated, and any disagreement leaves it undated,
    because a compilation of several years has no one year to give.
    """
    found: dict[str, set[int]] = {}
    for row in rows:
        if row.get("source") in ("discogs", "confirmed", "library") and row["proposed_year"]:
            found.setdefault(row["folder"], set()).add(int(row["proposed_year"]))
    return {
        folder: years.pop()
        for folder, years in found.items()
        if len(years) == 1 and len([r for r in rows if r["folder"] == folder and r.get("proposed_year")]) >= 2
    }


def trusted_years(connection: sqlite3.Connection, suspect_from: int) -> dict[tuple[str, str], int]:
    """The earliest year the library already records for each artist and title,
    counting only rows outside the suspect era."""
    found: dict[tuple[str, str], int] = {}
    for artist, title, year in connection.execute(
        "SELECT artist, title, MIN(year) FROM tracks "
        "WHERE year IS NOT NULL AND year < ? AND artist IS NOT NULL AND title IS NOT NULL "
        "GROUP BY lower(trim(artist)), lower(trim(title))",
        (suspect_from,),
    ):
        found[(comparable(artist), comparable(title))] = year
    return found


def open_library(path: str, writable: bool) -> sqlite3.Connection:
    """The library file, refused when it is not the layout this tool writes."""
    connection = (
        sqlite3.connect(path)
        if writable
        else sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    )
    version = connection.execute("PRAGMA user_version").fetchone()[0]
    if version != SCHEMA_VERSION:
        connection.close()
        raise SystemExit(
            f"{path} is a version {version} library file, and this tool writes "
            f"version {SCHEMA_VERSION}. Scan again into a fresh library file."
        )
    return connection


def run_match(args: argparse.Namespace) -> int:
    """Proposes a year for every suspect row and writes the CSV."""
    connection = open_library(args.library, writable=False)
    try:
        rows = suspects(connection, args.suspect_from, args.include_undated)
        known = trusted_years(connection, args.suspect_from)
    finally:
        connection.close()

    auth = credentials() if args.api else None
    if args.api and not auth:
        print("no Discogs consumer key and secret in .env, so no API lookups", file=sys.stderr)
    counts = {"library": 0, "discogs": 0, "cleared": 0, "unasked": 0}
    asked = 0
    # One lookup per catalog number, however many tracks share it.
    looked_up: dict[str, tuple[int | None, str]] = {}
    stopped = None

    from_export = export_years(args.export)
    for row in rows:
        key = (comparable(row["artist"]), comparable(row["title"]))
        year = known.get(key)
        if not year:
            # The release the export dates, but only when it dates it earlier
            # than the tag does, since the later of the two is the reissue.
            # The export dates the pressing that is owned. On an archival
            # label or a retrospective that is the year it was assembled, not
            # the year the music was made, so it says nothing here.
            listed = from_export.get(normalized(row["catalog_number"]))
            if is_archival(row["label"]) or is_retrospective(row["release_title"]):
                listed = None
            if listed and better_than(listed, row["current_year"]):
                row["proposed_year"] = listed
                row["source"] = "export"
                row["evidence"] = f"the collection export dates this release {listed}"
                counts["export"] = counts.get("export", 0) + 1
                continue
        if not year:
            # An alternate mix of a track the library already dates is the
            # same recording session as the track it is a mix of.
            base = base_title(row["title"])
            if base != row["title"]:
                original = known.get((comparable(row["artist"]), comparable(base)))
                if original and better_than(original, row["current_year"]):
                    row["proposed_year"] = original
                    row["source"] = "remix_of"
                    row["evidence"] = f"a mix of {base}, which the library dates {original}"
                    counts["remix_of"] = counts.get("remix_of", 0) + 1
                    continue
        if not year:
            # A year the release or the track states outright.
            spoken = stated_year(row["release_title"]) or stated_year(row["title"])
            if spoken and better_than(spoken, row["current_year"]):
                row["proposed_year"] = spoken
                row["source"] = "stated"
                row["evidence"] = f"the title states the year {spoken}"
                counts["stated"] = counts.get("stated", 0) + 1
                continue
        if year:
            row["proposed_year"] = year
            row["source"] = "library"
            row["evidence"] = f"another copy of this track in the library is dated {year}"
            counts["library"] += 1
            continue
        # A catalog number dates the whole release in one request, so it is
        # asked before the track search is.
        catalog_number = row["catalog_number"] or folder_catalog_number(row["folder"]) or ""
        if auth and not stopped and catalog_number and asked != args.api_limit:
            key = normalized(catalog_number)
            if key not in looked_up:
                asked += 1
                time.sleep(SECONDS_BETWEEN_REQUESTS)
                try:
                    looked_up[key] = release_year(
                        catalog_number, folder_artist_and_title(row["folder"])[0], auth
                    )
                except RateLimited:
                    stopped = catalog_number
                    looked_up[key] = (None, "")
            found, why = looked_up[key]
            if found and better_than(found, row["current_year"]):
                row["proposed_year"] = found
                row["source"] = "release_lookup"
                row["evidence"] = f"{catalog_number} is {why}"
                counts["release_lookup"] = counts.get("release_lookup", 0) + 1
                continue
        if not auth or stopped or asked == args.api_limit or not row["artist"] or not row["title"]:
            row["proposed_year"] = ""
            row["source"] = ""
            row["evidence"] = (
                "no artist or title to search on"
                if not row["artist"] or not row["title"]
                else "the run did not ask Discogs about this one"
            )
            counts["unasked"] += 1
            continue
        asked += 1
        time.sleep(SECONDS_BETWEEN_REQUESTS)
        try:
            results = search_tracks(row["artist"], row["title"], auth)
        except RateLimited:
            stopped = f"{row['artist']} - {row['title']}"
            row["proposed_year"] = ""
            row["source"] = ""
            row["evidence"] = "the run stopped here, because Discogs rate-limited it"
            counts["unasked"] += 1
            continue
        year, evidence = earliest_year(results, args.suspect_from)
        if year is not None and row["current_year"] is None:
            row["proposed_year"] = year
            row["source"] = "discogs"
            row["evidence"] = evidence
            counts["discogs"] += 1
            continue
        # The earliest release Discogs knows can still be later than the tag,
        # when the original never reached Discogs and only a reissue did. The
        # earlier of the two is the better record either way, so the tag
        # stands and the row is left as it is.
        if year is not None and year >= row["current_year"]:
            row["proposed_year"] = row["current_year"]
            row["source"] = "confirmed"
            row["evidence"] = (
                evidence
                if year == row["current_year"]
                else f"the earliest Discogs knows is {year}, later than the year on the file"
            )
            counts["confirmed"] = counts.get("confirmed", 0) + 1
            continue
        if year:
            row["proposed_year"] = year
            row["source"] = "discogs"
            row["evidence"] = evidence
            counts["discogs"] += 1
        else:
            # Clearing the year is the answer, not a failure. Every release the
            # search found is archival or itself a reissue, so nothing states
            # when the music was made, and an empty year beats a wrong one.
            row["proposed_year"] = ""
            row["source"] = "cleared"
            row["evidence"] = evidence or f"{len(results)} releases found, none dated"
            counts["cleared"] += 1

    # A track its own release dates takes that year rather than being cleared.
    settled = agreed_by_release(rows)
    for row in rows:
        if row["source"] == "cleared" and row["folder"] in settled:
            year = settled[row["folder"]]
            row["proposed_year"] = year
            row["source"] = "release"
            row["evidence"] = f"the other tracks of this release are dated {year}"
            counts["cleared"] -= 1
            counts["release"] = counts.get("release", 0) + 1

    with open(args.out, "w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows({key: cell(value) for key, value in row.items()} for row in rows)
    print(
        f"{len(rows)} rows need a year"
        f"{' (including rows that have none)' if args.include_undated else ''}. "
        f"{counts['library']} take a year from elsewhere in the library, "
        f"{counts.get('remix_of', 0)} from the track they are a mix of, "
        f"{counts.get('export', 0)} from the collection export, "
        f"{counts.get('stated', 0)} from a year stated in a title, "
        f"{counts.get('release_lookup', 0)} from a catalog number lookup, "
        f"{counts['discogs']} take a different year from Discogs, "
        f"{counts.get('confirmed', 0)} are confirmed as they stand, "
        f"{counts.get('release', 0)} take the year their release settled on, "
        f"{counts['cleared']} get their year cleared, "
        f"{counts['unasked']} were not asked about."
    )
    print(f"wrote {args.out}")
    if stopped:
        print(f"Discogs rate-limited the run at {stopped}", file=sys.stderr)
        return 1
    return 0


def run_apply(args: argparse.Namespace) -> int:
    """Writes the proposed years into the library."""
    with open(args.proposed, newline="", encoding="utf-8") as handle:
        rows = [
            {key: uncell(value) for key, value in row.items()}
            for row in csv.DictReader(handle)
            if row["source"]
        ]
    connection = open_library(args.library, writable=True)
    try:
        dated = cleared = 0
        for row in rows:
            year = int(row["proposed_year"]) if row["proposed_year"] else None
            connection.execute(
                "UPDATE tracks SET year = ?, year_is_approximate = 0 WHERE hash = ?",
                (year, row["hash"]),
            )
            if year:
                dated += 1
            else:
                cleared += 1
        connection.commit()
    finally:
        connection.close()
    print(f"dated {dated} tracks and cleared the year of {cleared}")
    return 0


APPROXIMATE_FIELDS = [
    "folder",
    "undated",
    "release_label",
    "release_catalog_number",
    "same_release_dated",
    "same_release_span",
    "same_artist_dated",
    "same_artist_span",
    "proposed_year",
    "basis",
]

# A span wider than this says the release collects several eras, so its middle
# is no estimate for any one track on it.
WIDEST_USEFUL_SPAN = 4


# A note at the end of a title saying this is another cut of the same
# recording, as in `Sunshrine (Mix 2)` or `Let's Turn On (Desk Mix 1)`.
VERSION_NOTE = re.compile(
    r"\s*[\(\[][^()\[\]]*\b(?:mix|version|edit|dub)\b[^()\[\]]*[\)\]]\s*$",
    re.IGNORECASE,
)

# A note naming someone who reworked the track. A remix is its own piece of
# work and can come years after what it remixes: Prana's `Boundless` is from
# 1996 and the Funkygong remix of it is from 2015, so the one says nothing
# about the other.
REMIX_CREDIT = re.compile(r"[\(\[][^()\[\]]*\bre-?mix(?:es|ed)?\b[^()\[\]]*[\)\]]", re.IGNORECASE)


def base_title(title: str) -> str:
    """The title with a trailing note about which cut this is dropped.

    `Sunshrine (Mix 2)` gives `Sunshrine`. A title naming a remixer comes back
    as it was, because a remix is not another cut of the same recording. A
    title with no note comes back as it was too, which is how the caller knows
    there was nothing to strip.
    """
    if REMIX_CREDIT.search(title):
        return title
    stripped = title
    while True:
        shorter = VERSION_NOTE.sub("", stripped).strip()
        if shorter == stripped or not shorter:
            return stripped
        stripped = shorter


def better_than(found: int, current: int | None) -> bool:
    """Whether a year is worth writing over the one a row has.

    A row with no year takes any year at all. A row that has one keeps it
    unless the year found is earlier, since the later of two is the reissue.
    """
    return current is None or found < current


def spread(years: list[int]) -> str:
    """A run of years as `1996` or `1995-1998`."""
    if not years:
        return ""
    return str(years[0]) if years[0] == years[-1] else f"{years[0]}-{years[-1]}"


def middle(years: list[int]) -> int:
    """The middle year of a run, rounding down on an even count."""
    return sorted(years)[(len(years) - 1) // 2]


def run_approximate(args: argparse.Namespace) -> int:
    """Proposes an approximate year for each release that has undated tracks.

    Nothing is written to the library. The CSV is a list to read, and `apply`
    takes it once the years in it are the right ones.
    """
    connection = open_library(args.library, writable=False)
    try:
        rows = list(
            connection.execute(
                "SELECT path, artist, year, COALESCE(label,''), COALESCE(catalog_number,'') "
                "FROM tracks"
            )
        )
    finally:
        connection.close()

    by_folder: dict[str, list[tuple]] = {}
    by_artist: dict[str, list[int]] = {}
    for path, artist, year, label, catalog_number in rows:
        by_folder.setdefault(release_folder(path), []).append(
            (artist, year, label, catalog_number)
        )
        if artist and year:
            by_artist.setdefault(comparable(artist), []).append(year)

    proposals = []
    for folder, tracks in sorted(by_folder.items()):
        undated = [track for track in tracks if track[1] is None]
        if not undated:
            continue
        here = sorted(track[1] for track in tracks if track[1] is not None)
        artists = {comparable(track[0]) for track in undated if track[0]}
        elsewhere = sorted(
            year for artist in artists for year in by_artist.get(artist, [])
        )
        label = next((track[2] for track in tracks if track[2]), "")
        catalog_number = next((track[3] for track in tracks if track[3]), "")

        # The release's own dated tracks answer first, and only when they sit
        # close enough together to describe one era. The artist's other work
        # answers second, on the same condition.
        year, basis = None, "no dated track on this release or by these artists"
        if here and here[-1] - here[0] <= WIDEST_USEFUL_SPAN:
            year = middle(here)
            basis = f"{len(here)} dated tracks on this release span {spread(here)}"
        elif elsewhere and elsewhere[-1] - elsewhere[0] <= WIDEST_USEFUL_SPAN:
            year = middle(elsewhere)
            basis = f"{len(elsewhere)} dated tracks by these artists span {spread(elsewhere)}"
        elif here:
            basis = f"the release spans {spread(here)}, too wide to place one track in"
        elif elsewhere:
            basis = f"these artists span {spread(elsewhere)}, too wide to place one track in"

        proposals.append(
            {
                "folder": folder,
                "undated": len(undated),
                "release_label": label,
                "release_catalog_number": catalog_number,
                "same_release_dated": len(here),
                "same_release_span": spread(here),
                "same_artist_dated": len(elsewhere),
                "same_artist_span": spread(elsewhere),
                "proposed_year": year or "",
                "basis": basis,
            }
        )

    with open(args.out, "w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=APPROXIMATE_FIELDS)
        writer.writeheader()
        writer.writerows({key: cell(value) for key, value in row.items()} for row in proposals)
    proposed = [row for row in proposals if row["proposed_year"]]
    print(
        f"{len(proposals)} releases have undated tracks, "
        f"{sum(row['undated'] for row in proposals)} tracks in all. "
        f"{len(proposed)} releases get a proposed year, covering "
        f"{sum(row['undated'] for row in proposed)} tracks."
    )
    print(f"wrote {args.out}")
    return 0


def run_apply_approximate(args: argparse.Namespace) -> int:
    """Writes the approximate years from a reviewed proposal CSV.

    Only a row that names a year is written, and only over a track that has
    none, so a year already established is never overwritten by an estimate.
    Every year written here is marked approximate, which is what tells a
    later query that the track is placed in an era rather than dated.
    """
    with open(args.proposed, newline="", encoding="utf-8") as handle:
        rows = [
            {key: uncell(value) for key, value in row.items()}
            for row in csv.DictReader(handle)
            if row["proposed_year"].strip()
        ]
    connection = open_library(args.library, writable=True)
    try:
        written = 0
        for row in rows:
            year = int(row["proposed_year"])
            for (path,) in connection.execute(
                "SELECT path FROM tracks WHERE year IS NULL AND path LIKE ? ESCAPE '\\'",
                (escaped_like(row["folder"]) + "/%",),
            ).fetchall():
                if release_folder(path) != row["folder"]:
                    continue
                connection.execute(
                    "UPDATE tracks SET year = ?, year_is_approximate = 1 "
                    "WHERE path = ? AND year IS NULL",
                    (year, path),
                )
                written += 1
        connection.commit()
    finally:
        connection.close()
    print(f"dated {written} tracks that had no year, each marked approximate")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--library", default=LIBRARY, help="the library index")
    commands = parser.add_subparsers(dest="command", required=True)

    matcher = commands.add_parser("match", help="propose a year for every suspect row")
    matcher.add_argument("--out", required=True, help="the CSV to write")
    matcher.add_argument("--export", required=True, help="the Discogs collection export CSV")
    matcher.add_argument(
        "--suspect-from",
        type=int,
        default=SUSPECT_FROM,
        metavar="YEAR",
        help=f"treat a year at or after this one as a reissue date (default {SUSPECT_FROM})",
    )
    matcher.add_argument(
        "--api", action="store_true", help="ask the Discogs track search"
    )
    matcher.add_argument(
        "--include-undated",
        action="store_true",
        help="work on rows that have no year at all, as well as suspect ones",
    )
    matcher.add_argument(
        "--api-limit",
        type=int,
        default=None,
        metavar="N",
        help="ask the Discogs API at most N times, for a trial run",
    )
    matcher.set_defaults(run=run_match)

    applier = commands.add_parser("apply", help="write a match CSV into the library")
    applier.add_argument("proposed", help="the CSV `match` wrote")
    applier.set_defaults(run=run_apply)

    guesser = commands.add_parser(
        "approximate", help="propose a year for each release that has undated tracks"
    )
    guesser.add_argument("--out", required=True, help="the CSV to write")
    guesser.set_defaults(run=run_approximate)

    stamper = commands.add_parser(
        "apply-approximate", help="write a reviewed approximate-year CSV into the library"
    )
    stamper.add_argument("proposed", help="the CSV `approximate` wrote, once reviewed")
    stamper.set_defaults(run=run_apply_approximate)

    args = parser.parse_args()
    return args.run(args)


if __name__ == "__main__":
    sys.exit(main())
