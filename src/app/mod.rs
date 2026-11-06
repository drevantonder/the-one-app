//! Apps: code anyone deploys into the network, which runs on its nodes with the keys a call hands them. A deploy
//! (`deploy`) builds an app into a bundle (`bundle`), a blob its signed deploy names; the runtime (`runtime`, with the
//! shim every app runs inside, `runtime/shim.js`) runs it in workerd; and every request it makes goes through the
//! node's outbound port (`outbound`), which fills in the Vault's secrets for their hosts.

pub mod bundle;
#[cfg(feature = "node")]
pub mod deploy;
#[cfg(feature = "node")]
pub mod outbound;
#[cfg(feature = "node")]
pub mod runtime;
