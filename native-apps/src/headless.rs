//! The app with no window, for what the platform runs beside it when nothing's open (a Windows Widgets board provider,
//! the app woken by a notification in the background, a home-screen widget's button): the same program, signed in as the window is,
//! answering one `kind` of thing, given as JSON, with JSON. Windows' helper starts it as `--headless <kind> <json>`
//! (an answer on stdout); Swift and Kotlin call `run_here` across FFI or JNI. Whatever a platform would otherwise work
//! out itself (a note's link, a widget's id or headline) is worked out here, so they only show it.
//!
//! - `call` `{ app, tool, input }`: runs a tool as the user, then keeps the widgets again → `{ "ok": … }`
//! - `widgets` `{}`: the pages' widgets kept again where the platform's own read them (`crate::widgets_dir`) → `{ "dir" }`
//! - `open-note` `{ n }`: a note sent to this device, opened with its key → `{ id, title, body, app, link, join }`,
//!   `link` what a tap opens and `join` the code of a device asking to join, if it does
//! - `join` `{ code, approve }`: a device asking to join, let in or turned down → `{ "ok": null }`
//! - `register-notifications` `{ via, address }`: this device's push address registered with the notifications Worker → `{ "ok": null }`
//! - `open` `{ link }`: a link of the app's, opened in its window (a tapped notification's)
//!
//! Each fails as `{ "error": "…" }`. On a computer each is asked of its onet, as the window asks it (`crate::here`). On a
//! phone, while the app runs, it answers these itself with its own client, already connected (`serve`): the command asks
//! it over a loopback port its folder names, with a token only this user can read there, and starts the client itself
//! only when no app answers.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

/// Where the window keeps what it keeps (Tauri's app local data folder), found with no window.
pub fn data() -> PathBuf {
    match crate::APP.get() {
        Some(app) => app.data.clone(),
        None => crate::platform::data_without_window().unwrap_or_else(|| PathBuf::from(".")),
    }
}

/// Answers `args` if they ask for one of these (`--headless <kind> <json>`) → whether they did.
pub fn answer(args: &[String]) -> bool {
    if args.get(1).map(String::as_str) != Some("--headless") {
        return false;
    }
    let kind = args.get(2).cloned().unwrap_or_default();
    let input: Value = args.get(3).and_then(|j| serde_json::from_str(j).ok()).unwrap_or_else(|| json!({}));
    let out = running(&kind, &input).unwrap_or_else(|| answered(tauri::async_runtime::block_on(run(&kind, &input, None))));
    println!("{out}");
    true
}

fn answered(out: Result<Value, String>) -> Value {
    out.unwrap_or_else(|e| json!({ "error": e }))
}

/// The running app's answer, if one runs here and answers.
fn running(kind: &str, input: &Value) -> Option<Value> {
    let said = std::fs::read_to_string(data().join("headless")).ok()?;
    let (port, token) = said.trim().split_once(' ')?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port.parse().ok()?));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok()?;
    writeln!(stream, "{}", json!({ "token": token, "kind": kind, "input": input })).ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

/// The running app answering these for what runs beside it (`running`): a loopback port, and a token, in `headless` in
/// its folder, which only this user reads.
#[cfg(desktop)]
pub fn serve() {
    let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", 0)) else { return };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else { return };
    let token = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>());
    let file = data().join("headless");
    let _ = std::fs::create_dir_all(data());
    let _ = std::fs::write(&file, format!("{port} {token}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
    }
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let token = token.clone();
            tauri::async_runtime::spawn(async move {
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    return;
                }
                let asked: Value = serde_json::from_str(&line).unwrap_or_default();
                let out = if asked["token"].as_str() == Some(token.as_str()) {
                    run_here(asked["kind"].as_str().unwrap_or_default(), &asked["input"]).await
                } else {
                    json!({ "error": "Not this app's." })
                };
                let _ = writeln!(&stream, "{out}");
            });
        }
    });
}

/// The user signed in here, and this device's key for them: `(address, "<address> <user>", key)`.
#[cfg(mobile)]
fn signed_in(data: &std::path::Path) -> Result<(String, String, [u8; 32]), String> {
    let address = crate::address_in(data).ok_or("Sign in first.")?;
    let user = crate::kept_in(data, "user").ok_or("Sign in first.")?;
    let id = format!("{address} {user}");
    let key = crate::secrets::device(&id).ok_or("Sign in first.")?;
    Ok((address, id, key))
}

/// The running app's device if it's been let in, else one started from what it keeps.
#[cfg(mobile)]
async fn device(mine: Option<Arc<onet::Device>>) -> Result<crate::Reach, String> {
    if let Some(d) = mine {
        return Ok(crate::Reach::Device(d));
    }
    let data = data();
    let (address, id, key) = signed_in(&data)?;
    let device = crate::client(&data, &address, &id, key);
    match device.connect().await {
        Ok(true) => Ok(crate::Reach::Device(Arc::new(device))),
        Ok(false) => Err("This device hasn't been approved yet.".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// A computer's onet.
#[cfg(desktop)]
async fn device(_: Option<Arc<onet::Device>>) -> Result<crate::Reach, String> {
    Ok(crate::Reach::Onet)
}

/// One of them, in this process: in the running app with its device if it's been let in yet, or else one started
/// from what it keeps (an iPhone's widget's button, which may start it in the background).
pub async fn run_here(kind: &str, input: &Value) -> Value {
    #[cfg(mobile)]
    let mine = crate::APP.get().and_then(|a| a.device().ok());
    #[cfg(desktop)]
    let mine = None;
    answered(run(kind, input, mine).await)
}

/// One of them, with `mine`, the running app's device, or else one started here.
async fn run(kind: &str, input: &Value, mine: Option<Arc<onet::Device>>) -> Result<Value, String> {
    let text = |key: &str| input[key].as_str().map(str::to_string).ok_or_else(|| format!("{kind} wants {key}: see native-apps/src/headless.rs"));
    match kind {
        "call" => {
            let device = device(mine).await?;
            let id = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 8]>());
            let asked = json!({ "id": id, "app": text("app")?, "tool": text("tool")?, "input": input["input"], "client": crate::platform::CLIENT });
            let out = crate::node(&device, "POST", "/api/call", Some(asked)).await?;
            // What a tool did shows on the widgets.
            crate::widgets_changed(&device).await;
            Ok(json!({ "ok": out["ok"] }))
        }
        "widgets" => {
            let dir = crate::platform::widgets_dir();
            crate::widgets::refresh(&device(mine).await?, &dir).await?;
            Ok(json!({ "dir": dir }))
        }
        "open-note" => {
            #[cfg(mobile)]
            let (note, _) = onet::open_note(signed_in(&data())?.2, &text("n")?).map_err(|e| e.to_string())?;
            // Sealed to the computer's key, which its onet opens it with.
            #[cfg(desktop)]
            let note: notifications::Note = serde_json::from_value(crate::here::asked("/api/notifications/open", Some(json!({ "n": text("n")? })), 30).await?["note"].take()).map_err(|e| e.to_string())?;
            let mut out = json!({ "id": note.id, "title": note.title, "body": note.body, "app": note.app, "link": note.link(), "join": note.join() });
            // Absent, not null: Kotlin's optString reads a null as "null".
            out.as_object_mut().unwrap().retain(|_, v| !v.is_null());
            Ok(out)
        }
        "join" => {
            // A note's code is auth's to write and nobody compares it here: only the short one is taken, which lets in only
            // one of this user's own devices whose sign-in names it, and never someone else's or one that would take
            // another device out, each of which has a card to read first.
            let code = crate::device_code(&text("code")?);
            let code = code.get(..key::JOIN_CODE_SHORT).unwrap_or(&code).to_string();
            let device = device(mine).await?;
            match input["approve"] == true {
                true => {
                    let about = crate::node(&device, "POST", "/api/devices/approve", Some(json!({ "code": code }))).await?;
                    if !scope::settings::approves_from_a_note(&about) {
                        return Err("Open Settings › Devices to answer it.".to_string());
                    }
                    crate::node(&device, "POST", "/api/devices/approve", Some(json!({ "code": code, "confirm": true }))).await?
                }
                false => crate::node(&device, "POST", "/api/devices/refuse", Some(json!({ "code": code }))).await?,
            };
            Ok(json!({ "ok": null }))
        }
        "register-notifications" => {
            let address = notifications::Address { via: text("via")?, address: text("address")?, ..Default::default() };
            device(mine).await?.register_notifications(&address).await?;
            Ok(json!({ "ok": null }))
        }
        "open" => {
            crate::APP.get().ok_or("The app hasn't started.")?;
            crate::opened(&text("link")?);
            Ok(json!({ "ok": null }))
        }
        _ => Err(format!("{kind} isn't something this answers: see native-apps/src/headless.rs")),
    }
}
