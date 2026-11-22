//! Windows only: embeds the Olvin icon (`assets/logo/olvin-icon.ico`) into the
//! `.exe`. A no-op on every other target.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/logo/olvin-icon.ico");
    // `cfg(windows)` is the build host, `CARGO_CFG_TARGET_OS` the target: the
    // resource compiler is only there when both are Windows.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/logo/olvin-icon.ico");
        res.set("FileDescription", "Olvin Editor (dev)");
        res.set("ProductName", "Olvin");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the exe icon: {e}");
        }
    }
}
