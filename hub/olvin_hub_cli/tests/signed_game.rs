//! The whole signing chain on a real hub project: `keygen`, `build --release`
//! (public key compiled in, signed manifest), then the game itself checks its
//! files at start and refuses to run when one was changed. Compiles the engine
//! core in release, so it is `#[ignore]`d; CI runs it with `--ignored`.
//! The hub config lives in a temporary XDG_CONFIG_HOME.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use olvin_hub::{Catalog, EngineSource, Project};

fn hub(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_olvin-hub"))
        .args(args)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("OLVIN_TRUST", "1")
        .env_remove("OLVIN_SIGNING_KEY")
        .env_remove("OLVIN_SIGNING_KEY_DATA")
        // Shared with the test run: the engine is already compiled there.
        .env(
            "CARGO_TARGET_DIR",
            std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| dir.join("target")),
        )
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
#[ignore = "compiles the engine core in release; run with --ignored (CI does)"]
fn signed_release_game_checks_itself_and_refuses_tampered_files() {
    let dir = std::env::temp_dir().join(format!("olvin-signed-game-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // A headless game with only the integrity module: the template adds the plugin.
    let catalog = Catalog::builtin();
    let project = Project::create_with(
        &dir,
        "smoke",
        &EngineSource::default(),
        &catalog,
        &["integrity"],
    )
    .unwrap();
    let main = fs::read_to_string(project.root().join("src/main.rs")).unwrap();
    assert!(main.contains("integrity_plugin!"), "{main}");
    fs::create_dir_all(project.root().join("assets/scripts")).unwrap();
    fs::write(
        project.root().join("assets/scripts/rules.lua"),
        "-- original",
    )
    .unwrap();

    let key = dir.join("keys/release.key");
    let made = hub(&dir, &["keygen", "--out", key.to_str().unwrap()]);
    assert!(made.status.success(), "{}", text(&made));
    let public = fs::read_to_string(key.with_extension("pub")).unwrap();
    let public = public.trim().rsplit(':').next().unwrap().to_owned();

    let out = dir.join("out/smoke");
    let built = hub(
        &dir,
        &[
            "build",
            "--release",
            "-p",
            project.root().to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--sign-key",
            key.to_str().unwrap(),
        ],
    );
    assert!(built.status.success(), "{}", text(&built));
    assert!(text(&built).contains("signed:   yes"), "{}", text(&built));

    // From outside, as an installer would.
    let verify = |dir: &Path| hub(dir, &["verify", out.to_str().unwrap(), "--pubkey", &public]);
    let ok = verify(&dir);
    assert!(ok.status.success(), "{}", text(&ok));

    // The game verifies itself and runs.
    let run = Command::new(out.join("smoke")).output().unwrap();
    let log = text(&run);
    assert!(run.status.success(), "{log}");
    assert!(
        log.contains("integrity:") && log.contains("signature Valid"),
        "{log}"
    );
    assert!(log.contains("headless app is running"), "{log}");

    // One changed script: both the outside check and the game say no.
    fs::write(out.join("assets/scripts/rules.lua"), "-- cheat").unwrap();
    let bad = verify(&dir);
    assert!(!bad.status.success());
    assert!(
        text(&bad).contains("modified: assets/scripts/rules.lua"),
        "{}",
        text(&bad)
    );
    let run = Command::new(out.join("smoke")).output().unwrap();
    let log = text(&run);
    assert!(log.contains("integrity check FAILED"), "{log}");
    assert!(
        !log.contains("headless app is running"),
        "the game ran on after a failed check: {log}"
    );
    fs::remove_dir_all(dir).unwrap();
}
