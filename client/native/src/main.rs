// No console window on Windows, even in a debug build: what it says goes where whoever started it sends its output
// (the throwaway's log, the helper's pipe).
#![cfg_attr(any(not(debug_assertions), windows), windows_subsystem = "windows")]

fn main() {
    // What the platform runs beside the window asks with no window (src/headless.rs).
    if !one_native::headless(&std::env::args().collect::<Vec<_>>()) {
        one_native::run()
    }
}
