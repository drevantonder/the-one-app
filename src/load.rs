//! How loaded this machine is: memory, CPU, GPU, disk, and the containers it hosts, as `GET /api/state` and each answer to another device
//! say it (`sample`, on a node, takes it by a thread). Device info, like `build`: never an entry.


#[cfg(feature = "node")]
pub mod sample;
#[cfg(feature = "node")]
pub use sample::current;
