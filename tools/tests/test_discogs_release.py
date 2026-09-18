"""Acceptance tests for tools/discogs_release.py. A coder makes these pass without editing them."""

import csv
import pathlib
import re
import sqlite3
import subprocess
import sys
import tempfile
import unittest

TOOL = pathlib.Path(__file__).resolve().parents[1] / "discogs_release.py"
CRATE_INDEX = TOOL.parents[1] / "crates" / "library" / "src" / "index.rs"

sys.path.insert(0, str(TOOL.parent))
import discogs_release  # noqa: E402


EXPORT_HEADER = [
    "Catalog#",
    "Artist",
    "Title",
    "Label",
    "Format",
    "Rating",
    "Released",
    "release_id",
    "CollectionFolder",
    "Date Added",
]


def export_row(catalog_number: str, artist: str, title: str, label: str, release_id: str = "1") -> list[str]:
    return [catalog_number, artist, title, label, "CD", "", "1996", release_id, "cd", ""]


def write_export(folder: pathlib.Path, rows: list[list[str]]) -> pathlib.Path:
    path = folder / "collection.csv"
    with open(path, "w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(EXPORT_HEADER)
        writer.writerows(rows)
    return path


def write_library(folder: pathlib.Path, paths: list[str]) -> pathlib.Path:
    """A library file of the version this tool writes, with one row per path
    and only the columns the tool reads and writes."""
    path = folder / "library.sqlite"
    connection = sqlite3.connect(path)
    connection.execute(
        "CREATE TABLE tracks (hash TEXT PRIMARY KEY, path TEXT NOT NULL, "
        "label TEXT, catalog_number TEXT, release_title TEXT, "
        "track_number INTEGER, release_data_source TEXT)"
    )
    for number, file_path in enumerate(paths):
        connection.execute(
            "INSERT INTO tracks (hash, path) VALUES (?, ?)", (f"{number:064d}", file_path)
        )
    connection.execute(f"PRAGMA user_version = {discogs_release.SCHEMA_VERSION}")
    connection.commit()
    connection.close()
    return path


def run(*arguments: str) -> tuple[int, str, str]:
    finished = subprocess.run(
        [sys.executable, str(TOOL), *arguments], capture_output=True, text=True
    )
    return finished.returncode, finished.stdout, finished.stderr


class ReadingAFolderName(unittest.TestCase):
    def test_a_catalog_number_compares_without_its_spacing(self):
        self.assertEqual(discogs_release.normalized("TO3 CD 002"), "TO3CD002")
        self.assertEqual(discogs_release.normalized("to3-cd002"), "TO3CD002")

    def test_a_release_folder_is_found_past_a_disc_folder(self):
        self.assertEqual(
            discogs_release.release_folder("/goa/BPC - Blueprint [DATCD002]/CD2/01 a.mp3"),
            "/goa/BPC - Blueprint [DATCD002]",
        )
        self.assertEqual(
            discogs_release.release_folder("/goa/Etnica - Alien Protein/02 b.mp3"),
            "/goa/Etnica - Alien Protein",
        )

    def test_a_bracketed_catalog_number_is_read_and_its_absence_reported(self):
        self.assertEqual(
            discogs_release.folder_catalog_number("/goa/Doof - Let's Turn On [TIPCD10]"),
            "TIPCD10",
        )
        self.assertIsNone(discogs_release.folder_catalog_number("/goa/Alphanaut"))

    def test_a_folder_name_gives_its_artist_and_release_title(self):
        self.assertEqual(
            discogs_release.folder_artist_and_title("/goa/Doof - Let's Turn On [TIPCD10]"),
            ("Doof", "Let's Turn On"),
        )
        self.assertEqual(
            discogs_release.folder_artist_and_title("/goa/Alphanaut"), (None, "Alphanaut")
        )

    def test_a_track_number_comes_from_the_front_of_the_file_name(self):
        self.assertEqual(discogs_release.track_number("/goa/x/01 Etnica - Alien.mp3"), 1)
        self.assertEqual(discogs_release.track_number("/goa/x/12. Etnica - Alien.mp3"), 12)
        # A three-digit number is a disc number and a track number.
        self.assertEqual(discogs_release.track_number("/goa/x/203 - Etnica - Alien.mp3"), 3)
        self.assertIsNone(discogs_release.track_number("/goa/x/Etnica - Alien.mp3"))


class ComparingTwoCatalogNumbers(unittest.TestCase):
    """A folder writes a catalog number the way its owner files it and Discogs
    writes it the way the label printed it."""

    def test_the_one_release_under_two_spellings(self):
        for mine, theirs in [
            ("SZ051", "SPIRIT ZONE 051"),
            ("TRANR604CD", "TRANRCD604"),
            ("BMPHQCD01", "bmphqcd001"),
            ("PHNKL2049-2", "2049-2"),
            ("MPCD01", "MPCD1"),
            ("PSY-35", "PSY-035"),
            ("KRBCD519765", "519765"),
        ]:
            self.assertTrue(
                discogs_release.names_the_same_release(mine, theirs), f"{mine} and {theirs}"
            )

    def test_a_cd_folder_does_not_take_an_lp_catalog_number(self):
        for mine, theirs in [
            ("TIPCD10", "TIP LP 10"),
            ("BR058CD", "BR058LP"),
            ("BFLCD23", "BFLLP23"),
            ("AFRCD01", "AFR LP 1"),
            ("HADSHCD01", "LP001"),
        ]:
            self.assertFalse(
                discogs_release.names_the_same_release(mine, theirs), f"{mine} and {theirs}"
            )

    def test_numbers_that_disagree_are_two_releases(self):
        self.assertFalse(discogs_release.names_the_same_release("PHNKL2080-2", "BALLLP01"))
        self.assertFalse(discogs_release.names_the_same_release("HMCD01", "BALLLP003"))

    def test_a_catalog_number_with_no_digits_matches_nothing(self):
        self.assertFalse(discogs_release.names_the_same_release("ANJUNA", "ANJUNACD001"))
        self.assertFalse(discogs_release.names_the_same_release("unk", "CLP 0014-2"))


class Matching(unittest.TestCase):
    def test_a_catalog_number_in_the_folder_name_matches_the_export(self):
        with tempfile.TemporaryDirectory() as folder:
            here = pathlib.Path(folder)
            export = write_export(
                here, [export_row("TIP CD 10", "Doof", "Let's Turn On", "TIP Records")]
            )
            library = write_library(here, ["/goa/Doof - Let's Turn On [TIPCD10]/01 Doof - Destination Bom.mp3"])
            out = here / "proposed.csv"
            code, output, _ = run(
                "--library", str(library), "match", "--export", str(export), "--out", str(out)
            )
            self.assertEqual(code, 0, output)
            with open(out, encoding="utf-8") as handle:
                rows = list(csv.DictReader(handle))
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]["data_source"], "discogs_export")
            self.assertEqual(rows[0]["label"], "TIP Records")
            self.assertEqual(rows[0]["release_title"], "Let's Turn On")

    def test_a_folder_with_no_catalog_number_matches_on_its_title(self):
        with tempfile.TemporaryDirectory() as folder:
            here = pathlib.Path(folder)
            export = write_export(
                here, [export_row("AFR010", "Technossomy", "Synthetic Flesh", "Flying Rhino")]
            )
            library = write_library(here, ["/goa/Technossomy - Synthetic Flesh/01 a.mp3"])
            out = here / "proposed.csv"
            code, output, _ = run(
                "--library", str(library), "match", "--export", str(export), "--out", str(out)
            )
            self.assertEqual(code, 0, output)
            with open(out, encoding="utf-8") as handle:
                rows = list(csv.DictReader(handle))
            self.assertEqual(rows[0]["data_source"], "discogs_export")
            self.assertEqual(rows[0]["catalog_number"], "AFR010")

    def test_a_title_match_needs_the_artist_to_agree(self):
        with tempfile.TemporaryDirectory() as folder:
            here = pathlib.Path(folder)
            export = write_export(
                here, [export_row("AFR010", "Technossomy", "Synthetic Flesh", "Flying Rhino")]
            )
            library = write_library(here, ["/goa/Some Other Act - Synthetic Flesh/01 a.mp3"])
            out = here / "proposed.csv"
            code, _, _ = run(
                "--library", str(library), "match", "--export", str(export), "--out", str(out)
            )
            self.assertEqual(code, 0)
            with open(out, encoding="utf-8") as handle:
                rows = list(csv.DictReader(handle))
            self.assertEqual(rows[0]["data_source"], "")

    def test_a_folder_takes_the_one_release_under_another_spelling(self):
        with tempfile.TemporaryDirectory() as folder:
            here = pathlib.Path(folder)
            export = write_export(
                here, [export_row("SPIRIT ZONE 051", "Etnica", "Equator", "Spirit Zone")]
            )
            library = write_library(here, ["/goa/Etnica - Equator [SZ051]/01 a.mp3"])
            out = here / "proposed.csv"
            code, _, _ = run(
                "--library", str(library), "match", "--export", str(export), "--out", str(out)
            )
            self.assertEqual(code, 0)
            with open(out, encoding="utf-8") as handle:
                rows = list(csv.DictReader(handle))
            self.assertEqual(rows[0]["data_source"], "discogs_export")
            self.assertEqual(rows[0]["catalog_number"], "SPIRIT ZONE 051")

    def test_a_cd_folder_is_left_for_the_api_when_only_the_lp_is_owned(self):
        """A title match would answer with the LP pressing, so the folder goes
        to the API to search for its own catalog number instead."""
        with tempfile.TemporaryDirectory() as folder:
            here = pathlib.Path(folder)
            export = write_export(
                here, [export_row("TIP LP 10", "Doof", "Let's Turn On", "TIP Records")]
            )
            library = write_library(here, ["/goa/Doof - Let's Turn On [TIPCD10]/01 a.mp3"])
            out = here / "proposed.csv"
            code, _, _ = run(
                "--library", str(library), "match", "--export", str(export), "--out", str(out)
            )
            self.assertEqual(code, 0)
            with open(out, encoding="utf-8") as handle:
                rows = list(csv.DictReader(handle))
            self.assertEqual(rows[0]["data_source"], "")
            self.assertIn("export lists no release", rows[0]["note"])

    def test_an_empty_export_resolves_nothing_and_names_no_network(self):
        """Without `--api` the match reads only the export, so an export that
        lists nothing leaves every folder unresolved."""
        with tempfile.TemporaryDirectory() as folder:
            here = pathlib.Path(folder)
            export = write_export(here, [])
            library = write_library(here, ["/goa/Doof - Let's Turn On [TIPCD10]/01 a.mp3"])
            out = here / "proposed.csv"
            code, output, _ = run(
                "--library", str(library), "match", "--export", str(export), "--out", str(out)
            )
            self.assertEqual(code, 0, output)
            self.assertIn("0 from the API", output)
            with open(out, encoding="utf-8") as handle:
                rows = list(csv.DictReader(handle))
            self.assertEqual(rows[0]["data_source"], "")


def search_result(catno: str, title: str, year: str, label: str, identifier: int) -> dict:
    return {"catno": catno, "title": title, "year": year, "label": [label], "id": identifier}


class ChoosingFromASearch(unittest.TestCase):
    """Discogs matches a catalog number loosely, so a search for `EOR003`
    answers with six releases by four acts. These are the answers it gave."""

    EOR003 = [
        search_result("EOR003", "Brass (8) - Look On The Bright Side", "2022", "Eor", 1),
        search_result("EOR003", "Lorrenzzeti - Don't Stop", "2016", "X", 2),
        search_result("EOR-003", "Hiroshi Hasegawa - Split", "2016", "Y", 3),
        search_result("EOR 003", "The Unknown Controllers - Ftumchk / Venus Tongue", "1998", "Elektrik Orgasm Records", 4),
        search_result("EOR 003", "The Unknown Controllers - Ftumchk / Venus Tongue", "1999", "Elektrik Orgasm Records", 5),
        search_result("EOR 003", "Demoniac - Prepare For War", "1994", "Z", 6),
    ]

    def test_the_act_the_folder_names_settles_it(self):
        chosen = discogs_release.from_search(self.EOR003, "EOR003", "The Unknown Controllers")
        self.assertEqual(chosen["label"], "Elektrik Orgasm Records")
        self.assertEqual(chosen["title"], "Ftumchk / Venus Tongue")
        # Two pressings of the one release, and the earlier one is the answer.
        self.assertEqual(chosen["release_id"], "4")

    def test_results_by_several_acts_and_no_act_named_stay_ambiguous(self):
        self.assertIsNone(discogs_release.from_search(self.EOR003, "EOR003", None))

    def test_a_catalog_number_that_only_appears_inside_another_is_no_match(self):
        """A search for `SZ051` answers with `KSZ 051` and `SZ0511-96`, and
        neither is that catalog number."""
        results = [
            search_result("KSZ 051", "Mother Rat - Just Beginning", "2024", "A", 7),
            search_result("SZ0511-96", "Various - Something", "1996", "B", 8),
        ]
        self.assertIsNone(discogs_release.from_search(results, "SZ051", "Etnica"))

    def test_a_number_after_an_act_name_is_not_part_of_the_name(self):
        results = [search_result("EOR003", "Brass (8) - Look On The Bright Side", "2022", "Eor", 1)]
        chosen = discogs_release.from_search(results, "EOR003", "Brass")
        self.assertEqual(chosen["title"], "Look On The Bright Side")

    def test_a_compilation_folder_matches_a_various_artists_release(self):
        results = [search_result("TIP CD 1", "Various - Order Odonata", "1994", "TIP", 9)]
        chosen = discogs_release.from_search(results, "TIPCD1", "VA")
        self.assertEqual(chosen["title"], "Order Odonata")


class Applying(unittest.TestCase):
    def test_applying_writes_every_track_under_a_matched_folder(self):
        with tempfile.TemporaryDirectory() as folder:
            here = pathlib.Path(folder)
            export = write_export(
                here, [export_row("DATCD002", "Blue Planet Corporation", "A Blueprint For Survival", "DAT Records")]
            )
            paths = [
                "/goa/BPC - A Blueprint [DATCD002]/CD1/01 BPC - Intro.mp3",
                "/goa/BPC - A Blueprint [DATCD002]/CD1/02 BPC - Midian.mp3",
                "/goa/BPC - A Blueprint [DATCD002]/CD2/01 BPC - Crystal.mp3",
                "/goa/Unmatched Act - Nothing/01 a.mp3",
            ]
            library = write_library(here, paths)
            out = here / "proposed.csv"
            run("--library", str(library), "match", "--export", str(export), "--out", str(out))
            code, output, _ = run("--library", str(library), "apply", str(out))
            self.assertEqual(code, 0, output)

            connection = sqlite3.connect(library)
            stored = dict(
                (path, (label, catalog_number, title, number, source))
                for path, label, catalog_number, title, number, source in connection.execute(
                    "SELECT path, label, catalog_number, release_title, track_number, "
                    "release_data_source FROM tracks"
                )
            )
            connection.close()
            self.assertEqual(
                stored[paths[0]],
                ("DAT Records", "DATCD002", "A Blueprint For Survival", 1, "discogs_export"),
            )
            self.assertEqual(stored[paths[1]][3], 2)
            # A release over two discs numbers each disc from one, so the
            # track number is the position on the disc.
            self.assertEqual(stored[paths[2]][3], 1)
            # A folder the match could not resolve keeps empty release columns.
            self.assertEqual(stored[paths[3]], (None, None, None, None, None))

    def test_a_library_of_another_version_is_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            here = pathlib.Path(folder)
            library = write_library(here, ["/goa/a/01 a.mp3"])
            connection = sqlite3.connect(library)
            connection.execute("PRAGMA user_version = 3")
            connection.commit()
            connection.close()
            export = write_export(here, [])
            code, _, errors = run(
                "--library", str(library), "match", "--export", str(export), "--out", str(here / "p.csv")
            )
            self.assertEqual(code, 1)
            self.assertIn("version 3 library file", errors)


class LayoutVersion(unittest.TestCase):
    def test_the_tools_write_the_layout_version_the_crate_writes(self):
        # discogs_year.py imports the same constant, so one check covers both.
        text = CRATE_INDEX.read_text(encoding="utf-8")
        found = re.search(r"pub const SCHEMA_VERSION: u32 = (\d+);", text)
        self.assertIsNotNone(
            found, "crates/library/src/index.rs does not state SCHEMA_VERSION as this test reads it"
        )
        self.assertEqual(int(found.group(1)), discogs_release.SCHEMA_VERSION)


if __name__ == "__main__":
    unittest.main()
