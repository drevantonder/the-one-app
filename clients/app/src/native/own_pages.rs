//! The client's own pages, drawn as an app's are (the Recovery Key's sheet, a window that didn't open, signing in, an update), and its sheets.

use super::*;

/// A page of the client's own (`RECOVERY`, `UNOPENED`), called `name`, drawn with the UI library as an app's page is,
/// which it loads as one does, and no styles of its own.
pub(super) async fn client_page(path: &str, name: &str) -> anyhow::Result<Page> {
    let module = Url::parse(CLIENT)?.join(path)?;
    let Some(tables) = tables(&[module.join("/ui/client/tokens.json")?, module.join("/ui/client/styles.json")?]).await? else {
        anyhow::bail!("the UI library needs a newer runtime than this program's")
    };
    Ok(Page { page: module.clone(), module, trusted: false, tables, name: Some(name.to_owned()), icon: None })
}

/// What the client's recovery key window is for: showing the key of this device's person, or of `subject`, a group or
/// the network, or with `typed`, letting this device in with the one they type; and who hears whether it was done.
#[derive(Clone)]
pub(super) struct Sheet {
    pub(super) typed: bool,
    pub(super) subject: Option<String>,
    /// The window of the app that asked, which it's a sheet of (`WindowOptions::owner`).
    pub(super) owner: Option<WindowId>,
    pub(super) done: Arc<Mutex<Option<tokio::sync::oneshot::Sender<bool>>>>,
}

/// Where the client's own pages are, the client's and no app's (`hosts::file`), with the UI library they're drawn with,
/// which is built into this program too.
pub(crate) const CLIENT: &str = "oneapp://client.localhost";

/// The client's own pages, by their paths there: the recovery key window's, and the one in an app's window that didn't
/// open.
pub(crate) const PAGES: &[(&str, &[u8])] = &[
    (RECOVERY, include_bytes!("../recovery.js")),
    (UNOPENED, include_bytes!("../unopened.js")),
    (SIGN_IN, include_bytes!("../sign-in.js")),
    (UPDATE, include_bytes!("../update.js")),
    ("/client/ask.js", include_bytes!("../ask.js")),
];

pub(super) const RECOVERY: &str = "/client/recovery.js";

pub(super) const UNOPENED: &str = "/client/unopened.js";

/// The launcher's while the device isn't in its network, where it signs in, and while its servers say it's behind
/// them, where it updates (`Instead`).
pub(super) const SIGN_IN: &str = "/client/sign-in.js";

pub(super) const UPDATE: &str = "/client/update.js";

/// Shows its person the recovery key, theirs or `subject`'s, or with `typed` asks them for it, in a window of the
/// client's own over the app that asked, which is told only whether they saw it or the device is in now: no app's
/// page is handed a recovery key, as an Apple device shows one in its own system's UI and never in an app.
/// It's boxed, as a window it opens asks the client what a window's page asks it (`hosts::command`), which opens this.
pub(crate) fn recovery(typed: bool, subject: Option<String>, owner: Option<WindowId>) -> std::pin::Pin<Box<dyn Future<Output = Result<bool, String>> + Send>> {
    Box::pin(async move {
        let (done_to, done) = tokio::sync::oneshot::channel();
        let sheet = Sheet { typed, subject, owner, done: Arc::new(Mutex::new(Some(done_to))) };
        let started = Instant::now();
        window(started, "client/recovery.js", None, "", Kind::Sheet(sheet)).await.map_err(|error| format!("{error:#}"))?;
        Ok(done.await.unwrap_or(false))
    })
}
