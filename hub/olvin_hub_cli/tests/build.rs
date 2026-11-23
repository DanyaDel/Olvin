//! `olvin-hub build` end to end. The hub config lives in a temporary
//! XDG_CONFIG_HOME, so a test never reads or writes the real one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olvin-hub-cli-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A game that builds offline in seconds: `olvin` is a stub crate.
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
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nolvin = { path = \"olvin\" }\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.rs"),
        "use olvin as _;\nfn main() { println!(\"tiny\"); }\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("assets/a.txt"), "a").unwrap();
    root
}

fn hub(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_olvin-hub"))
        .args(args)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("HOME", dir.join("home"))
        // The nested cargo gets its own target dir, not the test run's.
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .unwrap()
}

#[test]
fn build_release_makes_folder_and_archive() {
    let dir = temp_dir("release");
    let game = tiny_game(&dir);
    let out = dir.join("dist-out");
    let result = hub(
        &dir,
        &[
            "build",
            "--release",
            "--project",
            game.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--archive",
            "tar.gz",
        ],
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        result.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(stdout.contains("1 asset files"), "{stdout}");
    assert!(out.join("tiny").is_file());
    assert!(out.join("assets/a.txt").is_file());
    assert!(dir.join("dist-out.tar.gz").is_file());
    assert!(dir.join("target/release").is_dir());
    assert!(
        !dir.join("config")
            .join("olvin-hub")
            .join("config.toml")
            .exists(),
        "build does not write the hub config"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn build_reports_errors() {
    let dir = temp_dir("errors");
    let game = tiny_game(&dir);
    let result = hub(
        &dir,
        &[
            "build",
            "--project",
            game.to_str().unwrap(),
            "--archive",
            "rar",
        ],
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("unknown archive format"));

    let result = hub(
        &dir,
        &["build", "--project", dir.join("nope").to_str().unwrap()],
    );
    assert!(!result.status.success());
    fs::remove_dir_all(dir).unwrap();
}

fn broken_config(dir: &Path) {
    let cfg = dir.join("config/olvin-hub");
    fs::create_dir_all(&cfg).unwrap();
    fs::write(cfg.join("config.toml"), "engine = [oops").unwrap();
}

#[test]
fn module_commands_are_all_or_nothing_and_ignore_a_broken_config() {
    let dir = temp_dir("modules");
    let game = tiny_game(&dir);
    let game_arg = game.to_str().unwrap();
    broken_config(&dir);
    let manifest = || fs::read_to_string(game.join("Cargo.toml")).unwrap();
    let before = manifest();

    // an unknown module after a valid one: nothing printed as added, nothing saved
    let result = hub(&dir, &["add", "rand", "nope", "-p", game_arg]);
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stdout).contains("+ rand"));
    assert_eq!(manifest(), before);

    // a broken hub config does not block project commands
    let result = hub(&dir, &["add", "saves", "-p", game_arg]);
    assert!(result.status.success(), "{result:?}");
    assert!(manifest().contains("ron"));

    // `serde` and the `saves` that needs it go together, in either order
    let result = hub(&dir, &["remove", "serde", "saves", "-p", game_arg]);
    assert!(result.status.success(), "{result:?}");
    let text = manifest();
    assert!(!text.contains("ron") && !text.contains("serde"), "{text}");

    // commands that do need the config say what is wrong with it
    let result = hub(&dir, &["projects"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("config.toml"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn editor_remove_rejects_unknown_elements() {
    let dir = temp_dir("editor-remove");
    let result = hub(&dir, &["editor", "remove", "nope"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("no editor element `nope`"));
    fs::remove_dir_all(dir).unwrap();
}

/// A game whose dependency lives outside the project, so rustc sees an
/// absolute source path under the (fake) home folder, like an engine checkout
/// or a registry crate would. `olvin::here()` prints where it thinks it is.
fn leaky_game(dir: &Path) -> PathBuf {
    let home = dir.join("home");
    let stub = home.join("olvin-stub");
    fs::create_dir_all(stub.join("src")).unwrap();
    fs::write(
        stub.join("Cargo.toml"),
        "[package]\nname = \"olvin\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        stub.join("src/lib.rs"),
        "pub fn here() -> &'static str { file!() }\n",
    )
    .unwrap();
    let root = home.join("leaky");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"leaky\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nolvin = { path = \"../olvin-stub\" }\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"{}\", olvin::here()); }\n",
    )
    .unwrap();
    root
}

fn contains(path: &Path, needle: &str) -> bool {
    let bytes = fs::read(path).unwrap();
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// B3: a release build does not carry the home folder; `--no-remap` does (the
/// control that proves the check can see a leak). The binary is also checked
/// with `tools/check_binary_strings.py`; CI sets `OLVIN_KEEP_TEST_BINARY` to a
/// folder to get the remapped binary and its forbidden strings for that script.
#[test]
fn release_hides_the_home_folder_unless_remapping_is_off() {
    let dir = temp_dir("remap");
    let game = leaky_game(&dir);
    let game_arg = game.to_str().unwrap();
    let home = dir.join("home");
    let home_str = home.to_str().unwrap();

    let remapped = dir.join("out-remapped");
    let result = hub(
        &dir,
        &[
            "build",
            "--release",
            "-p",
            game_arg,
            "-o",
            remapped.to_str().unwrap(),
        ],
    );
    assert!(result.status.success(), "{result:?}");
    let bin = remapped.join("leaky");
    assert!(
        !contains(&bin, home_str),
        "the home folder is in the binary"
    );
    let run = Command::new(&bin).output().unwrap();
    let shown = String::from_utf8_lossy(&run.stdout).trim().to_owned();
    assert_eq!(shown, "~/olvin-stub/src/lib.rs");

    let leaky = dir.join("out-leaky");
    let result = hub(
        &dir,
        &[
            "build",
            "--release",
            "--no-remap",
            "-p",
            game_arg,
            "-o",
            leaky.to_str().unwrap(),
        ],
    );
    assert!(result.status.success(), "{result:?}");
    assert!(
        contains(&leaky.join("leaky"), home_str),
        "control: no remap leaks"
    );

    if let Some(keep) = std::env::var_os("OLVIN_KEEP_TEST_BINARY") {
        let keep = PathBuf::from(keep);
        fs::create_dir_all(&keep).unwrap();
        fs::copy(&bin, keep.join("remapped-game")).unwrap();
        fs::copy(leaky.join("leaky"), keep.join("leaky-game")).unwrap();
        fs::write(keep.join("forbidden.txt"), format!("{home_str}\n")).unwrap();
    }
    fs::remove_dir_all(dir).unwrap();
}
