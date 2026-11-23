#!/usr/bin/env python3
"""Fails when a built binary contains strings it must not (roadmap B3, E1).

A release build should not carry the developer's home folder, user name or the
project path (panic locations, debug info), and a literal wrapped in
`olvin_core::secret::obf!` must not sit in `.rodata`. This reads the file as
bytes and looks for each forbidden string (UTF-8 and UTF-16LE, as Windows
binaries store some strings).

    python3 tools/check_binary_strings.py BINARY --forbid STR [--forbid STR ...]
    python3 tools/check_binary_strings.py BINARY --auto          # $HOME, user, cwd, cargo home
    python3 tools/check_binary_strings.py BINARY --require STR   # positive control

`--auto` forbids what leaks most often: `$HOME`, the user name, `$CARGO_HOME`,
the repository root and the working directory (very short or root-like values
such as `/` or `root` are skipped: they would match unrelated text). This is a
check on the build, not protection: it shows that the hub's path remapping did
its job, it says nothing about a binary someone patched afterwards.
"""

import argparse
import getpass
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MIN_LEN = 4


def auto_forbidden():
    found = {}

    def add(label, value):
        value = (value or "").rstrip("/\\")
        if len(value) >= MIN_LEN and value not in ("/root", "/home", "/Users"):
            found.setdefault(value, label)

    add("$HOME", os.environ.get("HOME"))
    add("$USERPROFILE", os.environ.get("USERPROFILE"))
    try:
        add("user name", getpass.getuser())
    except Exception:
        pass
    add("$USER", os.environ.get("USER") or os.environ.get("USERNAME"))
    add("$CARGO_HOME", os.environ.get("CARGO_HOME"))
    add("repository root", str(ROOT))
    add("working directory", os.getcwd())
    return found


def occurrences(data, needle):
    encodings = {"utf-8": needle.encode(), "utf-16le": needle.encode("utf-16le")}
    hits = []
    for name, raw in encodings.items():
        pos = data.find(raw)
        if pos >= 0:
            hits.append((name, pos, data.count(raw)))
    return hits


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("binary", type=Path)
    parser.add_argument("--forbid", action="append", default=[], metavar="STR")
    parser.add_argument("--require", action="append", default=[], metavar="STR")
    parser.add_argument("--auto", action="store_true")
    args = parser.parse_args()

    data = args.binary.read_bytes()
    forbidden = {s: "--forbid" for s in args.forbid}
    if args.auto:
        forbidden.update(auto_forbidden())
    failed = False
    for needle, label in forbidden.items():
        for encoding, pos, count in occurrences(data, needle):
            failed = True
            print(f"FOUND {needle!r} ({label}, {encoding}) x{count}, first at byte {pos}")
    for needle in args.require:
        if not occurrences(data, needle):
            failed = True
            print(f"MISSING {needle!r}: the positive control is not in the binary")
    if failed:
        print(f"{args.binary}: check failed", file=sys.stderr)
        return 1
    print(f"{args.binary}: ok ({len(forbidden)} forbidden, {len(args.require)} required)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
