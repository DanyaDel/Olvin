//! E1 end to end: builds a tiny program in release and looks at its bytes with
//! `tools/check_binary_strings.py`. Slow (a nested cargo build), so it is
//! `#[ignore]`d; CI and `cargo test -p olvin_core --test obf_binary -- --ignored`
//! run it. The plain literal is the control that proves the check can see text.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const OBFUSCATED: &str = "OLVIN-OBF-LITERAL-4f9c27ab";
const PLAIN: &str = "OLVIN-PLAIN-LITERAL-4f9c27ab";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "builds a binary in release; run with --ignored (CI does)"]
fn obfuscated_literal_is_not_in_the_release_binary() {
    let work = std::env::temp_dir().join(format!("olvin-obf-bin-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join("src")).unwrap();
    let core = repo().join("crates/olvin_core").canonicalize().unwrap();
    fs::write(
        work.join("Cargo.toml"),
        format!(
            "[package]\nname = \"obf_probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nolvin_core = {{ path = {core:?} }}\n\n[workspace]\n"
        ),
    )
    .unwrap();
    fs::write(
        work.join("src/main.rs"),
        format!(
            "fn main() {{\n    let secret = olvin_core::secret::obf!(\"{OBFUSCATED}\");\n    \
             let plain = std::hint::black_box(\"{PLAIN}\");\n    println!(\"{{secret}} {{plain}}\");\n}}\n"
        ),
    )
    .unwrap();
    // The same versions as the workspace, so the nested build needs nothing new.
    let _ = fs::copy(repo().join("Cargo.lock"), work.join("Cargo.lock"));

    // Not under /tmp (tmpfs): the target dir goes next to the workspace's own.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.join("target"));
    let target = target.join("obf-probe");
    let status = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["build", "--release", "--manifest-path"])
        .arg(work.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .unwrap();
    assert!(status.success(), "the probe did not build");
    let binary = target.join("release/obf_probe");

    // It still works: the literal is rebuilt at run time.
    let run = Command::new(&binary).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        format!("{OBFUSCATED} {PLAIN}")
    );

    let script = repo().join("tools/check_binary_strings.py");
    let check = Command::new("python3")
        .arg(&script)
        .arg(&binary)
        .args(["--forbid", OBFUSCATED, "--require", PLAIN])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    // And the control the other way round: forbidding the plain one must fail.
    let control = Command::new("python3")
        .arg(&script)
        .arg(&binary)
        .args(["--forbid", PLAIN])
        .output()
        .unwrap();
    assert!(
        !control.status.success(),
        "the script missed a plain literal"
    );
    let _ = fs::remove_dir_all(&work);
}
