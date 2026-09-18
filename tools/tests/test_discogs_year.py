"""Acceptance tests for tools/discogs_year.py. A coder makes these pass without editing them."""

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import discogs_year  # noqa: E402


def result(year, title, label=None):
    return {"year": year, "title": title, "label": [label] if label else []}


class JudgingALabel(unittest.TestCase):
    def test_a_label_that_presses_older_material_is_archival(self):
        for label in ["DAT Records", "DAT Mafia Recordings", "memo604", "Digital Reprints"]:
            self.assertTrue(discogs_year.is_archival(label), label)

    def test_a_label_that_also_releases_new_music_is_not(self):
        """Suntrip, Zion 604 and Anjuna press reissues and new music both, so
        the label alone cannot date a release."""
        for label in ["Suntrip Records", "Zion 604", "Anjuna Records", "Flying Rhino Records"]:
            self.assertFalse(discogs_year.is_archival(label), label)

    def test_self_released_is_not_archival(self):
        self.assertFalse(discogs_year.is_archival("Not On Label (Self-released)"))


class JudgingATitle(unittest.TestCase):
    def test_a_title_that_says_it_collects_older_music(self):
        for title in [
            "Digitalis - The Early Years 1995-2000",
            "Nervasystem - Early Daze Compilation",
            "Prana - Single Collection 2",
            "VA - Mind Rewind",
            "Doof - Let's Turn On - Remixed & Remastered",
            "Denshi Danshi - Spirit Of The 90s",
        ]:
            self.assertTrue(discogs_year.is_retrospective(title), title)

    def test_an_ordinary_release_title(self):
        for title in [
            "Battle Of The Future Buddhas - Digging Mud",
            "Etnica - Tribute / Intense Visitation",
            "Hallucinogen - The Lone Deranger",
        ]:
            self.assertFalse(discogs_year.is_retrospective(title), title)


class ReadingAYearFromATitle(unittest.TestCase):
    def test_a_release_that_names_its_recording_year(self):
        self.assertEqual(discogs_year.stated_year("Etnica - Live In Athens 1996"), 1996)
        self.assertEqual(discogs_year.stated_year("Astral Projection (Summer 1996 Live Mix)"), 1996)

    def test_a_span_of_years_states_no_year(self):
        self.assertIsNone(discogs_year.stated_year("Digitalis - The Early Years 1995-2000"))

    def test_a_title_with_no_year(self):
        self.assertIsNone(discogs_year.stated_year("Hallucinogen - The Lone Deranger"))

    def test_a_number_that_is_not_a_year(self):
        self.assertIsNone(discogs_year.stated_year("Track 604 / 12345"))


class ChoosingAYear(unittest.TestCase):
    def test_the_earliest_release_dates_the_music(self):
        year, evidence = discogs_year.earliest_year(
            [
                result("2018", "Etnica - The Juggeling Alchemists", "DAT Records"),
                result("1995", "Etnica - Tribute / Intense Visitation", "Blue Room Released"),
                result("2024", "Etnica - Reissue", "Suntrip Records"),
            ],
            2005,
        )
        self.assertEqual(year, 1995)
        self.assertIn("Blue Room Released", evidence)

    def test_a_recent_year_is_taken_when_the_release_is_the_music_s_own(self):
        """A self-released 2011 album is from 2011, not a reissue of anything."""
        year, _ = discogs_year.earliest_year(
            [result("2011", "Battle Of The Future Buddhas - Digging Mud", "Not On Label")], 2005
        )
        self.assertEqual(year, 2011)

    def test_an_archival_pressing_is_no_evidence_of_a_year(self):
        year, evidence = discogs_year.earliest_year(
            [result("2013", "Crop Circles - Full Mental Jackpot EP", "DAT Records")], 2005
        )
        self.assertIsNone(year)
        self.assertIn("archival", evidence)

    def test_one_archival_label_among_several_is_enough(self):
        """Discogs lists the recording studio and the artist's own imprint
        alongside the label, and only the label says what the release is."""
        year, _ = discogs_year.earliest_year(
            [
                {
                    "year": "2013",
                    "title": "Crop Circles - Full Mental Jackpot EP",
                    "label": ["DAT Records", "Soundbusters Recording Studios", "Etnicanet"],
                }
            ],
            2005,
        )
        self.assertIsNone(year)

    def test_an_archival_pressing_that_names_its_recording_year(self):
        """`Live In Athens 1996` is the recording, whoever pressed it later."""
        year, evidence = discogs_year.earliest_year(
            [result("2012", "Etnica - Live In Athens 1996", "DAT Records")], 2005
        )
        self.assertEqual(year, 1996)
        self.assertIn("states the year", evidence)

    def test_a_retrospective_collection_is_no_evidence_of_a_year(self):
        year, evidence = discogs_year.earliest_year(
            [result("2013", "Nervasystem - Early Daze Compilation", "Not On Label")], 2005
        )
        self.assertIsNone(year)
        self.assertIn("retrospective", evidence)

    def test_a_year_before_the_music_existed_is_a_different_recording(self):
        year, _ = discogs_year.earliest_year(
            [
                result("1974", "Some Other Act - Same Name", "Old Label"),
                result("1996", "Etnica - Alien Protein", "Blue Room Released"),
            ],
            2005,
        )
        self.assertEqual(year, 1996)

    def test_nothing_dated_gives_no_year(self):
        year, evidence = discogs_year.earliest_year([], 2005)
        self.assertIsNone(year)
        self.assertIn("no dated release", evidence)


class SettlingARelease(unittest.TestCase):
    def test_a_release_whose_tracks_agree_dates_the_rest_of_it(self):
        rows = [
            {"folder": "/a", "source": "confirmed", "proposed_year": 2011},
            {"folder": "/a", "source": "confirmed", "proposed_year": 2011},
            {"folder": "/a", "source": "cleared", "proposed_year": ""},
        ]
        self.assertEqual(discogs_year.agreed_by_release(rows), {"/a": 2011})

    def test_one_dated_track_does_not_settle_a_release(self):
        rows = [
            {"folder": "/a", "source": "confirmed", "proposed_year": 2011},
            {"folder": "/a", "source": "cleared", "proposed_year": ""},
        ]
        self.assertEqual(discogs_year.agreed_by_release(rows), {})

    def test_a_compilation_of_several_years_settles_on_none(self):
        rows = [
            {"folder": "/a", "source": "discogs", "proposed_year": 1995},
            {"folder": "/a", "source": "discogs", "proposed_year": 1997},
            {"folder": "/a", "source": "cleared", "proposed_year": ""},
        ]
        self.assertEqual(discogs_year.agreed_by_release(rows), {})


class ReadingAVersionNote(unittest.TestCase):
    def test_another_cut_of_the_same_recording_gives_its_base_title(self):
        for title, base in [
            ("Sunshrine (Mix 2)", "Sunshrine"),
            ("Let's Turn On (Desk Mix 1)", "Let's Turn On"),
            ("Angelina (Alternate Mix)", "Angelina"),
            ("Hypnotized (Original Version)", "Hypnotized"),
            ("The Tale Of Taketori (604 Edit)", "The Tale Of Taketori"),
            ("Ganymede (Guitar Mix)", "Ganymede"),
        ]:
            self.assertEqual(discogs_year.base_title(title), base)

    def test_a_remix_is_its_own_work_and_keeps_its_title(self):
        """Prana's `Boundless` is from 1996 and the Funkygong remix of it is
        from 2015, so the one says nothing about the other."""
        for title in [
            "Boundless (Funkygong Remix)",
            "Alien Pets (Filteria Remix)",
            "Time Dilation (Etnica Remix)",
            "Life Is A Gas (X-Dream remix)",
        ]:
            self.assertEqual(discogs_year.base_title(title), title)

    def test_a_title_with_no_note_is_unchanged(self):
        self.assertEqual(discogs_year.base_title("Stratosfear"), "Stratosfear")

    def test_a_title_that_is_only_a_note_is_left_alone(self):
        self.assertEqual(discogs_year.base_title("(Live Mix)"), "(Live Mix)")


class DecidingWhetherAYearIsWorthWriting(unittest.TestCase):
    def test_a_row_with_no_year_takes_any_year(self):
        self.assertTrue(discogs_year.better_than(2011, None))
        self.assertTrue(discogs_year.better_than(1996, None))

    def test_a_row_that_has_a_year_keeps_it_unless_the_new_one_is_earlier(self):
        self.assertTrue(discogs_year.better_than(1996, 2013))
        self.assertFalse(discogs_year.better_than(2013, 1996))
        self.assertFalse(discogs_year.better_than(1996, 1996))


class DescribingARunOfYears(unittest.TestCase):
    def test_a_span_reads_as_one_year_or_a_range(self):
        self.assertEqual(discogs_year.spread([1996]), "1996")
        self.assertEqual(discogs_year.spread([1995, 1996, 1998]), "1995-1998")
        self.assertEqual(discogs_year.spread([]), "")

    def test_the_middle_year_rounds_down_on_an_even_count(self):
        self.assertEqual(discogs_year.middle([1995, 1996, 1998]), 1996)
        self.assertEqual(discogs_year.middle([1996, 1998]), 1996)
        self.assertEqual(discogs_year.middle([1997]), 1997)

    def test_the_widest_span_worth_estimating_from(self):
        """A release spanning more than four years collects several eras, so
        its middle places no single track on it."""
        self.assertEqual(discogs_year.WIDEST_USEFUL_SPAN, 4)


if __name__ == "__main__":
    unittest.main()
