//! The client for the iPhone's notification extension (Swift, by uniffi): it opens a note pushed to this device with
//! the device's key, and nothing else, in the extension's few megabytes.

uniffi::setup_scaffolding!();

/// Why a note didn't open, in words a user reads.
#[derive(Debug, uniffi::Error)]
pub enum OneError {
    Failed { reason: String },
}

impl std::fmt::Display for OneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let OneError::Failed { reason } = self;
        f.write_str(reason)
    }
}

impl std::error::Error for OneError {}

impl From<one_client::Error> for OneError {
    fn from(e: one_client::Error) -> Self {
        OneError::Failed { reason: e.to_string() }
    }
}

fn failed(message: &str) -> OneError {
    OneError::Failed { reason: message.into() }
}

/// A note pushed to this device, opened: what to show, its app (the notifications it's grouped with), what a tap opens (`link`, as every platform's), and the code of a device asking to join, which its
/// notification answers with Approve and Not Me.
#[derive(uniffi::Record)]
pub struct Note {
    pub title: String,
    pub body: String,
    pub app: String,
    pub link: Option<String>,
    pub join: Option<String>,
}

/// Opens a note pushed to the device whose key is `secret` (`n` in the push, base64url). It only opens: nothing
/// connects and nothing starts, so a notification extension calls it in its few megabytes.
#[uniffi::export]
pub fn open_note(secret: Vec<u8>, pushed: String) -> Result<Note, OneError> {
    let secret: [u8; 32] = secret.try_into().map_err(|_| failed("a device's key is 32 bytes"))?;
    let (n, _) = one_client::open_note(secret, &pushed)?;
    let (link, join) = (n.link(), n.join().map(str::to_string));
    Ok(Note { title: n.title, body: n.body, app: n.app, link, join })
}
