//! The app keeps itself on its network's latest build, and its window on the latest page (`page`), by itself and
//! without asking, as Chrome and VS Code update and start again: the network's only channel is its dev channel, where
//! its person sees each change as soon as it's live. Its servers tell it as the network's update moves
//! (`notifications/one/update`, `OneCore.Release.instructed/1`), with the latest build of each `one` and the build of
//! the edge, which serves the page, and it asks again as it reconnects, for what it missed.
//!
//! A computer's app takes a newer build of itself (on a device that's also a server, its server puts it in place as it
//! takes it, `cli::serving`), or keeps a newer page, and starts again on it (`computer::restart`), its app windows
//! opening again where they were. A phone's app reloads a newer page once it's off screen; its own builds come from
//! its store (TestFlight). Either waits while a window on screen holds something its person typed and hasn't sent: in
//! a web view a field typed in that still holds text, which its page says (`editing`, from `initialization-script.js`),
//! and in a strict window an input with typing to undo (`strict::typed`), until that window is put away or closed, or
//! what's typed is sent.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

/// The windows whose page says they hold something typed, by their label (`hosts::command`): `main`, the Home
/// Screen's, and each app's.
static EDITING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
/// A newer build or page is in place, to start again on, or reload, once nothing typed is in the way.
static DUE: AtomicBool = AtomicBool::new(false);
/// The edge's build the network last said, whose page has been asked for.
static EDGE: Mutex<Option<String>> = Mutex::new(None);
/// One build taken at a time.
#[cfg(desktop)]
static TAKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What the network's update says, as a server tells the app (`notifications/one/update`).
pub async fn heard(update: Value) {
    #[cfg(desktop)]
    if let Some(latest) = update["latest"][cli::serving::name()].as_str() {
        take(latest).await
    }
    if let Some(edge) = update["edge"].as_str() {
        let before = EDGE.lock().expect("nothing panics holding it").replace(edge.to_owned());
        if before.as_deref() != Some(edge) {
            page().await
        }
    }
}

/// Asks the network for what the app missed while it didn't reach it: its latest build, and its page.
pub async fn caught_up() {
    #[cfg(desktop)]
    if let Ok(views) = crate::views() {
        match cli::serving::latest_program(&views.client).await {
            Ok(Some(latest)) => take(&latest).await,
            Ok(None) => {}
            Err(error) => eprintln!("couldn't ask the network for its latest build: {error:#}"),
        }
    }
    page().await
}

/// Takes `latest`, if it's newer than this build: put in place by this device's server, where it's one, or downloaded
/// here.
#[cfg(desktop)]
async fn take(latest: &str) {
    if cli::serving::count(latest) <= cli::serving::count(::client::BUILD) {
        return;
    }
    let _one = TAKING.lock().await;
    // On a device that's also a server, its server puts it in place as it takes it, and the network's update moves on,
    // which the app is told again.
    if !DUE.load(Ordering::Relaxed) && cli::serving::server().is_none() {
        let Ok(views) = crate::views() else { return };
        match cli::serving::update(&views.client).await {
            Ok(said) => eprintln!("{said}"),
            Err(error) => return eprintln!("couldn't take {latest}: {error:#}"),
        }
    }
    // What's started again is the program's file, which is what's checked.
    let Ok(program) = crate::computer::program() else { return };
    let Some(there) = cli::serving::build_of(&program) else { return };
    if cli::serving::count(&there) > cli::serving::count(::client::BUILD) {
        eprintln!("{there} is in place of {}", ::client::BUILD);
        due().await
    }
}

/// Asks the network for its page, which is due once a newer one is kept.
pub async fn page() {
    let Ok(views) = crate::views() else { return };
    let kept = async { crate::page::update(&views.client, &crate::app().shell.home()?).await }.await;
    match kept {
        Ok(true) => due().await,
        Ok(false) => {}
        Err(error) => eprintln!("couldn't ask the network for its page: {error:#}"),
    }
}

async fn due() {
    DUE.store(true, Ordering::Relaxed);
    ready().await
}

/// Window `label`'s page says whether it holds something typed.
pub fn editing(label: &str, editing: bool) {
    let mut typed = EDITING.lock().expect("nothing panics holding it");
    if editing {
        typed.insert(label.to_owned());
    } else {
        typed.remove(label);
    }
    drop(typed);
    crate::spawn(ready());
}

/// Window `label` closed, or went off screen: what it held is no longer in the way.
pub fn gone(label: &str) {
    EDITING.lock().expect("nothing panics holding it").remove(label);
    crate::spawn(ready());
}

/// Starts again on what's due, or on a phone reloads it, unless a window on screen holds something typed, or a phone's
/// window is on screen.
pub async fn ready() {
    if !DUE.load(Ordering::Relaxed) || typing().await {
        return;
    }
    #[cfg(mobile)]
    if crate::views().is_ok_and(|views| views.on_screen()) {
        return;
    }
    if !DUE.swap(false, Ordering::Relaxed) {
        return;
    }
    #[cfg(desktop)]
    {
        eprintln!("starting again on what's new");
        crate::app().shell.restart()
    }
    #[cfg(mobile)]
    {
        eprintln!("reloading the window's new page");
        if let Ok(home) = crate::app().shell.home() {
            crate::page::load(&home);
        }
        crate::app().shell.reload()
    }
}

/// Whether a window on screen holds something its person typed and hasn't sent.
async fn typing() -> bool {
    let shown = crate::views().map(|views| views.on_screen.lock().expect("no view panics").clone()).unwrap_or_default();
    if EDITING.lock().expect("nothing panics holding it").iter().any(|label| shown.contains(label)) {
        return true;
    }
    #[cfg(any(desktop, target_os = "android"))]
    if crate::strict::typed().await {
        return true;
    }
    false
}
