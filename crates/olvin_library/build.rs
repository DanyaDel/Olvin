//! `include_dir!` does not tell cargo which files it read (that needs nightly),
//! so the repo-root `library/` folder is watched here.

fn main() {
    println!("cargo:rerun-if-changed=../../library");
    // Folder mtimes do not change when a file inside is edited: list the files.
    watch(std::path::Path::new("../../library"));
}

fn watch(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            watch(&path);
        }
    }
}
