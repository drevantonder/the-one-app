//! What differs by platform, one module per OS, each with the same functions (what the app's commands and start ask of
//! the platform), so lib.rs asks `platform::x()` and names no OS. What every computer does the same way is `desktop`'s,
//! and what both phones do `mobile`'s; an OS's module takes those (`pub use super::desktop::*`) and defines only what it
//! does its own way, which a glob import gives way to.

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub mod apple;
#[cfg(target_os = "macos")]
mod menubar;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod iphone;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "android")]
pub use android::*;
#[cfg(target_os = "ios")]
pub use iphone::*;
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use mac::*;
#[cfg(windows)]
pub use windows::*;
