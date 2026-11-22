#!/usr/bin/env python3
"""Checks the wiki sources: relative links and anchors, images, SUMMARY
coverage, ru/en parity and the page template.

Usage: python3 docs/wiki/tools/check_links.py
Exit code 1 and one line per problem when something is wrong.
"""
import re
import sys
from pathlib import Path

WIKI = Path(__file__).resolve().parents[1]
LANGS = ("ru", "en")
SEE_ALSO = {"ru": "См. также", "en": "See also"}
# Pages that do not need a "See also" section.
NO_SEE_ALSO = {"SUMMARY.md"}

LINK = re.compile(r"(?<!!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
IMG = re.compile(r"!\[[^\]]*\]\(([^)\s]+)")
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$", re.M)


def slug(title: str) -> str:
    """mdBook's heading id: lowercase, keep word chars, `-` and `_`, spaces to `-`."""
    out = []
    for ch in title.strip().lower():
        if ch.isalnum() or ch in "-_":
            out.append(ch)
        elif ch.isspace():
            out.append("-")
    return "".join(out)


def anchors(path: Path, cache: dict) -> set:
    if path not in cache:
        text = re.sub(r"```.*?```", "", path.read_text(), flags=re.S)
        seen, ids = {}, set()
        for _, title in HEADING.findall(text):
            title = re.sub(r"`|\*|\[([^\]]*)\]\([^)]*\)", lambda m: m.group(1) or "", title)
            s = slug(title)
            n = seen.get(s, 0)
            seen[s] = n + 1
            ids.add(s if n == 0 else f"{s}-{n}")
        cache[path] = ids
    return cache[path]


def summary_pages(src: Path) -> list:
    text = (src / "SUMMARY.md").read_text()
    return [m for m in re.findall(r"\]\(([^)]+\.md)\)", text)]


def main() -> int:
    problems = []
    cache: dict = {}
    page_sets = {}
    for lang in LANGS:
        src = WIKI / lang / "src"
        listed = set(summary_pages(src))
        on_disk = {str(p.relative_to(src)) for p in src.rglob("*.md") if p.name != "SUMMARY.md"}
        for missing in sorted(on_disk - listed):
            problems.append(f"{lang}: {missing} is not in SUMMARY.md")
        for ghost in sorted(listed - on_disk):
            problems.append(f"{lang}: SUMMARY.md lists {ghost}, which does not exist")
        page_sets[lang] = on_disk
        for page in sorted(src.rglob("*.md")):
            rel = page.relative_to(WIKI)
            text = page.read_text()
            body = re.sub(r"```.*?```", "", text, flags=re.S)
            body = re.sub(r"`[^`\n]*`", "", body)
            if page.name not in NO_SEE_ALSO and not re.search(
                rf"^##\s+{SEE_ALSO[lang]}\s*$", text, re.M
            ):
                problems.append(f"{rel}: no '## {SEE_ALSO[lang]}' section")
            if not text.lstrip().startswith("# "):
                problems.append(f"{rel}: must start with a '# Title' heading")
            for target in LINK.findall(body) + IMG.findall(body):
                if re.match(r"^(https?:|mailto:)", target):
                    continue
                path, _, frag = target.partition("#")
                dest = page if path == "" else (page.parent / path).resolve()
                if path and not dest.exists():
                    problems.append(f"{rel}: broken link {target}")
                    continue
                if frag and dest.suffix == ".md" and frag not in anchors(dest, cache):
                    problems.append(f"{rel}: no anchor #{frag} in {dest.name}")
    for only in sorted(page_sets["ru"] ^ page_sets["en"]):
        side = "ru" if only in page_sets["ru"] else "en"
        problems.append(f"{only} exists only in {side}")
    for p in problems:
        print(p)
    if problems:
        print(f"{len(problems)} problem(s)")
        return 1
    n = sum(len(v) for v in page_sets.values())
    print(f"ok: {n} pages, links, anchors, SUMMARY and ru/en parity")
    return 0


if __name__ == "__main__":
    sys.exit(main())
