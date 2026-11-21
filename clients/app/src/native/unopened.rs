//! A window whose app didn't open, or stopped: the client's own page in its place, saying why.

use super::*;

/// Opens a window for `entrypoint`, at `link`, for `subject`, that says only why its page isn't there, as Telemetry
/// hears (`open`): one this program can't draw, that didn't open, or that `crashed` once it had. It's the problem box
/// DESIGN.md has, with Try Again, or Reload where it crashed, on the client's own page (`UNOPENED`); where that can't be
/// drawn either, as with no UI library to draw it with, it's said in words alone.
pub(crate) fn unopened(entrypoint: String, subject: Option<String>, link: String, why: String, crashed: bool) {
    crate::spawn(async move {
        let name = named(&entrypoint, subject.as_deref()).await;
        let said = if crashed { format!("{name} stopped working: {why}") } else { format!("{name} didn't open: {why}") };
        // It's the app's window, as its client knows it, so the app opened again brings it forward.
        let unopened = Unopened { entrypoint: entrypoint.clone(), subject: subject.clone(), link: link.clone(), name: name.clone(), why, crashed };
        match window(Instant::now(), &entrypoint, subject.as_deref(), &link, Kind::Unopened(unopened)).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => eprintln!("{entrypoint}'s problem didn't open: {error:#}"),
        }
        let Ok(native) = Native::get() else { return };
        let tag = json!({"entrypoint": entrypoint});
        native.run(move |windows| {
            let options = WindowOptions { title: name.clone(), app_id: ::client::NAME.to_owned(), size: Size { width: Pixels(1024.0), height: Pixels(720.0) }, tag, panel: None, places: None, frame: None, root: false, icon: None, owner: None, snapshot: None, title_row: false };
            if let Err(error) = windows.open(options, Err(said), |_| {}) {
                eprintln!("{name}'s window didn't open: {error:#}");
            }
        });
    });
}

/// `entrypoint`'s name, as its app lists it for `subject`, or its app's where it isn't listed.
pub(super) async fn named(entrypoint: &str, subject: Option<&str>) -> String {
    let listed = crate::hosts::command("native", "apps", json!({"subject": subject})).await.unwrap_or_default();
    let name = listed_page(&listed, entrypoint).and_then(|page| page["name"].as_str()).filter(|name| !name.contains('/'));
    name.unwrap_or_else(|| entrypoint.split('/').next().unwrap_or_default()).to_owned()
}

/// An app's window that didn't open: its entrypoint, who and where it was opened for, its app's name, and why.
#[derive(Clone)]
pub(crate) struct Unopened {
    pub(super) entrypoint: String,
    pub(super) subject: Option<String>,
    pub(super) link: String,
    pub(super) name: String,
    pub(super) why: String,
    /// Whether it crashed after it opened, rather than not opening.
    pub(super) crashed: bool,
}

impl Unopened {
    /// The Home Screen, which didn't open for `why` before its page was known.
    #[cfg(not(mobile))]
    pub(crate) fn launcher(why: String) -> Self {
        Unopened { entrypoint: UNOPENED.trim_start_matches('/').to_owned(), subject: None, link: String::new(), name: "The Home Screen".to_owned(), why, crashed: false }
    }
}
