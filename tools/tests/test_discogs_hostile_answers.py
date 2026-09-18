"""Acceptance tests for what the two Discogs tools do with an answer made to break them. A coder makes these pass without editing them.

Both tools build their request address from a module constant named `API`,
which these tests point at a server on this machine.
"""

import csv
import http.server
import pathlib
import sys
import tempfile
import threading
import unittest
import urllib.error

TOOLS = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
import discogs_release  # noqa: E402
import discogs_year  # noqa: E402


class Recorder(http.server.BaseHTTPRequestHandler):
    """Answers every request with what its server was told to, and records the headers it was sent."""

    def do_GET(self):  # noqa: N802
        self.server.seen.append(dict(self.headers))
        if self.server.redirect_to:
            self.send_response(302)
            self.send_header("Location", self.server.redirect_to + self.path)
            self.end_headers()
            return
        body = self.server.body
        self.send_response(200)
        for name, value in self.server.extra_headers.items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *arguments):
        pass


def serve(host: str, redirect_to: str = "", body: bytes = b'{"results": []}', extra_headers=None):
    server = http.server.ThreadingHTTPServer((host, 0), Recorder)
    server.seen = []
    server.redirect_to = redirect_to
    server.body = body
    server.extra_headers = extra_headers or {}
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class TheCredentialsStayWithTheHostTheyWereMeantFor(unittest.TestCase):
    def check(self, module, call):
        elsewhere = serve("127.0.0.1")
        first = serve("127.0.0.1", redirect_to=f"http://localhost:{elsewhere.server_port}")
        original = module.API
        module.API = f"http://127.0.0.1:{first.server_port}"
        try:
            try:
                call()
            except (urllib.error.URLError, ValueError):
                pass
        finally:
            module.API = original
            first.shutdown()
            elsewhere.shutdown()
        self.assertEqual(len(first.seen), 1)
        self.assertIn("Authorization", first.seen[0])
        for headers in elsewhere.seen:
            self.assertNotIn("Authorization", headers)

    def test_the_release_tool_sends_no_credentials_to_a_redirected_host(self):
        self.check(discogs_release, lambda: discogs_release.search_releases("TIPCD01", ("key", "secret")))

    def test_the_year_tool_sends_no_credentials_to_a_redirected_host(self):
        self.check(discogs_year, lambda: discogs_year.search_tracks("Hallucinogen", "LSD", ("key", "secret")))


class AnAnswerOfTheWrongFormIsNotACrash(unittest.TestCase):
    def test_a_rate_limit_header_that_is_not_a_number_does_not_raise_value_error(self):
        for module, call in (
            (discogs_release, lambda: discogs_release.search_releases("TIPCD01", ("key", "secret"))),
            (discogs_year, lambda: discogs_year.search_tracks("Hallucinogen", "LSD", ("key", "secret"))),
        ):
            server = serve("127.0.0.1", extra_headers={"X-Discogs-Ratelimit-Remaining": "plenty"})
            original = module.API
            module.API = f"http://127.0.0.1:{server.server_port}"
            try:
                self.assertEqual(call(), [])
            finally:
                module.API = original
                server.shutdown()

    def test_results_that_are_not_a_list_of_objects_give_no_results(self):
        for body in (b'{"results": "none"}', b'{"results": [1, "two", null]}', b'[]', b'"text"'):
            server = serve("127.0.0.1", body=body)
            original = discogs_release.API
            discogs_release.API = f"http://127.0.0.1:{server.server_port}"
            try:
                found = discogs_release.search_releases("TIPCD01", ("key", "secret"))
                self.assertEqual(discogs_release.from_search(found, "TIPCD01", None), None, body)
            finally:
                discogs_release.API = original
                server.shutdown()

    def test_a_year_that_is_not_a_number_and_a_label_that_is_text_settle_on_a_release(self):
        results = [
            {"catno": "TIPCD01", "title": "Hallucinogen - Twisted", "year": "MCMXCVI", "label": "Dragonfly Records", "id": 7},
            {"catno": "TIPCD01", "title": "Hallucinogen - Twisted", "year": "1995", "label": ["Dragonfly Records"], "id": 8},
        ]
        release = discogs_release.from_search(results, "TIPCD01", "Hallucinogen")
        self.assertEqual(release["release_id"], "8")
        self.assertEqual(release["label"], "Dragonfly Records")

        only_text = discogs_release.from_search(results[:1], "TIPCD01", "Hallucinogen")
        # A label that is text is the whole label or no label, never its first character.
        self.assertIn(only_text["label"], ("Dragonfly Records", ""))

    def test_fields_of_the_wrong_type_give_no_release_and_no_exception(self):
        for result in (
            {"catno": 5, "title": "A - B", "year": 1995},
            {"catno": "TIPCD01", "title": None, "year": 1995},
            {"catno": "TIPCD01", "title": ["A - B"], "year": [1995], "label": {"name": "x"}},
        ):
            discogs_release.from_search([result], "TIPCD01", None)
        self.assertEqual(discogs_year.earliest_year([{"year": "MCMXCVI", "title": "A - B"}], 2005)[0], None)


class ATitleCannotBecomeASpreadsheetFormula(unittest.TestCase):
    def test_a_cell_that_begins_with_a_formula_character_is_written_as_text(self):
        self.assertTrue(hasattr(discogs_release, "cell"), "discogs_release.cell(value) is the contract")
        for value in ("=HYPERLINK(\"http://x\")", "+1", "-1", "@SUM(A1)", "\t=1", "\r=1"):
            safe = discogs_release.cell(value)
            self.assertTrue(safe.startswith("'"), value)
            self.assertEqual(safe[1:], value)
        for value in ("Twisted", "", "1995", "Dragonfly Records"):
            self.assertEqual(discogs_release.cell(value), value)
        self.assertEqual(discogs_release.cell(12), 12)


if __name__ == "__main__":
    unittest.main()
