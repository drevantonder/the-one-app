//! The update path: the signed record at the network's address (`latest`), the platforms a release is for (`platform`),
//! a release's program and its signature (`release`), and `one update`, a device taking the newest build (`update`).
//! Nothing of the network: no onet, app runtime, relay or who's who, so a device stuck on anything else can still take
//! the fix.

pub mod latest;
pub mod platform;
#[cfg(feature = "machine")]
pub mod release;
#[cfg(feature = "machine")]
pub mod update;
