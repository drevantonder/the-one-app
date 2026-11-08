//! AI Usage's service: the AI plans signed in on its machine, read from the providers themselves; the app asks every
//! machine's service and merges by account. Its accounts are the provider sign-ins kept here: Claude Code's own and
//! every account claude-swap keeps, Codex's and OpenCode Go's. It asks each provider for that account's numbers and
//! gives back only those (used %, reset times, per-model windows, plan): a token never leaves it, not to the app, not
//! in a log or an error. Read only, never a refresh (which would sign the tool it belongs to out): a sign-in whose
//! access token ran out says so, since it renews the next time its tool is used.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use one_service::{Answer, Call, Host, Service};


const ASK: Duration = Duration::from_secs(15);

fn main() -> Result<()> {
    one_service::serve(Service { name: "ai-usage", prefer_here: &[], call: |h, c| call(h, c).map(Answer::new), start: None })
}

/// A sign-in found on this machine: who it is, and its token, which only ever reaches its own provider.
struct Account {
    provider: &'static str,
    /// The tool that renews it: "Claude Code", "claude-swap", "Codex", "OpenCode".
    tool: &'static str,
    email: String,
    org: String,
    active: bool,
    disabled: bool,
    plan: Option<String>,
    token: String,
    expires_at: Option<u64>,
    /// Per-provider: Codex's account id.
    account_id: String,
}

fn reading(a: &Account) -> Value {
    // The label a user tells accounts apart by, as claude-swap's list gave them: the team (Personal for one's own
    // plan) and the email.
    let label = if a.email.is_empty() { String::new() } else if a.org.is_empty() || a.org.ends_with("'s Organization") { format!("Personal · {}", a.email) } else { format!("{} · {}", a.org, a.email) };
    let mut out = json!({ "provider": a.provider, "account": label, "email": a.email, "org": a.org, "active": a.active, "disabled": a.disabled, "plan": a.plan });
    if let Some(at) = a.expires_at.filter(|at| *at < now_ms()) {
        out["error"] = json!(format!("its sign-in ran out {} ago; it renews when {} is next used", ago(at), a.tool));
        out["expired"] = json!(true);
        return out;
    }
    match fetch(a) {
        Ok(read) => {
            out.as_object_mut().unwrap().extend(read.as_object().unwrap().clone());
        }
        Err(err) => out["error"] = json!(format!("{err:#}")),
    }
    out
}

fn ago(at: u64) -> String {
    let m = now_ms().saturating_sub(at) / 60000;
    if m < 60 { format!("{m} min") } else if m < 1440 { format!("{} h", m / 60) } else { format!("{} d", m / 1440) }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn call(_host: &Host, call: &Call) -> Result<Value> {
    match call.function.as_str() {
        "accounts" => Ok(json!({ "accounts": discover().iter().map(|a| json!({
            "provider": a.provider, "account": if a.org.is_empty() { a.email.clone() } else { format!("{} · {}", a.org, a.email) },
            "email": a.email, "org": a.org, "active": a.active, "disabled": a.disabled,
            "expired": a.expires_at.is_some_and(|at| at < now_ms()),
        })).collect::<Vec<_>>() })),
        "usage" => {
            let accounts = discover();
            let readings = std::thread::scope(|s| {
                let asked: Vec<_> = accounts.iter().map(|a| s.spawn(move || reading(a))).collect();
                asked.into_iter().filter_map(|t| t.join().ok()).collect::<Vec<_>>()
            });
            Ok(json!({ "accounts": readings }))
        }
        function => bail!("ai-usage has no function {function}"),
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()))
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn discover() -> Vec<Account> {
    let mut out = claude_swap_accounts();
    if out.is_empty()
        && let Some(a) = claude_code_account()
    {
        out.push(a);
    }
    if let Some(a) = codex_account() {
        out.push(a);
    }
    if let Some(a) = opencode_account() {
        out.push(a);
    }
    out
}

/// The OAuth fields of a credentials file's JSON (`~/.claude/.credentials.json`, and claude-swap's .enc files, which
/// are the same JSON base64'd): its access token and when it runs out.
fn oauth(text: &str) -> Option<(String, Option<u64>)> {
    let o = serde_json::from_str::<Value>(text).ok()?["claudeAiOauth"].clone();
    let token = o["accessToken"].as_str()?.to_string();
    Some((token, o["expiresAt"].as_u64()))
}

/// A claude-swap account's credentials: its `.creds-<n>-<email>.enc` file (base64'd JSON), or on a Mac the Keychain
/// item claude-swap keeps instead.
fn cswap_credentials(dir: &Path, num: &str, email: &str) -> Option<String> {
    let file = dir.join("credentials").join(format!(".creds-{num}-{email}.enc"));
    if let Ok(text) = std::fs::read_to_string(&file) {
        let decoded = data_encoding::BASE64.decode(text.trim().as_bytes()).ok()?;
        return String::from_utf8(decoded).ok();
    }
    keychain("claude-swap", &format!("account-{num}-{email}"))
}

/// A Keychain item's text (macOS only): what `security find-generic-password -w` gives, nothing when it has none.
fn keychain(_service: &str, _account: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let account = if _account.is_empty() { std::env::var("USER").ok()? } else { _account.to_string() };
        let out = std::process::Command::new("/usr/bin/security").args(["find-generic-password", "-s", _service, "-a", &account, "-w"]).output().ok()?;
        if out.status.success() { return Some(String::from_utf8_lossy(&out.stdout).trim().to_string()); }
    }
    None
}

/// claude-swap's accounts, in its rotation order, from its sequence.json; each one's credentials from its own file
/// (or Keychain item). Its root is `$XDG_DATA_HOME/claude-swap` (~/.local/share/claude-swap) on Linux and
/// ~/.claude-swap-backup on a Mac.
fn claude_swap_accounts() -> Vec<Account> {
    let mut roots = vec![home().join(".claude-swap-backup")];
    match std::env::var_os("XDG_DATA_HOME").filter(|x| !x.is_empty()) {
        Some(x) => roots.insert(0, PathBuf::from(x).join("claude-swap")),
        None => roots.insert(0, home().join(".local/share/claude-swap")),
    }
    let Some(dir) = roots.into_iter().find(|d| d.join("sequence.json").is_file()) else { return Vec::new() };
    let Some(seq) = read_json(&dir.join("sequence.json")) else { return Vec::new() };
    let active = seq["activeAccountNumber"].as_u64().unwrap_or(0);
    let Some(accounts) = seq["accounts"].as_object() else { return Vec::new() };
    let mut order: Vec<String> = seq["sequence"].as_array().into_iter().flatten().filter_map(|n| n.as_u64().map(|n| n.to_string())).collect();
    for n in accounts.keys() {
        if !order.contains(n) {
            order.push(n.clone());
        }
    }
    let mut out = Vec::new();
    for n in order {
        let Some(meta) = accounts.get(&n) else { continue };
        let email = meta["email"].as_str().unwrap_or("").to_string();
        let Some(creds) = cswap_credentials(&dir, &n, &email) else { continue };
        let Some((token, expires_at)) = oauth(&creds) else { continue };
        out.push(Account {
            provider: "claude",
            tool: "Claude Code",
            email,
            org: meta["organizationName"].as_str().unwrap_or("").to_string(),
            active: active.to_string() == n,
            disabled: meta["disabled"].as_bool().unwrap_or(false),
            plan: None,
            token,
            expires_at,
            account_id: String::new(),
        });
    }
    out
}

/// Claude Code's own sign-in, when no claude-swap is here: `~/.claude/.credentials.json` (on a Mac, the Keychain
/// item Claude Code uses), its user and team from `~/.claude.json`'s oauthAccount.
fn claude_code_account() -> Option<Account> {
    let config = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".claude"));
    let text = std::fs::read_to_string(config.join(".credentials.json")).ok().or_else(|| keychain("Claude Code-credentials", ""))?;
    let (token, expires_at) = oauth(&text)?;
    let me = read_json(&home().join(".claude.json")).unwrap_or_default()["oauthAccount"].clone();
    Some(Account {
        provider: "claude",
        tool: "Claude Code",
        email: me["emailAddress"].as_str().unwrap_or("").to_string(),
        org: me["organizationName"].as_str().unwrap_or("").to_string(),
        active: true,
        disabled: false,
        plan: None,
        token,
        expires_at,
        account_id: String::new(),
    })
}

/// Codex's sign-in: `~/.codex/auth.json`'s access token and account id, its email in the id token's claims.
fn codex_account() -> Option<Account> {
    let dir = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".codex"));
    let auth = read_json(&dir.join("auth.json"))?;
    let tokens = &auth["tokens"];
    let token = tokens["access_token"].as_str()?.to_string();
    let claims = jwt_claims(tokens["id_token"].as_str().unwrap_or(""));
    Some(Account {
        provider: "codex",
        tool: "Codex",
        email: claims["email"].as_str().unwrap_or("").to_string(),
        org: String::new(),
        active: true,
        disabled: false,
        plan: claims["https://api.openai.com/auth"]["chatgpt_plan_type"].as_str().or_else(|| claims["chatgpt_plan_type"].as_str()).map(str::to_string),
        token,
        expires_at: None,
        account_id: tokens["account_id"].as_str().unwrap_or("").to_string(),
    })
}

/// The claims of a JWT's payload, without checking it (it's this machine's own file, only its email and plan read).
fn jwt_claims(token: &str) -> Value {
    let Some(part) = token.split('.').nth(1) else { return Value::Null };
    data_encoding::BASE64URL_NOPAD.decode(part.as_bytes()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

/// OpenCode Go's sign-in: `~/.local/share/opencode/auth.json` where older OpenCode kept it, or the `credential`
/// table of its opencode.db, which holds `{"type":"key","key":"oc_sk_…"}` for the `opencode-go` integration.
fn opencode_account() -> Option<Account> {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("opencode");
    if let Some(auth) = read_json(&data.join("auth.json")) {
        let key = auth["opencode"]["key"].as_str().or_else(|| auth["opencode"]["access"].as_str()).or_else(|| auth["opencodego"]["key"].as_str());
        if let Some(key) = key {
            return Some(opencode(key.to_string(), String::new()));
        }
    }
    let db = data.join("opencode.db");
    if db.is_file()
        && let Some((key, email)) = opencode_db(&db)
    {
        return Some(opencode(key, email));
    }
    None
}

fn opencode(key: String, email: String) -> Account {
    Account { provider: "opencodego", tool: "OpenCode", email, org: String::new(), active: true, disabled: false, plan: Some("Go".into()), token: key, expires_at: None, account_id: String::new() }
}

/// The Go key and, where it's kept, the account's email: `credential.value` for `opencode-go`, `account.email`.
fn opencode_db(db: &Path) -> Option<(String, String)> {
    let conn = rusqlite::Connection::open_with_flags_and_vfs(format!("file:{}?mode=ro", db.display()), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI, "unix").ok()?;
    let _ = conn.busy_timeout(Duration::from_secs(2));
    let key: String = conn.query_row("select value from credential where integration_id = 'opencode-go' limit 1", [], |r| r.get(0)).ok()?;
    let key = serde_json::from_str::<Value>(&key).ok()?["key"].as_str()?.to_string();
    let email: String = conn.query_row("select email from account limit 1", [], |r| r.get(0)).unwrap_or_default();
    Some((key, email))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(ASK)).build().into()
}

/// A GET with the account's bearer token; only ever to its provider's own usage endpoint.
fn get(a: &Account, url: &str, headers: &[(&str, &str)]) -> Result<Value> {
    let mut req = agent().get(url).header("Authorization", format!("Bearer {}", a.token));
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let mut resp = req.call().map_err(|err| match err {
        ureq::Error::StatusCode(401 | 403) => anyhow!("its sign-in was refused; it renews when {} is next used", a.tool),
        ureq::Error::StatusCode(429) => anyhow!("the provider is rate limiting reads"),
        _ => anyhow!("{} couldn't be reached", a.provider_title()),
    })?;
    Ok(resp.body_mut().read_json()?)
}

impl Account {
    fn provider_title(&self) -> &'static str {
        match self.provider {
            "claude" => "Claude",
            "codex" => "Codex",
            _ => "OpenCode",
        }
    }
}

fn win(used: Option<f64>, resets_at: Option<u64>) -> Value {
    json!({ "used": used, "resetsAt": resets_at })
}

/// An ISO 8601 time, or unix seconds or ms, as ms.
fn as_ms(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64().map(|t| if t < 1_000_000_000_000 { t * 1000 } else { t }),
        Value::String(s) => s.parse::<jiff::Timestamp>().ok().map(|t| t.as_millisecond() as u64).or_else(|| s.parse::<u64>().ok().map(|t| if t < 1_000_000_000_000 { t * 1000 } else { t })),
        _ => None,
    }
}

fn pct(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn fetch(a: &Account) -> Result<Value> {
    match a.provider {
        "claude" => claude_usage(a),
        "codex" => codex_usage(a),
        _ => opencode_usage(a),
    }
}

/// Claude's own usage endpoint, as Claude Code asks it: five-hour and seven-day windows, the per-model weekly ones
/// (Sonnet, Opus), and extra usage credits.
fn claude_usage(a: &Account) -> Result<Value> {
    let j = get(a, "https://api.anthropic.com/api/oauth/usage", &[("anthropic-beta", "oauth-2025-04-20"), ("User-Agent", "claude-code/2.0")])?;
    let w = |k: &str| win(pct(&j[k]["utilization"]), as_ms(&j[k]["resets_at"]));
    let mut models = Vec::new();
    for (key, name) in [("seven_day_sonnet", "Sonnet"), ("seven_day_opus", "Opus")] {
        if j.get(key).is_some_and(|m| m.is_object()) {
            models.push(json!({ "name": name, "used": pct(&j[key]["utilization"]), "resetsAt": as_ms(&j[key]["resets_at"]) }));
        }
    }
    let mut out = json!({ "session": w("five_hour"), "weekly": w("seven_day"), "monthly": win(None, None), "models": models });
    if j["extra_usage"].is_object() {
        out["extra"] = json!({ "used": j["extra_usage"]["used_credits"], "limit": j["extra_usage"]["monthly_limit"] });
    }
    Ok(out)
}

/// Codex's usage endpoint, which Codex itself calls: the primary (five-hour) and secondary (weekly) windows, and the
/// plan it signs in as.
fn codex_usage(a: &Account) -> Result<Value> {
    let j = get(a, "https://chatgpt.com/backend-api/wham/usage", &[("ChatGPT-Account-Id", &a.account_id), ("User-Agent", "codex/1.0")])?;
    let rate = &j["rate_limit"];
    let w = |k: &str| {
        let w = &rate[k];
        win(pct(&w["used_percent"]), as_ms(&w["reset_at"]))
    };
    let mut out = json!({ "session": w("primary_window"), "weekly": w("secondary_window"), "monthly": win(None, None), "models": [] });
    if out["session"]["used"].is_null() && out["weekly"]["used"].is_null() {
        bail!("Codex's usage endpoint gave no windows");
    }
    if let Some(plan) = j["plan_type"].as_str().filter(|p| !p.is_empty()) {
        out["plan"] = json!(plan);
    }
    Ok(out)
}

/// OpenCode Go's usage: the rolling (five-hour), weekly and monthly windows, each `{"percent","resetsAt","status"}`.
fn opencode_usage(a: &Account) -> Result<Value> {
    let j = get(a, "https://opencode.ai/zen/go/v1/usage", &[("User-Agent", "opencode/2.2.5")])?;
    let u = &j["usage"];
    let w = |k: &str| win(pct(&u[k]["percent"]), as_ms(&u[k]["resetsAt"]));
    let out = json!({ "session": w("rolling"), "weekly": w("weekly"), "monthly": w("monthly"), "models": [] });
    if out["session"]["used"].is_null() && out["weekly"]["used"].is_null() {
        bail!("OpenCode's usage endpoint gave no windows");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn oauth_fields() {
        let creds = data_encoding::BASE64.encode(br#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x","expiresAt":1793766965457}}"#);
        let (token, expires_at) = super::oauth(&String::from_utf8(data_encoding::BASE64.decode(creds.as_bytes()).unwrap()).unwrap()).unwrap();
        assert_eq!(token, "sk-ant-oat01-x");
        assert_eq!(expires_at, Some(1793766965457));
        assert!(super::oauth("{}").is_none());
    }
}
