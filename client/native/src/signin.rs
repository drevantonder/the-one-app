//! This app's sign-in, as every client signs in (one_client::sign_in), with sign-in and consent in a browser that comes back to a
//! port on this machine (RFC 8252's loopback redirect), or on a phone to the app's own link (`<cli>://signed-in/`),
//! which an iPhone's system sign-in sheet takes and an Android phone's Chrome Custom Tab opens the app on. The refresh
//! token is kept in the OS's secret store; access tokens only in memory. It names the device it's for (`join`), so the
//! user's devices can let it in by this sign-in.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use one_client::sign_in::{Auth, Failed, Tokens, signed_out};

use crate::secrets;

pub use one_client::sign_in::subject;

pub struct SignIn {
    pub address: String,
    pub name: String,
    /// Where it keeps its registration with auth (not a secret): `sign-in`, its address, its id there, and where
    /// auth comes back to.
    data: std::path::PathBuf,
    access: Mutex<Option<(String, Instant)>>,
    auth: Mutex<Option<Arc<Auth>>>,
}

/// Where the loopback's sign-in comes back to, as it registers it (the port is any).
const LOOPBACK: &str = "http://127.0.0.1/callback";

fn failed(message: impl Into<String>) -> Failed {
    Failed { signed_out: false, message: message.into() }
}

impl SignIn {
    pub fn new(address: &str, name: &str, data: std::path::PathBuf) -> SignIn {
        SignIn { address: address.trim_end_matches('/').into(), name: name.into(), data, access: Mutex::default(), auth: Mutex::default() }
    }

    /// The registration kept: `(address, client id, where it comes back to)`.
    fn kept(&self) -> Option<(String, String, String)> {
        let kept = std::fs::read_to_string(self.data.join("sign-in")).ok()?;
        let mut lines = kept.lines();
        Some((lines.next()?.to_string(), lines.next()?.to_string(), lines.next().unwrap_or(LOOPBACK).to_string()))
    }

    fn auth(&self) -> Result<Arc<Auth>, Failed> {
        let mut kept = self.auth.lock().unwrap();
        if let Some(s) = kept.as_ref() {
            return Ok(s.clone());
        }
        let s = Arc::new(Auth::at(&self.address)?);
        *kept = Some(s.clone());
        Ok(s)
    }

    pub fn signed_in(&self) -> bool {
        secrets::refresh().is_some()
    }

    /// This app's id as a client of auth at this address, coming back to `back`: as it registered, or registered now.
    fn client(&self, s: &Auth, back: &str) -> Result<String, Failed> {
        if let Some((address, id, kept_back)) = self.kept()
            && address == self.address
            && kept_back == back
        {
            return Ok(id);
        }
        let id = s.register(&self.name, back, true)?;
        let _ = std::fs::create_dir_all(&self.data);
        std::fs::write(self.data.join("sign-in"), format!("{}\n{id}\n{back}", self.address)).map_err(|e| failed(e.to_string()))?;
        Ok(id)
    }

    /// Signs in on a phone: `open` shows the authorization URL in the system's sign-in sheet (which shares the browser's
    /// passkeys and sign-ins) and gives back where it came back to, the app's own link; `join` is the device it's for.
    #[cfg_attr(desktop, allow(dead_code))]
    pub fn sign_in_here(&self, join: &str, open: impl FnOnce(String) -> Result<String, String>) -> Result<(), Failed> {
        let s = self.auth()?;
        let back = format!("{}://signed-in/", one::config::CLI);
        let asked = s.ask(&self.client(&s, &back)?, &back, Some(join));
        let came = url::Url::parse(&open(s.url(&asked)?).map_err(failed)?).map_err(|e| failed(e.to_string()))?;
        self.keep(s.finish(&asked, &came.query_pairs().into_owned().collect())?)
    }

    /// Signs in: `open` shows the authorization URL (in the system's browser), which comes back to this machine's loopback
    /// port; `join` is the device it's for. Blocks until it's back, or 5 minutes.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    pub fn sign_in(&self, join: &str, open: impl FnOnce(String) -> Result<(), String>) -> Result<(), Failed> {
        let s = self.auth()?;
        let id = self.client(&s, LOOPBACK)?;
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| failed(e.to_string()))?;
        let at = listener.local_addr().map_err(|e| failed(e.to_string()))?;
        let redirect = format!("http://{at}/callback");
        let asked = s.ask(&id, &redirect, Some(join));
        let (came, given_up) = (mpsc::channel(), Arc::new(AtomicBool::new(false)));
        let (tx, stop) = (came.0, given_up.clone());
        std::thread::spawn(move || {
            for mut s in listener.incoming().flatten() {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let mut line = String::new();
                BufReader::new(&s).read_line(&mut line).ok();
                let path = line.split(' ').nth(1).unwrap_or_default().to_string();
                let page = "<!doctype html><meta charset=utf-8><meta name=color-scheme content=\"light dark\"><body style=\"font:15px system-ui;display:grid;place-items:center;height:90vh;color:GrayText\"><p>You’re signed in. Go back to {}.</p>";
                let page = page.replace("{}", one::config::NAME);
                let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}", page.len());
                if path.starts_with("/callback")
                    && let Ok(u) = url::Url::parse(&format!("http://x{path}"))
                {
                    let _ = tx.send(u.query_pairs().into_owned().collect::<HashMap<_, _>>());
                    return;
                }
            }
        });
        open(s.url(&asked)?).map_err(failed)?;
        let got = came.1.recv_timeout(Duration::from_secs(300)).map_err(|_| {
            // The loopback's thread, waiting on its next connection, given one to end with.
            given_up.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(at);
            failed("Signing in took too long. Try again.")
        })?;
        self.keep(s.finish(&asked, &got)?)
    }

    /// The tokens a grant gave: the refresh token kept before anything else can ask, as each works once.
    fn keep(&self, tokens: Tokens) -> Result<(), Failed> {
        *self.access.lock().unwrap() = Some((tokens.access, Instant::now() + Duration::from_secs(tokens.expires_in)));
        if let Some(refresh) = tokens.refresh {
            secrets::set_refresh(&refresh).map_err(failed)?;
        }
        Ok(())
    }

    /// An access token good for a minute more, new from the refresh token if it's older.
    pub fn token(&self) -> Result<String, Failed> {
        if let Some((t, until)) = self.access.lock().unwrap().clone()
            && until > Instant::now() + Duration::from_secs(60)
        {
            return Ok(t);
        }
        let refresh = secrets::refresh().ok_or_else(|| signed_out("Sign in again."))?;
        let client = self.kept().map(|k| k.1).unwrap_or_default();
        let tokens = self.auth()?.refresh(&client, &refresh).inspect_err(|e| {
            if e.signed_out {
                secrets::forget_refresh();
            }
        })?;
        let access = tokens.access.clone();
        self.keep(tokens)?;
        Ok(access)
    }

    /// Revokes the refresh token, so it opens nothing again, and forgets it here.
    pub fn sign_out(&self) {
        let refresh = secrets::refresh();
        secrets::forget_refresh();
        *self.access.lock().unwrap() = None;
        if let (Ok(s), Some(refresh), Some((_, client, _))) = (self.auth(), refresh, self.kept()) {
            s.revoke(&client, &refresh);
        }
    }
}
