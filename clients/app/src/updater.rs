//! The app keeps itself on its network's latest build, and its window on the latest page (`page`), by itself and
//! without asking, as Chrome and VS Code update and start again: the network's only channel is its dev channel, where
//! its person sees each change as soon as it's live. Its servers tell it as the network's update moves
//! (`notifications/one/update`, `OneCore.Release.instructed/1`), with the latest build of each `one` and the build of
//! the edge, which serves the page; it asks as it opens, as it reconnects, and as a window comes on screen, at most
//! once a minute, for what it missed; and a computer's app hears its own program replaced, as its device's server or
//! `one update` replaces it, so that alone has it start again, whatever it heard. What it decides it reports
//! (`one.client.update.*`), as Telemetry shows.
//!
//! A computer's app takes a newer build of itself (on a device that's also a server, its server puts it in place as it
//! restarts on it, `cli::serving`), or keeps a newer page, and starts again on it (`computer::restart`), its app windows
//! opening again where they were. A phone's app reloads a newer page once it's off screen; its own builds come from
//! its store (TestFlight). Either waits while a window on screen holds something its person typed and hasn't sent: in
//! a web view a field typed in that still holds text, which its page says (`editing`, from `initialization-script.js`),
//! and in a native window an input with typing to undo (`renderer::typed`), until that window is put away or closed, or
//! what's typed is sent.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

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
/// When the network was last asked what the app missed (`looked`).
static ASKED: Mutex<Option<Instant>> = Mutex::new(None);
/// Whether what it waits for has been reported, once for each time something's due.
static WAITING: AtomicBool = AtomicBool::new(false);

/// Starts keeping the app on what's latest, as it opens: a computer's hears its own program replaced, and it asks the
/// network what it has.
pub fn start() {
    #[cfg(desktop)]
    replaced();
    looked();
}

/// A window came on screen, or the app opened: the network is asked what the app missed, if it wasn't in the last
/// minute.
pub fn looked() {
    let mut asked = ASKED.lock().expect("nothing panics holding it");
    if asked.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
        return;
    }
    *asked = Some(Instant::now());
    crate::spawn(caught_up());
}

/// Says what it decided to Telemetry, as `name`.
pub(crate) fn report(name: &str, attributes: Value) {
    eprintln!("{name} {attributes}");
    if let Ok(views) = crate::views() {
        views.client.report(json!({"name": name, "attributes": attributes}));
    }
}

/// Hears this program's file replaced, by its device's server, `one update` or anyone, and starts again on it once
/// it's a newer build (`in_place`), whether or not the network said so: the system says each change to its folder
/// (FSEvents, inotify, ReadDirectoryChangesW).
#[cfg(desktop)]
fn replaced() {
    use notify::{RecursiveMode, Watcher};
    let Ok(program) = cli::serving::program() else { return };
    let (Some(folder), Some(name)) = (program.parent().map(std::path::Path::to_owned), program.file_name().map(ToOwned::to_owned)) else { return };
    let heard = move |event: notify::Result<notify::Event>| {
        // Opening it isn't changing it: inotify says each open, and asking its build (`one --version`) opens it, which
        // on Linux heard itself over and over, hundreds of programs started a second.
        if event.is_ok_and(|event| !event.kind.is_access() && event.paths.iter().any(|path| path.file_name() == Some(&name))) {
            crate::spawn(in_place())
        }
    };
    let watching = notify::recommended_watcher(heard).and_then(|mut watcher| watcher.watch(&folder, RecursiveMode::NonRecursive).map(|()| watcher));
    match watching {
        // It hears for as long as the app runs.
        Ok(watcher) => std::mem::forget(watcher),
        Err(error) => eprintln!("the app doesn't hear its program replaced: {error}"),
    }
}

/// What the network's update says, as a server tells the app (`notifications/one/update`).
pub async fn heard(update: Value) {
    report("one.client.update.heard", json!({"one.build.latest": update["latest"][cli_name()], "one.edge.build": update["edge"]}));
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
    // On a device that's also a server, its server puts it in place as it restarts on it, which is heard (`replaced`).
    if cli::serving::server().is_none() {
        let _one = TAKING.lock().await;
        if !DUE.load(Ordering::Relaxed) {
            let Ok(views) = crate::views() else { return };
            match cli::serving::update(&views.client).await {
                Ok(said) => eprintln!("{said}"),
                Err(error) => return report("one.client.update.failed", json!({"one.build.latest": latest, "error": format!("{error:#}")})),
            }
        }
    }
    in_place().await
}

/// Whether the program's file is a newer build than the one running, which is then due: what's started again is that
/// file.
#[cfg(desktop)]
async fn in_place() {
    let Ok(program) = cli::serving::program() else { return };
    let Ok(Some(there)) = tokio::task::spawn_blocking(move || cli::serving::build_of(&program)).await else { return };
    // The system says each change to the program's folder, often several for one replacement: the first is taken.
    if cli::serving::count(&there) > cli::serving::count(::client::BUILD) && !DUE.swap(true, Ordering::Relaxed) {
        report("one.client.update.due", json!({"one.build.next": there}));
        WAITING.store(false, Ordering::Relaxed);
        ready().await
    }
}

/// Asks the network for its page, which is due once a newer one is kept: one ask at a time, as each keeps what it
/// downloads in the same folder (`page::update`).
pub async fn page() {
    static ASKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = ASKING.lock().await;
    let Ok(views) = crate::views() else { return };
    let kept = async { crate::page::update(&views.client, &crate::app().shell.home()?).await }.await;
    match kept {
        Ok(true) => {
            report("one.client.update.due", json!({"one.page": true}));
            due().await
        }
        Ok(false) => {}
        Err(error) => eprintln!("couldn't ask the network for its page: {error:#}"),
    }
}

async fn due() {
    DUE.store(true, Ordering::Relaxed);
    WAITING.store(false, Ordering::Relaxed);
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
    if !DUE.load(Ordering::Relaxed) {
        return;
    }
    if let Some(typed) = typing().await {
        if !WAITING.swap(true, Ordering::Relaxed) {
            report("one.client.update.waiting", json!({"one.typed_in": typed}));
        }
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
        // What it reports is kept, and told from the next start where it isn't before.
        report("one.client.update.restart", json!({}));
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

/// Which window on screen holds something its person typed and hasn't sent, if one does: its label, or `native`.
async fn typing() -> Option<String> {
    let shown = crate::views().map(|views| views.on_screen.lock().expect("no view panics").clone()).unwrap_or_default();
    if let Some(label) = EDITING.lock().expect("nothing panics holding it").iter().find(|label| shown.contains(*label)) {
        return Some(label.clone());
    }
    if crate::native::typed().await {
        return Some("native".to_owned());
    }
    None
}

/// This program's name among the network's builds, on a computer.
fn cli_name() -> String {
    #[cfg(desktop)]
    return cli::serving::name();
    #[cfg(mobile)]
    return String::new();
}
