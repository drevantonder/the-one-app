//! The sandbox app's wall on Windows, as `bwrap` is on Linux and `sandbox-exec` on a Mac (`Sandbox.start/3`):
//!
//!     wall [--read <path>]... [--write <path>]... [--env NAME=value]... [--memory <bytes>] -- <program> [<argument>]...
//!
//! It starts the program in an AppContainer, Windows' own sandbox for apps, as Chromium's sandbox and Microsoft's MXC
//! do, with its standard input, output and error, and ends with its exit code. In it the program has:
//!
//! - no network: the AppContainer has no capabilities, so Windows lets it reach nothing, the machine itself neither.
//! - none of the person's files: an AppContainer reads only what's granted to it, or to every app (Windows' own
//!   files). Each `--read` path is granted to the One App's AppContainer to read, once, and `--write` to write.
//! - a folder of its own, which `TMPDIR`, `TEMP` and `TMP` name and it starts in, granted to this run's own
//!   capability alone, so no other program behind the wall reads it, and removed once it ends.
//! - the environment `--env` gives it, and `SystemRoot`, which Windows' own libraries need.
//! - at most `--memory` bytes, and nothing left once this ends however it ends: it's in a job object this holds.

#[cfg(windows)]
mod windows;

fn main() {
    #[cfg(windows)]
    std::process::exit(windows::main());
    #[cfg(not(windows))]
    {
        eprintln!("The wall is Windows': Linux's is bubblewrap and a Mac's Seatbelt.");
        std::process::exit(2);
    }
}
