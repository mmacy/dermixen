"""Acceptance tests for tools/grid_check.py. A coder makes these pass without editing them."""

import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from synth import track, write_wav  # noqa: E402

TOOL = pathlib.Path(__file__).resolve().parents[1] / "grid_check.py"
BPM = 140.0
HALF_BEAT = 0.5 * 60.0 / BPM


def run(*args) -> tuple[int, str]:
    done = subprocess.run([sys.executable, str(TOOL), *map(str, args)], capture_output=True, text=True)
    return done.returncode, done.stdout + done.stderr


class GridCheck(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.dir = pathlib.Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def file(self, name: str, **kwargs) -> pathlib.Path:
        path = self.dir / name
        write_wav(path, track(BPM, 70.0, 0.5, **kwargs))
        return path

    def test_a_grid_on_the_kick_reads_a_small_offset_and_no_drift(self):
        code, out = run(self.file("kick.wav"), "--bpm", BPM, "--first-beat", 0.5, "--from", 0, "--to", 128)
        self.assertEqual(code, 0, out)
        starts = re.search(r"kick starts ([+-]\d+) ms", out)
        self.assertIsNotNone(starts, out)
        self.assertLessEqual(abs(int(starts.group(1))), 40, out)
        drift = re.search(r"drift ([+-][\d.]+) ms per 1000 beats", out)
        self.assertLessEqual(abs(float(drift.group(1))), 10.0, out)
        fitted = re.search(r"tempo fits ([\d.]+)", out)
        self.assertAlmostEqual(float(fitted.group(1)), BPM, delta=0.02, msg=out)
        self.assertIn("one kick onset per beat", out)

    def test_a_grid_on_the_bass_note_reads_half_a_beat_and_says_which_first_beat_fixes_it(self):
        code, out = run(self.file("kick.wav"), "--bpm", BPM, "--first-beat", 0.5 + HALF_BEAT, "--from", 0, "--to", 128)
        self.assertEqual(code, 0, out)
        starts = re.search(r"kick starts ([+-]\d+) ms", out)
        self.assertIsNotNone(starts, out)
        self.assertAlmostEqual(abs(int(starts.group(1))), HALF_BEAT * 1000.0, delta=30, msg=out)
        fix = re.search(r"--first-beat ([\d.]+)", out)
        self.assertIsNotNone(fix, out)
        self.assertAlmostEqual(float(fix.group(1)), 0.5, delta=0.04, msg=out)

    def test_a_stretch_with_no_kick_does_not_bend_the_fit(self):
        # The kick stops for sixty-four beats while the bass keeps going, which
        # is a breakdown. The checks in it are off the line, and the rest are
        # not pulled off it.
        path = self.file("gap.wav", kick_beats=lambda beat: not 64 <= beat < 128)
        code, out = run(path, "--bpm", BPM, "--first-beat", 0.5, "--from", 0, "--to", 160)
        self.assertEqual(code, 0, out)
        drift = re.search(r"drift ([+-][\d.]+) ms per 1000 beats", out)
        self.assertLessEqual(abs(float(drift.group(1))), 10.0, out)
        fitted = re.search(r"tempo fits ([\d.]+)", out)
        self.assertAlmostEqual(float(fitted.group(1)), BPM, delta=0.02, msg=out)
        on_line = re.search(r"(\d+) of (\d+) checks within 20 ms", out)
        self.assertGreaterEqual(int(on_line.group(1)), int(on_line.group(2)) * 0.5, out)
        starts = re.search(r"kick starts ([+-]\d+) ms", out)
        self.assertLessEqual(abs(int(starts.group(1))), 40, out)

    def test_two_kicks_half_a_beat_apart_are_both_reported(self):
        # A rendered handover whose two tracks' grids are half a beat apart
        # has two kicks per beat. The report says so and how far apart.
        one = track(BPM, 70.0, 0.5)
        other = track(BPM, 70.0, 0.5 + HALF_BEAT, bass_beats=lambda beat: False)
        path = self.dir / "clip.wav"
        write_wav(path, (one + other) * 0.6)
        code, out = run(path, "--bpm", BPM, "--first-beat", 0.5, "--from", 0, "--to", 128)
        self.assertEqual(code, 0, out)
        two = re.search(r"two kick onsets per beat: ([+-]\d+) ms and ([+-]\d+) ms", out)
        self.assertIsNotNone(two, out)
        apart = abs(int(two.group(1)) - int(two.group(2)))
        self.assertAlmostEqual(apart, HALF_BEAT * 1000.0, delta=40, msg=out)

    def test_a_tempo_off_by_two_percent_is_still_measured(self):
        # The offsets walk more than a beat across the stretch when the tempo
        # given is two percent off, and the report still gives the true tempo.
        path = self.file("kick.wav")
        code, out = run(path, "--bpm", BPM * 0.98, "--first-beat", 0.5, "--from", 0, "--to", 160)
        self.assertEqual(code, 0, out)
        fitted = re.search(r"tempo fits ([\d.]+)", out)
        self.assertAlmostEqual(float(fitted.group(1)), BPM, delta=0.05, msg=out)

    def test_every_check_is_counted_and_a_stretch_that_opens_without_a_kick_is_found(self):
        # The first thirty-two beats have no kick at all, so the first checks
        # find nothing. Every check the program makes is in the count, and
        # once the kick arrives the checks sit on the line.
        path = self.file("late.wav", kick_beats=lambda beat: beat >= 32, bass_beats=lambda beat: beat >= 32)
        code, out = run(path, "--bpm", BPM, "--first-beat", 0.5, "--from", 0, "--to", 160)
        self.assertEqual(code, 0, out)
        on_line = re.search(r"(\d+) of (\d+) checks within 20 ms", out)
        self.assertEqual(int(on_line.group(2)), 20, out)
        self.assertGreaterEqual(int(on_line.group(1)), 14, out)
        starts = re.search(r"kick starts ([+-]\d+) ms", out)
        self.assertLessEqual(abs(int(starts.group(1))), 40, out)

    def test_the_suggested_first_beat_is_never_before_the_file_starts(self):
        # The kick sits further before the grid beat than the grid's first
        # beat sits after the start of the file, so the naive answer would be
        # negative. The suggestion is a whole number of beats later instead.
        path = self.dir / "early.wav"
        write_wav(path, track(BPM, 70.0, 0.349))
        code, out = run(path, "--bpm", BPM, "--first-beat", 0.100, "--from", 8, "--to", 160)
        self.assertEqual(code, 0, out)
        fix = re.search(r"--first-beat (-?[\d.]+)", out)
        self.assertIsNotNone(fix, out)
        suggested = float(fix.group(1))
        self.assertGreaterEqual(suggested, 0.0, out)
        beat = 60.0 / BPM
        wanted = 0.349 - 0.020
        self.assertAlmostEqual(((suggested - wanted) + beat / 2) % beat - beat / 2, 0.0, delta=0.03, msg=out)


if __name__ == "__main__":
    unittest.main()
