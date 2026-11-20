//! Encryption's native code in a client: what only a device does. A person's key is made on a device of theirs and never
//! leaves one unencrypted: it's kept encrypted to each device they let in, and what a server is given of it is one
//! artifact's keys, encrypted to that server. What a person types that's secret is encrypted on the device for the
//! servers that open what it's kept in, so nothing that carries a call can read it.
//!
//! None of that is a command of its own. A tool says which of its inputs a device fills in, by the input's `format`
//! in the tool's schema, and whatever calls the tool through a client (an app, the CLI, an agent over MCP) gets
//! them filled in the same way (`client::InputResolver`). These are the ones `Encrypts` fills in:
//!
//! - `secret`: the text given is encrypted for the servers that the app's tool named by `unlocked_on` lists.
//! - `passkey-keys`: the owner's key, encrypted to each of their passkeys that the app's tool named by `passkeys` lists
//!   with a key from its PRF output and that can't unlock it yet.
//!
//! Nothing here hands a key to a server or another device: a person's passkey does that, on the network's sign-in
//! page (`apps/sign-in/edge/page/passkey.ts`). A device only takes its own copy, with the recovery key (`recover`).
//!
//! What encryption is made of (`cipher`) is a server's too: the encryption app's NIF
//! (`apps/encryption/server/nif/encryption`) builds this crate without its feature `client`, which is all of the
//! above, so a server and a device do the same thing with the same code, and the NIF needs nothing of a client.

pub mod cipher;
#[cfg(feature = "client")]
mod device;
#[cfg(feature = "client")]
pub use device::*;
