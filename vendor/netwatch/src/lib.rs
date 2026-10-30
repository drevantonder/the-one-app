//! Networking related utilities

pub mod interfaces;
pub mod ip;
mod ip_family;
pub mod netmon;
#[cfg(not(wasm_browser))]
mod udp;

pub use self::ip_family::IpFamily;
#[cfg(not(wasm_browser))]
pub use self::udp::{UdpSender, UdpSocket};

// The One App: libc defines the routing socket constants only for macOS, though iOS shares them
// (XNU's net/route.h). This stands in for libc on other Apple systems.
#[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
#[allow(dead_code)]
pub(crate) mod rtlibc {
    pub use libc::*;
    use libc::c_int;
    pub const RTM_ADD: c_int = 0x1;
    pub const RTM_DELETE: c_int = 0x2;
    pub const RTM_CHANGE: c_int = 0x3;
    pub const RTM_GET: c_int = 0x4;
    pub const RTM_LOSING: c_int = 0x5;
    pub const RTM_REDIRECT: c_int = 0x6;
    pub const RTM_MISS: c_int = 0x7;
    pub const RTM_LOCK: c_int = 0x8;
    pub const RTM_RESOLVE: c_int = 0xb;
    pub const RTM_NEWADDR: c_int = 0xc;
    pub const RTM_DELADDR: c_int = 0xd;
    pub const RTM_IFINFO: c_int = 0xe;
    pub const RTM_NEWMADDR: c_int = 0xf;
    pub const RTM_DELMADDR: c_int = 0x10;
    pub const RTM_IFINFO2: c_int = 0x12;
    pub const RTM_NEWMADDR2: c_int = 0x13;
    pub const RTM_GET2: c_int = 0x14;
    pub const RTM_VERSION: c_int = 5;
    pub const RTAX_DST: c_int = 0;
    pub const RTAX_GATEWAY: c_int = 1;
    pub const RTAX_NETMASK: c_int = 2;
    pub const RTAX_IFP: c_int = 4;
    pub const RTAX_BRD: c_int = 7;
    pub const RTAX_MAX: c_int = 8;
    pub const RTA_IFP: c_int = 0x10;
    pub const RTF_GATEWAY: c_int = 0x2;
}
#[cfg(not(all(target_vendor = "apple", not(target_os = "macos"))))]
#[allow(unused_imports)]
pub(crate) use libc as rtlibc;
