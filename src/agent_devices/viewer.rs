//! The viewer: a page to watch and use a device of another kind than linux (Linux devices have Cua's), served on
//! VIEWER_PORT over HTTPS on the tailnet, as theirs is. It shows the device's screen as fast as it can be taken, and
//! sends clicks, typing and keys back, where the kind takes them. A link's ticket is what lets someone in, for as long
//! as the link works. And VNC input, as Windows takes it.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use super::Devices;
use super::kinds::{Action, Vm};

pub const VIEWER_PORT: u16 = 32099;

/// The tickets of links given, each the device it lets someone watch, and until when.
static TICKETS: Mutex<Option<HashMap<String, (String, jiff::Timestamp)>>> = Mutex::new(None);

/// A link to watch and use a device for `minutes`.
pub fn vm_view_url(v: &Vm, minutes: i64) -> Result<String> {
    let port = VIEWER_PORT.to_string();
    super::linux::tailscale(&["serve", "--bg", &format!("--https={port}"), &format!("http://127.0.0.1:{port}")])?;
    let name = super::linux::tailnet_name()?;
    let t = super::random_hex(24);
    TICKETS.lock().unwrap().get_or_insert_default().insert(t.clone(), (v.name.clone(), super::now() + jiff::SignedDuration::from_mins(minutes)));
    Ok(format!("https://{name}:{port}/{t}/"))
}

fn refused(status: StatusCode, why: impl Into<String>) -> Response {
    (status, why.into()).into_response()
}

impl Devices {
    /// Serves the viewer for as long as this runs.
    pub(super) fn serve_viewer(self: Arc<Self>) {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(err) => return tracing::warn!("the viewer: {err}"),
        };
        runtime.block_on(async move {
            let app = axum::Router::new()
                .route("/{ticket}/", get(page))
                .route("/{ticket}/screen", get(screen))
                .route("/{ticket}/act", post(act))
                .with_state(self);
            match tokio::net::TcpListener::bind(("127.0.0.1", VIEWER_PORT)).await {
                Ok(ln) => {
                    if let Err(err) = axum::serve(ln, app).await {
                        tracing::warn!("the viewer: {err}");
                    }
                }
                Err(err) => tracing::warn!("the viewer: {err}"),
            }
        });
    }

    /// The device a ticket lets someone watch, which counts as a call to it.
    fn ticketed(&self, ticket: &str) -> Result<Vm, Box<Response>> {
        let device = TICKETS.lock().unwrap().get_or_insert_default().get(ticket).filter(|(_, until)| super::now() < *until).map(|(d, _)| d.clone());
        let Some(device) = device else { return Err(Box::new(refused(StatusCode::FORBIDDEN, "This link has expired: ask for another."))) };
        let Some(v) = self.vm_named(&device) else { return Err(Box::new(refused(StatusCode::NOT_FOUND, "That device is gone."))) };
        self.touch(&v.name); // someone's watching it
        Ok(v)
    }
}

async fn page(State(ds): State<Arc<Devices>>, UrlPath(ticket): UrlPath<String>) -> Response {
    tokio::task::spawn_blocking(move || {
        let mut v = match ds.ticketed(&ticket) {
            Ok(v) => v,
            Err(r) => return *r,
        };
        let k = v.kind();
        if k.running(&ds, &v) {
            let _ = ds.awake(&mut v); // a paused one carries on for whoever looks
        }
        let html = PAGE.replace("{name}", &v.name).replace("{input}", if k.input() { "true" } else { "false" }).replace("{limits}", &serde_json::to_string(k.limits()).unwrap_or_default());
        ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], html).into_response()
    })
    .await
    .unwrap_or_else(|_| refused(StatusCode::INTERNAL_SERVER_ERROR, "failed"))
}

async fn screen(State(ds): State<Arc<Devices>>, UrlPath(ticket): UrlPath<String>) -> Response {
    tokio::task::spawn_blocking(move || {
        let v = match ds.ticketed(&ticket) {
            Ok(v) => v,
            Err(r) => return *r,
        };
        let k = v.kind();
        if !k.running(&ds, &v) {
            return refused(StatusCode::CONFLICT, format!("{} is stopped.", v.name));
        }
        let shot = k.screen(&ds, &v).and_then(|img| Ok((super::picture::jpeg_of(&img, 0)?.0, img.width(), img.height())));
        match shot {
            Ok((jpeg, w, h)) => {
                ([(header::CONTENT_TYPE, "image/jpeg".to_string()), (header::CACHE_CONTROL, "no-store".to_string()), (header::HeaderName::from_static("x-width"), w.to_string()), (header::HeaderName::from_static("x-height"), h.to_string())], jpeg)
                    .into_response()
            }
            Err(err) => refused(StatusCode::BAD_GATEWAY, format!("{err:#}")),
        }
    })
    .await
    .unwrap_or_else(|_| refused(StatusCode::INTERNAL_SERVER_ERROR, "failed"))
}

async fn act(State(ds): State<Arc<Devices>>, UrlPath(ticket): UrlPath<String>, body: axum::body::Bytes) -> Response {
    tokio::task::spawn_blocking(move || {
        let v = match ds.ticketed(&ticket) {
            Ok(v) => v,
            Err(r) => return *r,
        };
        let Ok(a) = serde_json::from_slice::<Action>(&body) else { return refused(StatusCode::BAD_REQUEST, "an action, posted") };
        match v.kind().act(&ds, &v, &a) {
            Ok(()) => StatusCode::OK.into_response(),
            Err(err) => refused(StatusCode::BAD_REQUEST, format!("{err:#}")),
        }
    })
    .await
    .unwrap_or_else(|_| refused(StatusCode::INTERNAL_SERVER_ERROR, "failed"))
}

/// The page: the screen, filling the window, and what the person does on it sent back as actions.
const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{name}</title>
<style>
  html, body { margin: 0; height: 100%; background: #111; overflow: hidden; }
  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; user-select: none; -webkit-user-drag: none; outline: none; }
  #note { position: absolute; left: 0; right: 0; bottom: 0; padding: 6px 12px; font: 13px system-ui; color: #ccc; background: #000a; display: none; }
</style>
<img id="s" tabindex="0" alt="">
<div id="note"></div>
<script>
  const input = {input}, s = document.getElementById("s"), note = document.getElementById("note");
  let w = 0, h = 0;
  if (!input) { note.textContent = {limits}; note.style.display = "block"; }
  async function loop() {
    for (;;) {
      try {
        const r = await fetch("screen", { cache: "no-store" });
        if (r.ok) {
          w = +r.headers.get("X-Width"); h = +r.headers.get("X-Height");
          const url = URL.createObjectURL(await r.blob());
          await new Promise((done) => { s.onload = s.onerror = done; s.src = url; });
          URL.revokeObjectURL(s.dataset.was || "");
          s.dataset.was = url;
        } else {
          note.textContent = await r.text(); note.style.display = "block";
          await new Promise((t) => setTimeout(t, 2000));
        }
      } catch { await new Promise((t) => setTimeout(t, 2000)); }
    }
  }
  loop();
  // A point on the page, in the device's pixels.
  function at(e) {
    const r = s.getBoundingClientRect(), scale = Math.min(r.width / w, r.height / h);
    const x = (e.clientX - r.left - (r.width - w * scale) / 2) / scale, y = (e.clientY - r.top - (r.height - h * scale) / 2) / scale;
    return { x: Math.max(0, Math.min(w - 1, x)), y: Math.max(0, Math.min(h - 1, y)) };
  }
  const act = (a) => input && fetch("act", { method: "POST", body: JSON.stringify(a) });
  let down = null;
  s.addEventListener("pointerdown", (e) => { s.focus(); down = at(e); e.preventDefault(); });
  s.addEventListener("pointerup", (e) => {
    if (!down) return;
    const up = at(e), moved = Math.hypot(up.x - down.x, up.y - down.y) > 12;
    act(moved ? { do: "swipe", x: down.x, y: down.y, to_x: up.x, to_y: up.y } : { do: e.button === 2 ? "right_click" : "click", ...up });
    down = null;
  });
  s.addEventListener("contextmenu", (e) => e.preventDefault());
  s.addEventListener("wheel", (e) => { e.preventDefault(); const p = at(e); act({ do: "scroll", ...p, to_x: p.x, to_y: p.y - e.deltaY }); }, { passive: false });
  const named = { Enter: "enter", Backspace: "backspace", Tab: "tab", Escape: "escape", ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right", Delete: "delete", Home: "home", End: "end" };
  s.addEventListener("keydown", (e) => {
    const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.metaKey && "win"].filter(Boolean);
    if (e.key.length === 1 && !mods.length) act({ do: "type", text: e.key });
    else if (named[e.key] || (e.key.length === 1 && mods.length)) act({ do: "key", key: [...mods, named[e.key] || e.key.toLowerCase()].join("-") });
    else return;
    e.preventDefault();
  });
</script>"#;

// ── VNC ────────────────────────────────────────────────────────────────────

/// Gives a device input through its VNC server, as a person watching it would: the pointer goes exactly where it's
/// sent, in the screen's pixels, and keys go as X keysyms. The connection is shared, leaving anyone else watching
/// connected. Its server is a socket in the device's folder, which only this machine's user can open: on a port, any
/// program reaching this machine's 127.0.0.1, one in another device among them, could drive it.
pub fn vnc_act(socket: &Path, a: &Action) -> Result<()> {
    let mut c = UnixStream::connect(socket).map_err(|err| anyhow!("its screen: {err}"))?;
    c.set_read_timeout(Some(Duration::from_secs(60)))?;
    c.set_write_timeout(Some(Duration::from_secs(60)))?;
    vnc_hello(&mut c)?;
    let sleep = |ms| std::thread::sleep(Duration::from_millis(ms));
    let pointer = |c: &mut UnixStream, mask: u8, x: f64, y: f64| {
        let (x, y) = (x.max(0.0) as u16, y.max(0.0) as u16);
        let _ = c.write_all(&[5, mask, (x >> 8) as u8, x as u8, (y >> 8) as u8, y as u8]);
    };
    let key = |c: &mut UnixStream, sym: u32, down: bool| {
        let mut b = [4, down as u8, 0, 0, 0, 0, 0, 0];
        b[4..].copy_from_slice(&sym.to_be_bytes());
        let _ = c.write_all(&b);
        sleep(15);
    };
    let click = |c: &mut UnixStream, button: u8| {
        pointer(c, 0, a.x, a.y);
        sleep(40);
        pointer(c, button, a.x, a.y);
        sleep(60);
        pointer(c, 0, a.x, a.y);
    };
    match a.r#do.as_str() {
        "click" => click(&mut c, 1),
        "right_click" => click(&mut c, 4),
        "double_click" => {
            click(&mut c, 1);
            click(&mut c, 1);
        }
        "scroll" => {
            let wheel = if a.to_y > a.y { 8 } else { 16 }; // up, or down
            for _ in 0..(((a.y - a.to_y).abs() / 100.0) as i64).max(1) {
                pointer(&mut c, wheel, a.x, a.y);
                pointer(&mut c, 0, a.x, a.y);
            }
        }
        "swipe" => {
            pointer(&mut c, 1, a.x, a.y);
            for i in 1..=10 {
                let f = i as f64 / 10.0;
                sleep(20);
                pointer(&mut c, 1, a.x + (a.to_x - a.x) * f, a.y + (a.to_y - a.y) * f);
            }
            pointer(&mut c, 0, a.to_x, a.to_y);
        }
        "type" => {
            for ch in a.text.chars() {
                key(&mut c, keysym(ch), true);
                key(&mut c, keysym(ch), false);
            }
        }
        "key" => {
            let mut syms = Vec::new();
            for part in a.key.to_lowercase().split(['-', '+']).filter(|p| !p.is_empty()) {
                let sym = named_key(part).or_else(|| {
                    let mut chars = part.chars();
                    match (chars.next(), chars.next()) {
                        (Some(ch), None) => Some(keysym(ch)),
                        _ => None,
                    }
                });
                syms.push(sym.ok_or_else(|| anyhow!("no key {part:?}"))?);
            }
            for s in &syms {
                key(&mut c, *s, true);
            }
            for s in syms.iter().rev() {
                key(&mut c, *s, false);
            }
        }
        other => bail!("no action {other:?}"),
    }
    sleep(50); // for the server to read it before the connection closes
    Ok(())
}

/// Opens an RFB session with no password, shared.
fn vnc_hello(c: &mut UnixStream) -> Result<()> {
    let mut version = [0u8; 12];
    c.read_exact(&mut version)?;
    c.write_all(b"RFB 003.008\n")?;
    let mut n = [0u8; 1];
    c.read_exact(&mut n)?;
    if n[0] == 0 {
        bail!("its screen refused the connection");
    }
    let mut types = vec![0u8; n[0] as usize];
    c.read_exact(&mut types)?;
    if !types.contains(&1) {
        bail!("its screen wants a password");
    }
    c.write_all(&[1])?;
    let mut result = [0u8; 4];
    if c.read_exact(&mut result).is_err() || u32::from_be_bytes(result) != 0 {
        bail!("its screen refused the connection");
    }
    c.write_all(&[1])?; // shared
    let mut init = [0u8; 24];
    c.read_exact(&mut init)?;
    let mut name = vec![0u8; u32::from_be_bytes([init[20], init[21], init[22], init[23]]) as usize];
    c.read_exact(&mut name)?;
    // SetEncodings: raw, and pointer type change. QEMU takes a client's pointer for a mouse's movements until it hears
    // the client knows pointers can be absolute: then each move reaches its USB tablet as that many pixels more, which
    // Windows reads as the screen's bottom-right corner, wherever it was sent.
    c.write_all(&[2, 0, 0, 2, 0, 0, 0, 0, 0xff, 0xff, 0xfe, 0xff])?;
    Ok(())
}

fn named_key(name: &str) -> Option<u32> {
    Some(match name {
        "enter" | "return" => 0xff0d,
        "backspace" => 0xff08,
        "tab" => 0xff09,
        "escape" | "esc" => 0xff1b,
        "delete" | "del" => 0xffff,
        "home" => 0xff50,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "page_up" | "pgup" => 0xff55,
        "page_down" | "pgdn" => 0xff56,
        "end" => 0xff57,
        "insert" => 0xff63,
        "space" => 0x20,
        "shift" => 0xffe1,
        "ctrl" | "control" => 0xffe3,
        "alt" => 0xffe9,
        "win" | "windows" | "super" | "meta" | "cmd" => 0xffeb,
        f if f.len() >= 2 && f.starts_with('f') => {
            let n: u32 = f[1..].parse().ok().filter(|n| (1..=12).contains(n))?;
            0xffbe + n - 1
        }
        _ => return None,
    })
}

/// The key that types ch. QEMU presses or lets go of shift for it as the character needs.
fn keysym(ch: char) -> u32 {
    match ch {
        '\n' => 0xff0d,
        '\t' => 0xff09,
        c if (c as u32) < 0x100 => c as u32,
        c => 0x0100_0000 + c as u32,
    }
}
