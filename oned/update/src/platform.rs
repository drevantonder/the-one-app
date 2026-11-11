//! The platforms a release is for: what installers and devices name each, and what Go and a user do.

/// A platform a release is for: what the record, installers and app runtimes name it, and what Go and a user do.
pub struct Platform {
    /// Rust's target triple, which an app runtime and a release name it by.
    pub target: &'static str,
    /// Go's `GOOS` and `GOARCH`.
    pub os: &'static str,
    pub arch: &'static str,
    pub said: &'static str,
}

/// Platforms the installer knows. A release has each one some device here can build.
pub static PLATFORMS: [Platform; 4] = [
    Platform { target: "x86_64-unknown-linux-gnu", os: "linux", arch: "amd64", said: "Linux" },
    Platform { target: "aarch64-unknown-linux-gnu", os: "linux", arch: "arm64", said: "Linux on ARM" },
    Platform { target: "aarch64-apple-darwin", os: "darwin", arch: "arm64", said: "Mac" },
    Platform { target: WINDOWS, os: "windows", arch: "amd64", said: "Windows" },
];

/// Windows' target: only the program itself is built for it, on a Linux device that has the toolchain (`mingw-w64`).
pub const WINDOWS: &str = "x86_64-pc-windows-gnu";

impl Platform {
    /// Whether a service can run on it. A Windows computer runs apps but no services yet: the sandbox each needs isn't
    /// made for Windows (ARCHITECTURE.md's Decided says why).
    pub fn runs_services(&self) -> bool {
        self.target != WINDOWS
    }
}
