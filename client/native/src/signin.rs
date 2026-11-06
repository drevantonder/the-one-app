//! Signing in, as every client does: an OAuth client of the install's sign-in,
//! registered the first time, with sign-in and consent in a browser that comes back to a port on this machine (RFC
//! 8252's loopback redirect). The refresh token is kept in the OS's secret store; access tokens only in memory. It names
//! the device it's for (`join`), so the person's devices can let it in by this sign-in.

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
    access: Mutex<Option<(String, Instant)>>,
    meta: Mutex<Option<Value>>,
}

fn random(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    BASE64URL_NOPAD.encode(&b)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into()
}

impl Auth {
    pub fn new(address: &str, name: &str) -> Auth {
        Auth { address: address.trim_end_matches('/').into(), name: name.into(), access: Mutex::default(), meta: Mutex::default() }
    }

    fn key(&self, k: &str) -> String {
        format!("{} {k}", self.address)
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
        secrets::get(&self.key("refresh")).is_some()
    }

    fn client(&self, m: &Value) -> Result<String, String> {
        if let Some(id) = secrets::get(&self.key("client")) {
            return Ok(String::from_utf8_lossy(&id).into());
        }
        let body = json!({
            "client_name": self.name, "redirect_uris": ["http://127.0.0.1/callback"], "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"], "application_type": "native",
        });
        let mut r = agent().post(m["registration_endpoint"].as_str().unwrap_or_default()).send_json(body).map_err(|e| e.to_string())?;
        let out: Value = r.body_mut().read_json().unwrap_or_default();
        let id = out["client_id"].as_str().filter(|_| r.status().as_u16() < 300).ok_or_else(|| {
            out["error_description"].as_str().unwrap_or("This device couldn’t sign up to sign in.").to_string()
        })?;
        secrets::set(&self.key("client"), id.as_bytes())?;
        Ok(id.into())
    }

    /// Signs in: `open` shows the authorization URL (in a window of the app, or the system's browser), which comes
    /// back to this machine's loopback port; `join` is the device it's for. Blocks until it's back, or 5 minutes.
    pub fn sign_in(&self, join: &str, open: impl FnOnce(String) -> Result<(), String>) -> Result<(), String> {
        let m = self.meta()?;
        let id = self.client(&m)?;
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let redirect = format!("http://{}/callback", listener.local_addr().unwrap());
        let (verifier, state) = (random(32), random(16));
        let challenge = BASE64URL_NOPAD.encode(&Sha256::digest(verifier.as_bytes()));
        let mut u = url::Url::parse(m["authorization_endpoint"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
        u.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("scope", "openid profile offline_access")
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("prompt", "consent")
            .append_pair("join", join);
        open(u.to_string())?;
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
        if let Some(e) = got.get("error") {
            return Err(if e == "access_denied" { "You didn’t let this device in.".into() } else { got.get("error_description").unwrap_or(e).clone() });
        }
        let code = got.get("code").filter(|_| got.get("state") == Some(&state)).ok_or("That sign-in didn’t start here. Sign in again.")?;
        self.grant(&m, &[("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &redirect), ("client_id", &id), ("code_verifier", &verifier)])
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
            secrets::set(&self.key("refresh"), refresh.as_bytes())?;
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
        let refresh = secrets::get(&self.key("refresh")).ok_or("Sign in again.")?;
        let refresh = String::from_utf8_lossy(&refresh).to_string();
        let id = secrets::get(&self.key("client")).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
        let m = self.meta()?;
        if let Err(e) = self.grant(&m, &[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &id)]) {
            if e.starts_with("Sign in again") {
                secrets::delete(&self.key("refresh"));
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
        let refresh = secrets::get(&self.key("refresh"));
        secrets::delete(&self.key("refresh"));
        *self.access.lock().unwrap() = None;
        if let (Ok(m), Some(refresh)) = (self.meta(), refresh)
            && let Some(at) = m["revocation_endpoint"].as_str()
        {
            let id = secrets::get(&self.key("client")).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
            let refresh = String::from_utf8_lossy(&refresh).to_string();
            let _ = agent().post(at).send_form([("token", refresh.as_str()), ("token_type_hint", "refresh_token"), ("client_id", id.as_str())]);
        }
    }
}
