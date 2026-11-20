#!/usr/bin/env python3
"""Changelog data for the desktop app's Changelog page, made from git.

One section per release tag (v*), newest first; the commits after the newest
tag form a section of their own with this build's version (PSHB_VERSION).
Each commit subject "Area: outcome; outcome" becomes one change with its
area, a group for the page's filter (app, board, other) and its outcomes
as items; the commit's text below the subject becomes its details,
paragraphs and "- " points with their lines joined (the page wraps them),
without trailers such as Co-Authored-By. Build, CI and test commits are
left out.

Usage: tools/changelog.py [-o FILE]   write the JSON (default: stdout)
       tools/changelog.py --check     generate and check it (make test)

Without git, tags or full history (shallow clone) it still writes valid
data: the newest commits as one section, or no sections at all.
"""
import argparse
import json
import os
import re
import subprocess
import sys

REPO = os.environ.get("GITHUB_REPOSITORY", "upchui/PS-HID-Bridge")
# Commit areas (or each part of "A, B and C") that are not about the app or
# the board: left out when every part is one of these.
HIDDEN = {"ci", "build", "tests", "test", "tools", "toolchain", "release", "firmware build", "latency_bench"}
BOARD_WORDS = ("board", "firmware", "web interface", "usb", "bootsel")
APP_WORDS = ("desktop", "app", "page", "cli", "shelly", "menu", "tray", "wizard")
FALLBACK_COUNT = 50
SEP_FIELD, SEP_RECORD = "\x1f", "\x1e"
# Lines at the end of a commit text that name people, not changes.
TRAILER = re.compile(r"^(co-authored-by|signed-off-by|reviewed-by|acked-by):", re.IGNORECASE)


def git(*args):
    out = subprocess.run(["git", *args], capture_output=True, text=True, check=True)
    return out.stdout


def parts_of(area):
    return [p for p in re.split(r",\s*|\s+and\s+", area.lower()) if p]


def group_of(area):
    parts = parts_of(area)
    if any(w in p for p in parts for w in BOARD_WORDS):
        return "board"
    if any(w in p for p in parts for w in APP_WORDS):
        return "app"
    return "other"


def hidden(area):
    parts = parts_of(area)
    return bool(parts) and all(p in HIDDEN for p in parts)


def details_of(body):
    """A commit text as blocks {"bullet", "text"}: a blank line ends a block,
    "- " (or "* ") at the start of a line begins a point, indented lines
    continue it; line breaks become spaces."""
    blocks = []
    cur = None
    for raw in body.splitlines():
        line = raw.rstrip()
        stripped = line.strip()
        if TRAILER.match(stripped):
            continue
        if not stripped:
            cur = None
            continue
        point = re.match(r"^[-*] +(.*)$", line)
        if point:
            cur = {"bullet": True, "text": point.group(1).strip()}
            blocks.append(cur)
        elif cur is None:
            cur = {"bullet": False, "text": stripped}
            blocks.append(cur)
        else:
            cur["text"] += " " + stripped
    return blocks


def change_of(short, date, subject, body=""):
    m = re.match(r"^([^:]{1,40}): (.+)$", subject)
    area, text = (m.group(1), m.group(2)) if m else ("", subject)
    if hidden(area):
        return None
    items = [t.strip() for t in text.split("; ") if t.strip()]
    items = [t[0].upper() + t[1:] for t in items]
    return {"area": area, "group": group_of(area) if area else "other", "items": items, "hash": short, "date": date,
            "details": details_of(body)}


def commits(rev_range, max_count=None):
    args = ["log", "--no-merges", f"--format=%h{SEP_FIELD}%cs{SEP_FIELD}%s{SEP_FIELD}%b{SEP_RECORD}"]
    if max_count:
        args.append(f"--max-count={max_count}")
    out = []
    for rec in git(*args, *rev_range).split(SEP_RECORD):
        rec = rec.strip("\n")
        if not rec:
            continue
        short, date, subject, body = rec.split(SEP_FIELD, 3)
        c = change_of(short, date, subject, body)
        if c:
            out.append(c)
    return out


def release_tags():
    """Release tags reachable from HEAD, oldest first: (name, commit date)."""
    names = git("tag", "--merged", "HEAD", "--list", "v*").split()
    tags = []
    for name in names:
        when, date = git("log", "-1", "--format=%ct %cs", name).split()
        tags.append((int(when), name, date))
    tags.sort()
    return [(name, date) for _, name, date in tags]


def build():
    version = os.environ.get("PSHB_VERSION", "")
    try:
        head = git("rev-parse", "--short", "HEAD").strip()
        shallow = git("rev-parse", "--is-shallow-repository").strip() == "true"
        tags = [] if shallow else release_tags()
    except (OSError, subprocess.CalledProcessError) as e:
        print(f"changelog: no git history ({e}): empty changelog", file=sys.stderr)
        return {"repo": REPO, "releases": []}

    releases = []
    prev = None
    for name, date in tags:
        rng = [f"{prev}..{name}"] if prev else [name]
        releases.append({
            "version": name[1:],
            "tag": name,
            "date": date,
            "build": False,
            "compare": f"{prev}...{name}" if prev else "",
            "changes": commits(rng),
        })
        prev = name
    if prev:
        after = commits([f"{prev}..HEAD"])
        if after:
            releases.append({
                "version": version or head,
                "tag": "",
                "date": after[0]["date"],
                "build": True,
                "compare": f"{prev}...{head}",
                "changes": after,
            })
    else:
        newest = commits(["HEAD"], FALLBACK_COUNT)
        if newest:
            releases.append({"version": version or head, "tag": "", "date": newest[0]["date"],
                             "build": True, "compare": "", "changes": newest})
    releases = [r for r in releases if r["changes"]]
    releases.reverse()
    return {"repo": REPO, "releases": releases}


# A commit text and what the page gets from it (--check).
SAMPLE_BODY = """Why it happened, over
two lines.

- First point,
  continued here.
- Second point.
Tests: the cases.

Co-Authored-By: Someone <someone@example.com>
"""
SAMPLE_DETAILS = [
    {"bullet": False, "text": "Why it happened, over two lines."},
    {"bullet": True, "text": "First point, continued here."},
    {"bullet": True, "text": "Second point. Tests: the cases."},
]


def check(data):
    errors = []
    if details_of(SAMPLE_BODY) != SAMPLE_DETAILS:
        errors.append(f"commit text split wrongly: {details_of(SAMPLE_BODY)}")
    if not data["releases"]:
        errors.append("no releases")
    for r in data["releases"]:
        if not re.fullmatch(r"\d+\.\d+\.\d+(-[\w.]+)?|[0-9a-f]{7,}", r["version"]):
            errors.append(f"odd version {r['version']!r}")
        for c in r["changes"]:
            if hidden(c["area"]):
                errors.append(f"{r['version']}: hidden area {c['area']!r}")
            if not c["items"] or c["group"] not in ("app", "board", "other"):
                errors.append(f"{r['version']}: bad change {c}")
            for d in c["details"]:
                if not isinstance(d.get("bullet"), bool) or not d.get("text") or "\n" in d["text"]:
                    errors.append(f"{r['version']} {c['hash']}: bad details block {d}")
                elif TRAILER.match(d["text"]) or "Co-Authored-By" in d["text"]:
                    errors.append(f"{r['version']} {c['hash']}: a trailer in the details")
    versions = [r["version"] for r in data["releases"]]
    if len(set(versions)) != len(versions):
        errors.append("versions twice")
    return errors


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("-o", "--output")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    data = build()
    if args.check:
        # Round trip through JSON: what the app reads.
        data = json.loads(json.dumps(data))
        errors = check(data)
        for e in errors:
            print(f"changelog: {e}", file=sys.stderr)
        n = sum(len(r["changes"]) for r in data["releases"])
        detailed = sum(1 for r in data["releases"] for c in r["changes"] if c["details"])
        print(f"changelog: {len(data['releases'])} versions, {n} changes ({detailed} with details)"
              + (" -- FAILED" if errors else ""))
        sys.exit(1 if errors else 0)
    text = json.dumps(data, ensure_ascii=False, separators=(",", ":"))
    if args.output:
        os.makedirs(os.path.dirname(os.path.abspath(args.output)), exist_ok=True)
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(text)
    else:
        print(text)


if __name__ == "__main__":
    main()
