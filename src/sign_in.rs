//! This machine's CLI and MCP signed in at the network's address, as every other client is: an OAuth client of its auth
//! service, registered the first time it signs in. The user approves a short code by signing in on any device
//! (`cloudflare/auth`'s `/sign-in/code`), here in the browser if this machine has a screen. The refresh token is kept in a
//! file only this machine's user reads, outside the node's data: the node is only ever sent access tokens.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use one::config::{CLI, NAME};
use one_client::sign_in::{Asked, Auth};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What's kept: the network's address, this machine's client there, and its tokens.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Kept {
    pub address: String,
    pub client: String,
    #[serde(default)]
    pub refresh: Option<String>,
    #[serde(default)]
    pub access: Option<String>,
    /// When the access token expires (milliseconds).
    #[serde(default)]
    pub until: u64,
}

/// Where it's kept: in the user's config, one for each node this CLI reaches (`<CLI>_ADDR`); a throwaway network's, in its
/// folder, which stopping it deletes.
pub fn path() -> PathBuf {
    if let one::mode::Mode::Throwaway(t) = one::mode::here() {
        return t.home().join("sign-in.json");
    }
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".config"));
    let name = match std::env::var(one::config::env("ADDR")) {
        Ok(addr) => format!("sign-in-{}.json", addr.replace(|c: char| !c.is_ascii_alphanumeric(), "-")),
        Err(_) => "sign-in.json".into(),
    };
    config.join(CLI).join(name)
}

pub fn kept() -> Option<Kept> {
    serde_json::from_slice(&std::fs::read(path()).ok()?).ok()
}

/// Writes it whole, readable only by this machine's user, so a reader never sees half.
fn keep(kept: &Kept) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = path();
    std::fs::create_dir_all(path.parent().expect("a file in a folder"))?;
    let new = path.with_extension("new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&new)?;
    file.write_all(&serde_json::to_vec(kept)?)?;
    file.sync_all()?;
    std::fs::rename(&new, &path)?;
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into()
}

fn said(out: &Value, otherwise: &str) -> String {
    out["error_description"].as_str().or(out["error"].as_str()).or(out["message"].as_str()).unwrap_or(otherwise).to_string()
}

/// Where a code's sign-in comes back to.
fn back(address: &str) -> String {
    format!("{address}/sign-in/code/done")
}

/// This machine's client of `auth`: as it registered, or registered now.
fn client(auth: &Auth) -> Result<String> {
    if let Some(kept) = kept().filter(|k| k.address == auth.address && !k.client.is_empty()) {
        return Ok(kept.client);
    }
    Ok(auth.register(&format!("{NAME} on {}", hostname()), &back(&auth.address), false)?)
}

/// A sign-in under way: the code the user approves, and what turns it into tokens here.
pub struct Started {
    pub code: String,
    pub url: String,
    client: String,
    auth: Auth,
    asked: Asked,
}

/// Starts signing in to the network at `address`, for the device `join` names (its key ‖ seal key, hex), if it's one
/// that then asks to be let in: → the code the user approves, and its link.
pub fn start(address: &str, join: Option<&str>) -> Result<Started> {
    let auth = Auth::at(address)?;
    let client = client(&auth)?;
    let asked = auth.ask(&client, &back(&auth.address), join);
    let mut resp = agent().post(format!("{}/sign-in/code", auth.address)).send_json(serde_json::json!({ "query": asked.query }))?;
    let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    let (Some(code), Some(url)) = (out["code"].as_str(), out["url"].as_str()) else {
        bail!("auth gave no code: {}", said(&out, "it may need updating"));
    };
    Ok(Started { code: code.into(), url: url.into(), client, auth, asked })
}

pub fn finish(started: Started) -> Result<()> {
    let address = &started.auth.address;
    let code = loop {
        let mut resp = agent().post(format!("{address}/sign-in/code/take")).send_json(serde_json::json!({ "state": started.asked.state }))?;
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        match resp.status().as_u16() {
            202 => std::thread::sleep(Duration::from_secs(2)),
            200 if out["code"].is_string() => break out["code"].as_str().unwrap_or_default().to_string(),
            _ => bail!("{}", said(&out, "signing in didn't work")),
        }
    };
    let mut kept = Kept { address: address.clone(), client: started.client.clone(), ..Default::default() };
    keep_tokens(&mut kept, started.auth.code(&started.asked, &code))
}

/// What a grant gave, kept: each refresh token works once, so the new one is kept before anything else can ask. A
/// refusal means this machine is signed out.
fn keep_tokens(kept: &mut Kept, tokens: Result<one_client::sign_in::Tokens, one_client::sign_in::Failed>) -> Result<()> {
    let tokens = match tokens {
        Ok(t) => t,
        Err(e) if e.signed_out => {
            let _ = std::fs::remove_file(path());
            bail!("this machine is signed out ({e}): {CLI} sign-in");
        }
        Err(e) => return Err(e.into()),
    };
    kept.access = Some(tokens.access);
    kept.until = now_ms() + tokens.expires_in * 1000;
    if let Some(refresh) = tokens.refresh {
        kept.refresh = Some(refresh);
    }
    keep(kept)
}

/// The access token to send this machine's node, good for a minute more, refreshed if it's older: None if this
/// machine isn't signed in. One process at a time refreshes, since each refresh token works once.
pub fn token() -> Result<Option<String>> {
    let fresh = |k: &Kept| k.access.clone().filter(|_| k.until > now_ms() + 60_000);
    let Some(kept) = kept() else { return Ok(None) };
    if let Some(access) = fresh(&kept) {
        return Ok(Some(access));
    }
    let _lock = Lock::take()?;
    // Another process may have refreshed it while this one waited.
    let Some(mut kept) = self::kept() else { return Ok(None) };
    if let Some(access) = fresh(&kept) {
        return Ok(Some(access));
    }
    let refresh = kept.refresh.clone().context(format!("this machine is signed out: {CLI} sign-in"))?;
    let tokens = Auth::at(&kept.address)?.refresh(&kept.client, &refresh);
    keep_tokens(&mut kept, tokens)?;
    Ok(kept.access)
}

/// Signs this machine out: its refresh token is revoked, and forgotten here. Waits for a refresh another process is
/// making, so what that keeps is what's revoked and forgotten, not kept again after; the lock's file stays, as every
/// process must lock the same one.
pub fn sign_out() -> Result<bool> {
    let _lock = Lock::take()?;
    let Some(kept) = kept() else { return Ok(false) };
    let _ = std::fs::remove_file(path());
    if let (Some(refresh), Ok(auth)) = (&kept.refresh, Auth::at(&kept.address)) {
        auth.revoke(&kept.client, refresh);
    }
    Ok(true)
}

/// Held while this machine refreshes its token: an exclusive lock on a file beside it.
struct Lock(std::fs::File);

impl Lock {
    fn take() -> Result<Lock> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(path().with_extension("lock"))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            bail!("couldn't wait for another {NAME} command signing in: {}", std::io::Error::last_os_error());
        }
        Ok(Lock(file))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

pub fn hostname() -> String {
    std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "this machine".into())
}

/// Whether a browser here would be in front of the user: a Mac's own session, or a Linux desktop's, not SSH.
pub fn has_screen() -> bool {
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some() {
        return false;
    }
    cfg!(target_os = "macos") || std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub fn open(url: &str) -> bool {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}
