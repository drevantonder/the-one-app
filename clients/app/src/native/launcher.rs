//! The launcher's window: a computer's panel, or a phone's Home Screen, and the client's own page in its place where its page can't be drawn.

use super::*;

/// Opens the launcher's page natively: on a computer in a panel put away until it's summoned (`computer`), on a phone
/// as its root window, under every app's. → its window, once it's open. A computer's whose page this program can't
/// draw, or that didn't open, is a panel that says why, and Telemetry hears it; where its native windows can't start,
/// there's none, and the system's alert says so (`computer::unnative`). A phone's is its page's.
pub(crate) async fn launcher(entrypoint: &str) -> Option<WindowId> {
    let started = Instant::now();
    let (opened_to, opened) = tokio::sync::oneshot::channel();
    let why = match window(started, entrypoint, None, entrypoint, Kind::Launcher(Some(opened_to), None)).await {
        Ok(true) => return opened.await.ok().flatten(),
        Ok(false) => "this One App doesn't draw its page: it needs a newer One App".to_owned(),
        Err(error) => format!("{error:#}"),
    };
    reported(entrypoint, started, &why);
    unopened_launcher(entrypoint, why, false).await
}

/// A phone's launcher that isn't drawn natively is its page's.
#[cfg(mobile)]
pub(super) async fn unopened_launcher(_: &str, _: String, _: bool) -> Option<WindowId> {
    None
}

/// The launcher's panel in place of one whose page didn't open, or `crashed` once it had: its problem box, with Try
/// Again, or Reload where it crashed, on the client's own page (`UNOPENED`), or where that can't be drawn either, why
/// in words alone; where this computer's native windows can't start, none.
#[cfg(desktop)]
pub(super) async fn unopened_launcher(entrypoint: &str, why: String, crashed: bool) -> Option<WindowId> {
    let name = named(entrypoint, None).await;
    let said = if crashed { format!("{name} stopped working: {why}") } else { format!("{name} didn't open: {why}") };
    let unopened = Unopened { entrypoint: entrypoint.to_owned(), subject: None, link: entrypoint.to_owned(), name, why: why.clone(), crashed };
    if let Some(id) = launcher_instead(Instead::Unopened(unopened)).await {
        return Some(id);
    }
    let (shown_to, shown) = tokio::sync::oneshot::channel();
    if let Ok(native) = Native::get() {
        native.run(move |windows| {
            let options = WindowOptions {
                title: NAME.to_owned(),
                app_id: ::client::NAME.to_owned(),
                size: Size { width: Pixels(computer::SIZE.width as f32), height: Pixels(computer::SIZE.height as f32) },
                tag: json!({}),
                panel: Some(computer::CORNER as f32),
                places: crate::app().shell.home().ok().map(|home| home.join("launcher-places.json")),
                frame: None,
                root: false,
                icon: None,
                owner: None,
                snapshot: None,
                title_row: false,
            };
            let _ = shown_to.send(windows.open(options, Err(said), |_| {}).ok());
        });
    }
    let id = shown.await.ok().flatten();
    if id.is_none() {
        computer::unnative(why);
    }
    id
}

/// The launcher's panel `old`, whose page failed as it started, or `crashed` once it had, replaced by one with its
/// problem box (`unopened_launcher`), which shows if it was showing. The old one goes first.
#[cfg(desktop)]
pub(super) fn replace_launcher(entrypoint: String, why: String, crashed: bool, old: Option<WindowId>) {
    let (showing_to, showing) = tokio::sync::oneshot::channel();
    if let (Some(old), Ok(native)) = (old, Native::get()) {
        native.run(move |windows| {
            let _ = showing_to.send(windows.showing(old));
            windows.close(old);
        });
    }
    crate::spawn(async move {
        let showing = showing.await.unwrap_or(false);
        computer::native(unopened_launcher(&entrypoint, why, crashed).await);
        if showing {
            computer::summon();
        }
    });
}

/// What a computer's launcher shows in place of the Home Screen, on a page of the client's own: why the Home Screen's
/// page didn't open, with Try Again; where the device signs in, while it isn't in its network; or Update Required,
/// while its servers say this program is behind them.
pub(crate) enum Instead {
    Unopened(Unopened),
    SignIn,
    Behind,
}

/// Opens the launcher, put away until it's summoned, with `instead` in place of the Home Screen. → its window, once
/// it's open.
#[cfg(desktop)]
pub(crate) async fn launcher_instead(instead: Instead) -> Option<WindowId> {
    let entrypoint = match &instead {
        Instead::Unopened(unopened) => unopened.entrypoint.clone(),
        Instead::SignIn => SIGN_IN.trim_start_matches('/').to_owned(),
        Instead::Behind => UPDATE.trim_start_matches('/').to_owned(),
    };
    let (opened_to, opened) = tokio::sync::oneshot::channel();
    match window(Instant::now(), &entrypoint, None, &entrypoint, Kind::Launcher(Some(opened_to), Some(instead))).await {
        Ok(_) => opened.await.ok().flatten(),
        Err(error) => {
            eprintln!("the launcher's {entrypoint} didn't open: {error:#}");
            None
        }
    }
}
