//! The OS's secret store (keyring): the Secret Service on Linux, the Keychain on macOS and the iPhone, the Credential
//! Manager on Windows, under the custom build's name. Values are kept base64, since some stores take only text.

use data_encoding::BASE64;

const SERVICE: &str = one::config::NAME;

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
