//! T1: the CLI does not run cargo on a project nobody trusted. The trust list
//! lives in a temporary XDG_CONFIG_HOME, never the real one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olvin-hub-trust-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A game that builds offline in seconds: `olvin` is a stub crate. Its build
/// script leaves a marker file, so a test can tell whether cargo ran it.
fn tiny_game(dir: &Path) -> PathBuf {
    let root = dir.join("tiny");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("olvin/src")).unwrap();
    fs::write(
        root.join("olvin/Cargo.toml"),
        "[package]\nname = \"olvin\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("olvin/src/lib.rs"), "").unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2024\"\nbuild = \"build.rs\"\n\n[dependencies]\nolvin = { path = \"olvin\" }\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        root.join("build.rs"),
        "fn main() { std::fs::write(\"build-script-ran\", \"yes\").unwrap(); }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"tiny\"); }\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    root
}

/// `trust_env`: the value of OLVIN_TRUST, or unset.
fn hub(dir: &Path, args: &[&str], trust_env: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olvin-hub"));
    cmd.args(args)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env_remove("OLVIN_TRUST");
    if let Some(value) = trust_env {
        cmd.env("OLVIN_TRUST", value);
    }
    cmd.output().unwrap()
}

fn err(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn out(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn an_untrusted_project_is_not_built_until_trusted() {
    let dir = temp_dir("build");
    let game = tiny_game(&dir);
    let game_arg = game.to_str().unwrap();
    let marker = game.join("build-script-ran");

    // Refused, in one line that says why and how to go on; cargo never started.
    let refused = hub(&dir, &["build", "-p", game_arg], None);
    assert!(!refused.status.success());
    let message = err(&refused);
    assert!(message.contains("not a trusted project"), "{message}");
    assert!(
        message.contains("build.rs") && message.contains("--trust"),
        "{message}"
    );
    assert_eq!(message.trim().lines().count(), 1, "{message}");
    assert!(
        !marker.exists(),
        "the build script of an untrusted project ran"
    );
    assert!(!dir.join("target").exists());

    // Any value but a yes in OLVIN_TRUST does not count.
    let refused = hub(&dir, &["build", "-p", game_arg], Some("0"));
    assert!(!refused.status.success());
    assert!(!marker.exists());

    // `--trust`: allowed once, with the warning on stderr.
    let allowed = hub(&dir, &["build", "-p", game_arg, "--trust"], None);
    assert!(allowed.status.success(), "{}", err(&allowed));
    assert!(err(&allowed).contains("warning:"), "{}", err(&allowed));
    assert!(marker.is_file());
    assert!(
        !dir.join("config/olvin/trusted-projects.toml").exists(),
        "--trust is for one run, it does not remember"
    );
    fs::remove_file(&marker).unwrap();

    // So does the environment variable.
    let allowed = hub(&dir, &["build", "-p", game_arg], Some("1"));
    assert!(allowed.status.success(), "{}", err(&allowed));
    assert!(marker.is_file());
    fs::remove_file(&marker).unwrap();

    // `trust` remembers it; then no flag and no warning.
    let listed = hub(&dir, &["projects"], None);
    assert!(out(&listed).is_empty() || !out(&listed).contains("tiny"));
    let registered = hub(&dir, &["register", game_arg], None);
    assert!(registered.status.success(), "{}", err(&registered));
    assert!(out(&hub(&dir, &["projects"], None)).contains("(not trusted)"));
    let trusted = hub(&dir, &["trust", game_arg], None);
    assert!(trusted.status.success(), "{}", err(&trusted));
    assert!(
        out(&trusted).contains("build.rs"),
        "the explanation is shown"
    );
    assert!(!out(&hub(&dir, &["projects"], None)).contains("(not trusted)"));
    let built = hub(&dir, &["build", "-p", game_arg], None);
    assert!(built.status.success(), "{}", err(&built));
    assert!(!err(&built).contains("not a trusted"), "{}", err(&built));
    assert!(marker.is_file());

    // `untrust` takes it back.
    assert!(hub(&dir, &["untrust", game_arg], None).status.success());
    let refused = hub(&dir, &["build", "-p", game_arg], None);
    assert!(!refused.status.success());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn run_is_gated_too() {
    let dir = temp_dir("run");
    let game = tiny_game(&dir);
    let refused = hub(&dir, &["run", "-p", game.to_str().unwrap()], None);
    assert!(!refused.status.success());
    assert!(
        err(&refused).contains("not a trusted project"),
        "{}",
        err(&refused)
    );
    assert!(!game.join("build-script-ran").exists());
    fs::remove_dir_all(dir).unwrap();
}

/// A project the hub made is trusted from the start; one it only registered is not.
#[test]
fn projects_made_by_the_hub_are_trusted() {
    let dir = temp_dir("new");
    let made = hub(&dir, &["new", "mine", "--dir", dir.to_str().unwrap()], None);
    assert!(made.status.success(), "{}", err(&made));
    let list = out(&hub(&dir, &["projects"], None));
    assert!(
        list.contains("mine") && !list.contains("(not trusted)"),
        "{list}"
    );
    let file = fs::read_to_string(dir.join("config/olvin/trusted-projects.toml")).unwrap();
    assert!(file.contains("mine"), "{file}");

    let other = tiny_game(&dir);
    assert!(
        hub(&dir, &["register", other.to_str().unwrap()], None)
            .status
            .success()
    );
    let list = out(&hub(&dir, &["projects"], None));
    assert!(list.contains("(not trusted)"), "{list}");
    fs::remove_dir_all(dir).unwrap();
}
