//! Encryption for plugins on a server. A server gives it to plugins as something to import (`Cipher`), and One Core
//! doesn't use it: which artifacts are encrypted, and who holds their keys, is the encryption plugin's. What it does
//! is the client's (`client::cipher`), so a server and a device do the same thing with the same code.
//!
//! An artifact's keys never become anything Elixir can read: a server holds them here, behind a reference, and so
//! no log, error or answer can show them.

use std::path::Path;
use std::sync::RwLock;

use client::cipher::{self, Key};
use iroh::PublicKey;
use rustler::{Atom, Binary, Env, NewBinary, Resource, ResourceArc};

mod atoms {
    rustler::atoms! { ok }
}

/// An artifact's keys on this server, by epoch. With none, the server no longer opens the artifact.
struct Keys(RwLock<Vec<Key>>);

#[rustler::resource_impl]
impl Resource for Keys {}

fn binary<'a>(env: Env<'a>, bytes: &[u8]) -> Binary<'a> {
    let mut binary = NewBinary::new(env, bytes.len());
    binary.as_mut_slice().copy_from_slice(bytes);
    binary.into()
}

/// A backup key a device handed this server, to restore from a backup with while that's asked.
struct BackupKey(iroh::SecretKey);

#[rustler::resource_impl]
impl Resource for BackupKey {}

/// What was encrypted to the key kept in `key_file`, for what `about` says.
fn opened(key_file: &str, about: impl Fn(&str) -> String, sealed: &[u8]) -> Result<Vec<u8>, String> {
    let key = client::key(Path::new(key_file)).map_err(|error| format!("{error:#}"))?;
    let opened = cipher::open(&key, &about(&key.public().to_string()), sealed).map_err(|error| error.to_string())?;
    Ok(opened.to_vec())
}

/// The keys of the artifact named `artifact` that `slot` hands the server whose key is kept in `key_file`.
#[rustler::nif(schedule = "DirtyIo")]
fn keys(key_file: &str, artifact: &str, slot: Binary) -> Result<ResourceArc<Keys>, String> {
    let joined = opened(key_file, |server| cipher::for_server(artifact, server), slot.as_slice())?;
    let given = cipher::split(&joined).map_err(|error| error.to_string())?;
    Ok(ResourceArc::new(Keys(RwLock::new(given))))
}

/// Lets go of an artifact's keys: whatever holds them opens nothing with them from now on.
#[rustler::nif]
fn forget(keys: ResourceArc<Keys>) -> Atom {
    keys.0.write().map(|mut keys| keys.clear()).ok();
    atoms::ok()
}

/// Encrypts a blob under its artifact's newest key. `name` is the blob's whole name in the bucket.
#[rustler::nif(schedule = "DirtyCpu")]
fn wrap<'a>(env: Env<'a>, keys: ResourceArc<Keys>, name: &str, data: Binary) -> Result<Binary<'a>, String> {
    let keys = keys.0.read().map_err(|_| "an earlier call panicked")?;
    let wrapped = cipher::wrap(&keys, name, data.as_slice()).map_err(|error| error.to_string())?;
    Ok(binary(env, &wrapped))
}

/// A blob as it was before `wrap`.
#[rustler::nif(schedule = "DirtyCpu")]
fn unwrap<'a>(env: Env<'a>, keys: ResourceArc<Keys>, name: &str, blob: Binary) -> Result<Binary<'a>, String> {
    let keys = keys.0.read().map_err(|_| "an earlier call panicked")?;
    let data = cipher::unwrap(&keys, name, blob.as_slice()).map_err(|error| error.to_string())?;
    Ok(binary(env, &data))
}

/// What a person typed on their device and it encrypted to the key kept in `key_file`.
#[rustler::nif(schedule = "DirtyIo")]
fn secret<'a>(env: Env<'a>, key_file: &str, sealed: Binary) -> Result<Binary<'a>, String> {
    Ok(binary(env, &opened(key_file, cipher::for_secret, sealed.as_slice())?))
}

/// Encrypts `data` to the machine whose key is `to`, for what `about` says.
#[rustler::nif]
fn seal<'a>(env: Env<'a>, to: &str, about: &str, data: Binary) -> Result<Binary<'a>, String> {
    let to: PublicKey = to.parse().map_err(|_| "that isn't a machine's key")?;
    Ok(binary(env, &cipher::seal(&to, about, data.as_slice()).map_err(|error| error.to_string())?))
}

/// The backup key that `sealed` hands the server whose key is kept in `key_file`, and its public half.
#[rustler::nif(schedule = "DirtyIo")]
fn backup_key(key_file: &str, sealed: Binary) -> Result<(ResourceArc<BackupKey>, String), String> {
    let bytes = opened(key_file, cipher::for_restoring, sealed.as_slice())?;
    let bytes: [u8; 32] = bytes.as_slice().try_into().map_err(|_| "that isn't a backup key")?;
    let key = iroh::SecretKey::from_bytes(&bytes);
    let public = key.public().to_string();
    Ok((ResourceArc::new(BackupKey(key)), public))
}

/// What a backup holds, encrypted to the backup key for what `about` says.
#[rustler::nif(schedule = "DirtyCpu")]
fn backup_open<'a>(
    env: Env<'a>,
    key: ResourceArc<BackupKey>,
    about: &str,
    sealed: Binary,
) -> Result<Binary<'a>, String> {
    Ok(binary(env, &cipher::open(&key.0, about, sealed.as_slice()).map_err(|error| error.to_string())?))
}

rustler::init!("Elixir.Cipher");
