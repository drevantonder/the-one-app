//! The kernel services (`kernel/`) running in the pinned workerd, as nodes and the auth service use them.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use data_encoding::BASE64URL_NOPAD as B64U;
use one::kernel::{self, Served};
use serde_json::{Value, json};

/// The kernel's files as they are in this checkout, tests and all.
fn files() -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((path.strip_prefix(root).unwrap().to_string_lossy().into(), std::fs::read(&path).unwrap()));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("kernel");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out
}

fn module(entry: &str) -> Vec<u8> {
    kernel::module_from(files(), entry).unwrap()
}

fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// A workerd running kernel services, stopped when dropped.
struct Running {
    child: std::process::Child,
    dir: std::path::PathBuf,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run(servers: Vec<Served>) -> Running {
    let dir = std::env::temp_dir().join(format!("one-kernel-test-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let addrs: Vec<String> = servers.iter().map(|s| s.listen.clone()).collect();
    let config = kernel::write_config(&dir, &servers).unwrap();
    let child = kernel::start(&config).unwrap();
    let running = Running { child, dir };
    for addr in addrs {
        let started = std::time::Instant::now();
        while std::net::TcpStream::connect(&addr).is_err() {
            assert!(started.elapsed() < Duration::from_secs(30), "workerd didn't start");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    running
}

fn post(url: &str, headers: &[(&str, &str)], body: Value) -> (u16, Value) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut req = agent.post(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut resp = req.send_json(body).unwrap();
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

#[test]
fn the_kernels_own_tests_pass() {
    let dir = std::env::temp_dir().join(format!("one-kernel-tests-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tests.js"), module("test/tests.js")).unwrap();
    std::fs::write(
        dir.join("config.capnp"),
        format!(
            "using Workerd = import \"/workerd/workerd.capnp\";\nconst config :Workerd.Config = (services = [(name = \"tests\", worker = (modules = [(name = \"tests.js\", esModule = embed \"tests.js\")], compatibilityDate = \"{}\"))]);\n",
            one::apps::COMPATIBILITY_DATE
        ),
    )
    .unwrap();
    let out = std::process::Command::new(one::apps::workerd().unwrap()).arg("test").arg(dir.join("config.capnp")).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "the kernel's tests failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

/// A vault, and a stand-in auth service that signs any claims, with the recovery key's secret half.
struct Vault {
    _running: Running,
    url: String,
    schedules: String,
    node: String,
    auth: String,
    issuer: String,
    secret: String,
    recovery: [u8; 32],
}

impl Vault {
    fn start() -> Vault {
        let (vault, auth, schedules, node) = (port(), port(), port(), port());
        let (recovery, recovery_public) = kernel::twin(&rand::random());
        let secret = B64U.encode(&rand::random::<[u8; 32]>());
        let issuer = "https://auth.test/api/auth".to_string();
        let own: [u8; 32] = rand::random();
        let schedules_key = json!({ "secret": B64U.encode(&own), "public": B64U.encode(&kernel::public_of(&own)) }).to_string();
        let bucket = json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }).to_string();
        let running = run(vec![
            Served { name: "auth".into(), module: module("test/auth.js"), class: None, listen: format!("127.0.0.1:{auth}"), bindings: vec![], outbound: None },
            Served { name: "sites".into(), module: module("test/sites.js"), class: None, listen: format!("127.0.0.1:{}", port()), bindings: vec![], outbound: None },
            Served {
                name: "vault".into(),
                module: module("vault/worker.js"),
                class: Some("Vault".into()),
                listen: format!("127.0.0.1:{vault}"),
                bindings: vec![
                    ("AUTH".into(), "service:auth".into()),
                    ("ISSUER".into(), issuer.clone()),
                    ("KERNEL_SECRET".into(), secret.clone()),
                    ("RECOVERY_KEY".into(), B64U.encode(&recovery_public)),
                    ("BUCKET".into(), bucket),
                ],
                outbound: Some("sites".into()),
            },
            Served {
                name: "schedules".into(),
                module: module("schedules/worker.js"),
                class: Some("Schedules".into()),
                listen: format!("127.0.0.1:{schedules}"),
                bindings: vec![
                    ("VAULT".into(), "service:vault".into()),
                    ("AUTH".into(), "service:auth".into()),
                    ("ISSUER".into(), issuer.clone()),
                    ("KERNEL_SECRET".into(), secret.clone()),
                    ("SCHEDULES_KEY".into(), schedules_key),
                ],
                outbound: Some("node".into()),
            },
            Served { name: "node".into(), module: module("test/node.js"), class: None, listen: format!("127.0.0.1:{node}"), bindings: vec![("VAULT".into(), "service:vault".into())], outbound: None },
        ]);
        Vault {
            _running: running,
            url: format!("http://127.0.0.1:{vault}"),
            schedules: format!("http://127.0.0.1:{schedules}"),
            node: format!("http://127.0.0.1:{node}"),
            auth: format!("http://127.0.0.1:{auth}"),
            issuer,
            secret,
            recovery,
        }
    }

    /// Asks as the auth service does.
    fn internal(&self, path: &str, body: Value) -> (u16, Value) {
        post(&format!("{}/internal/{path}", self.url), &[("x-kernel-secret", &self.secret)], body)
    }

    /// An access token for `user` from `client`, good for an hour.
    fn token(&self, user: &str, client: &str) -> (String, u64) {
        let exp = now() / 1000 + 3600;
        let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
        let token = agent.post(format!("{}/sign", self.auth)).send_json(json!({ "iss": self.issuer, "sub": user, "azp": client, "client_id": client, "aud": "https://node.test", "scope": "openid", "exp": exp })).unwrap().body_mut().read_to_string().unwrap();
        (token, exp * 1000)
    }

    /// A client of `user`'s, added with their passkey: → its access token.
    fn client(&self, user: &str, passkey: &str, client: &str) -> String {
        let code = format!("{client}'s authorization code for {user}");
        assert_eq!(self.internal("grant", json!({ "user": user, "from": passkey, "to": code, "client": client })).0, 200);
        let (token, until) = self.token(user, client);
        assert_eq!(self.internal("rekey", json!({ "from": code, "to": format!("{client}'s refresh token for {user}"), "token": token, "until": until })).1["moved"], true);
        token
    }

    fn ask(&self, token: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
        self.ask_at(&self.url, token, method, path, body)
    }

    fn ask_at(&self, server: &str, token: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let url = format!("{server}{path}");
        let mut resp = match method {
            "GET" => agent.get(url).header("authorization", format!("Bearer {token}")).call().unwrap(),
            "DELETE" => agent.delete(url).header("authorization", format!("Bearer {token}")).call().unwrap(),
            _ => agent.post(url).header("authorization", format!("Bearer {token}")).send_json(body).unwrap(),
        };
        (resp.status().as_u16(), resp.body_mut().read_json().unwrap())
    }

    /// The scope keys the recovery key opens.
    fn recovered(&self) -> std::collections::BTreeMap<String, [u8; 32]> {
        let (_, out) = self.internal("recovery", json!({}));
        out["scopes"].as_array().unwrap().iter().map(|s| (s["scope"].as_str().unwrap().to_string(), kernel::open_sealed(&self.recovery, &B64U.decode(s["sealed"].as_str().unwrap().as_bytes()).unwrap()).unwrap().try_into().unwrap())).collect()
    }
}

const ANDRE: &str = "00000000000000a1";
const SAM: &str = "00000000000000b2";

#[test]
fn a_client_added_with_a_passkey_gets_keys_for_what_it_may_use() {
    let v = Vault::start();
    let vault = kernel::Vault { url: v.url.clone() };
    let me = format!("user:{ANDRE}");

    // Andre's first passkey makes his keys, his scope's and, as the install's first person, Everyone's.
    let (status, _) = v.internal("passkey", json!({ "user": ANDRE, "secret": "prf output of the MacBook passkey", "name": "MacBook" }));
    assert_eq!(status, 200);
    let scopes = v.recovered();
    assert_eq!(scopes.keys().cloned().collect::<Vec<_>>(), vec!["global".to_string(), me.clone()]);
    let (_, held) = v.internal("scopes", json!({ "user": ANDRE, "secret": "prf output of the MacBook passkey" }));
    assert_eq!(held["scopes"].as_array().unwrap().len(), 2);

    // Without the kernel's secret, nobody but the auth service may ask that.
    let (status, _) = post(&format!("{}/internal/passkey", v.url), &[("x-kernel-secret", "a guess")], json!({ "user": ANDRE, "secret": "a passkey of my own making" }));
    assert_eq!(status, 403);
    // Nor add a passkey to Andre's without one he has.
    let (status, _) = v.internal("passkey", json!({ "user": ANDRE, "secret": "someone else's passkey prf", "from": "not a passkey of andre's" }));
    assert_eq!(status, 403);

    // Claude Code is added at its consent screen, with that passkey, for Andre's own scope only. Until its first token
    // it has no keys; once the auth service swaps its code for tokens, the token opens what it may use.
    let (status, _) = v.internal("grant", json!({ "user": ANDRE, "from": "prf output of the MacBook passkey", "to": "the authorization code", "name": "Claude Code", "client": "claude-code", "scopes": [me] }));
    assert_eq!(status, 200);
    let (token, until) = v.token(ANDRE, "claude-code");
    assert!(vault.keys(&token, &[(&me, "board", None)]).unwrap_err().to_string().contains("signs in again"));
    let (_, moved) = v.internal("rekey", json!({ "from": "the authorization code", "to": "the first refresh token", "token": token, "until": until }));
    assert_eq!(moved["moved"], true);

    // Nodes check the token offline, as the vault does.
    let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
    let jwks: Value = agent.get(format!("{}/jwks", v.auth)).call().unwrap().body_mut().read_json().unwrap();
    let claims = kernel::verify(&token, &jwks, &v.issuer, now()).unwrap();
    assert_eq!((claims.sub.as_str(), claims.azp.as_str(), claims.for_resource("https://node.test")), (ANDRE, "claude-code", true));
    assert!(kernel::verify(&token, &jwks, "https://someone-else.test", now()).is_err());
    assert!(kernel::verify(&token, &jwks, &v.issuer, until + 1000).is_err());

    let keys = vault.keys(&token, &[(&me, "board", None), (&me, "board", Some("main"))]).unwrap();
    let board = kernel::app_key(&scopes[&me], "board");
    assert_eq!(keys[0].key, board, "the vault derives an app's key as nodes do");
    assert_eq!(keys[1].key, kernel::database_key(&board, "main"));
    assert!(vault.keys(&token, &[("global", "board", None)]).unwrap_err().to_string().contains("may not use global"));

    // The code is spent; the refresh token moves on with each refresh, and only the newest opens anything.
    let (_, again) = v.internal("rekey", json!({ "from": "the authorization code", "to": "an attacker's refresh token", "token": token, "until": until }));
    assert_eq!(again["moved"], false);
    let (second, until) = v.token(ANDRE, "claude-code");
    let (_, moved) = v.internal("rekey", json!({ "from": "the first refresh token", "to": "the second refresh token", "token": second, "until": until }));
    assert_eq!(moved["moved"], true);
    assert_eq!(vault.keys(&second, &[(&me, "board", None)]).unwrap()[0].key, board);
    let (_, stale) = v.internal("rekey", json!({ "from": "the first refresh token", "to": "a third refresh token", "token": second, "until": until }));
    assert_eq!(stale["moved"], false);

    // A token the auth service didn't sign opens nothing.
    let forged = format!("{}.{}", &second[..second.rfind('.').unwrap()], B64U.encode(&[0; 64]));
    assert!(vault.keys(&forged, &[(&me, "board", None)]).unwrap_err().to_string().contains("signature"));

    // Every unseal is in Andre's own record.
    let record: Value = agent.get(format!("{}/unseals", v.url)).header("authorization", format!("Bearer {second}")).call().unwrap().body_mut().read_json().unwrap();
    let unseals = record["unseals"].as_array().unwrap();
    assert_eq!(unseals.len(), 2);
    assert_eq!(unseals[1]["client"], "claude-code");
    assert_eq!(unseals[1]["what"], json!([format!("{me}/board"), format!("{me}/board/main")]));

    // Andre's clients: the passkey and Claude Code. Removing Claude Code's copy leaves its token's session, until
    // it expires, but its refresh token opens nothing again.
    let copies: Value = agent.get(format!("{}/copies", v.url)).header("authorization", format!("Bearer {second}")).call().unwrap().body_mut().read_json().unwrap();
    let names: Vec<&str> = copies["copies"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["MacBook", "Claude Code"]);
    let claude = copies["copies"][1]["id"].as_str().unwrap();
    assert_eq!(claude, kernel::id_of(b"the second refresh token"));
    let left: Value = agent.delete(format!("{}/copies/{claude}", v.url)).header("authorization", format!("Bearer {second}")).call().unwrap().body_mut().read_json().unwrap();
    assert_eq!(left["left"], 1);
    let (_, gone) = v.internal("rekey", json!({ "from": "the second refresh token", "to": "a fourth", "token": second, "until": until }));
    assert_eq!(gone["moved"], false);
}

#[test]
fn nodes_get_create_only_urls_for_objects_in_scopes_they_may_use() {
    let v = Vault::start();
    let vault = kernel::Vault { url: v.url.clone() };
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    v.internal("passkey", json!({ "user": SAM, "secret": "sam's own passkey prf output" }));
    let mine = kernel::folder_of(&format!("user:{ANDRE}"));
    v.internal("grant", json!({ "user": ANDRE, "from": "andre's passkey prf output", "to": "andre's code here", "client": "iphone" }));
    let (token, until) = v.token(ANDRE, "iphone");
    v.internal("rekey", json!({ "from": "andre's code here", "to": "andre's refresh token", "token": token, "until": until }));

    let name = format!("{mine}/board/main/changes/00000000000000000001");
    let newest = format!("{mine}/board/main/newest");
    let urls = vault.urls(&token, &[&format!("{mine}/board/main/copy")], &[&name], &[(&newest, "\"etag-1\"")]).unwrap();
    assert_eq!(url_query(&urls.replace[&newest])["X-Amz-SignedHeaders"], "host;if-match");
    let create = url_query(&urls.create[&name]);
    assert!(urls.create[&name].starts_with(&format!("https://s3.test/install/{mine}/board/main/changes/")));
    assert_eq!(create["X-Amz-SignedHeaders"], "host;if-none-match");
    assert!(urls.until > now());
    assert_eq!(url_query(&urls.read[&format!("{mine}/board/main/copy")])["X-Amz-SignedHeaders"], "host");

    // Sam's objects, and names that climb out of an app, aren't Andre's to reach; Everyone's are (he's its first).
    let sams = format!("{}/board/main/copy", kernel::folder_of(&format!("user:{SAM}")));
    assert!(vault.urls(&token, &[&sams], &[], &[]).is_err());
    assert!(vault.urls(&token, &[], &[&format!("{mine}/board/../../x")], &[]).is_err());
    assert!(vault.urls(&token, &[], &[&format!("{mine}/board")], &[]).is_err());
    assert!(vault.urls(&token, &[], &[], &[(&name, "\"etag-1\"")]).is_err(), "only a newest is replaced");
    assert!(vault.urls(&token, &["log/00aa/000000000001"], &["log/00aa/000000000002"], &[]).is_ok());
    assert!(vault.urls(&token, &["global/board/main/newest"], &[], &[]).is_ok());

    // Once Andre shares Everyone with Sam, Sam's clients may reach it too.
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let shared = agent.post(format!("{}/share", v.url)).header("authorization", format!("Bearer {token}")).send_json(json!({ "scope": "global", "user": SAM })).unwrap();
    assert_eq!(shared.status(), 200);
    v.internal("grant", json!({ "user": SAM, "from": "sam's own passkey prf output", "to": "sam's authorization code", "client": "iphone" }));
    let (sam, until) = v.token(SAM, "iphone");
    v.internal("rekey", json!({ "from": "sam's authorization code", "to": "sam's refresh token", "token": sam, "until": until }));
    let andre = vault.keys(&token, &[("global", "board", None)]).unwrap();
    assert_eq!(vault.keys(&sam, &[("global", "board", None)]).unwrap(), andre);
    assert!(vault.keys(&sam, &[(&format!("user:{ANDRE}"), "board", None)]).is_err());
    // A client limited to Sam's own scope can't reach the install's log.
    v.internal("grant", json!({ "user": SAM, "from": "sam's own passkey prf output", "to": "sam's second code here", "client": "mac", "scopes": [format!("user:{SAM}")] }));
    let (mac, until) = v.token(SAM, "mac");
    v.internal("rekey", json!({ "from": "sam's second code here", "to": "sam's mac refresh token", "token": mac, "until": until }));
    assert!(vault.urls(&mac, &["log/00aa/000000000001"], &[], &[]).is_err());
}

fn url_query(url: &str) -> std::collections::BTreeMap<String, String> {
    let query = url.split_once('?').unwrap().1;
    query.split('&').map(|p| p.split_once('=').unwrap()).map(|(k, v)| (k.to_string(), v.replace("%3B", ";"))).collect()
}

#[test]
fn the_vault_fills_an_apps_requests_with_secrets_it_never_shows() {
    let v = Vault::start();
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    let token = v.client(ANDRE, "andre's passkey prf output", "mac");
    let me = format!("user:{ANDRE}");
    let id = "0123456789abcdef";
    let item = json!({ "id": id, "title": "Google sign-in", "type": "Google account", "hosts": ["oauth2.googleapis.com"], "apps": ["gmail"],
        "fields": { "client_id": { "value": "the client id" }, "refresh_token": { "value": "the refresh token itself", "secret": true } } });
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": me, "item": item })).0, 200);

    // Listed, a secret field says only that it's secret.
    let (_, list) = v.ask(&token, "GET", "/items", json!({}));
    assert!(!list.to_string().contains("the refresh token itself"));
    assert_eq!(list["items"][0]["fields"], json!({ "client_id": { "value": "the client id" }, "refresh_token": { "secret": true } }));

    // Gmail's refresh goes out with it filled in, to its host, through the vault.
    let fetch = |app: &str, url: &str, body: &str| v.ask(&token, "POST", "/fetch", json!({ "app": app, "url": url, "method": "POST", "headers": { "content-type": "text/plain" }, "body": body }));
    let (status, sent) = fetch("gmail", "https://oauth2.googleapis.com/token", &format!("grant_type=refresh_token&refresh_token={{{{{id}:refresh_token}}}}&client_id={{{{client_id}}}}"));
    assert_eq!(status, 200, "{sent}");
    let echoed: Value = serde_json::from_slice(&B64U.decode(sent["body64"].as_str().unwrap().as_bytes()).unwrap()).unwrap();
    assert_eq!(echoed["body"], "grant_type=refresh_token&refresh_token=the refresh token itself&client_id=the client id");
    assert_eq!(echoed["url"], "https://oauth2.googleapis.com/token");

    // Never to another host, never for an app it isn't switched on for, never over plain http.
    let (status, refused) = fetch("gmail", "https://evil.example.com/", &format!("{{{{{id}:refresh_token}}}}"));
    assert_eq!(status, 400);
    assert!(refused["error"].as_str().unwrap().contains("sent only to oauth2.googleapis.com"));
    let (_, other) = fetch("calendar", "https://oauth2.googleapis.com/token", &format!("{{{{{id}:refresh_token}}}}"));
    let echoed: Value = serde_json::from_slice(&B64U.decode(other["body64"].as_str().unwrap().as_bytes()).unwrap()).unwrap();
    assert_eq!(echoed["body"], format!("{{{{{id}:refresh_token}}}}"));
    assert_eq!(fetch("gmail", "http://oauth2.googleapis.com/token", "x").0, 400);

    // A sidecar named among an item's hosts gets its field filled into what it sends on.
    let key = json!({ "id": "00000000000000ff", "title": "Work browser", "hosts": ["agent-computers"], "apps": ["agent-computers"], "fields": { "key": { "generate": true } } });
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": me, "item": key })).0, 200);
    let (_, filled) = v.ask(&token, "POST", "/fill", json!({ "app": "agent-computers", "host": "agent-computers", "text": "{{00000000000000ff:key}}" }));
    assert_eq!(filled["text"].as_str().unwrap().len(), 43, "a generated key: {filled}");
    assert_eq!(v.ask(&token, "POST", "/fill", json!({ "app": "agent-computers", "host": "shell", "text": "{{00000000000000ff:key}}" })).0, 400);

    // Each use is in Andre's record.
    let (_, record) = v.ask(&token, "GET", "/unseals", json!({}));
    let what: Vec<String> = record["unseals"].as_array().unwrap().iter().flat_map(|u| u["what"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string())).collect();
    assert!(what.contains(&format!("gmail used {id}")) && what.contains(&"agent-computers used 00000000000000ff".to_string()), "{what:?}");

    // Deleted, it fills nothing.
    assert_eq!(v.ask(&token, "POST", "/items", json!({ "scope": me, "item": { "id": id, "deleted": true } })).0, 200);
    let (_, gone) = fetch("gmail", "https://oauth2.googleapis.com/token", &format!("{{{{{id}:refresh_token}}}}"));
    let echoed: Value = serde_json::from_slice(&B64U.decode(gone["body64"].as_str().unwrap().as_bytes()).unwrap()).unwrap();
    assert_eq!(echoed["body"], format!("{{{{{id}:refresh_token}}}}"));
}

/// A browser's cookies and Origin, for asking the auth service as its screens do.
struct Browser {
    origin: String,
    cookies: std::collections::BTreeMap<String, String>,
}

impl Browser {
    fn send(&mut self, method: &str, path: &str, body: Option<Value>, form: Option<&[(&str, &str)]>) -> (u16, Value, Option<String>) {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).build().into();
        let url = if path.starts_with("http") { path.to_string() } else { format!("{}{path}", self.origin) };
        let cookie = self.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
        let mut resp = match (method, body, form) {
            ("GET", ..) => agent.get(&url).header("origin", &self.origin).header("cookie", &cookie).call().unwrap(),
            (_, _, Some(form)) => agent.post(&url).header("origin", &self.origin).send_form(form.iter().copied()).unwrap(),
            (_, body, _) => agent.post(&url).header("origin", &self.origin).header("cookie", &cookie).send_json(body.unwrap_or(json!({}))).unwrap(),
        };
        for set in resp.headers().get_all("set-cookie") {
            let pair = set.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.insert(name.trim().into(), value.into());
        }
        let location = resp.headers().get("location").map(|l| l.to_str().unwrap().to_string());
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::String(text)), location)
    }

    fn post(&mut self, path: &str, body: Value) -> (u16, Value) {
        let (status, out, _) = self.send("POST", path, Some(body), None);
        (status, out)
    }

    fn get(&mut self, path: &str) -> (u16, Value, Option<String>) {
        self.send("GET", path, None, None)
    }
}

/// A passkey kept in software, as an authenticator keeps one: an Ed25519 key, with attestation "none".
struct Passkey {
    key: ed25519_dalek::SigningKey,
    id: Vec<u8>,
    rp: String,
    origin: String,
}

fn cbor(v: &ciborium::Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(v, &mut out).unwrap();
    out
}

impl Passkey {
    fn client_data(&self, kind: &str, challenge: &str) -> Vec<u8> {
        json!({ "type": kind, "challenge": challenge, "origin": self.origin, "crossOrigin": false }).to_string().into_bytes()
    }

    fn rp_hash(&self) -> Vec<u8> {
        use sha2::Digest;
        sha2::Sha256::digest(self.rp.as_bytes()).to_vec()
    }

    /// What `navigator.credentials.create` gives, as JSON, for registration options.
    fn create(&self, options: &Value) -> Value {
        use ciborium::Value as C;
        let cose = C::Map(vec![(C::Integer(1.into()), C::Integer(1.into())), (C::Integer(3.into()), C::Integer((-8).into())), (C::Integer((-1).into()), C::Integer(6.into())), (C::Integer((-2).into()), C::Bytes(self.key.verifying_key().to_bytes().to_vec()))]);
        let mut auth = self.rp_hash();
        auth.push(0x45 | 0x18); // user present, verified, backed up, with a credential
        auth.extend(0u32.to_be_bytes());
        auth.extend([0; 16]);
        auth.extend((self.id.len() as u16).to_be_bytes());
        auth.extend(&self.id);
        auth.extend(cbor(&cose));
        let attestation = C::Map(vec![(C::Text("fmt".into()), C::Text("none".into())), (C::Text("attStmt".into()), C::Map(vec![])), (C::Text("authData".into()), C::Bytes(auth))]);
        let id = B64U.encode(&self.id);
        let client = self.client_data("webauthn.create", options["challenge"].as_str().unwrap());
        json!({ "id": id, "rawId": id, "type": "public-key", "clientExtensionResults": {},
            "response": { "clientDataJSON": B64U.encode(&client), "attestationObject": B64U.encode(&cbor(&attestation)), "transports": ["internal"] } })
    }

    /// What `navigator.credentials.get` gives, as JSON, for authentication options.
    fn get(&self, options: &Value, user: &str) -> Value {
        use ed25519_dalek::Signer;
        use sha2::Digest;
        let mut auth = self.rp_hash();
        auth.push(0x05 | 0x18);
        auth.extend(1u32.to_be_bytes());
        let client = self.client_data("webauthn.get", options["challenge"].as_str().unwrap());
        let signature = self.key.sign(&[&auth[..], &sha2::Sha256::digest(&client)[..]].concat());
        let id = B64U.encode(&self.id);
        json!({ "id": id, "rawId": id, "type": "public-key", "clientExtensionResults": {},
            "response": { "clientDataJSON": B64U.encode(&client), "authenticatorData": B64U.encode(&auth), "signature": B64U.encode(&signature.to_bytes()), "userHandle": B64U.encode(user.as_bytes()) } })
    }
}

/// The real auth service and vault, as an install runs them.
struct Kernel {
    running: Running,
    auth: String,
    vault: String,
    secret: String,
    recovery: [u8; 32],
}

const RESOURCE: &str = "https://node.test/mcp";

impl Kernel {
    fn start() -> Kernel {
        let (vault, auth) = (port(), port());
        let (recovery, recovery_public) = kernel::twin(&rand::random());
        let secret = B64U.encode(&rand::random::<[u8; 32]>());
        let origin = format!("http://localhost:{auth}");
        let bucket = json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }).to_string();
        let running = run(vec![
            Served {
                name: "auth".into(),
                module: module("auth/worker.js"),
                class: Some("Auth".into()),
                listen: format!("127.0.0.1:{auth}"),
                bindings: vec![
                    ("VAULT".into(), "service:vault".into()),
                    ("AUTH_URL".into(), origin.clone()),
                    ("AUTH_SECRET".into(), B64U.encode(&rand::random::<[u8; 32]>())),
                    ("KERNEL_SECRET".into(), secret.clone()),
                    ("RESOURCES".into(), json!([RESOURCE]).to_string()),
                ],
                outbound: None,
            },
            Served {
                name: "vault".into(),
                module: module("vault/worker.js"),
                class: Some("Vault".into()),
                listen: format!("127.0.0.1:{vault}"),
                bindings: vec![
                    ("AUTH".into(), "service:auth".into()),
                    ("ISSUER".into(), format!("{origin}/api/auth")),
                    ("KERNEL_SECRET".into(), secret.clone()),
                    ("RECOVERY_KEY".into(), B64U.encode(&recovery_public)),
                    ("BUCKET".into(), bucket),
                ],
                outbound: None,
            },
        ]);
        Kernel { running, auth: origin, vault: format!("http://127.0.0.1:{vault}"), secret, recovery }
    }

    fn browser(&self) -> Browser {
        Browser { origin: self.auth.clone(), cookies: Default::default() }
    }

    fn passkey(&self) -> Passkey {
        Passkey { key: ed25519_dalek::SigningKey::from_bytes(&rand::random()), id: rand::random::<[u8; 16]>().to_vec(), rp: "localhost".into(), origin: self.auth.clone() }
    }
}

fn query_of(url: &str) -> std::collections::BTreeMap<String, String> {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    query.split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (dec(k), dec(v))).collect()
}

fn dec(s: &str) -> String {
    let bytes = s.replace('+', " ").into_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 {
            out.push(u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap(), 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[test]
fn an_mcp_client_signs_in_with_a_passkey_and_its_tokens_open_its_keys() {
    let k = Kernel::start();
    let mut browser = k.browser();

    // The authorization server says where everything is, where MCP clients look for it.
    let (status, meta, _) = browser.get("/.well-known/oauth-authorization-server/api/auth");
    assert_eq!(status, 200, "{meta}");
    let issuer = meta["issuer"].as_str().unwrap().to_string();
    assert_eq!(issuer, format!("{}/api/auth", k.auth));
    for field in ["authorization_endpoint", "token_endpoint", "registration_endpoint", "jwks_uri"] {
        assert!(meta[field].is_string(), "no {field} in {meta}");
    }

    // The install's first person makes an account; nobody else can without an invite.
    let (status, made) = browser.post("/api/auth/sign-up/email", json!({ "name": "Andre", "email": "andre@example.com", "password": "a long enough password" }));
    assert_eq!(status, 200, "{made}");
    let user = made["user"]["id"].as_str().unwrap().to_string();
    assert!(user.len() == 16 && user.chars().all(|c| c.is_ascii_hexdigit()), "a user id is 16 hex characters: {user}");
    let (status, _) = k.browser().post("/api/auth/sign-up/email", json!({ "name": "Mallory", "email": "mallory@example.com", "password": "a long enough password" }));
    assert_ne!(status, 200, "a stranger can't sign up");

    // He makes a passkey.
    let passkey = k.passkey();
    let (status, options, _) = browser.get("/api/auth/passkey/generate-register-options");
    assert_eq!(status, 200, "{options}");
    let (status, registered) = browser.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "name": "MacBook" }));
    assert_eq!(status, 200, "{registered}");

    // Claude Code registers itself and sends him to authorize it.
    let (status, client) = k.browser().post(meta["registration_endpoint"].as_str().unwrap(), json!({
        "client_name": "Claude Code", "redirect_uris": ["http://localhost:9999/callback"], "token_endpoint_auth_method": "none",
        "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"] }));
    assert_eq!(status, 201, "{client}");
    let client_id = client["client_id"].as_str().unwrap().to_string();
    let verifier = B64U.encode(&rand::random::<[u8; 32]>());
    let challenge = { use sha2::Digest; B64U.encode(&sha2::Sha256::digest(verifier.as_bytes())) };
    let authorize = format!(
        "{}?response_type=code&client_id={client_id}&redirect_uri={}&scope={}&state=s1&code_challenge={challenge}&code_challenge_method=S256&resource={}",
        meta["authorization_endpoint"].as_str().unwrap(), enc("http://localhost:9999/callback"), enc("openid offline_access"), enc(RESOURCE)
    );
    let (status, _, location) = browser.get(&authorize);
    assert_eq!(status, 302);
    let location = location.unwrap();
    assert!(location.contains("/consent?"), "signed in, he's sent to consent: {location}");
    let oauth_query = location.split_once('?').unwrap().1.to_string();

    // At the consent screen, his passkey (and its PRF output) opens his keys; he lets Claude Code use his own.
    let (_, options, _) = browser.get("/api/auth/passkey/generate-authenticate-options");
    let (status, signed) = browser.post("/api/auth/passkey/verify-authentication", json!({ "response": passkey.get(&options, &user) }));
    assert_eq!(status, 200, "{signed}");
    let prf = B64U.encode(&rand::random::<[u8; 32]>());
    let (_, about, _) = browser.get(&format!("/consent/about?client_id={client_id}"));
    assert_eq!(about["client"], "Claude Code", "{about}");
    let (status, scopes) = browser.post("/consent/scopes", json!({ "prf": prf, "credential": B64U.encode(&passkey.id) }));
    assert_eq!(status, 200, "{scopes}");
    let mine = format!("user:{user}");
    let mut opened: Vec<String> = scopes["scopes"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
    opened.sort();
    assert_eq!(opened, ["global".to_string(), mine.clone()], "his first passkey makes his keys, and Everyone's");
    let (status, wrong) = browser.post("/consent/scopes", json!({ "prf": B64U.encode(&rand::random::<[u8; 32]>()), "credential": B64U.encode(&passkey.id) }));
    assert_eq!(status, 403, "another PRF output opens nothing: {wrong}");
    let (status, accepted) = browser.post("/consent/accept", json!({ "oauth_query": oauth_query, "accept": true, "prf": prf, "scopes": [mine] }));
    assert_eq!(status, 200, "{accepted}");
    let redirect = accepted["url"].as_str().unwrap();
    let params = query_of(redirect);
    assert!(redirect.starts_with("http://localhost:9999/callback"), "{redirect}");
    assert_eq!(params["state"], "s1");

    // Claude Code swaps the code for tokens: a JWT that nodes check offline, and that opens its keys.
    let (status, tokens, _) = k.browser().send("POST", meta["token_endpoint"].as_str().unwrap(), None, Some(&[
        ("grant_type", "authorization_code"), ("code", &params["code"]), ("redirect_uri", "http://localhost:9999/callback"),
        ("client_id", &client_id), ("code_verifier", &verifier), ("resource", RESOURCE)]));
    assert_eq!(status, 200, "{tokens}");
    let access = tokens["access_token"].as_str().unwrap().to_string();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_string();
    let (_, jwks, _) = k.browser().get(meta["jwks_uri"].as_str().unwrap());
    let claims = kernel::verify(&access, &jwks, &issuer, now()).unwrap();
    assert_eq!((claims.sub.as_str(), claims.azp.as_str(), claims.for_resource(RESOURCE)), (user.as_str(), client_id.as_str(), true), "{claims:?}");

    let vault = kernel::Vault { url: k.vault.clone() };
    let (_, recovery) = post(&format!("{}/internal/recovery", k.vault), &[("x-kernel-secret", &k.secret)], json!({}));
    let sealed = recovery["scopes"].as_array().unwrap().iter().find(|s| s["scope"] == mine.as_str()).unwrap()["sealed"].as_str().unwrap().to_string();
    let scope_key: [u8; 32] = kernel::open_sealed(&k.recovery, &B64U.decode(sealed.as_bytes()).unwrap()).unwrap().try_into().unwrap();
    assert_eq!(vault.keys(&access, &[(&mine, "board", None)]).unwrap()[0].key, kernel::app_key(&scope_key, "board"));
    assert!(vault.keys(&access, &[("global", "board", None)]).is_err(), "he gave it only his own");

    // Better Auth keeps the refresh token only as its hash.
    let dbs = std::fs::read_dir(k.running.dir.join("data/auth/auth")).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "sqlite"));
    let conn = dbs.map(|db| rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()).find(|c| c.prepare("select token from oauthRefreshToken").is_ok()).expect("Better Auth's database");
    let stored: Vec<String> = conn.prepare("select token from oauthRefreshToken").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    let hashed = { use sha2::Digest; B64U.encode(&sha2::Sha256::digest(refresh.as_bytes())) };
    assert_eq!(stored, vec![hashed], "only the refresh token's hash is kept");
    drop(conn);

    // Refreshing moves its keys to the new refresh token; the old one opens nothing.
    let refreshed = |token: &str| k.browser().send("POST", meta["token_endpoint"].as_str().unwrap(), None, Some(&[("grant_type", "refresh_token"), ("refresh_token", token), ("client_id", &client_id), ("resource", RESOURCE)]));
    let (status, again, _) = refreshed(&refresh);
    assert_eq!(status, 200, "{again}");
    let access2 = again["access_token"].as_str().unwrap();
    assert_ne!(again["refresh_token"].as_str().unwrap(), refresh);
    assert_eq!(vault.keys(access2, &[(&mine, "board", None)]).unwrap()[0].key, kernel::app_key(&scope_key, "board"));

    // Signing Claude Code out (revoking its refresh token) leaves it nothing to open.
    let latest = again["refresh_token"].as_str().unwrap().to_string();
    let (status, revoked, _) = k.browser().send("POST", &format!("{issuer}/oauth2/revoke"), None, Some(&[("token", &latest), ("token_type_hint", "refresh_token"), ("client_id", &client_id)]));
    assert_eq!(status, 200, "{revoked}");
    let (_, moved) = post(&format!("{}/internal/rekey", k.vault), &[("x-kernel-secret", &k.secret)], json!({ "from": latest, "to": "anything at all, long", "token": access2, "until": now() + 60_000 }));
    assert_eq!(moved["moved"], false);
    let (status, _, _) = refreshed(&refresh);
    assert_ne!(status, 200, "a spent refresh token is refused");

    // Removing his passkey: it opens nothing again, at the next consent.
    let (_, list, _) = browser.get("/api/auth/passkey/list-user-passkeys");
    let (status, _) = browser.post("/api/auth/passkey/delete-passkey", json!({ "id": list[0]["id"] }));
    assert_eq!(status, 200);
    let (_, held) = post(&format!("{}/internal/scopes", k.vault), &[("x-kernel-secret", &k.secret)], json!({ "user": user, "secret": prf }));
    assert!(held["error"].is_string(), "{held}");
}


#[test]
fn a_schedule_runs_once_as_its_person_with_only_what_it_was_let_use() {
    let v = Vault::start();
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    let token = v.client(ANDRE, "andre's passkey prf output", "iphone");
    let me = format!("user:{ANDRE}");

    // Andre lets Morning summary run, using Gmail, at 9 in Melbourne; it runs next when Rust says it does.
    let spec = json!({ "op": "summary", "at": "09:00", "tz": "Australia/Melbourne", "days": ["mon", "tue", "wed", "thu", "fri"] });
    let mut add = json!({ "app": "gmail", "name": "morning", "scope": me, "title": "Morning summary", "what": "Gmail, read only", "node": "https://node.test" });
    add.as_object_mut().unwrap().extend(spec.as_object().unwrap().clone());
    let (status, made) = v.ask_at(&v.schedules, &token, "POST", "/schedules", add);
    assert_eq!(status, 200, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    let (_, list) = v.ask_at(&v.schedules, &token, "GET", "/schedules", json!({}));
    let listed = &list["schedules"][0];
    assert_eq!((listed["title"].as_str(), listed["what"].as_str()), (Some("Morning summary"), Some("Gmail, read only")));
    let rust = one::schedules::Schedule::parse("morning", &spec).unwrap();
    assert_eq!(listed["next"].as_u64(), rust.next(now()), "JavaScript and Rust agree when it runs next");

    // Run now: the node is handed a run whose token opens Gmail's key in Andre's scope, and nothing else.
    let (status, ran) = v.ask_at(&v.schedules, &token, "POST", &format!("/schedules/{id}/run"), json!({}));
    assert_eq!((status, ran["run"]["status"].as_str()), (200, Some("ran")), "{ran}");
    let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
    let runs: Value = agent.get(format!("{}/runs", v.node)).call().unwrap().body_mut().read_json().unwrap();
    let seen = &runs[0];
    assert_eq!(seen["host"], "node.test");
    let run: kernel::Run = serde_json::from_value(seen["run"].clone()).unwrap();
    assert_eq!((run.app.as_str(), run.schedule.as_str(), run.op.as_str(), run.scope.as_str()), ("gmail", "morning", "summary", me.as_str()));
    assert!(run.run.starts_with(&format!("{id}/")));
    let session: kernel::Session = serde_json::from_value(seen["session"]["body"].clone()).unwrap();
    assert_eq!((session.user.as_str(), session.client.as_deref(), session.scopes.clone(), session.apps.clone()), (ANDRE, Some("schedules"), vec![me.clone()], Some(vec!["gmail".to_string()])));
    assert_eq!(seen["keys"]["status"], 200);
    assert_eq!(seen["other"]["status"], 403, "a run can't use an app it wasn't let use");
    let (_, record) = v.ask(&token, "GET", "/unseals", json!({}));
    assert_eq!(record["unseals"][0]["client"], "schedules", "the run's unseal is in Andre's record");

    // Stopped, its secret opens nothing again.
    assert_eq!(v.ask_at(&v.schedules, &token, "DELETE", &format!("/schedules/{id}"), json!({})).0, 200);
    assert_eq!(v.ask_at(&v.schedules, &token, "POST", &format!("/schedules/{id}/run"), json!({})).0, 404);
    let (_, list) = v.ask_at(&v.schedules, &token, "GET", "/schedules", json!({}));
    assert_eq!(list["schedules"], json!([]));
}

#[test]
fn a_schedules_alarm_runs_each_slot_once() {
    let v = Vault::start();
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    let token = v.client(ANDRE, "andre's passkey prf output", "iphone");
    let add = json!({ "app": "board", "name": "tidy", "scope": format!("user:{ANDRE}"), "op": "tidy", "every": "1m", "node": "https://node.test" });
    assert_eq!(v.ask_at(&v.schedules, &token, "POST", "/schedules", add).0, 200);
    let started = std::time::Instant::now();
    loop {
        let (_, list) = v.ask_at(&v.schedules, &token, "GET", "/schedules", json!({}));
        if list["schedules"][0]["last"]["status"] == "ran" {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(75), "the alarm didn't run it: {list}");
        std::thread::sleep(Duration::from_millis(500));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
    let runs: Value = agent.get(format!("{}/runs", v.node)).call().unwrap().body_mut().read_json().unwrap();
    assert_eq!(runs.as_array().unwrap().len(), 1, "one slot, one run");
}
