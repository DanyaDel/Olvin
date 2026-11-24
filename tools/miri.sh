#!/usr/bin/env bash
# Runs the tests of the code that has `unsafe` under Miri (needs the nightly
# toolchain with the miri component: `rustup +nightly component add miri`).
# Lua itself is C code and cannot run under Miri, so what is checked is the
# loan that lends `ScriptCtx` to Lua callbacks (`olvin_script::lend`) and the
# secret wipe test of `olvin_core::secret`.
set -euo pipefail
cd "$(dirname "$0")/.."
export MIRIFLAGS="${MIRIFLAGS:--Zmiri-strict-provenance}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/miri}"
cargo +nightly miri test -p olvin_script --no-default-features --lib -- lend::
cargo +nightly miri test -p olvin_core --lib -- secret::
