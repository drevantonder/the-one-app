// The NIF's module is named for its build, `OneCore.Nifs.B<build>`, so a server loads a changed NIF as a new module
// beside the one before (`OneCore.Nif`). Whoever builds it gives the build, its input hash, in NIF_BUILD_<CRATE>
// (`tooling/nifs`, `core/server/mix.exs`); a build without it is named for none, and no server loads it.
fn main() {
    let var = format!("NIF_BUILD_{}", std::env::var("CARGO_PKG_NAME").unwrap().to_uppercase());
    println!("cargo::rerun-if-env-changed={var}");
    let build = std::env::var(&var).unwrap_or_default();
    let init = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("init.rs");
    std::fs::write(init, format!("rustler::init!(\"Elixir.OneCore.Nifs.B{build}\");\n")).unwrap();
}
