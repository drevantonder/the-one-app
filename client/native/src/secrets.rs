//! The OS's secret store (the Secret Service, the Credential Manager, the Keychain), its items named as the Mac and
//! iPhone apps named theirs, so the native app reads those: this app's device keys, one for each user at each address
//! (service `<cli> device`, account `<address> <sub>`, or `signing in` until a sign-in says whose), and the sign-in's
//! refresh token (service `<cli> sign-in`, account `refresh`). Kept as bytes, as they were.

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
mod store {
    fn entry(service: &str, account: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(service, account).ok()
    }

    pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        match entry(service, account).ok_or("the secret store isn't there")?.get_secret() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("the secret store: {e}")),
        }
    }

    pub fn set(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
        entry(service, account).ok_or("the secret store isn't there")?.set_secret(value).map_err(|e| format!("the secret store: {e}"))
    }

    pub fn delete(service: &str, account: &str) {
        if let Some(e) = entry(service, account) {
            let _ = e.delete_credential();
        }
    }
}

/// The Keychain, as the Mac and iPhone apps kept it (the Swift apps' DeviceKeys and Keychain, whose items this reads): on an
/// iPhone readable once it's been unlocked since it started (a widget's button works from the lock screen), on this
/// device only. On an iPhone the device keys are kept in the group Info.plist's `KeychainGroup` names (the App Group's),
/// where the notification extension reads them too; the refresh token is the app's alone. A Mac names no group.
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod store {
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{PasswordOptions, delete_generic_password_options, generic_password, set_generic_password_options};

    /// Where the device keys are kept, as Info.plist says (none run outside a bundle).
    fn group(service: &str) -> Option<String> {
        static GROUP: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        if !service.ends_with(" device") {
            return None;
        }
        GROUP.get_or_init(|| crate::platform::apple::info("KeychainGroup")).clone()
    }

    /// The item's query, in its group.
    fn options(service: &str, account: &str) -> PasswordOptions {
        let mut o = PasswordOptions::new_generic_password(service, account);
        if let Some(group) = group(service) {
            o.set_access_group(&group);
        }
        o
    }

    fn add(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
        let mut o = options(service, account);
        if cfg!(target_os = "ios") || group(service).is_some() {
            let only = SecAccessControl::create_with_protection(Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly), 0).map_err(|e| e.to_string())?;
            o.set_access_control(only);
        }
        set_generic_password_options(value, o).map_err(|e| format!("the Keychain: {e}"))
    }

    pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        const NOT_THERE: i32 = -25300;
        match generic_password(options(service, account)) {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.code() == NOT_THERE => Ok(None),
            Err(e) => Err(format!("the Keychain: {e}")),
        }
    }

    pub fn set(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
        let _ = delete_generic_password_options(options(service, account));
        add(service, account, value)
    }

    pub fn delete(service: &str, account: &str) {
        let _ = delete_generic_password_options(options(service, account));
    }
}

/// Android's Keystore, through Kotlin (android/.../Host.kt): each value sealed by a key that never leaves the phone, in the
/// preferences the Kotlin app kept its device keys in, under the names it gave them (`device:<hash of the id>`, and
/// `device:` until a sign-in says whose), so a phone that had it signed in still has its keys.
#[cfg(target_os = "android")]
mod store {
    use data_encoding::{BASE64, BASE64URL_NOPAD};
    use serde_json::json;
    use sha2::{Digest, Sha256};

    /// The name Kotlin keeps an item under.
    fn name(service: &str, account: &str) -> String {
        if service.ends_with(" device") {
            return match account {
                "signing in" => "device:".into(),
                "this device" => "this-device".into(),
                id => format!("device:{}", BASE64URL_NOPAD.encode(&Sha256::digest(id.as_bytes()))),
            };
        }
        "refresh".into()
    }

    pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        let said = crate::platform::host("get", json!({ "name": name(service, account) }))?;
        Ok(said.as_str().and_then(|s| BASE64.decode(s.as_bytes()).ok()))
    }

    pub fn set(service: &str, account: &str, value: &[u8]) -> Result<(), String> {
        crate::platform::host("set", json!({ "name": name(service, account), "value": BASE64.encode(value) })).map(drop)
    }

    pub fn delete(service: &str, account: &str) {
        let _ = crate::platform::host("delete", json!({ "name": name(service, account) }));
    }
}

/// `<cli> <name>`; a throwaway build's, `<cli> throwaway <name>`, beside the user's own.
fn service(name: &str) -> String {
    let throwaway = if env!("IDENTIFIER").ends_with(".throwaway") { " throwaway" } else { "" };
    format!("{}{throwaway} {name}", one::config::CLI)
}

/// What's kept, if it can be read: a store that can't answer now says nothing is there, and says so in the log.
fn get(name: &str, account: &str) -> Option<Vec<u8>> {
    store::get(&service(name), account).inspect_err(|e| one::logs::record(&format!("reading {name}: {e}"))).ok().flatten()
}

fn set(name: &str, account: &str, value: &[u8]) -> Result<(), String> {
    store::set(&service(name), account, value)
}

fn delete(name: &str, account: &str) {
    store::delete(&service(name), account)
}

/// The key a sign-in here is for, until it knows whose.
const SIGNING_IN: &str = "signing in";

/// `id`'s device key (`<address> <sub>`), if it has one.
pub fn device(id: &str) -> Option<[u8; 32]> {
    get("device", id).and_then(|b| b.try_into().ok())
}

/// `id`'s device key, made the first time: the one its sign-in was for (`signing_in`), if there is one.
pub fn device_made(id: &str) -> Result<[u8; 32], String> {
    // A store that can't answer now isn't one with no key: a new key here would replace the one this device is let in by.
    if let Some(key) = store::get(&service("device"), id)?.and_then(|b| b.try_into().ok()) {
        return Ok(key);
    }
    let key = device(SIGNING_IN).unwrap_or_else(onet::new_secret);
    set("device", id, &key)?;
    delete("device", SIGNING_IN);
    Ok(key)
}

/// The key the next sign-in is for, made the first time, so whoever answers its request can check that sign-in.
pub fn signing_in() -> Result<[u8; 32], String> {
    if let Some(key) = device(SIGNING_IN) {
        return Ok(key);
    }
    let key = onet::new_secret();
    set("device", SIGNING_IN, &key)?;
    Ok(key)
}

/// An iPhone's id of its own, made the first time and kept past signing out (not a user's device key): what signing
/// in again replaces its sign-in under, as the iPhone app's was.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub fn this_device() -> Result<[u8; 32], String> {
    if let Some(key) = device("this device") {
        return Ok(key);
    }
    let key = onet::new_secret();
    set("device", "this device", &key)?;
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

/// A short name for `id`, for its folder: FNV-1a's 64 bits in hex, as the Mac and iPhone apps name it; Android's, the
/// first 8 bytes of its SHA-256 in hex, as the Kotlin app named it.
#[cfg(target_os = "android")]
pub fn folder(id: &str) -> String {
    use sha2::{Digest, Sha256};
    data_encoding::HEXLOWER.encode(&Sha256::digest(id.as_bytes())[..8])
}

#[cfg(not(target_os = "android"))]
pub fn folder(id: &str) -> String {
    let hash = id.bytes().fold(14_695_981_039_346_656_037u64, |h, b| (h ^ b as u64).wrapping_mul(1_099_511_628_211));
    format!("{hash:x}")
}
