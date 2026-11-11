//! The network's Workers on Cloudflare (auth, the relay, the URL signer, ...) as devices meet them: building and running
//! them in workerd, deploying them (`one cloudflare`, `cli`), oned on Cloudflare's Worker (`worker`), Basin (`basin`) and
//! which build of the Workers and web client to deploy (`newest`).

pub mod basin;
pub mod cli;
pub mod worker;
mod newest;
mod workers;
pub use newest::Newest;
pub use workers::*;
