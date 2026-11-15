//! Encryption for a server's plugins: the encryption plugin's NIF, which its Elixir loads as `Cipher` and any plugin
//! that uses it calls. It names no plugin: each says what it encrypts for (`about`) itself. What it does is
//! `encryption_client::cipher`, which a device runs too, so a server and a device do the same thing with the same
//! code.
//!
//! A key never becomes anything Elixir can read: a server holds keys here, behind a reference, and so no log, error
//! or answer can show them.

use std::sync::RwLock;

use encryption_client::cipher::{self, Key};
use iroh_base::{PublicKey, SecretKey};
use rustler::{Atom, Binary, Env, NewBinary, Resource, ResourceArc};

mod atoms {
    rustler::atoms! { ok }
}

/// Keys this server holds: a machine's own, or what was handed to it, such as an artifact's for each epoch, the
/// first first. With none, they open nothing.
struct Keys(RwLock<Vec<Key>>);

#[rustler::resource_impl]
impl Resource for Keys {}

fn binary<'a>(env: Env<'a>, bytes: &[u8]) -> Binary<'a> {
    let mut binary = NewBinary::new(env, bytes.len());
    binary.as_mut_slice().copy_from_slice(bytes);
    binary.into()
}

fn said(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The newest of `keys` as a machine's key, which what's encrypted to it opens with.
fn machine(keys: &Keys) -> Result<SecretKey, String> {
    let keys = keys.0.read().map_err(|_| "an earlier call panicked")?;
    Ok(SecretKey::from_bytes(keys.last().ok_or("these keys were let go of")?))
}

/// The key kept in `file`, as a machine keeps its own: the one it's reached by.
#[rustler::nif(schedule = "DirtyIo")]
fn key(file: &str) -> Result<ResourceArc<Keys>, String> {
    let bytes = std::fs::read(file).map_err(|error| format!("{file}: {error}"))?;
    let key = cipher::key(&bytes).map_err(|_| format!("{file} doesn't hold a key"))?;
    Ok(ResourceArc::new(Keys(RwLock::new(vec![key]))))
}

/// The public half of the newest of `keys`, as a machine's key is named: what's encrypted to them is encrypted to it.
#[rustler::nif]
fn public(keys: ResourceArc<Keys>) -> Result<String, String> {
    Ok(machine(&keys)?.public().to_string())
}

/// Lets go of keys: whatever holds them opens nothing with them from now on.
#[rustler::nif]
fn forget(keys: ResourceArc<Keys>) -> Atom {
    keys.0.write().map(|mut keys| keys.clear()).ok();
    atoms::ok()
}

/// Encrypts `data` to the machine whose key is `to`, for what `about` says, which is all it then opens for.
#[rustler::nif(schedule = "DirtyCpu")]
fn seal<'a>(env: Env<'a>, to: &str, about: &str, data: Binary) -> Result<Binary<'a>, String> {
    let to: PublicKey = to.parse().map_err(|_| "that isn't a machine's key")?;
    Ok(binary(env, &cipher::seal(&to, about, data.as_slice()).map_err(said)?))
}

/// What was encrypted to the newest of `keys` for what `about` says.
#[rustler::nif(schedule = "DirtyCpu")]
fn open<'a>(env: Env<'a>, keys: ResourceArc<Keys>, about: &str, sealed: Binary) -> Result<Binary<'a>, String> {
    Ok(binary(env, &cipher::open(&machine(&keys)?, about, sealed.as_slice()).map_err(said)?))
}

/// What `open` gives, held as keys rather than given: an artifact's for each epoch, or one key, 32 bytes each.
#[rustler::nif(schedule = "DirtyCpu")]
fn open_keys(keys: ResourceArc<Keys>, about: &str, sealed: Binary) -> Result<ResourceArc<Keys>, String> {
    let opened = cipher::open(&machine(&keys)?, about, sealed.as_slice()).map_err(said)?;
    Ok(ResourceArc::new(Keys(RwLock::new(cipher::split(&opened).map_err(said)?))))
}

/// Encrypts a blob under the newest of an artifact's `keys`. `name` is its whole name in the bucket.
#[rustler::nif(schedule = "DirtyCpu")]
fn wrap<'a>(env: Env<'a>, keys: ResourceArc<Keys>, name: &str, data: Binary) -> Result<Binary<'a>, String> {
    let keys = keys.0.read().map_err(|_| "an earlier call panicked")?;
    Ok(binary(env, &cipher::wrap(&keys, name, data.as_slice()).map_err(said)?))
}

/// A blob as it was before `wrap`.
#[rustler::nif(schedule = "DirtyCpu")]
fn unwrap<'a>(env: Env<'a>, keys: ResourceArc<Keys>, name: &str, blob: Binary) -> Result<Binary<'a>, String> {
    let keys = keys.0.read().map_err(|_| "an earlier call panicked")?;
    Ok(binary(env, &cipher::unwrap(&keys, name, blob.as_slice()).map_err(said)?))
}

rustler::init!("Elixir.Cipher");
