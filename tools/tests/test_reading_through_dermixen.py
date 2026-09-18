"""The profiling programs read audio through `dermixen decode`, not ffmpeg.

`track_profile.py` and `grid_check.py` find the `dermixen` executable by the
`DERMIXEN` environment variable, then on the path, then in the repository's
build folders, `target/release` and `target/debug`, and when it is nowhere
they say to run `cargo build --release -p dermixen-cli`. A `DERMIXEN` that
names a file which does not exist is an error naming the variable and the
file, not a reason to look elsewhere. Each program decodes one span with
`dermixen decode FILE OUT --from START --for LENGTH` and reads the WAV that
command writes. These tests run the programs with a stand-in for the
executable and with a stand-in for ffmpeg, so they need neither the real
command nor ffmpeg, except the last, which needs the release build.
"""

import os
import pathlib
import stat
import subprocess
import sys
import tempfile
import unittest
import wave

import numpy as np

TOOLS = pathlib.Path(__file__).resolve().parents[1]
REPOSITORY = TOOLS.parent
RELEASE = REPOSITORY / "target" / "release" / "dermixen"
SAMPLE_RATE = 44100


def noise_file(folder: pathlib.Path, seconds: float = 3.0) -> pathlib.Path:
    """Three seconds of quiet noise as a 16-bit stereo WAV, which any command reads."""
    count = int(SAMPLE_RATE * seconds)
    samples = np.random.default_rng(3).uniform(-0.1, 0.1, (count, 2))
    pcm = np.round(samples * 32767).astype("<i2")
    path = folder / "noise.wav"
    with wave.open(str(path), "wb") as out:
        out.setnchannels(2)
        out.setsampwidth(2)
        out.setframerate(SAMPLE_RATE)
        out.writeframes(pcm.tobytes())
    return path


def stand_in(folder: pathlib.Path, name: str, log: pathlib.Path) -> pathlib.Path:
    """An executable called `name` that appends its arguments to `log`, one per line, and exits 1."""
    path = folder / name
    path.write_text(
        "#!/bin/sh\n"
        f'for argument in "$@"; do printf "%s\\n" "$argument" >> "{log}"; done\n'
        f'echo "error: {name} stand-in refused" >&2\n'
        "exit 1\n"
    )
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def run(program: str, arguments: list[str], env: dict[str, str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(TOOLS / program), *arguments],
        capture_output=True,
        text=True,
        env=env,
    )


class ReadingThroughDermixen(unittest.TestCase):
    def test_a_named_executable_that_does_not_exist_is_an_error_naming_it(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = pathlib.Path(temp)
            track = noise_file(folder)
            env = dict(os.environ, DERMIXEN=str(folder / "nowhere" / "dermixen"))
            done = run("track_profile.py", ["seconds", str(track), "--window", "1"], env)
            self.assertNotEqual(done.returncode, 0, done.stdout)
            self.assertIn("DERMIXEN", done.stderr)
            self.assertIn(str(folder / "nowhere" / "dermixen"), done.stderr)

    def test_the_executable_named_by_dermixen_decodes_the_span(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = pathlib.Path(temp)
            track = noise_file(folder)
            log = folder / "calls.txt"
            fake = stand_in(folder, "dermixen", log)
            env = dict(os.environ, DERMIXEN=str(fake))
            done = run("track_profile.py", ["seconds", str(track), "--from", "1", "--to", "3"], env)
            # The stand-in refused, so the program failed and said why, in the
            # stand-in's words.
            self.assertNotEqual(done.returncode, 0, done.stdout)
            self.assertIn("cannot decode", done.stderr)
            self.assertIn("stand-in refused", done.stderr)
            calls = log.read_text().splitlines()
            self.assertEqual(calls[0], "decode", calls)
            self.assertEqual(calls[1], str(track), calls)
            self.assertTrue(calls[2].endswith(".wav"), calls)
            self.assertIn("--from", calls)
            self.assertIn("--for", calls)
            self.assertAlmostEqual(float(calls[calls.index("--from") + 1]), 1.0, places=3)
            self.assertAlmostEqual(float(calls[calls.index("--for") + 1]), 2.0, places=3)

            # grid_check.py reads the same way.
            log.unlink()
            done = run(
                "grid_check.py",
                [str(track), "--bpm", "140", "--first-beat", "0", "--from", "8", "--to", "16"],
                env,
            )
            self.assertNotEqual(done.returncode, 0, done.stdout)
            calls = log.read_text().splitlines()
            self.assertEqual(calls[0], "decode", calls)
            self.assertIn("--from", calls)
            self.assertIn("--for", calls)

    def test_a_named_executable_that_cannot_be_run_is_an_error_naming_it(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = pathlib.Path(temp)
            track = noise_file(folder)
            not_a_program = folder / "dermixen"
            not_a_program.write_text("not a program\n")
            env = dict(os.environ, DERMIXEN=str(not_a_program))
            done = run("track_profile.py", ["seconds", str(track), "--window", "1"], env)
            self.assertNotEqual(done.returncode, 0, done.stdout)
            self.assertIn("DERMIXEN", done.stderr)
            self.assertIn(str(not_a_program), done.stderr)
            self.assertNotIn("Traceback", done.stderr)

    @unittest.skipUnless(RELEASE.exists(), "build the command first: cargo build --release -p dermixen-cli")
    def test_a_beat_before_the_start_of_the_file_is_refused(self):
        # The search around a beat may reach before the file's first sample,
        # and that stretch is silence. A beat that itself sits before the
        # first sample is not in the file, so a report about it would be a
        # report about nothing, and the programs refuse it by name.
        with tempfile.TemporaryDirectory() as temp:
            folder = pathlib.Path(temp)
            track = noise_file(folder)
            env = dict(os.environ, DERMIXEN=str(RELEASE))
            grid = [str(track), "--bpm", "140", "--first-beat", "0.1"]
            for program, arguments in [
                ("grid_check.py", [*grid, "--from", "-4", "--to", "8"]),
                ("track_profile.py", ["beats", *grid, "--from", "-4", "--to", "8"]),
                ("track_profile.py", ["sections", *grid, "--from", "-4", "--to", "8"]),
            ]:
                done = run(program, arguments, env)
                self.assertNotEqual(done.returncode, 0, f"{program}: {done.stdout}")
                self.assertIn("beat -4", done.stderr, program)
                self.assertIn("before the start of the file", done.stderr, program)
            # Beat 0 is in the file, at a tenth of a second, and the search
            # before it is padded with silence rather than refused.
            done = run("track_profile.py", ["beats", *grid, "--from", "0", "--to", "8"], env)
            self.assertEqual(done.returncode, 0, done.stderr)
            done = run("track_profile.py", ["seconds", str(track), "--from", "-1", "--to", "2"], env)
            self.assertNotEqual(done.returncode, 0, done.stdout)
            self.assertIn("--from", done.stderr)

    @unittest.skipUnless(RELEASE.exists(), "build the command first: cargo build --release -p dermixen-cli")
    def test_ffmpeg_is_not_run_when_the_command_is_present(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = pathlib.Path(temp)
            track = noise_file(folder)
            log = folder / "ffmpeg-calls.txt"
            (folder / "bin").mkdir()
            stand_in(folder / "bin", "ffmpeg", log)
            env = dict(os.environ, DERMIXEN=str(RELEASE), PATH=str(folder / "bin"))
            done = run("track_profile.py", ["seconds", str(track), "--window", "1"], env)
            self.assertEqual(done.returncode, 0, done.stderr)
            self.assertGreaterEqual(len(done.stdout.strip().splitlines()), 3, done.stdout)
            self.assertFalse(log.exists(), "the program ran ffmpeg")


if __name__ == "__main__":
    unittest.main()
