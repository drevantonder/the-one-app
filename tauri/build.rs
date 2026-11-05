// The shell's own commands are listed, so each needs a permission a capability grants (capabilities/home.json): left
// unlisted, Tauri would let every window call them, screens too.
fn main() {
    let commands = tauri_build::AppManifest::new().commands(&["state", "open", "sign_out"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands)).expect("the shell's build")
}
