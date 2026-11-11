//! The update path: the signed record at the network's address (`latest`), the installers that read it (`install`), the
//! platforms a release is for (`platform`), installing a release's program (`release`), and `one update`, a device taking
//! the newest build (`update`).
//! Nothing of the network: no onet, app runtime, relay or who's who, so a device stuck on anything else can still take
//! the fix.

#[cfg(feature = "machine")]
pub mod install;
pub mod latest;
pub mod platform;
#[cfg(feature = "machine")]
pub mod release;
#[cfg(feature = "machine")]
pub mod update;
