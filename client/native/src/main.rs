// No console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // What the platform runs beside the window asks with no window (src/headless.rs).
    if !one_native::headless(&std::env::args().collect::<Vec<_>>()) {
        one_native::run()
    }
}
