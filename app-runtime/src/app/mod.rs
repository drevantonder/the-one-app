//! Apps: code anyone deploys into the network (deploy/), which runs on its app runtimes with the keys a call hands them. The
//! runtime (`runtime`, with the shim every app runs inside, `runtime/shim.js`) runs it in workerd; and every request it
//! makes goes through the app runtime's outbound port (`outbound`), which fills in the Vault's secrets for their hosts.

pub mod outbound;
pub mod runtime;
