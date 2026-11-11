//! Onboarding: what someone sees on a device before they're in the network, first time or coming back: the first screen
//! (`welcome.tsx`, @one/ui's Welcome, Join, Starting and Unavailable), proving who they are (auth's pages, `pages.tsx`
//! drawn to HTML when @one/ui is built and served by `pages.js`, `render.js` drawing them all to look at; `scope`'s
//! `sign_in_proof`, which a user's devices check a sign-in with), waiting to be let in, and starting (a pairing code and
//! admitting the machine that joins with it: its shapes are here, what serves them the app runtime's, `runtime/pairing.rs`).

#[cfg(not(target_family = "wasm"))]
pub mod join;
mod pairing;

pub use pairing::{JoinReply, JoinRequest, Pairing, PairingCode, pairings};

/// What a computer says while it isn't in a network yet, whatever else is signed in on it (the Mac app's own sign-in is
/// a client's): what the command line, `status` and a call that needs one all tell to do.
pub fn not_joined(device: &str) -> String {
    use custom_build::config::{CLI, YOURS};
    format!("{device} isn't in {YOURS} yet: {CLI} join <code> joins it with a pairing code, or {CLI} create <name> --as <your name> starts a new one")
}
