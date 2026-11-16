fn main() {
    #[cfg(feature = "window")]
    tauri_build::build();
    // `one` without its window, on Windows, is a console program that asks for no console window of its own when
    // nothing it's started from has one, as when Task Scheduler starts its server (Windows 11 24H2 and later).
    #[cfg(not(feature = "window"))]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_manifest::embed_manifest_file("one.manifest").expect("the manifest is embedded");
    }
}
