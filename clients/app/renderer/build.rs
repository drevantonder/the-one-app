//! A Mac's, an iPhone's and Windows' text input is Flutter's (`flutter/`, the plugin and its `TextInputModel`), built
//! here with the `cc` crate as Flutter's engine builds it, with the window's side of it (`src/platform/<os>/text_input.*`).
//!
//! The X11 window (`platform/linux/x11.rs`) links xkbcommon's X11 half by its unversioned name (`libxkbcommon-x11.so`), which a Linux
//! distribution ships only in its development package (Debian's and Ubuntu's `libxkbcommon-x11-dev`), while the library
//! itself (`libxkbcommon-x11.so.0`) is wherever X11 apps run. Where only that is, it's linked by a name of its own here,
//! so building needs nothing installed that running doesn't.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=flutter");
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => text_input(&["src/platform/mac/text_input.mm"], &["flutter/flutter/shell/platform/darwin/macos/framework/Source/FlutterTextInputPlugin.mm"]),
        Ok("ios") => text_input(
            &["src/platform/ios/text_input.mm", "src/platform/ios/device.mm"],
            &[
                "flutter/flutter/shell/platform/darwin/ios/framework/Source/FlutterTextInputPlugin.mm",
                "flutter/flutter/fml/platform/darwin/string_range_sanitization.mm",
            ],
        ),
        Ok("windows") => text_input(
            &["src/platform/windows/text_input.cc"],
            &["flutter/flutter/shell/platform/windows/text_input_plugin.cc", "flutter/flutter/shell/platform/windows/text_input_manager.cc"],
        ),
        Ok("linux") => x11(),
        _ => {}
    }
}

/// Flutter's text input plugin for this platform, its model, and the window's side of it, `ours`.
fn text_input(ours: &[&str], plugin: &[&str]) {
    for ours in ours {
        println!("cargo::rerun-if-changed={ours}");
    }
    let common = ["flutter/flutter/fml/string_conversion.cc", "flutter/flutter/shell/platform/common/text_input_model.cc", "flutter/flutter/shell/platform/common/text_editing_delta.cc"];
    let mut build = cc::Build::new();
    build.cpp(true).std("c++20").include("flutter").files(common).files(plugin).files(ours).warnings(false);
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "macos" {
        build.flag("-fobjc-arc");
        println!("cargo::rustc-link-lib=framework=Cocoa");
        println!("cargo::rustc-link-lib=framework=QuartzCore");
    } else if os == "ios" {
        // What the iOS plugin has of Flutter's engine that Swift makes there, and ICU's one call, which the system's
        // ICU answers (`src/platform/ios/include`).
        println!("cargo::rerun-if-changed=src/platform/ios/include");
        build.flag("-fobjc-arc").include("src/platform/ios/include");
        // FlutterMacros.h's, which mark what Flutter's framework exports and check it's built with ARC, as it is.
        build.define("FLUTTER_DARWIN_EXPORT", Some("")).define("FLUTTER_ASSERT_ARC", Some(""));
        println!("cargo::rustc-link-lib=framework=UIKit");
        println!("cargo::rustc-link-lib=icucore");
    } else {
        // std::wstring_convert, which Flutter's fml converts UTF-8 and UTF-16 with, is deprecated in C++17; and
        // MSVC's library, which cargo-xwin brings, asks for a newer clang than a Linux machine may have.
        build
            .define("_SILENCE_CXX17_CODECVT_HEADER_DEPRECATION_WARNING", None)
            .define("_ALLOW_COMPILER_AND_STL_VERSION_MISMATCH", None)
            .define("UNICODE", None)
            .define("NOMINMAX", None);
        println!("cargo::rustc-link-lib=imm32");
    }
    build.compile("flutter_text_input");
}

/// xkbcommon's X11 half, by a name of its own where only its library is.
fn x11() {
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
