"""Acceptance tests for the `sections` command of tools/track_profile.py. A coder makes these pass without editing them."""

import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from synth import track, write_wav  # noqa: E402

TOOL = pathlib.Path(__file__).resolve().parents[1] / "track_profile.py"
BPM = 140.0
LINE = re.compile(r"beat\s+(\d+)\s+(kick leaves|kick returns|level ([+-][\d.]+) dB)\s+\((\d+) mod 32\)")


def run(*args) -> tuple[int, str]:
    done = subprocess.run([sys.executable, str(TOOL), *map(str, args)], capture_output=True, text=True)
    return done.returncode, done.stdout + done.stderr


class Sections(unittest.TestCase):
    def test_sections_lists_where_the_kick_leaves_and_returns_and_where_the_level_steps(self):
        # The kick stops at beat 128 and returns at 160 while the bass keeps
        # going, and everything from beat 224 on is twice as loud.
        with tempfile.TemporaryDirectory() as temp:
            path = pathlib.Path(temp) / "sections.wav"
            write_wav(
                path,
                track(
                    BPM,
                    120.0,
                    0.5,
                    kick_beats=lambda beat: not 128 <= beat < 160,
                    gain=lambda beat: 2.0 if beat >= 224 else 1.0,
                ),
            )
            code, out = run("sections", path, "--bpm", BPM, "--first-beat", 0.5, "--from", 0, "--to", 256)
        self.assertEqual(code, 0, out)
        found = [(int(m.group(1)), m.group(2), m.group(3), int(m.group(4))) for m in LINE.finditer(out)]
        self.assertTrue(found, out)

        def near(beat: int, kind: str):
            return [f for f in found if abs(f[0] - beat) <= 1 and f[1].startswith(kind)]

        self.assertTrue(near(128, "kick leaves"), out)
        self.assertTrue(near(160, "kick returns"), out)
        steps = near(224, "level")
        self.assertTrue(steps, out)
        self.assertAlmostEqual(float(steps[0][2]), 6.0, delta=1.5, msg=out)
        for beat, kind, _, residue in found:
            if abs(beat - 128) <= 1 or abs(beat - 160) <= 1 or abs(beat - 224) <= 1:
                self.assertEqual(residue, beat % 32, out)
        # Nothing changes in the body, so no line names a beat in it.
        self.assertFalse([f for f in found if 40 <= f[0] <= 120], out)

    def test_a_stray_kick_inside_a_break_is_not_the_return(self):
        # One kick at beat 140, in the middle of a break that runs from 128 to
        # 160, is a fill. The kick returns at 160, when it stays.
        with tempfile.TemporaryDirectory() as temp:
            path = pathlib.Path(temp) / "stray.wav"
            write_wav(
                path,
                track(BPM, 120.0, 0.5, kick_beats=lambda beat: beat == 140 or not 128 <= beat < 160),
            )
            code, out = run("sections", path, "--bpm", BPM, "--first-beat", 0.5, "--from", 0, "--to", 256)
        self.assertEqual(code, 0, out)
        found = [(int(m.group(1)), m.group(2)) for m in LINE.finditer(out)]
        returns = [beat for beat, kind in found if kind == "kick returns"]
        leaves = [beat for beat, kind in found if kind == "kick leaves"]
        self.assertEqual(len(returns), 1, out)
        self.assertLessEqual(abs(returns[0] - 160), 1, out)
        self.assertEqual(len(leaves), 1, out)
        self.assertLessEqual(abs(leaves[0] - 128), 1, out)


if __name__ == "__main__":
    unittest.main()
