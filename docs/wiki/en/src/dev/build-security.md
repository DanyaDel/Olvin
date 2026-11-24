# Security of builds

A game that is in a player's hands cannot be fully protected. Each measure below says what it gives: **protection** (the threat is closed), **deterrence** (raises the price) or **signal** (tampering becomes visible). The reasoning is in [ADR 0002](https://github.com/DanyaDel/Olvin/blob/main/docs/adr/0002-build-security.en.md).

## What each protection does, and does not

| Measure | Does | Does NOT |
|---|---|---|
| Strip (`build --release`) | removes symbol names and debug info | hide the code; a disassembler still works |
| Path remap | no user name, home or project path in the binary | anything for strings you wrote yourself |
| Signed manifest (`keygen`, `build --release`) | an installer or launcher that holds your public key finds any changed, missing or added file (**protection** against a tampered download) | stop a patched game: the check lives in the game |
| Start-up check | the game stops (or warns) when its files do not match the manifest: a **signal** against casual editing of assets and scripts | resist someone who edits the executable: they remove the check or swap the key |
| `obf!("...")` | a literal is not plain text in the binary: deterrence | hide it from a debugger or memory dump |
| `SecretString`/`SecretBytes` | wipes a secret from memory after use; never prints it | protect a secret while it is in use |
| `cargo deny` / `cargo audit` | a bad licence, an unknown source, a known vulnerability fail CI | find unknown flaws or a malicious new release of a dependency |
| Trust prompt | asks before building or running a project the hub did not create | sandbox anything: a trusted project can do everything you can |

Logs and crash reports are local files. Nothing is sent anywhere. They show `~`, `<project>` and `<user>` instead of your paths and name, so you can attach them to a bug; check them anyway before you do.

## Sign a release

1. Create a key once: `olvin-hub keygen`. It writes `<config>/olvin-hub/keys/release.key` (mode 0600) and `release.pub`, and prints the public key. The hub refuses a key inside a project folder.
2. Keep the private key off the project and out of git, and back it up elsewhere: whoever has it can sign as you; if you lose it, old releases cannot be re-signed and a new key means a new public key in every installer.
3. Build: `olvin-hub build --release --locked --archive zip`. The hub finds the key itself, signs `manifest.json`, writes `manifest.sig` and compiles the public key into the game. For `--keep-symbols`, the symbol file lands in `symbols/` beside the output folder: keep it, do not ship it.
4. CI: give the key as the secret `OLVIN_SIGNING_KEY_DATA` (file contents) or as a file path in `OLVIN_SIGNING_KEY`, or use `--sign-key FILE`. Environment variables are visible to every process of the job.

## Verify

```sh
olvin-hub verify path/to/game-folder --pubkey release.pub
```

`--pubkey` takes the `.pub` file or the 64 hex digits. Without it only the hashes are compared and the output says the signature was not checked. Run this from an installer or launcher you trust; a check inside the game alone is only a signal.

## In the game

The `integrity` module (on by default in new projects) adds `integrity_plugin!()` for release builds. By default a failure stops the game; for a warning only use `IntegrityPlugin::new(option_env!("OLVIN_PUBKEY")).policy(Policy::Warn)`. A run through `cargo run` has no key and skips the check. Files added to `assets/` count as a failure; `logs/` and `saves/` are free.

## Trusting projects

The hub and the editor ask "Trust this project?" for a project they did not create: **Trust and continue**, **Open read-only (no build)** or **Cancel**. Building a project runs its `build.rs` and procedural macros, and those of every dependency, with your permissions. In the CLI, `build` and `run` need `--trust` (or `OLVIN_TRUST=1`) for such a project, or `olvin-hub trust PATH` once. The list is one file for both tools, `<config>/olvin/trusted-projects.toml`. Trust belongs to the folder, not its contents.

## See also

- [CLI olvin-hub](../reference/hub-cli.md)
- [Contributing guide](contributing.md)
