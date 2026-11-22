#!/usr/bin/env python3
"""Changelog data for the desktop app's Changelog page, made from git.

One section per release tag (v*), newest first; the commits after the newest
tag form a section of their own with this build's version (PSHB_VERSION).
Each commit subject "Area: outcome; outcome" becomes one change with its
area, a group for the page's filter (app, board, other) and its outcomes
as items; the commit's text below the subject becomes its details,
paragraphs and "- " points with their lines joined (the page wraps them),
without trailers such as Co-Authored-By. Build, CI and test commits are
left out; a version with nothing else keeps its section, without changes.

Also the project's contributors for the About page: every commit author and
Co-Authored-By, each once, in the order they first took part, by their
GitHub name where their address tells it (else by their name in git), with
what PEOPLE adds. Never an e-mail address; AI assistants and bots are left
out.

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
# What git does not tell about a contributor, by their name in git: the
# GitHub account and further names (shown after the GitHub name).
PEOPLE = {
    "theALHU": {"github": "upchui", "aka": ["Alexander Huthansl"]},
}
NOT_PEOPLE = re.compile(r"^claude\b|\[bot\]$|^dependabot\b|^github-actions\b", re.IGNORECASE)
NOT_PEOPLE_MAIL = re.compile(r"@anthropic\.com$", re.IGNORECASE)
# GitHub's private addresses name the account: 123+name@users.noreply.github.com.
GITHUB_MAIL = re.compile(r"^(?:\d+\+)?([a-z0-9](?:[a-z0-9-]*[a-z0-9])?)@users\.noreply\.github\.com$", re.IGNORECASE)
NAME_MAIL = re.compile(r"^\s*(.*?)\s*<([^>]*)>\s*$")


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


def count(rev_range):
    """Commits in the range, also those left out of the changelog."""
    return int(git("rev-list", "--no-merges", "--count", *rev_range).strip())


def person(name, mail):
    """A contributor {"name", "aka", "github"} from a name and address in git,
    or None for an assistant or a bot."""
    name, mail = name.strip(), mail.strip()
    if not name or NOT_PEOPLE.search(name) or NOT_PEOPLE_MAIL.search(mail):
        return None
    m = GITHUB_MAIL.match(mail)
    rule = PEOPLE.get(name, {})
    github = rule.get("github") or (m.group(1) if m else "")
    aka = [name] if github and github.lower() != name.lower() else []
    aka += [a for a in rule.get("aka", []) if a not in aka]
    return {"name": github or name, "aka": aka, "github": github}


def merge_people(found):
    """One entry per person, in the order given: the same name (GitHub or git,
    any case) means the same person; a GitHub name wins over a git name."""
    people = []
    for p in found:
        names = {n.lower() for n in [p["name"], *p["aka"]]}
        same = next((q for q in people if names & {n.lower() for n in [q["name"], *q["aka"]]}), None)
        if same is None:
            people.append(p)
            continue
        if p["github"] and not same["github"]:
            same["aka"] = [same["name"]] + same["aka"]
            same["name"] = same["github"] = p["github"]
        for n in [p["name"], *p["aka"]]:
            if n.lower() != same["name"].lower() and n.lower() not in {a.lower() for a in same["aka"]}:
                same["aka"].append(n)
    return people


def contributors():
    """Everyone who took part, oldest first (see the module text)."""
    out = git("log", "--reverse", f"--format=%an{SEP_FIELD}%ae{SEP_FIELD}"
              f"%(trailers:key=Co-authored-by,valueonly,separator=%x1d){SEP_RECORD}", "HEAD")
    found = []
    for rec in out.split(SEP_RECORD):
        rec = rec.strip("\n")
        if not rec:
            continue
        name, mail, co = rec.split(SEP_FIELD, 2)
        found.append(person(name, mail))
        for t in co.split("\x1d"):
            m = NAME_MAIL.match(t)
            if m:
                found.append(person(m.group(1), m.group(2)))
    return merge_people([p for p in found if p])


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
        return {"repo": REPO, "releases": [], "contributors": []}

    # A version whose commits are all left out (build, tests, the version
    # number) keeps its section, with no changes: the page shows it, so
    # it can be installed and this build finds itself (1.1.0 was missing).
    releases = []
    prev = None
    for name, date in tags:
        rng = [f"{prev}..{name}"] if prev else [name]
        if not count(rng):
            continue  # a second tag on the same commit
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
        rng = [f"{prev}..HEAD"]
        if count(rng):
            releases.append({
                "version": version or head,
                "tag": "",
                "date": git("log", "-1", "--format=%cs", "HEAD").strip(),
                "build": True,
                "compare": f"{prev}...{head}",
                "changes": commits(rng),
            })
    else:
        newest = commits(["HEAD"], FALLBACK_COUNT)
        if newest:
            releases.append({"version": version or head, "tag": "", "date": newest[0]["date"],
                             "build": True, "compare": "", "changes": newest})
    releases.reverse()
    return {"repo": REPO, "releases": releases, "contributors": contributors()}


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
# Names and addresses in git and the contributor they make (--check).
SAMPLE_PEOPLE = [
    (("Max Mustermann", "12345+maxm@users.noreply.github.com"),
     {"name": "maxm", "aka": ["Max Mustermann"], "github": "maxm"}),
    (("jan", "jan@example.com"), {"name": "jan", "aka": [], "github": ""}),
    (("theALHU", "someone@example.com"),
     {"name": "upchui", "aka": ["theALHU", "Alexander Huthansl"], "github": "upchui"}),
    (("Claude Opus 5.5", "noreply@anthropic.com"), None),
    (("dependabot[bot]", "49699333+dependabot[bot]@users.noreply.github.com"), None),
]


def check(data):
    errors = []
    if details_of(SAMPLE_BODY) != SAMPLE_DETAILS:
        errors.append(f"commit text split wrongly: {details_of(SAMPLE_BODY)}")
    for (name, mail), want in SAMPLE_PEOPLE:
        if person(name, mail) != want:
            errors.append(f"contributor from {name!r}: {person(name, mail)}, not {want}")
    merged = merge_people([person("Max Mustermann", "max@example.com"),
                           person("Max Mustermann", "1+maxm@users.noreply.github.com")])
    if merged != [{"name": "maxm", "aka": ["Max Mustermann"], "github": "maxm"}]:
        errors.append(f"one person twice or merged wrongly: {merged}")
    people = data.get("contributors", [])
    if data["releases"] and not people:
        errors.append("no contributors")
    for p in people:
        texts = [p["name"], p["github"], *p["aka"]]
        if any("@" in t for t in texts) or NOT_PEOPLE.search(p["name"]):
            errors.append(f"contributor {p}: an address, an assistant or a bot")
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
                # A trailer line itself (a text may well mention one).
                elif TRAILER.match(d["text"]):
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
        print(f"changelog: {len(data['releases'])} versions, {n} changes ({detailed} with details), "
              f"{len(data['contributors'])} contributors"
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
