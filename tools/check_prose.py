"""Score a Markdown document against a set of writing fingerprints.

Prints a table of fingerprints for one or more files, with the range each
one is expected to sit in next to it: em dashes, semicolons, sentence
length, contractions, marketing words, title-case headings, and openers
that describe the document. The ranges describe plain developer
documentation. A document a model wrote tends to miss several of them at
once. The score is a diagnostic, not a target: fixing the prose fixes the
numbers, never the other way around. `CLAUDE.md` states the rules the
numbers stand for.

Usage:

    python3 tools/check_prose.py README.md [more.md ...]
    cat README.md | python3 tools/check_prose.py -

Exit status is 0 either way; the table is the output.
"""

import re
import statistics
import sys

# (label, the typical value, "lo-hi" band considered in range)
BANDS = {
    "em_dash_per_1k":       (0.0, (0.0, 0.5)),
    "en_dash_per_1k":       (0.0, (0.0, 0.5)),
    "spaced_hyphen_per_1k": (2.0, (0.0, 15.0)),
    "sent_median":          (20, (12, 26)),
    "sent_p90":             (33, (20, 42)),
    "contractions_per_1k":  (15.0, (3.0, 30.0)),
    "semicolon_per_1k":     (0.1, (0.0, 2.0)),
    "parens_per_1k":        (10.0, (2.0, 40.0)),
    "excl_per_1k":          (0.0, (0.0, 1.0)),
    "you_per_1k":           (18.0, (3.0, 45.0)),
    "marketing_per_1k":     (0.0, (0.0, 0.5)),
    "title_case_headings":  (0, (0, 0)),
    "bold_bullet_dash":     (0, (0, 0)),
    "emoji":                (0, (0, 0)),
    "self_describing":      (0, (0, 0)),
}

MARKETING = re.compile(
    r"\b(seamless(ly)?|robust|powerful|intuitive|elegant|effortless(ly)?|blazing|"
    r"delightful|beautiful(ly)?|leverage[sd]?|utiliz\w+|delve[sd]?|streamlin\w+|"
    r"empower\w*|unlock\w*|supercharge\w*|game.?chang\w+|cutting.edge|"
    r"state.of.the.art|best.in.class|world.class|first.class|battle.tested|"
    r"production.ready|hassle.free|out of the box|under the hood|behind the scenes|"
    r"crucial(ly)?|vital|pivotal|paramount|comprehensive|holistic|"
    r"journey|landscape|ecosystem|paradigm|synerg\w+)\b", re.I)
CONTRACTION = re.compile(r"\b\w+'(s|t|re|ve|ll|d|m)\b")
SELF_DESCRIBING = re.compile(
    r"^\s*(>\s*)?(\*\*)?(This|The following|In this|Throughout this)\s+"
    r"(article|document|doc|page|guide|readme|section|tutorial|quickstart|how-to|"
    r"walkthrough|overview|reference|chapter|post|write-?up|note)s?\b", re.I | re.M)
EMOJI = re.compile(r"[\U0001F300-\U0001FAFF\u2600-\u27BF]")


def strip_code(text):
    text = re.sub(r"```.*?```", "", text, flags=re.S)
    text = re.sub(r"`[^`\n]*`", "x", text)
    text = re.sub(r"!\[[^\]]*\]\([^)]*\)", "", text)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    return text


def measure(raw):
    headings = re.findall(r"^#{1,6}\s+(.+)$", raw, flags=re.M)
    title_case = 0
    stop = {"a", "an", "the", "of", "in", "on", "to", "for", "and", "or", "with",
            "it", "is", "by", "as", "at", "vs"}
    for h in headings:
        h = re.sub(r"`[^`]*`", "", h)
        words = [w for w in h.split()
                 if re.match(r"^[A-Za-z][A-Za-z'-]*[,:!?]?$", w)     # plain words only
                 and w.lower().strip(",:!?") not in stop
                 and not re.search(r"[a-z][A-Z]", w)                  # not CamelCase
                 and not w.strip(",:!?").isupper()]                    # not an acronym
        if len(words) >= 2 and all(w[0].isupper() for w in words):
            title_case += 1
    # A bold-led bullet punctuated with an em or en dash. The bullet is not the
    # defect and is never counted on its own: the dash after the bold term is.
    # `**Term** - definition` and `**Term**: definition` are the author's own shapes.
    bold_lead = len(re.findall(r"^\s*[-*+]\s+\*\*[^*]+\*\*\s*[—–]", raw, flags=re.M))
    emoji = len(EMOJI.findall(raw))
    self_describing = len(SELF_DESCRIBING.findall(strip_code(raw)))

    text = strip_code(raw)
    prose = "\n".join(
        l for l in text.split("\n")
        if l.strip() and not l.lstrip().startswith(("#", "|", ">", "<", "---"))
    )
    words = prose.split()
    nw = max(len(words), 1)
    sents = [s for s in re.split(r"(?<=[.!?])\s+", prose) if len(s.split()) >= 3]
    sl = sorted(len(s.split()) for s in sents) or [0]
    per = lambda n: n / nw * 1000
    c = lambda pat: len(re.findall(pat, prose))
    return {
        "words": nw,
        "em_dash_per_1k": per(c("—")),
        "en_dash_per_1k": per(c("–")),
        "spaced_hyphen_per_1k": per(c(" - ")),
        "sent_median": statistics.median(sl),
        "sent_p90": sl[int(len(sl) * 0.9)] if sl else 0,
        "contractions_per_1k": per(len(CONTRACTION.findall(prose))),
        "semicolon_per_1k": per(c(";")),
        "parens_per_1k": per(c(r"\(")),
        "excl_per_1k": per(c("!")),
        "you_per_1k": per(c(r"\b[Yy]ou\b")),
        "marketing_per_1k": per(len(MARKETING.findall(prose))),
        "title_case_headings": title_case,
        "bold_bullet_dash": bold_lead,
        "emoji": emoji,
        "self_describing": self_describing,
    }


def report(name, m):
    print("\n%s  (%d prose words)" % (name, m["words"]))
    print("  %-22s %8s %9s %10s" % ("fingerprint", "doc", "author", ""))
    misses = 0
    for key, (typical, (lo, hi)) in BANDS.items():
        v = m[key]
        ok = lo <= v <= hi
        misses += not ok
        print("  %-22s %8.1f %9.1f %10s" % (key, v, typical, "" if ok else "<- out of range"))
    print("  %d of %d fingerprints out of range" % (misses, len(BANDS)))
    return misses


if __name__ == "__main__":
    paths = sys.argv[1:] or ["-"]
    for p in paths:
        text = sys.stdin.read() if p == "-" else open(p, errors="replace").read()
        report(p, measure(text))
        found = sorted(set(m.group(0).lower() for m in MARKETING.finditer(strip_code(text))))
        if found:
            print("  marketing words:", ", ".join(found))
