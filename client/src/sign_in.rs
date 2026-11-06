//! Signing in at the network's address, as every client but a browser does (the CLI and MCP, the native app): an OAuth
//! client of its auth, from its metadata (RFC 8414), registered the first time (RFC 7591), with an authorization
//! code and PKCE, refreshed, and revoked. Where a client keeps its registration and tokens, and how the person reaches
//! the authorization and comes back, are its own.

use std::collections::HashMap;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD as B64U;
use serde_json::{Value, json};

/// Why signing in didn't work, in words a person reads; `signed_out` when auth refused this client's tokens (one used
/// or revoked), so its person signs in again.
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

fn failed(message: impl Into<String>) -> Failed {
    Failed { signed_out: false, message: message.into() }
}

/// Signed out: nothing here signs in until its person does again.
pub fn signed_out(message: impl Into<String>) -> Failed {
    Failed { signed_out: true, message: message.into() }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into()
}

/// What auth said went wrong, or `otherwise`.
fn said(out: &Value, otherwise: &str) -> String {
    out["error_description"].as_str().or(out["error"].as_str()).or(out["message"].as_str()).unwrap_or(otherwise).to_string()
}

/// `fields` as a form's body, or a URL's query.
fn form(fields: &[(&str, &str)]) -> String {
    fields.iter().map(|(k, v)| format!("{}={}", one::url_encode(k), one::url_encode(v))).collect::<Vec<_>>().join("&")
}

/// The network's auth, as its metadata says it.
pub struct Auth {
    pub address: String,
    meta: Value,
}

/// An authorization asked for: its query, and the verifier and state it's finished with.
pub struct Asked {
    pub query: String,
    /// What a code's sign-in takes its code by (the CLI's).
    pub state: String,
    verifier: String,
    redirect: String,
    client: String,
}

/// What a grant gives: an access token, for `expires_in` seconds, and the next refresh token (each works once, so it's
/// kept before anything else can ask).
pub struct Tokens {
    pub access: String,
    pub expires_in: u64,
    pub refresh: Option<String>,
}

impl Auth {
    /// The auth of the network at `address`.
    pub fn at(address: &str) -> Result<Auth, Failed> {
        let address = address.trim_end_matches('/').to_string();
        let unreachable = || failed(format!("Can’t reach {address}. Check it, and that you’re online."));
        let mut r = agent().get(format!("{address}/.well-known/oauth-authorization-server")).call().map_err(|_| unreachable())?;
        if r.status() != 200 {
            return Err(unreachable());
        }
        let meta = r.body_mut().read_json().map_err(|_| unreachable())?;
        Ok(Auth { address, meta })
    }

    fn endpoint(&self, name: &str) -> Result<&str, Failed> {
        self.meta[name].as_str().ok_or_else(|| failed(format!("{}'s auth has no {name}.", self.address)))
    }

    /// Registers a client called `name` that comes back to `back` → its id. `native`: an app on a device (RFC 8252),
    /// coming back to its loopback or its own link.
    pub fn register(&self, name: &str, back: &str, native: bool) -> Result<String, Failed> {
        let mut body = json!({
            "client_name": name, "redirect_uris": [back], "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"],
        });
        if native {
            body["application_type"] = json!("native");
        }
        let mut r = agent().post(self.endpoint("registration_endpoint")?).send_json(body).map_err(|e| failed(e.to_string()))?;
        let ok = r.status().is_success();
        let out: Value = r.body_mut().read_json().unwrap_or_default();
        match out["client_id"].as_str() {
            Some(id) if ok => Ok(id.to_string()),
            _ => Err(failed(format!("This device couldn’t sign up to sign in: {}", said(&out, "auth said no")))),
        }
    }

    /// An authorization for `client`, coming back to `redirect`, for the device `join` names (its key ‖ seal key, hex),
    /// which its person's devices then let in by this sign-in.
    pub fn ask(&self, client: &str, redirect: &str, join: Option<&str>) -> Asked {
        let verifier = B64U.encode(&rand::random::<[u8; 32]>());
        let state = B64U.encode(&rand::random::<[u8; 32]>());
        let challenge = B64U.encode(&one::keys::sha256(verifier.as_bytes()));
        let mut fields = vec![
            ("response_type", "code"),
            ("client_id", client),
            ("redirect_uri", redirect),
            ("scope", "openid profile offline_access"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("prompt", "consent"),
        ];
        fields.extend(join.map(|j| ("join", j)));
        Asked { query: form(&fields), verifier, state: state.clone(), redirect: redirect.into(), client: client.into() }
    }

    /// Where the person goes to sign in for `asked`.
    pub fn url(&self, asked: &Asked) -> Result<String, Failed> {
        Ok(format!("{}?{}", self.endpoint("authorization_endpoint")?, asked.query))
    }

    /// What came back to `asked`'s redirect (its query's fields), exchanged for tokens.
    pub fn finish(&self, asked: &Asked, got: &HashMap<String, String>) -> Result<Tokens, Failed> {
        if let Some(e) = got.get("error") {
            return Err(failed(if e == "access_denied" { "You didn’t let this device in." } else { got.get("error_description").unwrap_or(e) }));
        }
        let code = got.get("code").filter(|_| got.get("state") == Some(&asked.state)).ok_or_else(|| failed("That sign-in didn’t start here. Sign in again."))?;
        self.code(asked, code)
    }

    /// `asked`'s code, exchanged for tokens.
    pub fn code(&self, asked: &Asked, code: &str) -> Result<Tokens, Failed> {
        self.grant(&[("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &asked.redirect), ("client_id", &asked.client), ("code_verifier", &asked.verifier)])
    }

    /// `client`'s refresh token exchanged for new tokens.
    pub fn refresh(&self, client: &str, refresh: &str) -> Result<Tokens, Failed> {
        self.grant(&[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", client)])
    }

    fn grant(&self, fields: &[(&str, &str)]) -> Result<Tokens, Failed> {
        let mut r = agent().post(self.endpoint("token_endpoint")?).header("content-type", "application/x-www-form-urlencoded").send(form(fields)).map_err(|e| failed(e.to_string()))?;
        let status = r.status().as_u16();
        let out: Value = r.body_mut().read_json().unwrap_or_default();
        match out["access_token"].as_str() {
            Some(access) if status == 200 => Ok(Tokens { access: access.into(), expires_in: out["expires_in"].as_u64().unwrap_or(3600), refresh: out["refresh_token"].as_str().map(str::to_string) }),
            _ if matches!(status, 400 | 401) => Err(signed_out(format!("Sign in again: {}", said(&out, "auth refused this device's sign-in")))),
            _ => Err(failed(said(&out, "Signing in didn’t work."))),
        }
    }

    /// `client`'s refresh token revoked, so it opens nothing again.
    pub fn revoke(&self, client: &str, refresh: &str) {
        if let Ok(at) = self.endpoint("revocation_endpoint") {
            let _ = agent().post(at).header("content-type", "application/x-www-form-urlencoded").send(form(&[("token", refresh), ("token_type_hint", "refresh_token"), ("client_id", client)]));
        }
    }
}

/// Who an access token is for: its `sub`.
pub fn subject(token: &str) -> Option<String> {
    let part = token.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&B64U.decode(part.trim_end_matches('=').as_bytes()).ok()?).ok()?;
    claims["sub"].as_str().filter(|s| !s.is_empty()).map(str::to_string)
}
