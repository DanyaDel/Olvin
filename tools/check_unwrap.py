#!/usr/bin/env python3
"""Counts non-test `.unwrap()` / `.expect(` per crate and fails when a crate
goes over its allowance in tools/unwrap_allowance.toml (issue #56).

Test code is skipped: files under `tests/`, `benches/`, `examples/`, modules
behind `#[cfg(test)]` (inline or `mod x;` files) and `#[cfg(test)]` items.
A crate missing from the allowance file has an allowance of 0.

    python3 tools/check_unwrap.py          # check (CI)
    python3 tools/check_unwrap.py --list   # print every counted call site
"""

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = {"target", ".git", ".claude", "tests", "benches", "examples"}
CALL = re.compile(r"\.unwrap\(\)|\.expect\(")
CFG_TEST = re.compile(r"^\s*#\[cfg\((?:all\()?test\b.*\)\]\s*$")
MOD_FILE = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+(\w+)\s*;")


def rust_files():
    for path in sorted(ROOT.rglob("*.rs")):
        if SKIP_DIRS.isdisjoint(path.relative_to(ROOT).parts):
            yield path


def crate_of(path):
    for parent in path.parents:
        manifest = parent / "Cargo.toml"
        if manifest.is_file():
            m = re.search(r'^\s*name\s*=\s*"([^"]+)"', manifest.read_text(), re.M)
            if m and "[package]" in manifest.read_text():
                return m.group(1)
        if parent == ROOT:
            break
    return "?"


def code_part(line):
    """The line without its `//` comment (strings with `//` are not a concern here)."""
    return line.split("//", 1)[0]


def skip_item(lines, i):
    """Index after the item starting at line `i` (balanced braces or `;`)."""
    depth, seen = 0, False
    while i < len(lines):
        code = code_part(lines[i])
        depth += code.count("{") - code.count("}")
        seen = seen or "{" in code
        i += 1
        if (seen and depth <= 0) or (not seen and code.rstrip().endswith(";")):
            break
    return i


def scan(path, test_files):
    """Yields (line number, text) of non-test calls; fills `test_files`."""
    lines = path.read_text().split("\n")
    i = 0
    while i < len(lines):
        if CFG_TEST.match(lines[i]):
            j = i + 1
            while j < len(lines) and lines[j].lstrip().startswith("#["):
                j += 1
            if j < len(lines) and (m := MOD_FILE.match(lines[j])):
                name = m.group(1)
                for base in (path.parent, path.parent / path.stem):
                    test_files.update({base / f"{name}.rs", base / name / "mod.rs"})
            i = skip_item(lines, j)
            continue
        if CALL.search(code_part(lines[i])):
            yield i + 1, lines[i].strip()
        i += 1


def main():
    counts, sites, test_files = {}, [], set()
    results = {}
    for path in rust_files():
        results[path] = list(scan(path, test_files))
    for path, found in results.items():
        if path in test_files:
            continue
        crate = crate_of(path)
        counts[crate] = counts.get(crate, 0) + len(found)
        sites += [(crate, path.relative_to(ROOT), n, text) for n, text in found]
    if "--list" in sys.argv:
        for crate, path, n, text in sites:
            print(f"{crate}: {path}:{n}: {text}")
    allowance = tomllib.loads((ROOT / "tools/unwrap_allowance.toml").read_text())
    allowed = allowance.get("crates", {})
    failed = False
    for crate in sorted(counts):
        count, limit = counts[crate], allowed.get(crate, 0)
        if count > limit:
            failed = True
            print(f"FAIL {crate}: {count} non-test unwrap/expect, allowed {limit}")
            for c, path, n, text in sites:
                if c == crate:
                    print(f"  {path}:{n}: {text}")
        elif count < limit:
            print(f"note {crate}: {count} < allowed {limit}, lower the allowance")
    print(f"total {sum(counts.values())} non-test unwrap/expect in {len(counts)} crates")
    if failed:
        print("Handle the error (`?`, `ok_or`, `let ... else`) or, if it cannot fail,")
        print("keep it with an `// invariant: ...` comment and raise the allowance.")
        sys.exit(1)


if __name__ == "__main__":
    main()
