//! `olvin-hub library ...` end to end. The hub config lives in a temporary
//! XDG_CONFIG_HOME, so a test never reads or writes the real one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olvin-hub-lib-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A project (any folder whose Cargo.toml has an `olvin` dependency) with a
/// user pack in `library/`.
fn project(dir: &Path) -> PathBuf {
    let root = dir.join("game");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"game\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nolvin = { version = \"0.1\" }\n\n[workspace]\n",
    )
    .unwrap();
    let pack = root.join("library/my-pack");
    fs::create_dir_all(pack.join("assets/scripts/my-pack")).unwrap();
    fs::write(
        pack.join("pack.ron"),
        r#"(id: "my-pack", name: "Mine", version: "1.0.0", category: Gameplay,
            description: "A user pack.", author: "me", license: "CC0-1.0",
            provides: ["scripts/my-pack/a.lua"], notes: "Attach it.")"#,
    )
    .unwrap();
    fs::write(pack.join("assets/scripts/my-pack/a.lua"), "return {}").unwrap();
    root
}

fn hub(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_olvin-hub"))
        .args(args)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("HOME", dir.join("home"))
        .env_remove("OLVIN_LIBRARY")
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
fn list_show_add_installed() {
    let dir = temp_dir("flow");
    let game = project(&dir);
    let g = game.to_str().unwrap();

    let list = hub(&dir, &["library", "list", "--project", g]);
    assert!(list.status.success(), "{}", text(&list));
    let list = text(&list);
    assert!(list.contains("my-pack") && list.contains("[ ]"), "{list}");

    let found = text(&hub(&dir, &["library", "list", "--project", g, "user"]));
    assert!(
        found.contains("my-pack") && !found.contains("fps-controller"),
        "{found}"
    );
    let none = text(&hub(
        &dir,
        &["library", "list", "--project", g, "-c", "shaders", "user"],
    ));
    assert!(none.contains("No packs match"), "{none}");
    assert!(
        !hub(&dir, &["library", "list", "-c", "nope"])
            .status
            .success()
    );

    let show = text(&hub(&dir, &["library", "show", "my-pack", "--project", g]));
    assert!(
        show.contains("Mine 1.0.0") && show.contains("not installed"),
        "{show}"
    );
    assert!(
        !hub(&dir, &["library", "show", "missing", "--project", g])
            .status
            .success()
    );

    let add = hub(&dir, &["library", "add", "my-pack", "--project", g]);
    assert!(add.status.success(), "{}", text(&add));
    assert!(text(&add).contains("Attach it."));
    assert!(game.join("assets/scripts/my-pack/a.lua").is_file());
    assert!(game.join("assets/scripts/my-pack/a.lua.meta").is_file());
    assert!(game.join("library.lock.ron").is_file());

    let installed = text(&hub(&dir, &["library", "installed", "--project", g]));
    assert!(
        installed.contains("my-pack") && installed.contains("1.0.0"),
        "{installed}"
    );
    let list = text(&hub(&dir, &["library", "list", "--project", g, "user"]));
    assert!(list.contains("[x]"), "{list}");
}

#[test]
fn a_clash_needs_a_choice() {
    let dir = temp_dir("clash");
    let game = project(&dir);
    let g = game.to_str().unwrap();
    let mine = game.join("assets/scripts/my-pack/a.lua");
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::write(&mine, "-- mine").unwrap();

    let refused = hub(&dir, &["library", "add", "my-pack", "--project", g]);
    assert!(!refused.status.success());
    assert!(text(&refused).contains("--keep-both"), "{}", text(&refused));
    assert_eq!(fs::read_to_string(&mine).unwrap(), "-- mine");

    let both = hub(
        &dir,
        &["library", "add", "my-pack", "--project", g, "--keep-both"],
    );
    assert!(both.status.success(), "{}", text(&both));
    assert_eq!(fs::read_to_string(&mine).unwrap(), "-- mine");
    assert!(game.join("assets/scripts/my-pack/a-2.lua").is_file());

    // The flags exclude each other.
    assert!(
        !hub(
            &dir,
            &[
                "library",
                "add",
                "my-pack",
                "--project",
                g,
                "--replace",
                "--skip"
            ]
        )
        .status
        .success()
    );
}

#[test]
fn the_library_lists_outside_a_project() {
    let dir = temp_dir("builtin");
    let out = hub(&dir, &["library", "list"]);
    assert!(out.status.success(), "{}", text(&out));
    let out = text(&out);
    assert!(out.contains("outside a project"), "{out}");
}
