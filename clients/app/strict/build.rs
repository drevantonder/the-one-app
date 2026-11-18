//! The X11 window (`platform/linux/x11.rs`) links xkbcommon's X11 half by its unversioned name (`libxkbcommon-x11.so`), which a Linux
//! distribution ships only in its development package (Debian's and Ubuntu's `libxkbcommon-x11-dev`), while the library
//! itself (`libxkbcommon-x11.so.0`) is wherever X11 apps run. Where only that is, it's linked by a name of its own here,
//! so building needs nothing installed that running doesn't.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let folders = [format!("/usr/lib/{arch}-linux-gnu"), format!("/lib/{arch}-linux-gnu"), "/usr/lib64".into(), "/usr/lib".into()];
    if folders.iter().any(|folder| std::path::Path::new(folder).join("libxkbcommon-x11.so").exists()) {
        return;
    }
    let Some(library) = folders.iter().map(|folder| std::path::Path::new(folder).join("libxkbcommon-x11.so.0")).find(|library| library.exists()) else {
        return;
    };
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo gives it"));
    let named = out.join("libxkbcommon-x11.so");
    let _ = std::fs::remove_file(&named);
    std::os::unix::fs::symlink(library, &named).expect("OUT_DIR is ours");
    println!("cargo::rustc-link-search=native={}", out.display());
}
