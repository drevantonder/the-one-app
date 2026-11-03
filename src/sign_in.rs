//! This machine's CLI and MCP signed in to the install's kernel, as every other client is: an OAuth client of its auth
//! service, registered the first time it signs in. The person approves a short code by signing in on any device
//! (`kernel/auth`'s `/sign-in/code`), here in the browser if this machine has a screen. The refresh token is kept in a
//! file only this machine's user reads, outside the node's data: the node is only ever sent access tokens.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;

use crate::config::{CLI, NAME};

/// What's kept: the install's address, this machine's client there, and its tokens.
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

/// Where it's kept: in the person's config, one for each node this CLI reaches (`<CLI>_ADDR`, as `try` sets it).
pub fn path() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".config"));
    let name = match std::env::var(format!("{}_ADDR", CLI.to_uppercase())) {
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

/// The auth service's endpoints, as it says them.
fn meta(address: &str) -> Result<Value> {
    let mut resp = agent().get(format!("{address}/.well-known/oauth-authorization-server")).call().with_context(|| format!("the sign-in service at {address} isn't answering"))?;
    if resp.status() != 200 {
        bail!("the sign-in service at {address} isn't answering");
    }
    Ok(resp.body_mut().read_json()?)
}

fn said(out: &Value, otherwise: &str) -> String {
    out["error_description"].as_str().or(out["error"].as_str()).or(out["message"].as_str()).unwrap_or(otherwise).to_string()
}

/// Where a code's sign-in comes back to.
fn back(address: &str) -> String {
    format!("{address}/sign-in/code/done")
}

/// This machine, as a client of the auth service: registered the first time it signs in there.
fn client(address: &str, meta: &Value) -> Result<String> {
    if let Some(kept) = kept().filter(|k| k.address == address && !k.client.is_empty()) {
        return Ok(kept.client);
    }
    let body = serde_json::json!({
        "client_name": format!("{NAME} on {}", hostname()),
        "redirect_uris": [back(address)],
        "token_endpoint_auth_method": "none",
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
    });
    let mut resp = agent().post(meta["registration_endpoint"].as_str().context("the sign-in service takes no new clients")?).send_json(&body)?;
    let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    match out["client_id"].as_str() {
        Some(id) if resp.status().is_success() => Ok(id.to_string()),
        _ => bail!("this machine couldn't sign up to sign in: {}", said(&out, "the sign-in service said no")),
    }
}

/// A sign-in under way: the code the person approves, and what turns it into tokens here.
pub struct Started {
    pub code: String,
    pub url: String,
    address: String,
    client: String,
    state: String,
    verifier: String,
    meta: Value,
}

/// Starts signing in to the install at `address`: → the code the person approves, and its link.
pub fn start(address: &str) -> Result<Started> {
    let address = address.trim_end_matches('/').to_string();
    let meta = meta(&address)?;
    let client = client(&address, &meta)?;
    let verifier = B64U.encode(&rand::random::<[u8; 32]>());
    let state = B64U.encode(&rand::random::<[u8; 32]>());
    let challenge = B64U.encode(&sha2::Sha256::digest(verifier.as_bytes()));
    let query = form(&[
        ("response_type", "code"),
        ("client_id", &client),
        ("redirect_uri", &back(&address)),
        ("scope", "openid profile offline_access"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("state", &state),
        ("prompt", "consent"),
    ]);
    let mut resp = agent().post(format!("{address}/sign-in/code")).send_json(serde_json::json!({ "query": query }))?;
    let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    let (Some(code), Some(url)) = (out["code"].as_str(), out["url"].as_str()) else {
        bail!("the sign-in service gave no code: {}", said(&out, "it may need updating"));
    };
    Ok(Started { code: code.into(), url: url.into(), address, client, state, verifier, meta })
}

/// Waits for the person to approve the code, then keeps the tokens it gives.
pub fn finish(started: Started) -> Result<()> {
    let code = loop {
        let mut resp = agent().post(format!("{}/sign-in/code/take", started.address)).send_json(serde_json::json!({ "state": started.state }))?;
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        match resp.status().as_u16() {
            202 => std::thread::sleep(Duration::from_secs(2)),
            200 if out["code"].is_string() => break out["code"].as_str().unwrap_or_default().to_string(),
            _ => bail!("{}", said(&out, "signing in didn't work")),
        }
    };
    let mut kept = Kept { address: started.address.clone(), client: started.client.clone(), ..Default::default() };
    grant(&started.meta, &mut kept, &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &back(&started.address)), ("client_id", &started.client), ("code_verifier", &started.verifier)])?;
    Ok(())
}

/// Swaps a code or refresh token for tokens, and keeps them: each refresh token works once, so the new one is kept
/// before anything else can ask. A refusal means this machine is signed out.
fn grant(meta: &Value, kept: &mut Kept, fields: &[(&str, &str)]) -> Result<()> {
    let endpoint = meta["token_endpoint"].as_str().context("the sign-in service gives no tokens")?;
    let mut resp = agent().post(endpoint).header("content-type", "application/x-www-form-urlencoded").send(form(fields))?;
    let status = resp.status().as_u16();
    let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    let Some(access) = out["access_token"].as_str().filter(|_| status == 200) else {
        if matches!(status, 400 | 401) {
            let _ = std::fs::remove_file(path());
            bail!("this machine is signed out ({}): {CLI} sign-in", said(&out, "its sign-in was refused"));
        }
        bail!("the sign-in service said {status}: {}", said(&out, "nothing"));
    };
    kept.access = Some(access.to_string());
    kept.until = now_ms() + out["expires_in"].as_u64().unwrap_or(3600) * 1000;
    if let Some(refresh) = out["refresh_token"].as_str() {
        kept.refresh = Some(refresh.to_string());
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
    let client = kept.client.clone();
    grant(&meta(&kept.address)?, &mut kept, &[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &client)])?;
    Ok(kept.access)
}

/// Signs this machine out: its refresh token is revoked, and forgotten here.
pub fn sign_out() -> Result<bool> {
    let Some(kept) = kept() else { return Ok(false) };
    let _ = std::fs::remove_file(path());
    let _ = std::fs::remove_file(path().with_extension("lock"));
    if let (Some(refresh), Ok(meta)) = (&kept.refresh, meta(&kept.address))
        && let Some(revoke) = meta["revocation_endpoint"].as_str()
    {
        let _ = agent().post(revoke).header("content-type", "application/x-www-form-urlencoded").send(form(&[("token", refresh), ("token_type_hint", "refresh_token"), ("client_id", &kept.client)]));
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

fn form(fields: &[(&str, &str)]) -> String {
    fields.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn hostname() -> String {
    std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "this machine".into())
}

/// Whether a browser here would be in front of the person: a Mac's own session, or a Linux desktop's, not SSH.
pub fn has_screen() -> bool {
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some() {
        return false;
    }
    cfg!(target_os = "macos") || std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Opens a link in this machine's browser.
pub fn open(url: &str) -> bool {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}
