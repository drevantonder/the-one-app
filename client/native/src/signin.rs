//! Signing in, as every client does: an OAuth client of the network's sign-in, registered the first time, with sign-in
//! and consent in a browser that comes back to a port on this machine (RFC 8252's loopback redirect), or on a phone to
//! the app's own link (`<cli>://signed-in/`), which an iPhone's system sign-in sheet takes and an Android phone's Chrome
//! Custom Tab opens the app on. The refresh token is kept in the
//! OS's secret store; access tokens only in memory. It names the device it's for (`join`), so the person's devices can
//! let it in by this sign-in.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use data_encoding::BASE64URL_NOPAD;
use rand::RngCore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::secrets;

pub struct Auth {
    pub address: String,
    pub name: String,
    /// Where it keeps its registration with sign-in (not a secret): `sign-in`, its address, its id there, and where
    /// sign-in comes back to if not the loopback.
    data: std::path::PathBuf,
    access: Mutex<Option<(String, Instant)>>,
    meta: Mutex<Option<Value>>,
}

fn random(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    BASE64URL_NOPAD.encode(&b)
}

/// Where the loopback's sign-in comes back to, as it registers it (the port is any).
const LOOPBACK: &str = "http://127.0.0.1/callback";

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into()
}

impl Auth {
    pub fn new(address: &str, name: &str, data: std::path::PathBuf) -> Auth {
        Auth { address: address.trim_end_matches('/').into(), name: name.into(), data, access: Mutex::default(), meta: Mutex::default() }
    }

    /// This app's id as a client of sign-in at this address, if it registered there to come back to `back`.
    fn client_id(&self, back: &str) -> Option<String> {
        let kept = std::fs::read_to_string(self.data.join("sign-in")).ok()?;
        let mut lines = kept.lines();
        let (address, id, kept_back) = (lines.next()?, lines.next()?, lines.next().unwrap_or(LOOPBACK));
        (address == self.address && kept_back == back).then(|| id.to_string())
    }

    /// The id it has, whichever way it signed in, for a refresh.
    fn any_client_id(&self) -> String {
        let kept = std::fs::read_to_string(self.data.join("sign-in")).unwrap_or_default();
        kept.lines().nth(1).unwrap_or_default().to_string()
    }

    fn meta(&self) -> Result<Value, String> {
        if let Some(m) = self.meta.lock().unwrap().clone() {
            return Ok(m);
        }
        let mut r = agent()
            .get(format!("{}/.well-known/oauth-authorization-server", self.address))
            .call()
            .map_err(|_| "Can’t reach that address. Check it, and that you’re online.".to_string())?;
        let m: Value = r.body_mut().read_json().map_err(|e| e.to_string())?;
        *self.meta.lock().unwrap() = Some(m.clone());
        Ok(m)
    }

    pub fn signed_in(&self) -> bool {
        secrets::refresh().is_some()
    }

    fn client(&self, m: &Value, back: &str) -> Result<String, String> {
        if let Some(id) = self.client_id(back) {
            return Ok(id);
        }
        let body = json!({
            "client_name": self.name, "redirect_uris": [back], "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"], "application_type": "native",
        });
        let mut r = agent().post(m["registration_endpoint"].as_str().unwrap_or_default()).send_json(body).map_err(|e| e.to_string())?;
        let out: Value = r.body_mut().read_json().unwrap_or_default();
        let id = out["client_id"].as_str().filter(|_| r.status().as_u16() < 300).ok_or_else(|| {
            out["error_description"].as_str().unwrap_or("This device couldn’t sign up to sign in.").to_string()
        })?;
        let _ = std::fs::create_dir_all(&self.data);
        std::fs::write(self.data.join("sign-in"), format!("{}\n{id}\n{back}", self.address)).map_err(|e| e.to_string())?;
        Ok(id.into())
    }

    /// The authorization request: its URL, and the verifier and state it's finished with.
    fn ask(&self, m: &Value, id: &str, redirect: &str, join: &str) -> Result<(String, String, String), String> {
        let (verifier, state) = (random(32), random(16));
        let challenge = BASE64URL_NOPAD.encode(&Sha256::digest(verifier.as_bytes()));
        let mut u = url::Url::parse(m["authorization_endpoint"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
        u.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", id)
            .append_pair("redirect_uri", redirect)
            .append_pair("scope", "openid profile offline_access")
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("prompt", "consent")
            .append_pair("join", join);
        Ok((u.to_string(), verifier, state))
    }

    /// What came back (the redirect's query), exchanged for tokens.
    fn finish(&self, m: &Value, id: &str, redirect: &str, verifier: &str, state: &str, got: std::collections::HashMap<String, String>) -> Result<(), String> {
        if let Some(e) = got.get("error") {
            return Err(if e == "access_denied" { "You didn’t let this device in.".into() } else { got.get("error_description").unwrap_or(e).clone() });
        }
        let code = got.get("code").filter(|_| got.get("state").map(String::as_str) == Some(state)).ok_or("That sign-in didn’t start here. Sign in again.")?;
        self.grant(m, &[("grant_type", "authorization_code"), ("code", code), ("redirect_uri", redirect), ("client_id", id), ("code_verifier", verifier)])
    }

    /// Signs in on a phone: `open` shows the authorization URL in the system's sign-in sheet (which shares Safari's
    /// passkeys and sign-ins) and gives back where it came back to, the app's own link; `join` is the device it's for.
    #[cfg_attr(not(any(target_os = "ios", target_os = "android")), allow(dead_code))]
    pub fn sign_in_here(&self, join: &str, open: impl FnOnce(String) -> Result<String, String>) -> Result<(), String> {
        let m = self.meta()?;
        let back = format!("{}://signed-in/", one::config::CLI);
        let id = self.client(&m, &back)?;
        let (url, verifier, state) = self.ask(&m, &id, &back, join)?;
        let came = url::Url::parse(&open(url)?).map_err(|e| e.to_string())?;
        self.finish(&m, &id, &back, &verifier, &state, came.query_pairs().into_owned().collect())
    }

    /// Signs in: `open` shows the authorization URL (in a window of the app, or the system's browser), which comes
    /// back to this machine's loopback port; `join` is the device it's for. Blocks until it's back, or 5 minutes.
    #[cfg_attr(any(target_os = "ios", target_os = "android"), allow(dead_code))]
    pub fn sign_in(&self, join: &str, open: impl FnOnce(String) -> Result<(), String>) -> Result<(), String> {
        let m = self.meta()?;
        let id = self.client(&m, LOOPBACK)?;
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let redirect = format!("http://{}/callback", listener.local_addr().unwrap());
        let (u, verifier, state) = self.ask(&m, &id, &redirect, join)?;
        open(u)?;
        listener.set_nonblocking(true).ok();
        let until = Instant::now() + Duration::from_secs(300);
        let got = loop {
            match listener.accept() {
                Ok((mut s, _)) => {
                    s.set_nonblocking(false).ok();
                    let mut line = String::new();
                    BufReader::new(&s).read_line(&mut line).ok();
                    let path = line.split(' ').nth(1).unwrap_or_default().to_string();
                    let page = "<!doctype html><meta charset=utf-8><meta name=color-scheme content=\"light dark\"><body style=\"font:15px system-ui;display:grid;place-items:center;height:90vh;color:GrayText\"><p>You’re signed in. Go back to {}.</p>";
                    let page = page.replace("{}", one::config::NAME);
                    let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}", page.len());
                    if path.starts_with("/callback") {
                        let u = url::Url::parse(&format!("http://x{path}")).map_err(|e| e.to_string())?;
                        break u.query_pairs().into_owned().collect::<std::collections::HashMap<_, _>>();
                    }
                }
                Err(_) if Instant::now() < until => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => return Err("Signing in took too long. Try again.".into()),
            }
        };
        self.finish(&m, &id, &redirect, &verifier, &state, got)
    }

    fn grant(&self, m: &Value, fields: &[(&str, &str)]) -> Result<(), String> {
        let mut r = agent().post(m["token_endpoint"].as_str().unwrap_or_default()).send_form(fields.iter().copied()).map_err(|e| e.to_string())?;
        let status = r.status().as_u16();
        let out: Value = r.body_mut().read_json().unwrap_or_default();
        let Some(access) = out["access_token"].as_str().filter(|_| status < 300) else {
            let why = out["error_description"].as_str().or(out["error"].as_str()).unwrap_or("Signing in didn’t work.");
            return Err(if status == 400 || status == 401 { format!("Sign in again. {why}") } else { why.into() });
        };
        let expires = out["expires_in"].as_f64().unwrap_or(3600.0);
        *self.access.lock().unwrap() = Some((access.into(), Instant::now() + Duration::from_secs_f64(expires)));
        // Each refresh token works once: the new one is kept before anything else can ask.
        if let Some(refresh) = out["refresh_token"].as_str() {
            secrets::set_refresh(refresh)?;
        }
        Ok(())
    }

    /// An access token good for a minute more, new from the refresh token if it's older.
    pub fn token(&self) -> Result<String, String> {
        if let Some((t, until)) = self.access.lock().unwrap().clone()
            && until > Instant::now() + Duration::from_secs(60)
        {
            return Ok(t);
        }
        let refresh = secrets::refresh().ok_or("Sign in again.")?;
        let id = self.any_client_id();
        let m = self.meta()?;
        if let Err(e) = self.grant(&m, &[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &id)]) {
            if e.starts_with("Sign in again") {
                secrets::forget_refresh();
            }
            return Err(e);
        }
        Ok(self.access.lock().unwrap().clone().unwrap().0)
    }

    /// Who the access token is for: its `sub`.
    pub fn subject(token: &str) -> String {
        let part = token.split('.').nth(1).unwrap_or_default();
        let claims: Value = BASE64URL_NOPAD.decode(part.trim_end_matches('=').as_bytes()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        claims["sub"].as_str().unwrap_or_default().into()
    }

    /// Revokes the refresh token, so it opens nothing again, and forgets it here.
    pub fn sign_out(&self) {
        let refresh = secrets::refresh();
        secrets::forget_refresh();
        *self.access.lock().unwrap() = None;
        if let (Ok(m), Some(refresh)) = (self.meta(), refresh)
            && let Some(at) = m["revocation_endpoint"].as_str()
        {
            let id = self.any_client_id();
            let _ = agent().post(at).send_form([("token", refresh.as_str()), ("token_type_hint", "refresh_token"), ("client_id", id.as_str())]);
        }
    }
}
