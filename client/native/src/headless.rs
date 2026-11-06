//! The app with no window, for what the platform runs beside it when nothing's open (a Windows Widgets board provider,
//! a push woken in the background): the same program, signed in as the window is, answering one thing as JSON on
//! stdout and ending.
//!
//! - `--call <app> <tool> <input JSON>`: runs a tool as the person → `{ "ok": … }` or `{ "error": "…" }`
//! - `--widgets`: the pages' widgets kept again where the platform's own read them (`widgets`) → `{ "dir": "…" }`
//! - `--open-note <pushed>`: a pushed note (its `n`), opened with this device's key → the note, or `{ "error" }`
//! - `--answer-join <code> approve|refuse`: a device asking to join, let in or turned down → `{ "ok": null }`
//! - `--register-push <via> <address>`: this device's push address (a WNS channel, from Windows' helper) registered
//!   with the push Worker, which carries its notes → `{ "ok": null }`

use std::path::PathBuf;

use serde_json::{Value, json};

/// Where the window keeps what it keeps (Tauri's app local data folder), found with no window.
pub fn data() -> PathBuf {
    dirs_local().unwrap_or_else(|| PathBuf::from(".")).join(crate::identifier())
}

fn dirs_local() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    return std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(not(any(windows, target_os = "macos")))]
    return std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
}

/// Answers `args` if they ask for one of these → whether they did.
pub fn answer(args: &[String]) -> bool {
    let Some(what) = args.get(1).filter(|a| a.starts_with("--") && ["--call", "--widgets", "--open-note", "--answer-join", "--register-push"].contains(&a.as_str())) else {
        return false;
    };
    let out = tauri::async_runtime::block_on(run(what, &args[2..])).unwrap_or_else(|e| json!({ "error": e }));
    println!("{out}");
    true
}

/// The person signed in here, and this device's key for them.
fn signed_in(data: &std::path::Path) -> Result<(String, String, [u8; 32]), String> {
    let read = |name: &str| std::fs::read_to_string(data.join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let address = crate::env("ADDRESS").or_else(|| Some(one::config::ADDRESS.to_string()).filter(|a| !a.is_empty())).or_else(|| read("address")).ok_or("Sign in first.")?;
    let user = read("user").ok_or("Sign in first.")?;
    let id = format!("{address} {user}");
    let key = crate::secrets::device(&id).ok_or("Sign in first.")?;
    Ok((address, id, key))
}

async fn device(data: &std::path::Path) -> Result<one_client::Device, String> {
    let (address, id, key) = signed_in(data)?;
    let device = one_client::Device::new(key, &address, &data.join("client").join(crate::secrets::folder(&id)).to_string_lossy()).keeping();
    match device.connect().await {
        Ok(true) => Ok(device),
        Ok(false) => Err("This device hasn't been approved yet.".into()),
        Err(e) => Err(e.to_string()),
    }
}

async fn post(device: &one_client::Device, path: &str, body: Value) -> Result<Value, String> {
    let r = device.fetch("POST", path, vec![("content-type".into(), "application/json".into())], serde_json::to_vec(&body).unwrap()).await.map_err(|e| e.to_string())?;
    let status = r.status;
    let out: Value = serde_json::from_slice(&r.body.bytes().await.map_err(|e| e.to_string())?).unwrap_or_default();
    match out["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None if !(200..300).contains(&status) => Err("That didn’t work. Try again.".into()),
        None => Ok(out),
    }
}

async fn run(what: &str, rest: &[String]) -> Result<Value, String> {
    let data = data();
    let arg = |i: usize| rest.get(i).cloned().ok_or_else(|| format!("{what} wants more: see client/native/src/headless.rs"));
    match what {
        "--call" => {
            let input: Value = serde_json::from_str(&rest.get(2).cloned().unwrap_or_else(|| "{}".into())).map_err(|e| e.to_string())?;
            let id = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 8]>());
            let out = post(&device(&data).await?, "/v1/call", json!({ "id": id, "app": arg(0)?, "op": arg(1)?, "input": input, "client": crate::client_name() })).await?;
            Ok(json!({ "ok": out["ok"] }))
        }
        "--widgets" => {
            let dir = data.join("widgets");
            crate::widgets::refresh(&device(&data).await?, &dir).await?;
            Ok(json!({ "dir": dir }))
        }
        "--open-note" => {
            let (_, _, key) = signed_in(&data)?;
            let (note, by) = one_client::open_note(key, &arg(0)?).map_err(|e| e.to_string())?;
            let mut out = serde_json::to_value(note).unwrap();
            out["by"] = json!(by);
            Ok(out)
        }
        "--answer-join" => {
            let code: String = arg(0)?.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            let device = device(&data).await?;
            match arg(1)?.as_str() {
                "approve" => post(&device, "/v1/devices/approve", json!({ "code": code, "confirm": true })).await?,
                _ => post(&device, "/v1/devices/refuse", json!({ "code": code })).await?,
            };
            Ok(json!({ "ok": null }))
        }
        "--register-push" => {
            let address = one::push::Address { via: arg(0)?, address: arg(1)?, ..Default::default() };
            device(&data).await?.register_push(&address).await.map_err(|e| e.to_string())?;
            Ok(json!({ "ok": null }))
        }
        _ => Err("Unknown".into()),
    }
}
