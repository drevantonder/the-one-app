//! The app: a window on One Core in a client (`core/client`). Its own page (`page`) draws nothing of its own but its
//! device's session, whether its network answers, and what's under way or wrong with it: it holds the frames. The
//! window always opens there, and follows where the client is with its network (`State`): a device
//! that isn't in its network is sent on to the network's sign-in page, with the code it's let in by, until its person
//! has let it in. Each app is a page of its app's, drawn in a frame of its own, which
//! reaches nothing here but the bridge the window's page gives it: what an app asks comes to `ask`. The launcher
//! is an app too, the one a network lists in the category `launcher`, which the window shows first and under every other.

//!
//! The window is Tauri's on a phone, and on a computer a window and a web view alone (`computer`), as each app's own
//! is (`host`). Either is a `Shell`, which what's here tells and asks of it; what the page asks (`invoke`) is answered
//! by the same commands on both.

#![cfg(feature = "window")]
// What an app's own window asks is answered by the futures the Home Screen's commands are, a deep nest of them (`hosts`).
#![recursion_limit = "256"]

#[cfg(mobile)]
use std::time::Instant;

/// Notifications' native code, which shows them on this device.
use notifications_client as notifications;

pub mod phone;
mod shortcuts;
#[cfg(not(mobile))]
pub mod computer;
#[cfg(not(mobile))]
mod login;
#[cfg(not(mobile))]
pub mod host;
mod hosts;
mod memory;
mod native;

#[cfg(mobile)]
pub(crate) mod page;
// The app's name, as `tauri.conf.json` has it (`build.rs`).
#[cfg(not(mobile))]
include!(concat!(env!("OUT_DIR"), "/name.rs"));
mod updater;

mod app;
mod commands;
mod session;
mod views;
#[cfg(mobile)]
mod mobile;
#[cfg(target_os = "ios")]
mod status_bar;
#[cfg(target_os = "ios")]
mod charger;
#[cfg(target_os = "ios")]
mod edge;

pub(crate) use app::*;
pub(crate) use commands::*;
pub(crate) use session::*;
pub(crate) use views::*;
#[cfg(mobile)]
pub(crate) use mobile::*;

/// The app on a computer: the launcher and its client (`computer`).
#[cfg(not(mobile))]
pub fn run() {
    computer::run()
}

#[cfg(mobile)]
#[cfg_attr(any(target_os = "android", target_os = "ios"), tauri::mobile_entry_point)]
pub fn run() {
    use tauri::http::{Response, StatusCode, header};
    let started = Instant::now();
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init()).plugin(tauri_plugin_deep_link::init());
    // A phone's push, which Linux has none of.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let builder = builder.plugin(notifications::plugin());
    #[cfg(target_os = "ios")]
    let builder = builder.plugin(tauri_plugin_haptics::init());
    #[cfg(target_os = "ios")]
    charger::register();
    builder
        .setup(move |app| {
            // An app's window is drawn natively, in a tao window over this one (`native`).
            #[cfg(target_os = "ios")]
            app.wry_plugin(native::Ios);
            begin(Phone(app.handle().clone()), started);
            // The window opens at its own page at once, which draws the mark until the client has opened behind it,
            // and then follows where the device is: a device that isn't in its network to the page that signs it
            // in. A phone opens the window again at the same address as it's made again (a change of text size),
            // so that's home too, which asks where the device is now.
            spawn(opens());
            links(app)?;
            window(app.handle())?;
            eprintln!("the window was made {} ms after the app started", started.elapsed().as_millis());
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("oneapp", |_, request, responder| {
            spawn(async move {
                let file = match views() {
                    Ok(views) => views.file(&request.uri().to_string()).await.map_err(|error| format!("{error:#}")),
                    Err(error) => Err(error),
                };
                let response = match file {
                    Ok((kind, file)) => Response::builder()
                        .header(header::CONTENT_TYPE, kind)
                        .header(header::CONTENT_SECURITY_POLICY, APP_MAY_LOAD)
                        .header(header::CACHE_CONTROL, "no-store")
                        .body(file),
                    Err(error) => Response::builder().status(StatusCode::NOT_FOUND).body(error.into_bytes()),
                };
                responder.respond(response.expect("these headers are valid"))
            });
        })
        .invoke_handler(tauri::generate_handler![
            apps,
            open_window,
            open_launcher,
            launcher_shown,
            request,
            refresh,
            upload,
            recovery_key,
            recover,
            client,
            serve,
            update,
            visible,
            editing,
            close,
            retry,
            reconnect,
            sign_out,
            own_page::start,
            push,
            settings,
            listen,
            push_token,
            tapped,
            opened,
            navigation_gestures,
            status_bar,
            theme,
            report,
        ])
        .run(tauri::generate_context!())
        .expect("the app starts");
}
