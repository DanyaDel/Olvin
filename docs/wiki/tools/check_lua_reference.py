#!/usr/bin/env python3
"""Fails when the Lua API reference and the code disagree.

Scans crates/olvin_script/src/lua/{api.rs,audio.rs,prelude.lua,mod.rs} for
every function registered on `ctx` (`m.add_method("name", ...)`, the wrappers
the prelude defines) and for the script callbacks (`CALLBACKS`), then checks
that docs/wiki/{ru,en}/src/reference/lua-api.md mentions each of them as
`ctx:name(` / `name` and mentions nothing that does not exist.

Usage: python3 docs/wiki/tools/check_lua_reference.py [--list]
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
LUA = ROOT / "crates/olvin_script/src/lua"
PAGES = [ROOT / f"docs/wiki/{lang}/src/reference/lua-api.md" for lang in ("ru", "en")]
# Names the pages may mention with `ctx:` that are not registered methods.
ALLOWED_EXTRA: set[str] = set()


def registered() -> set[str]:
    names: set[str] = set()
    for f in ("api.rs", "audio.rs"):
        text = (LUA / f).read_text()
        names |= set(re.findall(r'add_method\(\s*"([a-z_0-9]+)"', text))
    prelude = (LUA / "prelude.lua").read_text()
    # Wrappers assigned directly: `M.look_at = function`, `M.raycast = ...`.
    names |= set(re.findall(r"^\s*M\.([a-z_0-9]+)\s*=", prelude, re.M))
    # Getters / setters generated from a list of base names.
    for block in re.findall(r"for _, n in ipairs\(\{(.*?)\}\) do (getter|setter|setter_of)\(n\) end",
                            prelude, re.S):
        base = re.findall(r'"([a-z_0-9]+)"', block[0])
        names |= set(base)
    return names


def callbacks() -> set[str]:
    text = (LUA / "mod.rs").read_text()
    m = re.search(r"const CALLBACKS: \[&str; \d+\] = \[(.*?)\];", text, re.S)
    return set(re.findall(r'"([a-z_]+)"', m.group(1)))


def main() -> int:
    funcs, cbs = registered(), callbacks()
    if "--list" in sys.argv:
        print(len(funcs), "functions:", *sorted(funcs))
        print(len(cbs), "callbacks:", *sorted(cbs))
        return 0
    bad = 0
    for page in PAGES:
        text = page.read_text()
        documented = set(re.findall(r"ctx:([a-z_0-9]+)", text))
        for name in sorted(funcs - documented):
            print(f"{page.relative_to(ROOT)}: `ctx:{name}` is registered but not documented")
            bad += 1
        for name in sorted(documented - funcs - ALLOWED_EXTRA):
            print(f"{page.relative_to(ROOT)}: `ctx:{name}` is documented but does not exist")
            bad += 1
        for name in sorted(cb for cb in cbs if f"`{cb}`" not in text and f"{cb}(" not in text):
            print(f"{page.relative_to(ROOT)}: callback `{name}` is not documented")
            bad += 1
    if bad:
        print(f"{bad} problem(s)")
        return 1
    print(f"ok: {len(funcs)} functions and {len(cbs)} callbacks documented in {len(PAGES)} pages")
    return 0


if __name__ == "__main__":
    sys.exit(main())
