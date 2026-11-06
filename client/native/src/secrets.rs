//! The OS's secret store (keyring: the Secret Service, the Keychain, the Credential Manager), its items named as the
//! Mac and iPhone apps named theirs, so the native app reads those: this app's device keys, one for each person at each
//! address (service `<cli> device`, account `<address> <sub>`, or `signing in` until a sign-in says whose), and the
//! sign-in's refresh token (service `<cli> sign-in`, account `refresh`). Kept as bytes, as they were.

fn entry(service: &str, account: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(&format!("{} {service}", one::config::CLI), account).ok()
}

fn get(service: &str, account: &str) -> Option<Vec<u8>> {
    entry(service, account)?.get_secret().ok()
}

fn set(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
    entry(service, account).ok_or("the secret store isn't there")?.set_secret(value).map_err(|e| format!("the secret store: {e}"))
}

fn delete(service: &str, account: &str) {
    if let Some(e) = entry(service, account) {
        let _ = e.delete_credential();
    }
}

/// The key a sign-in here is for, until it knows whose.
const SIGNING_IN: &str = "signing in";

/// `id`'s device key (`<address> <sub>`), if it has one.
pub fn device(id: &str) -> Option<[u8; 32]> {
    get("device", id).and_then(|b| b.try_into().ok())
}

/// `id`'s device key, made the first time: the one its sign-in was for (`signing_in`), if there is one.
pub fn device_made(id: &str) -> Result<[u8; 32], String> {
    if let Some(key) = device(id) {
        return Ok(key);
    }
    let key = device(SIGNING_IN).unwrap_or_else(one_client::new_secret);
    set("device", id, &key)?;
    delete("device", SIGNING_IN);
    Ok(key)
}

/// The key the next sign-in is for, made the first time, so the person's devices can let it in by that sign-in.
pub fn signing_in() -> Result<[u8; 32], String> {
    if let Some(key) = device(SIGNING_IN) {
        return Ok(key);
    }
    let key = one_client::new_secret();
    set("device", SIGNING_IN, &key)?;
    Ok(key)
}

/// `id`'s device key goes: a key taken out can't be let in again.
pub fn forget_device(id: &str) {
    delete("device", id);
}

pub fn refresh() -> Option<String> {
    get("sign-in", "refresh").and_then(|b| String::from_utf8(b).ok())
}

pub fn set_refresh(token: &str) -> Result<(), String> {
    set("sign-in", "refresh", token.as_bytes())
}

pub fn forget_refresh() {
    delete("sign-in", "refresh");
}

/// A short name for `id`, for its folder: FNV-1a's 64 bits in hex, as the Mac and iPhone apps name it.
pub fn folder(id: &str) -> String {
    let hash = id.bytes().fold(14_695_981_039_346_656_037u64, |h, b| (h ^ b as u64).wrapping_mul(1_099_511_628_211));
    format!("{hash:x}")
}
