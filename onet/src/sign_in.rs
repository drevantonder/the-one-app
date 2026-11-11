//! Signing in at the network's address, as every client does (the native app, a browser's page
//! through its worker): an OAuth client of its auth, from its metadata (RFC 8414), registered the first time (RFC 7591), with an authorization code and PKCE, refreshed, and revoked. Each step is a `Request` made here and an answer
//! read here; who sends it is the client's (natively, `ureq`, below; in a browser, its worker's `fetch`), as are where
//! it keeps its registration and tokens, and how the user reaches the authorization and comes back.

use std::collections::HashMap;

use data_encoding::BASE64URL_NOPAD as B64U;
use serde_json::{Value, json};

/// Why signing in didn't work, in words a user reads; `signed_out` when auth refused this client's tokens (one used
/// or revoked), so its user signs in again.
#[derive(Debug, Clone)]
pub struct Failed {
    pub signed_out: bool,
    pub message: String,
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failed {}

pub fn failed(message: impl Into<String>) -> Failed {
    Failed { signed_out: false, message: message.into() }
}

/// Signed out: nothing here signs in until its user does again.
pub fn signed_out(message: impl Into<String>) -> Failed {
    Failed { signed_out: true, message: message.into() }
}

/// What auth said went wrong, or `otherwise`.
fn said(out: &Value, otherwise: &str) -> String {
    out["error_description"].as_str().or(out["error"].as_str()).or(out["message"].as_str()).unwrap_or(otherwise).to_string()
}

/// `fields` as a form's body, or a URL's query.
fn form(fields: &[(&str, &str)]) -> String {
    fields.iter().map(|(k, v)| format!("{}={}", custom_build::url_encode(k), custom_build::url_encode(v))).collect::<Vec<_>>().join("&")
}

/// A request to auth, for its client to send: → its status and JSON (null if it wasn't).
pub struct Request {
    pub method: &'static str,
    pub url: String,
    /// `application/json` or `application/x-www-form-urlencoded`, with the body; none for a GET.
    pub body: Option<(&'static str, String)>,
}

/// Where auth's metadata is, at `address`.
pub fn metadata(address: &str) -> Request {
    Request { method: "GET", url: format!("{}/.well-known/oauth-authorization-server", address.trim_end_matches('/')), body: None }
}

/// The network's auth, as its metadata says it.
pub struct Auth {
    pub address: String,
    meta: Value,
}

/// An authorization asked for: its query, and the verifier and state it's finished with.
pub struct Asked {
    pub query: String,
    state: String,
    verifier: String,
    redirect: String,
    client: String,
}

impl Asked {
    /// As a client keeps it while its user signs in elsewhere (a browser's page goes to auth and back).
    pub fn to_json(&self) -> Value {
        json!({ "query": self.query, "state": self.state, "verifier": self.verifier, "redirect": self.redirect, "client": self.client })
    }

    pub fn from_json(v: &Value) -> Option<Asked> {
        let s = |k: &str| v[k].as_str().map(str::to_string);
        Some(Asked { query: s("query")?, state: s("state")?, verifier: s("verifier")?, redirect: s("redirect")?, client: s("client")? })
    }
}

/// What a grant gives: an access token, for `expires_in` seconds, and the next refresh token (each works once, so it's
/// kept before anything else can ask).
pub struct Tokens {
    pub access: String,
    pub expires_in: u64,
    pub refresh: Option<String>,
}

impl Auth {
    /// The auth of the network at `address`, from what its `metadata` answered.
    pub fn of(address: &str, status: u16, meta: Value) -> Result<Auth, Failed> {
        let address = address.trim_end_matches('/').to_string();
        if status != 200 || !meta.is_object() {
            return Err(failed(format!("Can’t reach {address}. Check it, and that you’re online.")));
        }
        Ok(Auth { address, meta })
    }

    fn endpoint(&self, name: &str) -> Result<&str, Failed> {
        self.meta[name].as_str().ok_or_else(|| failed(format!("{}'s auth has no {name}.", self.address)))
    }

    /// Registering a client called `name` that comes back to `back`; `native`: an app on a device (RFC 8252), coming
    /// back to its loopback or its own link. Its answer is `registered`'s.
    pub fn registration(&self, name: &str, back: &str, native: bool) -> Result<Request, Failed> {
        let mut body = json!({
            "client_name": name, "redirect_uris": [back], "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"],
        });
        if native {
            body["application_type"] = json!("native");
        }
        Ok(Request { method: "POST", url: self.endpoint("registration_endpoint")?.into(), body: Some(("application/json", body.to_string())) })
    }

    /// What a registration answered → the client's id.
    pub fn registered(status: u16, out: &Value) -> Result<String, Failed> {
        match out["client_id"].as_str() {
            Some(id) if (200..300).contains(&status) => Ok(id.to_string()),
            _ => Err(failed(format!("This device couldn’t sign up to sign in: {}", said(out, "auth said no")))),
        }
    }

    /// An authorization for `client`, coming back to `redirect`, for the device `join` names (its key ‖ seal key, hex),
    /// which whoever answers its request then checks.
    pub fn ask(&self, client: &str, redirect: &str, join: &str) -> Asked {
        let verifier = B64U.encode(&rand::random::<[u8; 32]>());
        let state = B64U.encode(&rand::random::<[u8; 32]>());
        let challenge = B64U.encode(&key::sha256(verifier.as_bytes()));
        let fields = [
            ("response_type", "code"),
            ("client_id", client),
            ("redirect_uri", redirect),
            ("scope", "openid profile offline_access"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("prompt", "consent"),
            ("join", join),
        ];
        Asked { query: form(&fields), verifier, state: state.clone(), redirect: redirect.into(), client: client.into() }
    }

    /// Where the user goes to sign in for `asked`.
    pub fn url(&self, asked: &Asked) -> Result<String, Failed> {
        Ok(format!("{}?{}", self.endpoint("authorization_endpoint")?, asked.query))
    }

    /// What came back to `asked`'s redirect (its query's fields), exchanged for tokens: the grant to send.
    pub fn finishing(&self, asked: &Asked, got: &HashMap<String, String>) -> Result<Request, Failed> {
        if let Some(e) = got.get("error") {
            return Err(failed(if e == "access_denied" { "You didn’t let this device in." } else { got.get("error_description").unwrap_or(e) }));
        }
        let code = got.get("code").filter(|_| got.get("state") == Some(&asked.state)).ok_or_else(|| failed("That sign-in didn’t start here. Sign in again."))?;
        self.grant(&[("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &asked.redirect), ("client_id", &asked.client), ("code_verifier", &asked.verifier)])
    }

    /// `client`'s refresh token exchanged for new tokens: the grant to send.
    pub fn refresh_grant(&self, client: &str, refresh: &str) -> Result<Request, Failed> {
        self.grant(&[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", client)])
    }

    fn grant(&self, fields: &[(&str, &str)]) -> Result<Request, Failed> {
        Ok(Request { method: "POST", url: self.endpoint("token_endpoint")?.into(), body: Some(("application/x-www-form-urlencoded", form(fields))) })
    }

    /// What a grant answered → its tokens; a refused one (a used or revoked refresh token) signs out.
    pub fn granted(status: u16, out: &Value) -> Result<Tokens, Failed> {
        match out["access_token"].as_str() {
            Some(access) if status == 200 => Ok(Tokens { access: access.into(), expires_in: out["expires_in"].as_u64().unwrap_or(3600), refresh: out["refresh_token"].as_str().map(str::to_string) }),
            _ if matches!(status, 400 | 401) => Err(signed_out(format!("Sign in again: {}", said(out, "auth refused this device's sign-in")))),
            _ => Err(failed(said(out, "Signing in didn’t work."))),
        }
    }

    /// `client`'s refresh token revoked, so it opens nothing again, where auth revokes.
    pub fn revocation(&self, client: &str, refresh: &str) -> Option<Request> {
        let at = self.endpoint("revocation_endpoint").ok()?;
        Some(Request { method: "POST", url: at.into(), body: Some(("application/x-www-form-urlencoded", form(&[("token", refresh), ("token_type_hint", "refresh_token"), ("client_id", client)]))) })
    }
}

/// Natively, each step sent with ureq as it's made.
#[cfg(not(target_family = "wasm"))]
mod sent {
    use super::*;

    fn send(r: Request) -> Result<(u16, Value), Failed> {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(30))).http_status_as_error(false).build().into();
        let (request, twice) = match r.body {
            None => (ureq::http::Request::get(&r.url).body(Vec::new()), platform::hedge::Twice::Safe),
            Some((kind, body)) => (ureq::http::Request::post(&r.url).header("content-type", kind).body(body.into()), platform::hedge::Twice::Never),
        };
        let (mut answer, _) = platform::hedge::run(&agent, "auth", request.map_err(|e| failed(e.to_string()))?, twice).map_err(|e| failed(e.to_string()))?;
        Ok((answer.status().as_u16(), answer.body_mut().read_json().unwrap_or_default()))
    }

    impl Auth {
        /// The auth of the network at `address`.
        pub fn at(address: &str) -> Result<Auth, Failed> {
            let (status, meta) = send(metadata(address)).unwrap_or((0, Value::Null));
            Auth::of(address, status, meta)
        }

        /// Registers a client called `name` that comes back to `back` → its id.
        pub fn register(&self, name: &str, back: &str, native: bool) -> Result<String, Failed> {
            let (status, out) = send(self.registration(name, back, native)?)?;
            Auth::registered(status, &out)
        }

        /// What came back to `asked`'s redirect (its query's fields), exchanged for tokens.
        pub fn finish(&self, asked: &Asked, got: &HashMap<String, String>) -> Result<Tokens, Failed> {
            let (status, out) = send(self.finishing(asked, got)?)?;
            Auth::granted(status, &out)
        }

        /// `client`'s refresh token exchanged for new tokens.
        pub fn refresh(&self, client: &str, refresh: &str) -> Result<Tokens, Failed> {
            let (status, out) = send(self.refresh_grant(client, refresh)?)?;
            Auth::granted(status, &out)
        }

        /// `client`'s refresh token revoked, so it opens nothing again.
        pub fn revoke(&self, client: &str, refresh: &str) {
            if let Some(r) = self.revocation(client, refresh) {
                let _ = send(r);
            }
        }
    }
}

/// Who an access token is for: its `sub`.
pub fn subject(token: &str) -> Option<String> {
    let part = token.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&B64U.decode(part.trim_end_matches('=').as_bytes()).ok()?).ok()?;
    claims["sub"].as_str().filter(|s| !s.is_empty()).map(str::to_string)
}
