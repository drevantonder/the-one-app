// A Mac's build links ScreenCaptureKit's Swift bridge, whose Swift runtime is the system's, in /usr/lib/swift: a
// dependency's build script can't put that on this program's rpath, so this does.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
