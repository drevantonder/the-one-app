//! The app as it runs: its window (`Shell`), where the device is with its network, and its views.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use ::client::{Client, State};
use serde_json::Value;

use crate::Views;

/// What's the window's own, which differs by device: Tauri's on a phone (`Phone`), and on a computer the launcher's
/// (`computer`). What's here tells it and asks it only this.
pub(crate) trait Shell: Send + Sync + 'static {
    /// The folder the client is kept in.
    fn home(&self) -> anyhow::Result<std::path::PathBuf>;
    /// Tells the window's pages `event`, as Tauri's events: on a computer the launcher's, and each app's own window's
    /// those it hears too (`hosts`).
    fn emit(&self, event: &str, payload: Value);
    /// Opens the window again at its own page (`home`), and on a computer closes each app's own window.
    fn reload(&self);
    /// Opens `url` in the device's own browser.
    fn open_url(&self, url: &str) -> Result<(), String>;
    /// Starts the app again, on the build it was just updated to.
    fn restart(&self);
    /// The `one:` link the window opens once its page asks where it is (`start`): what the app was started with.
    fn opened_at(&self) -> Option<String>;
    /// Has the device hear its person's notifications (`listen`).
    fn listen(&self, client: Arc<Client>);
    /// Light or dark, or as the device is, for what's the window's own and not its page (`theme`).
    fn theme(&self, theme: Option<Theme>);
}

/// Light or dark, as Colour Mode has it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Theme {
    Light,
    Dark,
}

/// The app as it runs: its window (`Shell`), where the device is with its network (`start`), the Colour Mode chosen
/// on it (`chosen`), when it started, and its views once its client has opened.
pub(crate) struct App {
    pub(crate) shell: Box<dyn Shell>,
    /// Where this device is with its network, as the window follows it (`follows`), with how many moves it has
    /// made: nothing until the client has opened, and a client that couldn't open at all is still opening.
    pub(crate) start: tokio::sync::watch::Sender<Option<(u64, State)>>,
    /// The Colour Mode chosen on this device, as the last page to choose it said, for each page that opens after
    /// (`start`), and told to every page that's open.
    pub(crate) chosen: Mutex<Option<Option<Theme>>>,
    pub(crate) started: Instant,
    pub(crate) views: OnceLock<Views>,
}

static APP: OnceLock<App> = OnceLock::new();

/// The app, once it has started (`begin`).
pub(crate) fn app() -> &'static App {
    APP.get().expect("the app has started")
}

/// Starts the app in `shell`, as it began at `started`.
pub(crate) fn begin(shell: impl Shell, started: Instant) {
    let app = App { shell: Box::new(shell), start: tokio::sync::watch::Sender::new(None), chosen: Mutex::default(), started, views: OnceLock::new() };
    if APP.set(app).is_err() {
        panic!("the app starts once");
    }
}

/// The app's views, once its client has opened.
pub(crate) fn views() -> Result<&'static Views, String> {
    app().views.get().ok_or_else(|| "the app hasn't opened".to_owned())
}

/// Runs `future` on the app's runtime: Tauri's on a phone, and on a computer its own (`computer`).
pub(crate) fn spawn(future: impl std::future::Future<Output = ()> + Send + 'static) {
    #[cfg(mobile)]
    tauri::async_runtime::spawn(future);
    #[cfg(not(mobile))]
    crate::computer::runtime().spawn(future);
}
