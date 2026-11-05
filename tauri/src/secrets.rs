//! The OS's secret store (keyring): the Secret Service on Linux, the Keychain on macOS and the iPhone, the Credential
//! Manager on Windows. Values are kept base64, since some stores take only text.

use data_encoding::BASE64;

const SERVICE: &str = "The One App (Tauri spike)";

fn entry(key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, key).map_err(|e| e.to_string())
}

pub fn get(key: &str) -> Option<Vec<u8>> {
    let text = entry(key).ok()?.get_password().ok()?;
    BASE64.decode(text.as_bytes()).ok()
}

pub fn set(key: &str, value: &[u8]) -> Result<(), String> {
    entry(key)?.set_password(&BASE64.encode(value)).map_err(|e| format!("the secret store: {e}"))
}

pub fn delete(key: &str) {
    if let Ok(e) = entry(key) {
        let _ = e.delete_credential();
    }
}

/// Whether the store answers at all: set, read back and delete a value.
pub fn works() -> Result<(), String> {
    set("probe", b"probe")?;
    let back = get("probe");
    delete("probe");
    (back.as_deref() == Some(b"probe".as_slice())).then_some(()).ok_or_else(|| "the secret store gave back something else".into())
}
