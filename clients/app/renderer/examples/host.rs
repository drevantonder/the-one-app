//! Checks that this computer's host keeps the renderer's contract with it (`renderer::platform`'s): a window opened,
//! the inspector's `host` check run on it, and what held printed as JSON, exiting 1 where something didn't.
//!
//!     cargo run -p renderer --example host
//!
//! on a desktop: Linux's X11 (`xvfb-run -a` where there's no display), or Windows. Move the pointer over its window
//! while it runs, for the input check. A Mac's windows need AppKit's main thread, which the app runs: there, and on a
//! phone, ask an app's live window the same with the inspector (`one inspector windows`, then its `host` op).

use std::sync::mpsc;

use renderer::geometry::{Pixels, Size};
use renderer::platform::{Event, Native, WindowOptions};

fn main() -> anyhow::Result<()> {
    let native = Native::get()?;
    let (drawn_to, drawn) = mpsc::channel();
    native.run(move |windows| {
        let options = WindowOptions { title: "Host check".into(), app_id: "host-check".into(), size: Size { width: Pixels(480.0), height: Pixels(320.0) }, tag: serde_json::json!({}), panel: None, places: None, frame: None, root: false, icon: None, owner: None, snapshot: None, title_row: false };
        let to = drawn_to.clone();
        let opened = windows.open(options, Err("Checking how this computer's host draws".into()), move |event| {
            if matches!(event, Event::Drawn) {
                let _ = to.send(Ok(()));
            }
        });
        match opened {
            Ok(id) => WINDOW.with(|window| window.set(Some(id))),
            Err(error) => drop(drawn_to.send(Err(format!("{error:#}")))),
        }
    });
    drawn.recv()?.map_err(anyhow::Error::msg)?;
    let (said_to, said) = mpsc::channel();
    native.run(move |windows| {
        let Some(id) = WINDOW.with(std::cell::Cell::get) else { return };
        let request = renderer::inspect::Request::parse(r#"{"v":1,"id":1,"op":"host"}"#).map_err(|refused| anyhow::anyhow!("{refused}"));
        match request {
            Ok(request) => windows.inspect(id, request, Box::new(move |answer| drop(said_to.send(answer)))),
            Err(error) => drop(said_to.send(serde_json::json!({"error": format!("{error:#}")}))),
        }
    });
    let answer = said.recv()?;
    println!("{}", serde_json::to_string_pretty(&answer)?);
    if answer["result"]["ok"] != true {
        std::process::exit(1);
    }
    Ok(())
}

thread_local! {
    /// The window opened, on the windows' thread.
    static WINDOW: std::cell::Cell<Option<renderer::platform::WindowId>> = const { std::cell::Cell::new(None) };
}
