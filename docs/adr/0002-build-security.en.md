# 0002. Build, supply-chain and binary hardening

<p align="center"><a href="0002-build-security.md">Русский</a></p>

**Status:** Accepted — 2026-11-24 (roadmap phase 8: B1, B2, B3, B4, E1, E2, E3, S3, T1)

## Context

A game that is in the player's hands cannot be fully protected. Phase 8 of the roadmap therefore labels each measure: **protection** (the threat is closed), **deterrence** (raises the price, does not lock) or **signal** (a break-in becomes visible). This ADR records how the build side was done and what each piece really gives. Out of scope: physical access to the machine, legal measures, a compromised developer machine (that needs keys kept off it).

## Decision

| Code | What | Gives |
|---|---|---|
| B1 | `olvin-hub build --release` strips the executable (`CARGO_PROFILE_RELEASE_STRIP=symbols`). `--keep-symbols` links with line tables, strips the shipped copy with `objcopy` and writes symbols to `<out's parent>/symbols/<name>-<build id>.debug`, outside the folder and the archive. `--no-strip` turns it off. The game template gets `strip = "symbols"`, `debug = false`. | deterrence |
| B3 | Release passes `--remap-path-prefix` for `$HOME` (`~`), `$CARGO_HOME`, the rustc sysroot and the project folder (`/project`), appended to the user's own rustflags. `tools/check_binary_strings.py` looks for forbidden strings in a binary; the CLI test builds with and without `--no-remap` (the control) and CI runs the script. | deterrence (no user name or folder layout in the file) |
| B2 | `manifest.json` (path, size, SHA-256 of every shipped file, build id, engine version) and `manifest.sig` (Ed25519 over the exact bytes of the manifest). `olvin-hub keygen` makes the key; `build --release` signs and compiles the public key into the game (`OLVIN_PUBKEY`, read by `option_env!` in the game crate). `olvin-hub verify DIR --pubkey KEY` checks a folder from outside. | **protection** against a tampered download, when the signature is checked by an installer or launcher that holds the right key |
| E2 | `olvin_core::integrity::IntegrityPlugin` (feature `integrity`, module `integrity` in the hub, on in the template for release builds) checks the signature and every hash at start; `Policy::Enforce` stops the game (`AppExit` before the runner starts), `Warn` only logs. | **signal** only |
| B4 | `deny.toml` (licences, crates.io only, a ban list of HTTP/telemetry crates), `supply-chain` CI job (`cargo deny`, `cargo audit`) on push and weekly, `Cargo.lock` is committed, `olvin-hub build --locked`. | protection against a swapped or vulnerable dependency (as far as RustSec knows) |
| E1 | `olvin_core::secret::obf!("...")` over `obfstr`: the literal is scrambled in `.rodata` and rebuilt at run time. | deterrence |
| E3 | `SecretString`/`SecretBytes` (`zeroize` on drop, `Debug` prints nothing, no `Display`, not `Clone`); the signing key lives only in them. | protection against a later memory dump or a stray log line |
| S3 | `Redactor` in `Logging`: home -> `~`, project -> `<project>`, OS user name -> `<user>` in log lines, panic reports, crash files. Nothing is sent anywhere: the engine has no network client (checked: no HTTP crate in `Cargo.lock`, banned in `deny.toml`; the only listener is the opt-in profiler server `OLVIN_PROFILE_HTTP`); reports are local files. | privacy hygiene |
| T1 | Hub and editor ask "Trust this project?" before building or running a project they did not create. CLI: `--trust` / `OLVIN_TRUST=1`, `olvin-hub trust`/`untrust`. | consent against *accidentally* running code you only opened |

### Design points and trade-offs

- **`panic` stays `unwind`.** The template's release profile does not set `panic = "abort"`: scripts (`olvin_script::runtime`) and asset jobs (`olvin_assets`) use `catch_unwind`, so abort would turn one failing script into a dead game. The crash hook runs before an abort too, so the hook was not the obstacle. Fat LTO is not set: `lto = "thin"` stays, because compile time is paid on every release.
- **Ed25519 via `ed25519-dalek` 2.2**, not minisign. `minisign-verify` only verifies; signing would need an external tool in the developer's and CI's path. dalek signs and verifies in-process, is pure Rust, audited and widely used, and its key zeroizes itself. Cost: our own small file format (`olvin-ed25519-secret:<hex>`) instead of minisign's ecosystem. Version 3 exists; moving to it is a separate, mechanical change. SHA-256 (`sha2`) as the roadmap says, not blake3; it streams files in 256 KiB chunks on up to four threads and uses the hardware SHA extensions where present.
- **Key handling.** The private key is a 32-byte seed in a 0600 file under `<config>/olvin-hub/keys/`; a build refuses a key inside the project or the output folder, and refuses a file other users can read. CI can give the key as `OLVIN_SIGNING_KEY_DATA` (the file's content) or `OLVIN_SIGNING_KEY` (a path); environment variables are visible to every process of the job, so prefer a file in a runner temp folder removed afterwards.
- **Where the public key comes from.** The hub sets `OLVIN_PUBKEY` for the cargo process; the game reads it with `option_env!` (the `integrity_plugin!()` macro expands in the game crate). The key sits in the game, so a patched game can swap it: that is why E2 is only a signal. The macro is in the game crate so that a new key recompiles the game, not the engine. An unsigned build has no key and the plugin does nothing.
- **Stripping tools.** Splitting symbols needs `objcopy` or `llvm-objcopy` on Linux; on Windows the `.pdb` is copied; macOS is not supported yet and fails loudly instead of shipping symbols. Remapped paths also appear in the symbols, so a debugger needs `set substitute-path`.
- **rustflags.** The remap flags go in through `--config build.rustflags`, which cargo joins with `.cargo/config.toml`; a set `RUSTFLAGS` shadows every other source, so there ours are appended to it. A `target.<triple>.rustflags` entry would still shadow ours: that is why the CI check looks at the finished binary.
- **One trust list for hub and editor.** The roadmap draft said "in their config"; a single `<config>/olvin/trusted-projects.toml` (crate `olvin_trust`) means a decision made in the hub is not asked again in the editor. Trust is by canonical path, not inherited by sub-folders, and not re-checked when the files change (`git pull` after trusting is trusted too). The editor's "Open without building" blocks cargo/shell jobs; it does not make assets read-only, and `cargo run` typed in a terminal asks nobody.

### Startup cost of the integrity check

Measured on the vertical slice (`olvin-hub build --release -p apps/vertical_slice`): 14 files, 19.2 MB (the executable is 18.8 MB), Linux, 12 threads, SHA-NI, warm page cache, machine shared with other builds: `olvin-hub verify` took median 47 ms (min 24 ms, max 88 ms over 25 runs; the in-game check runs the same function). A cold cache is dominated by disk reads and was not measured. For a game with large assets the cost grows with the data read: roughly 0.5-1.5 GB/s from cache.

## Considered options

- **Encrypting the executable or packer-style protection**: slows start, trips antivirus, still falls to a memory dump. Not done.
- **`panic = "abort"`, fat LTO, `codegen-units = 1` in the template**: smaller and faster binaries, but see above; the rest is compile time for little gain in hiding anything.
- **Verifying the files only inside the game**: simplest, but then only a signal. The same check is exposed as `olvin-hub verify` and `integrity::verify_dir` for an installer.
- **minisign / sigstore / TUF**: more ecosystem, more moving parts; sigstore needs a network and an identity provider. May come with the online registry (B6, L5).
- **A keyless tamper seal (a hash in the binary)**: whoever patches the binary patches the hash.

## Consequences

- A patched executable can skip the check or carry its own key; only an outside verifier that holds your public key sees that the files are not yours. Docs and UI say so: this is a signal against casual tampering of assets and scripts and a protection against tampered downloads *when the installer checks it*.
- Release builds are slower with signing key lookup and hashing only by a few tens of milliseconds, but the first hub build recompiles the dependency graph once because the rustflags differ from a plain `cargo build --release`.
- `cargo audit` reports no vulnerabilities after `url` 2.5.9; the remaining warnings (unmaintained or unsound transitive crates such as `bincode`, `paste`, `ttf-parser`, `heapless` 0.8, `zvariant`) are not ignored and are listed by the weekly job; each needs an upstream release to go away.
- Not done: asset encryption (A1-A3), build-script warning for crates.io dependencies (B5), catalog signature (B6), containerised builds, a trust indicator in the project list, signing the hub's own releases.
