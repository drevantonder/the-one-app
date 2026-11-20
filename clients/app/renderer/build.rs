//! A Mac's, an iPhone's and Windows' text input is Flutter's (`flutter/`, the plugin and its `TextInputModel`), built
//! here with the `cc` crate as Flutter's engine builds it, with the window's side of it (`src/platform/<os>/text_input.*`).
//!
//! The X11 window (`platform/linux/x11.rs`) links xkbcommon's X11 half by its unversioned name (`libxkbcommon-x11.so`), which a Linux
//! distribution ships only in its development package (Debian's and Ubuntu's `libxkbcommon-x11-dev`), while the library
//! itself (`libxkbcommon-x11.so.0`) is wherever X11 apps run. Where only that is, it's linked by a name of its own here,
//! so building needs nothing installed that running doesn't.

#[path = "src/render/bindings.rs"]
#[allow(dead_code)]
mod bindings;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    quickjs();
    shaders();
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
    // A copy, which a machine of any kind makes: its build script runs on whatever builds it, Windows' too.
    std::fs::copy(library, &named).expect("OUT_DIR is ours");
    println!("cargo::rustc-link-search=native={}", out.display());
}

/// Which QuickJS this is, as `QUICKJS`: its crate's version and where it's from, as the workspace's lock pins them, so a
/// page's bytecode a build of this program compiled is kept for the next build on the same QuickJS (`js::QUICKJS`),
/// as V8's code cache is kept for its version, and compiled again only once QuickJS changes.
fn quickjs() {
    let lock = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("cargo says")).join("../../../Cargo.lock");
    println!("cargo::rerun-if-changed={}", lock.display());
    let text = std::fs::read_to_string(&lock).expect("the workspace has its lock");
    let package = text.split("[[package]]").find(|package| package.contains("name = \"rquickjs-sys\"")).expect("rquickjs-sys is locked");
    let said: String = package.lines().filter(|line| line.starts_with("version") || line.starts_with("source")).collect();
    // FNV-1a, as a folder's name.
    let hash = said.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3));
    println!("cargo::rustc-env=QUICKJS={hash:016x}");
}

/// Windows' shaders, compiled here to DXIL, as a game's and Chrome's ANGLE's are, so a window draws without a shader
/// compiler running on the device as the app starts (FXC took 0.4 to 0.6 s of the Surface's first window, 2 to 4 s
/// as an update restarted it, issue 26): each pipeline's two stages (`bindings::PIPELINES`), as HLSL by naga with the
/// registers wgpu-hal 30's Direct3D 12 root signature gives its layout (`create_pipeline_layout`, which this
/// follows, so naga moves with wgpu), and that by DXC (`mise install`). The renderer loads them through wgpu's shader
/// passthrough. Elsewhere there's nothing, and `$OUT_DIR/dxil.rs`, which lists them, lists none.
fn shaders() {
    println!("cargo::rerun-if-changed=src/render/shaders.wgsl");
    println!("cargo::rerun-if-changed=src/render/bindings.rs");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets it"));
    let mut listed = String::from("pub(crate) static DXIL: &[(&str, &str, &[u8], &[u8])] = &[\n");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let source = std::fs::read_to_string("src/render/shaders.wgsl").expect("the shaders are there");
        let module = naga::front::wgsl::parse_str(&source).expect("the shaders parse");
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .expect("the shaders are valid");
        for pipeline in bindings::PIPELINES {
            let compiled = [(naga::ShaderStage::Vertex, pipeline.vertex, "vs_6_0"), (naga::ShaderStage::Fragment, pipeline.fragment, "ps_6_0")].map(|(stage, entry, profile)| {
                let hlsl = hlsl(&module, &info, pipeline, stage);
                let file = |kind| out.join(format!("{}-{}-{entry}.{kind}", pipeline.vertex, pipeline.fragment));
                std::fs::write(file("hlsl"), hlsl).expect("OUT_DIR takes it");
                // As wgpu-hal runs DXC: HLSL 2018, which naga writes, strict.
                let dxc = std::process::Command::new("dxc")
                    .args(["-E", entry, "-T", profile, "-HV", "2018", "-Ges", "-no-warnings", "-Fo"])
                    .arg(file("dxil"))
                    .arg(file("hlsl"))
                    .status()
                    .expect("dxc compiles Windows' shaders: `mise install` puts it on PATH");
                assert!(dxc.success(), "dxc couldn't compile {entry}");
                file("dxil")
            });
            let [vertex, fragment] = compiled.map(|file| file.display().to_string());
            listed += &format!("    ({:?}, {:?}, include_bytes!({vertex:?}), include_bytes!({fragment:?})),\n", pipeline.vertex, pipeline.fragment);
        }
    }
    listed += "];\n";
    std::fs::write(out.join("dxil.rs"), listed).expect("OUT_DIR takes it");
}

/// One stage of `pipeline` as HLSL, bound where wgpu-hal binds it: each group's buffers and textures in order, CBVs,
/// SRVs and UAVs each counting up from register 0 in space 0, a group's samplers by their index in it (space 255)
/// with an SRV of their indices after its other SRVs, a dynamic storage buffer's offset in root constants at the
/// group's next CBV, and the special constants (first vertex and instance) at the CBV after the last group's.
fn hlsl(module: &naga::Module, info: &naga::valid::ModuleInfo, pipeline: &bindings::Pipeline, stage: naga::ShaderStage) -> String {
    use naga::back::hlsl;
    let (mut cbv, mut srv) = (0, 0);
    let target = |register| hlsl::BindTarget { space: 0, register, ..Default::default() };
    let mut options = hlsl::Options {
        shader_model: hlsl::ShaderModel::V6_0,
        fake_missing_bindings: false,
        zero_initialize_workgroup_memory: true,
        restrict_indexing: true,
        force_loop_bounding: true,
        ray_query_initialization_tracking: true,
        mesh_shader_primitive_indices_clamp: true,
        sampler_heap_target: hlsl::SamplerHeapBindTargets { standard_samplers: target(0), comparison_samplers: target(2048) },
        ..Default::default()
    };
    for (group, bindings) in pipeline.groups().into_iter().enumerate() {
        let group = group as u32;
        let mut storage = 0;
        let mut samplers = 0;
        for (binding, kind) in bindings.iter().enumerate() {
            let at = naga::ResourceBinding { group, binding: binding as u32 };
            let bound = match kind {
                bindings::Binding::Uniform => {
                    cbv += 1;
                    target(cbv - 1)
                }
                bindings::Binding::Storage => {
                    srv += 1;
                    storage += 1;
                    hlsl::BindTarget { dynamic_storage_buffer_offsets_index: Some(storage - 1), ..target(srv - 1) }
                }
                bindings::Binding::Texture { .. } => {
                    srv += 1;
                    target(srv - 1)
                }
                bindings::Binding::Sampler => {
                    samplers += 1;
                    hlsl::BindTarget { space: 255, ..target(samplers - 1) }
                }
            };
            options.binding_map.insert(at, bound);
        }
        if samplers > 0 {
            options.sampler_buffer_binding_map.insert(hlsl::SamplerIndexBufferKey { group }, target(srv));
            srv += 1;
        }
        if storage > 0 {
            options.dynamic_storage_buffer_offsets_targets.insert(group, hlsl::OffsetsBindTarget { space: 0, register: cbv, size: storage });
            cbv += 1;
        }
    }
    options.special_constants_binding = Some(target(cbv));
    let entry = if stage == naga::ShaderStage::Vertex { pipeline.vertex } else { pipeline.fragment };
    // A vertex stage's outputs are its fragment stage's inputs, as wgpu-hal writes it.
    let fragment = (stage == naga::ShaderStage::Vertex).then(|| hlsl::FragmentEntryPoint::new(module, pipeline.fragment).expect("its fragment stage is there"));
    let (module, info) = naga::back::pipeline_constants::process_overrides(module, info, Some((stage, entry)), &Default::default()).expect("the shaders have no overrides to fill");
    let mut hlsl = String::new();
    hlsl::Writer::new(&mut hlsl, &options, &hlsl::PipelineOptions { entry_point: Some((stage, entry.to_owned())) })
        .write(&module, &info, fragment.as_ref())
        .expect("naga writes the shaders as HLSL");
    hlsl
}
