//! Windows (MSVC): links the prebuilt `assets/icon.res` into the .exe, so
//! Explorer and the taskbar show the app icon. The MSVC linker accepts
//! `.res` files directly, so no resource compiler or build dependency is
//! needed. `assets/make_icons.py` regenerates the file. Other targets: no-op.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.res");
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") == "windows" && var("CARGO_CFG_TARGET_ENV") == "msvc" {
        let res = std::path::Path::new(&var("CARGO_MANIFEST_DIR")).join("assets/icon.res");
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
