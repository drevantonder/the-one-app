const COMMANDS: &[&str] = &["menu", "passkey", "push", "group"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).ios_path("ios").build()
}
