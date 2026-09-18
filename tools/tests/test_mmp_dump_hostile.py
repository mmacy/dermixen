"""Acceptance tests for tools/mmp_dump.py on playlists made to break it. A coder makes these pass without editing them.

The five playlists under `tools/tests/mmp/` are a few bytes each, except
`deep-nesting.mmp`, which nests `LIST` chunks 5,000 deep.
"""

import pathlib
import subprocess
import sys
import unittest

TOOL = pathlib.Path(__file__).resolve().parents[1] / "mmp_dump.py"
CASES = pathlib.Path(__file__).resolve().parent / "mmp"


def run(*arguments: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(TOOL), *arguments],
        capture_output=True,
        timeout=30,
        check=False,
    )


class ADamagedPlaylistIsReportedInOneLine(unittest.TestCase):
    def test_each_damaged_playlist_ends_in_status_one_and_no_traceback(self):
        for name in (
            "deep-nesting.mmp",
            "huge-mmvr.mmp",
            "short-trkh.mmp",
            "truncated-marker.mmp",
        ):
            for arguments in ((str(CASES / name),), (str(CASES / name), "--json")):
                with self.subTest(name=name, arguments=arguments[1:]):
                    done = run(*arguments)
                    self.assertEqual(done.returncode, 1, done.stderr)
                    self.assertNotIn(b"Traceback", done.stderr)
                    self.assertIn(name.encode(), done.stderr)
                    self.assertEqual(done.stdout, b"")

    def test_a_file_too_large_to_be_a_playlist_is_refused_before_it_is_read(self):
        # A sparse file costs no disk. The largest real playlist is under 1 MB.
        import tempfile

        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "huge.mmp"
            with open(path, "wb") as handle:
                handle.write(b"RIFF")
                handle.truncate(200 * 1024 * 1024)
            done = run(str(path))
            self.assertEqual(done.returncode, 1, done.stderr)
            self.assertNotIn(b"Traceback", done.stderr)
            self.assertIn(b"huge.mmp", done.stderr)


class TextFromAPlaylistCannotDriveTheTerminal(unittest.TestCase):
    def test_a_control_character_in_a_path_is_printed_in_a_visible_form(self):
        done = run(str(CASES / "escape-path.mmp"))
        self.assertEqual(done.returncode, 0, done.stderr)
        for byte in (0x1B, 0x07, 0x9B):
            self.assertNotIn(bytes([byte]), done.stdout)
        self.assertIn(b"\\x1b", done.stdout)

    def test_json_output_is_unchanged_because_json_escapes_the_character(self):
        import json

        done = run(str(CASES / "escape-path.mmp"), "--json")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertNotIn(b"\x1b", done.stdout)
        self.assertIn("\x1b", json.loads(done.stdout)["tracks"][0]["path"])


if __name__ == "__main__":
    unittest.main()
