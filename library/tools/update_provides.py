#!/usr/bin/env python3
"""Rewrites the `provides` list of every library/*/pack.ron from the files under assets/."""
import pathlib
import re

LIB = pathlib.Path(__file__).resolve().parents[1]
for manifest in sorted(LIB.glob("*/pack.ron")):
    assets = manifest.parent / "assets"
    files = sorted(str(f.relative_to(assets)).replace("\\", "/") for f in assets.rglob("*") if f.is_file())
    body = ",\n    ".join(f'"{f}"' for f in files)
    text = manifest.read_text()
    text = re.sub(r"provides: \[.*?\],\n", f"provides: [\n    {body},\n  ],\n", text, flags=re.S)
    manifest.write_text(text)
    print(manifest.parent.name, len(files))
