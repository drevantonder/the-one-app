//! How loaded this machine is: memory, CPU, GPU, disk, and the containers it hosts, as `GET /v1/state` and sync's `Hello`
//! say it (`sample`, on a node, takes it by a thread). Device info, like `build`: never an entry.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct Load {
    /// Used memory: what the OS can't hand to a new program without swapping. Linux's total minus available;
    /// Activity Monitor's "Memory Used" on a Mac (app memory, wired and compressed, not cached files).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Space>,
    /// The volume user files live on; total is used plus what's free to a user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<Space>,
    /// Percent busy, oldest first, one sample every 5 seconds.
    pub cpu: Vec<u8>,
    /// The busiest GPU's percent busy, like `cpu`; empty where none reports, or in a container (it sees its host's).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gpu: Vec<u8>,
    /// Whether this machine is itself a container.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub container: bool,
    /// The containers running here.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<Guest>,
    /// What this machine is, for its icon: `laptop`, `cloud` or `always_on`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

/// Bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Space {
    pub used: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct Guest {
    pub name: String,
    /// `system` (an Incus or LXD container) or `podman`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Space>,
}

#[cfg(feature = "node")]
pub mod sample;
#[cfg(feature = "node")]
pub use sample::current;
