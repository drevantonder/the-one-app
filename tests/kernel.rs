//! The kernel services (`kernel/`) running in the pinned workerd, as nodes and the auth service use them.

use std::collections::BTreeSet;
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
    store: Store,
}

impl Vault {
    fn start() -> Vault {
        let (vault, auth, schedules, node, sites) = (port(), port(), port(), port(), port());
        let (recovery, recovery_public) = kernel::twin(&rand::random());
        let secret = B64U.encode(&rand::random::<[u8; 32]>());
        let issuer = "https://auth.test/api/auth".to_string();
        let own: [u8; 32] = rand::random();
        let schedules_key = json!({ "secret": B64U.encode(&own), "public": B64U.encode(&kernel::public_of(&own)) }).to_string();
        let bucket = json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }).to_string();
        let running = run(vec![
            Served { name: "auth".into(), module: module("test/auth.js"), class: None, listen: format!("127.0.0.1:{auth}"), bindings: vec![], assets: None, outbound: None },
            Served { name: "sites".into(), module: module("test/sites.js"), class: None, listen: format!("127.0.0.1:{sites}"), bindings: vec![], assets: None, outbound: None },
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
                assets: None,
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
                assets: None,
                outbound: Some("node".into()),
            },
            Served { name: "node".into(), module: module("test/node.js"), class: None, listen: format!("127.0.0.1:{node}"), bindings: vec![("VAULT".into(), "service:vault".into())], assets: None, outbound: None },
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
            store: Store::Standin(format!("http://127.0.0.1:{sites}")),
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
        ask_at(server, token, method, path, body)
    }

    /// The scope keys the recovery key opens.
    fn recovered(&self) -> std::collections::BTreeMap<String, [u8; 32]> {
        recovered(&self.url, &self.secret, &self.recovery)
    }
}

/// Asks a kernel service with a bearer token.
fn ask_at(server: &str, token: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let url = format!("{server}{path}");
    let mut resp = match method {
        "GET" => agent.get(url).header("authorization", format!("Bearer {token}")).call().unwrap(),
        "DELETE" => agent.delete(url).header("authorization", format!("Bearer {token}")).call().unwrap(),
        _ => agent.post(url).header("authorization", format!("Bearer {token}")).send_json(body).unwrap(),
    };
    (resp.status().as_u16(), resp.body_mut().read_json().unwrap())
}

/// The scope keys the recovery key opens, at the vault at `url`.
fn recovered(url: &str, secret: &str, recovery: &[u8; 32]) -> std::collections::BTreeMap<String, [u8; 32]> {
    let (_, out) = post(&format!("{url}/internal/recovery"), &[("x-kernel-secret", secret)], json!({}));
    let b = |v: &Value| B64U.decode(v.as_str().unwrap().as_bytes()).unwrap();
    out["scopes"].as_array().unwrap().iter().map(|s| (s["scope"].as_str().unwrap().to_string(), kernel::open_sealed(recovery, &b(&s["sealed"])).unwrap().try_into().unwrap())).collect()
}

/// The install's bucket, as a test reads it: the stand-in sites' (`<url>/install/…`), or a real one.
enum Store {
    Standin(String),
    Real(one::bucket::Place),
}

impl Store {
    /// An app's key in a scope, as nodes open it: its copy in the bucket, sealed under the scope's key.
    fn app_key(&self, scope_key: &[u8; 32], scope: &str, app: &str) -> Option<[u8; 32]> {
        let name = kernel::app_key_name(scope, app);
        let sealed = match self {
            Store::Standin(url) => {
                let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
                let mut resp = agent.get(format!("{url}/install/{name}")).call().unwrap();
                (resp.status() == 200).then(|| resp.body_mut().read_to_vec().unwrap())
            }
            Store::Real(place) => place.get(&name).unwrap(),
        }?;
        Some(kernel::open_app_key(scope_key, app, &sealed).unwrap())
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
    let board = v.store.app_key(&scopes[&me], &me, "board").expect("the Board's key, in the bucket");
    assert_eq!(keys[0].key, board, "an app's key is kept in the bucket sealed under its scope's, as nodes open it");
    assert_ne!(board, keys[1].key);
    assert_eq!(keys[1].key, kernel::database_key(&board, "main"), "a database's key derives from its app's as nodes do");
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

    // Erasing the Board in his scope destroys its key: nothing opens its data again, and it starts over with a new one.
    assert_eq!(v.ask(&second, "POST", "/erase", json!({ "scope": me, "app": "board" })).0, 200);
    assert!(v.store.app_key(&scopes[&me], &me, "board").is_none(), "its key is gone from the bucket");
    assert_ne!(vault.keys(&second, &[(&me, "board", None)]).unwrap()[0].key, board);
    assert_eq!(v.ask(&second, "POST", "/erase", json!({ "scope": "global", "app": "board" })).0, 403, "only in a scope it may use");
}

#[test]
fn a_client_let_only_read_some_apps_gets_only_that() {
    let v = Vault::start();
    let vault = kernel::Vault { url: v.url.clone() };
    let me = format!("user:{ANDRE}");
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    let full = v.client(ANDRE, "andre's passkey prf output", "iphone");
    vault.keys(&full, &[(&me, "board", None), (&me, "gmail", None)]).unwrap();

    // The consent screen offers each scope with the apps that have data there.
    let (_, offered) = v.internal("scopes", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    assert_eq!(offered["apps"][&me], json!(["board", "gmail"]), "{offered}");

    // He lets Claude Code only read, and only the Board.
    let code = "claude code's authorization code";
    let (status, _) = v.internal("grant", json!({ "user": ANDRE, "from": "andre's passkey prf output", "to": code, "client": "claude-code", "scopes": [me], "apps": ["board"], "read_only": true }));
    assert_eq!(status, 200);
    let (token, until) = v.token(ANDRE, "claude-code");
    assert_eq!(v.internal("rekey", json!({ "from": code, "to": "claude code's refresh token", "token": token, "until": until })).1["moved"], true);
    let session = vault.session(&token).unwrap();
    assert_eq!((session.apps.clone(), session.read_only), (Some(vec!["board".to_string()]), true));
    assert!(vault.keys(&token, &[(&me, "board", None)]).is_ok());
    assert!(vault.keys(&token, &[(&me, "gmail", None)]).is_err(), "only the Board");
    let name = format!("{}/board/main/newest", one::backups::folder(&me));
    assert!(vault.urls(&token, &[&name], &[], &[]).is_ok(), "it reads");
    assert!(vault.urls(&token, &[], &[&name], &[]).unwrap_err().to_string().contains("only read"), "and writes nothing");
    assert_eq!(v.ask(&token, "POST", "/erase", json!({ "scope": me, "app": "board" })).0, 403);
    assert_eq!(v.ask(&token, "POST", "/fetch", json!({ "app": "board", "url": "https://example.com/send", "method": "POST" })).0, 403, "nor sends");
}

#[test]
fn nodes_get_create_only_urls_for_objects_in_scopes_they_may_use() {
    let v = Vault::start();
    let vault = kernel::Vault { url: v.url.clone() };
    v.internal("passkey", json!({ "user": ANDRE, "secret": "andre's passkey prf output" }));
    v.internal("passkey", json!({ "user": SAM, "secret": "sam's own passkey prf output" }));
    let mine = one::backups::folder(&format!("user:{ANDRE}"));
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
    let sams = format!("{}/board/main/copy", one::backups::folder(&format!("user:{SAM}")));
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

    /// Its PRF output, as a browser gets it at the install's address.
    fn prf(&self) -> String {
        B64U.encode(&self.id.repeat(2))
    }

    /// What `navigator.credentials.create` gives, as JSON, for registration options: only whether it has PRF, as the
    /// pages send it.
    fn create(&self, options: &Value) -> Value {
        self.create_with(options, true)
    }

    fn create_with(&self, options: &Value, prf: bool) -> Value {
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
        json!({ "id": id, "rawId": id, "type": "public-key", "clientExtensionResults": if prf { json!({ "prf": { "enabled": true } }) } else { json!({}) },
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

/// The real kernel services, as an install runs them: people reach them at its address, the gateway's.
struct Kernel {
    /// The workerd running them here, if not deployed.
    running: Option<Running>,
    address: String,
    auth: String,
    vault: String,
    schedules: String,
    secret: String,
    recovery: [u8; 32],
    /// The recovery key, as its holder keeps it.
    recovery_key: String,
    settings: kernel::Settings,
    store: Store,
}

const RESOURCE: &str = "https://node.test/mcp";

impl Kernel {
    /// Runs them as `one kernel serve` does, from new settings, with the stand-in sites as the bucket, the gateway
    /// sending `/v1/` to `node_url` and serving a web client of two pages.
    fn start(node_url: Option<String>) -> Kernel {
        let gateway = port();
        let (mut settings, recovery) = kernel::Settings::new("Test", &format!("http://localhost:{gateway}"));
        settings.ports = [port(), port(), port(), gateway];
        settings.auth_url = format!("http://127.0.0.1:{}", settings.ports[0]);
        (settings.vault_url, settings.schedules_url) = (format!("http://127.0.0.1:{}", settings.ports[1]), format!("http://127.0.0.1:{}", settings.ports[2]));
        settings.resources = vec![RESOURCE.into()];
        settings.node_url = node_url;
        settings.bucket = Some(json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }));
        let modules = kernel::SERVICES.iter().map(|(_, entry, _)| module(entry)).collect();
        let sites = port();
        let web = std::env::temp_dir().join(format!("one-kernel-web-{}", rand::random::<u64>()));
        std::fs::create_dir_all(web.join("screen")).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html><title>The web client</title>").unwrap();
        std::fs::write(web.join("frame.html"), "<!doctype html><title>A screen's frame</title>").unwrap();
        std::fs::write(web.join("screen/kit.css"), include_str!("../src/screen/kit.css")).unwrap();
        let mut served = kernel::served(&settings, modules, &web);
        served[1].outbound = Some("sites".into());
        served.push(Served { name: "sites".into(), module: module("test/sites.js"), class: None, listen: format!("127.0.0.1:{sites}"), bindings: vec![], assets: None, outbound: None });
        let running = run(served);
        let seed: [u8; 32] = data_encoding::HEXLOWER.decode(recovery.as_bytes()).unwrap().try_into().unwrap();
        Kernel {
            running: Some(running),
            address: settings.address.clone(),
            auth: settings.auth_url.clone(),
            vault: settings.vault_url.clone(),
            schedules: settings.schedules_url.clone(),
            secret: settings.kernel_secret.clone(),
            recovery: kernel::twin(&seed).0,
            recovery_key: recovery,
            settings,
            store: Store::Standin(format!("http://127.0.0.1:{sites}")),
        }
    }

    /// A kernel `one kernel deploy` put up for trying, never an install's own: `ONE_TEST_KERNEL` names its
    /// settings.json (its resources include https://node.test/mcp, and it has a bucket) and `ONE_TEST_KERNEL_RECOVERY`
    /// a file holding its recovery key.
    fn deployed() -> Kernel {
        let read = |var: &str| std::fs::read_to_string(std::env::var(var).unwrap_or_else(|_| panic!("{var} names a file"))).unwrap();
        let settings: kernel::Settings = serde_json::from_str(&read("ONE_TEST_KERNEL")).unwrap();
        assert!(settings.resources.iter().any(|r| r == RESOURCE), "its resources include {RESOURCE}");
        let fields = settings.bucket.as_ref().expect("it has a bucket").as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect();
        let seed: [u8; 32] = data_encoding::HEXLOWER.decode(read("ONE_TEST_KERNEL_RECOVERY").trim().as_bytes()).unwrap().try_into().unwrap();
        Kernel {
            running: None,
            address: settings.address.clone(),
            auth: settings.auth_url.clone(),
            vault: settings.vault_url.clone(),
            schedules: settings.schedules_url.clone(),
            secret: settings.kernel_secret.clone(),
            recovery: kernel::twin(&seed).0,
            recovery_key: read("ONE_TEST_KERNEL_RECOVERY").trim().to_string(),
            settings: settings.clone(),
            store: Store::Real(one::bucket::Place::from_fields(&fields).unwrap()),
        }
    }

    fn browser(&self) -> Browser {
        Browser { origin: self.address.clone(), cookies: Default::default() }
    }

    /// A passkey for the install's address, as passkeys belong to it.
    fn passkey(&self) -> Passkey {
        let rp = self.address.split("://").nth(1).unwrap().split([':', '/']).next().unwrap().to_string();
        Passkey { key: ed25519_dalek::SigningKey::from_bytes(&rand::random()), id: rand::random::<[u8; 16]>().to_vec(), rp, origin: self.address.clone() }
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
    signs_in(&Kernel::start(None));
}

/// The install's first person signs up, makes a passkey with PRF, and lets Claude Code in through OAuth; its tokens
/// open its keys, refresh, and are revoked; his passkey is removed. → a token that still opens his own scope, until it
/// expires, and his user id.
fn signs_in(k: &Kernel) -> (String, String) {
    let mut browser = k.browser();

    // The authorization server says where everything is, where MCP clients look for it.
    let (status, meta, _) = browser.get("/.well-known/oauth-authorization-server/api/auth");
    assert_eq!(status, 200, "{meta}");
    let issuer = meta["issuer"].as_str().unwrap().to_string();
    assert_eq!(issuer, format!("{}/api/auth", k.address));
    for field in ["authorization_endpoint", "token_endpoint", "registration_endpoint", "jwks_uri"] {
        assert!(meta[field].is_string(), "no {field} in {meta}");
    }

    // Nobody signs up uninvited, the install's first person too: there's no passkey to make without an invite link.
    for path in ["/api/auth/passkey/generate-register-options", "/api/auth/passkey/generate-register-options?context=a-link-nobody-made-for-anyone-at-all-here"] {
        let (status, _, _) = k.browser().get(path);
        assert_ne!(status, 200, "uninvited, nobody makes a passkey: {path}");
    }
    for path in ["/api/auth/sign-up/email", "/api/auth/sign-in/email", "/api/auth/sign-in/social"] {
        let (status, _) = k.browser().post(path, json!({ "name": "Mallory", "email": "mallory@example.com", "password": "a long enough password", "provider": "google" }));
        assert_ne!(status, 200, "passkeys only: {path}");
    }

    // A passkey without PRF opens nothing, so it isn't kept, and the link still works.
    let invite = kernel::invite(&k.settings, "Andre", None, || None).unwrap();
    assert!(invite.link.starts_with(&format!("{}/invite#", k.address)), "{}", invite.link);
    assert!(invite.until > now() + 2 * 86_400_000 && invite.until < now() + 8 * 86_400_000, "an invite works for a few days");
    let token = invite.link.split_once('#').unwrap().1.to_string();
    let mut stranger = k.browser();
    let (_, options, _) = stranger.get(&format!("/api/auth/passkey/generate-register-options?context={}", enc(&token)));
    let (status, refused) = stranger.post("/api/auth/passkey/verify-registration", json!({ "response": k.passkey().create_with(&options, false), "createSession": true }));
    assert_eq!(status, 400, "{refused}");
    assert!(refused["message"].as_str().unwrap_or("").contains("iCloud Keychain"), "it says which passkeys work: {refused}");

    // Invited (one kernel invite), he makes his passkey, which makes his keys; the link works once.
    let (passkey, user) = joins_with(k, &mut browser, &token);
    assert!(user.len() == 16 && user.chars().all(|c| c.is_ascii_hexdigit()), "a user id is 16 hex characters: {user}");
    let (status, spent, _) = k.browser().get(&format!("/api/auth/passkey/generate-register-options?context={}", enc(&token)));
    assert_ne!(status, 200, "a spent link makes no passkey: {spent}");

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
    let prf = passkey.prf();
    let (_, about, _) = browser.get(&format!("/consent/about?client_id={client_id}"));
    assert_eq!((about["client"].as_str(), about["person"].as_str()), (Some("Claude Code"), Some("Andre")), "{about}");
    let (status, scopes) = browser.post("/consent/scopes", json!({ "prf": prf, "credential": B64U.encode(&passkey.id) }));
    assert_eq!(status, 200, "{scopes}");
    let mine = format!("user:{user}");
    let mut opened: Vec<String> = scopes["scopes"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
    opened.sort();
    assert_eq!(opened, ["global".to_string(), mine.clone()], "his first passkey makes his keys, and Everyone's");
    assert_eq!(scopes["apps"][&mine], json!([]), "and nothing is there yet: {scopes}");
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
    let board = vault.keys(&access, &[(&mine, "board", None)]).unwrap()[0].key;
    let scope_key = recovered(&k.vault, &k.secret, &k.recovery)[&mine];
    assert_eq!(k.store.app_key(&scope_key, &mine, "board"), Some(board), "the Board's key is in the bucket, sealed under his scope's");
    assert!(vault.keys(&access, &[("global", "board", None)]).is_err(), "he gave it only his own");

    // Better Auth keeps the refresh token only as its hash.
    if let Some(running) = &k.running {
    let dbs = std::fs::read_dir(running.dir.join("data/auth/auth")).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "sqlite"));
    let conn = dbs.map(|db| rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()).find(|c| c.prepare("select token from oauthRefreshToken").is_ok()).expect("Better Auth's database");
    let stored: Vec<String> = conn.prepare("select token from oauthRefreshToken").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    let hashed = { use sha2::Digest; B64U.encode(&sha2::Sha256::digest(refresh.as_bytes())) };
    assert_eq!(stored, vec![hashed], "only the refresh token's hash is kept");
    drop(conn);
    }

    // Refreshing moves its keys to the new refresh token; the old one opens nothing.
    let refreshed = |token: &str| k.browser().send("POST", meta["token_endpoint"].as_str().unwrap(), None, Some(&[("grant_type", "refresh_token"), ("refresh_token", token), ("client_id", &client_id), ("resource", RESOURCE)]));
    let (status, again, _) = refreshed(&refresh);
    assert_eq!(status, 200, "{again}");
    let access2 = again["access_token"].as_str().unwrap();
    assert_ne!(again["refresh_token"].as_str().unwrap(), refresh);
    assert_eq!(vault.keys(access2, &[(&mine, "board", None)]).unwrap()[0].key, board);

    // Signing Claude Code out (revoking its refresh token) leaves it nothing to open.
    let latest = again["refresh_token"].as_str().unwrap().to_string();
    let (status, revoked, _) = k.browser().send("POST", &format!("{issuer}/oauth2/revoke"), None, Some(&[("token", &latest), ("token_type_hint", "refresh_token"), ("client_id", &client_id)]));
    assert_eq!(status, 200, "{revoked}");
    let (_, moved) = post(&format!("{}/internal/rekey", k.vault), &[("x-kernel-secret", &k.secret)], json!({ "from": latest, "to": "anything at all, long", "token": access2, "until": now() + 60_000 }));
    assert_eq!(moved["moved"], false);
    let (status, _, _) = refreshed(&refresh);
    assert_ne!(status, 200, "a spent refresh token is refused");

    // He adds a second passkey, with the one he has: it opens his keys too. One without PRF isn't kept.
    let (second, status) = adds_passkey(k, &mut browser, &passkey);
    assert_eq!(status, 200);
    let (_, options, _) = browser.get("/api/auth/passkey/generate-register-options");
    let (status, _) = browser.post("/api/auth/passkey/verify-registration", json!({ "response": k.passkey().create_with(&options, false) }));
    assert_eq!(status, 400, "a passkey without PRF isn't kept");
    let (status, _) = browser.post("/passkeys/add", json!({ "from": B64U.encode(&rand::random::<[u8; 32]>()), "from_credential": B64U.encode(&passkey.id), "prf": "a passkey's prf output here", "credential": B64U.encode(&second.id) }));
    assert_eq!(status, 403, "a new passkey is added only with one that opens his keys");

    // Removing his first passkey: it opens nothing again, at the next consent; his second still does.
    let (_, list, _) = browser.get("/api/auth/passkey/list-user-passkeys");
    assert_eq!(list.as_array().unwrap().len(), 2, "{list}");
    let first = list.as_array().unwrap().iter().find(|p| p["credentialID"] == B64U.encode(&passkey.id)).unwrap();
    let (status, _) = browser.post("/api/auth/passkey/delete-passkey", json!({ "id": first["id"] }));
    assert_eq!(status, 200);
    let (_, held) = post(&format!("{}/internal/scopes", k.vault), &[("x-kernel-secret", &k.secret)], json!({ "user": user, "secret": prf }));
    assert!(held["error"].is_string(), "{held}");
    let (status, scopes) = browser.post("/consent/scopes", json!({ "prf": second.prf(), "credential": B64U.encode(&second.id) }));
    assert_eq!(status, 200, "{scopes}");
    (access2.to_string(), user)
}

/// Someone joins with an invite link for `name` (as `user`, if given): → their passkey and user id.
fn joins(k: &Kernel, browser: &mut Browser, name: &str, user: Option<&str>) -> (Passkey, String) {
    let invite = kernel::invite(&k.settings, name, user, || None).unwrap();
    joins_with(k, browser, invite.link.split_once('#').unwrap().1)
}

/// Opening an invite link (its secret, `token`): they make a passkey with PRF, which signs them in, and its PRF output
/// makes their keys, or opens those the link carries. → their passkey and user id.
fn joins_with(k: &Kernel, browser: &mut Browser, token: &str) -> (Passkey, String) {
    let (status, about) = k.browser().post("/invite/about", json!({ "token": token }));
    assert_eq!(status, 200, "{about}");
    let passkey = k.passkey();
    let (status, options, _) = browser.get(&format!("/api/auth/passkey/generate-register-options?context={}", enc(token)));
    assert_eq!(status, 200, "{options}");
    let (status, made) = browser.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true }));
    assert_eq!(status, 200, "{made}");
    assert_eq!(made["user"]["name"], about["person"]);
    let user = made["user"]["id"].as_str().unwrap().to_string();
    let (status, keys) = browser.post("/invite/keys", json!({ "token": token, "prf": passkey.prf(), "credential": B64U.encode(&passkey.id) }));
    assert_eq!(status, 200, "{keys}");
    (passkey, user)
}

/// Someone signed in adds a passkey with one they have (`with`), as the Passkeys page does: → it, and how adding it
/// at the vault went.
fn adds_passkey(k: &Kernel, browser: &mut Browser, with: &Passkey) -> (Passkey, u16) {
    let new = k.passkey();
    let (status, options, _) = browser.get("/api/auth/passkey/generate-register-options");
    assert_eq!(status, 200, "{options}");
    let (status, made) = browser.post("/api/auth/passkey/verify-registration", json!({ "response": new.create(&options) }));
    assert_eq!(status, 200, "{made}");
    let (status, _) = browser.post("/passkeys/add", json!({ "from": with.prf(), "from_credential": B64U.encode(&with.id), "prf": new.prf(), "credential": B64U.encode(&new.id) }));
    (new, status)
}

#[test]
fn someone_who_lost_every_passkey_joins_again_with_a_link_that_carries_their_keys() {
    let k = Kernel::start(None);
    let (lost, andre) = joins(&k, &mut k.browser(), "Andre", None);
    let (_, held) = post(&format!("{}/internal/scopes", k.vault), &[("x-kernel-secret", &k.secret)], json!({ "user": andre, "secret": lost.prf() }));

    // He has keys, so his new link must carry them: only the recovery key opens them.
    let err = kernel::invite(&k.settings, "Andre", Some(&andre), || None).err().unwrap().to_string();
    assert!(err.contains("pipe the recovery key"), "{err}");
    let other = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>());
    assert!(kernel::invite(&k.settings, "Andre", Some(&andre), || Some(other)).is_err(), "another recovery key opens nothing");
    let invite = kernel::invite(&k.settings, "Andre", Some(&andre), || Some(k.recovery_key.clone())).unwrap();

    // Opening it on a new device, his new passkey opens what his lost one did.
    let mut browser = k.browser();
    let (passkey, user) = joins_with(&k, &mut browser, invite.link.split_once('#').unwrap().1);
    assert_eq!(user, andre);
    let (status, scopes) = browser.post("/consent/scopes", json!({ "prf": passkey.prf(), "credential": B64U.encode(&passkey.id) }));
    assert_eq!(status, 200, "{scopes}");
    assert_eq!(scopes["scopes"], held["scopes"]);

    // Someone new's link carries nothing, and needs no recovery key.
    let (sam, sam_id) = joins(&k, &mut k.browser(), "Sam", None);
    assert_ne!(sam_id, andre);
    let (_, theirs) = post(&format!("{}/internal/scopes", k.vault), &[("x-kernel-secret", &k.secret)], json!({ "user": sam_id, "secret": sam.prf() }));
    assert_eq!(theirs["scopes"], json!([format!("user:{sam_id}")]), "{theirs}");
}

/// The whole flow against a deployed kernel (see `Kernel::deployed`), its schedules handing runs to a stand-in node
/// here that a tunnel reaches: `ONE_TEST_NODE` is the https address that reaches port `ONE_TEST_NODE_PORT`.
#[test]
#[ignore]
fn a_deployed_kernel_signs_people_in_and_runs_what_they_let_run() {
    let k = Kernel::deployed();
    let (token, user) = signs_in(&k);
    let me = format!("user:{user}");

    // The node: it keeps each run it's handed, with its token.
    let runs: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>> = Default::default();
    let kept = runs.clone();
    let take = move |headers: axum::http::HeaderMap, axum::Json(run): axum::Json<Value>| {
        let token = headers["authorization"].to_str().unwrap().trim_start_matches("Bearer ").to_string();
        kept.lock().unwrap().push((token, run));
        async move { axum::Json(json!({ "ok": true })) }
    };
    let app = axum::Router::new().route("/kernel/run", axum::routing::post(take));
    let listener = std::net::TcpListener::bind(format!("127.0.0.1:{}", std::env::var("ONE_TEST_NODE_PORT").unwrap())).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || tokio::runtime::Runtime::new().unwrap().block_on(async move { axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app).await.unwrap() }));

    // He lets the Board tidy every minute; its first slot runs once, as him, with only the Board.
    let add = json!({ "app": "board", "name": "tidy", "scope": me, "title": "Tidy the board", "what": "Board", "op": "tidy", "every": "1m", "node": std::env::var("ONE_TEST_NODE").unwrap() });
    let (status, made) = ask_at(&k.schedules, &token, "POST", "/schedules", add);
    assert_eq!(status, 200, "{made}");
    let id = made["id"].as_str().unwrap().to_string();
    let started = std::time::Instant::now();
    loop {
        let (_, list) = ask_at(&k.schedules, &token, "GET", "/schedules", json!({}));
        let last = &list["schedules"][0]["last"];
        if !last.is_null() && last["status"] != "running" {
            assert_eq!(last["status"], "ran", "{list}");
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(150), "its alarm didn't run it: {list}");
        std::thread::sleep(Duration::from_secs(2));
    }
    let (status, _) = ask_at(&k.schedules, &token, "DELETE", &format!("/schedules/{id}"), json!({}));
    assert_eq!(status, 200);
    let runs = runs.lock().unwrap().clone();
    assert_eq!(runs.len(), 1, "one slot, one run: {runs:?}");
    let (run_token, run) = &runs[0];
    let run: kernel::Run = serde_json::from_value(run.clone()).unwrap();
    assert_eq!((run.app.as_str(), run.op.as_str(), run.scope.as_str()), ("board", "tidy", me.as_str()));
    let vault = kernel::Vault { url: k.vault.clone() };
    let session = vault.session(run_token).unwrap();
    assert_eq!((session.user.as_str(), session.client.as_deref(), session.apps.clone()), (user.as_str(), Some("schedules"), Some(vec!["board".to_string()])));
    assert_eq!(vault.keys(run_token, &[(&me, "board", None)]).unwrap()[0].key, vault.keys(&token, &[(&me, "board", None)]).unwrap()[0].key);
    assert!(vault.keys(run_token, &[(&me, "calendar", None)]).is_err(), "a run can't use an app it wasn't let use");
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

type Scripts = std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, (Value, Vec<u8>)>>>;
type Addresses = std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, Value>>>;
/// Static assets uploaded, by path, with their type.
type Assets = std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, (String, Vec<u8>)>>>;

/// A stand-in for Cloudflare's Workers API, for an account with the domain andre.example: it keeps each upload
/// (metadata and module) by Worker, as Cloudflare does, and where each is reached (its workers.dev address's
/// setting, or the host it's on), and the static assets uploaded, asking only for those it doesn't have.
fn cloudflare() -> (String, Scripts, Addresses, Assets) {
    use axum::routing::{get, post, put};
    let scripts: Scripts = Default::default();
    let addresses: Addresses = Default::default();
    let assets: Assets = Default::default();
    let offered: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>> = Default::default();
    let ok = |result: Value| axum::Json(json!({ "success": true, "errors": [], "result": result }));
    let list = {
        let scripts = scripts.clone();
        move || {
            let s = scripts.lock().unwrap();
            // Every Worker here was first uploaded with its Durable Object's migration, so it has had it.
            let tags: Vec<Value> = s.keys().map(|id| json!({ "id": id, "migration_tag": "v1" })).collect();
            async move { ok(json!(tags)) }
        }
    };
    let upload = {
        let scripts = scripts.clone();
        move |axum::extract::Path((_, name)): axum::extract::Path<(String, String)>, headers: axum::http::HeaderMap, body: axum::body::Bytes| {
            let kind = headers["content-type"].to_str().unwrap().to_string();
            let boundary = format!("--{}", kind.split("boundary=").nth(1).unwrap());
            let text = body.to_vec();
            let parts: Vec<&[u8]> = split(&text, boundary.as_bytes());
            let part = |n: &str| parts.iter().find(|p| String::from_utf8_lossy(&p[..p.len().min(200)]).contains(&format!("name=\"{n}\""))).map(|p| { let at = find(p, b"\r\n\r\n").unwrap() + 4; p[at..p.len() - 2].to_vec() }).unwrap();
            let meta: Value = serde_json::from_slice(&part("metadata")).unwrap();
            scripts.lock().unwrap().insert(name, (meta, part("worker.js")));
            async move { ok(json!({})) }
        }
    };
    let subdomain = {
        let addresses = addresses.clone();
        move |axum::extract::Path((_, name)): axum::extract::Path<(String, String)>, axum::Json(body): axum::Json<Value>| {
            addresses.lock().unwrap().insert(format!("{name}.workers.dev"), body["enabled"].clone());
            async move { ok(json!({})) }
        }
    };
    let domain = {
        let addresses = addresses.clone();
        move |axum::Json(body): axum::Json<Value>| {
            assert_eq!(body["zone_id"], "zone", "{body}");
            addresses.lock().unwrap().insert(body["hostname"].as_str().unwrap().into(), body["service"].clone());
            async move { ok(json!({})) }
        }
    };
    let zones = move |axum::extract::Query(q): axum::extract::Query<std::collections::BTreeMap<String, String>>| async move {
        ok(if q["name"] == "andre.example" { json!([{ "id": "zone", "name": "andre.example" }]) } else { json!([]) })
    };
    let session = {
        let (assets, offered) = (assets.clone(), offered.clone());
        move |axum::Json(body): axum::Json<Value>| {
            let have: BTreeSet<String> = assets.lock().unwrap().keys().cloned().collect();
            let mut wanted = Vec::new();
            for (path, file) in body["manifest"].as_object().unwrap() {
                let hash = file["hash"].as_str().unwrap().to_string();
                assert_eq!(hash.len(), 32, "a hash is 32 hex characters");
                offered.lock().unwrap().insert(hash.clone(), path.clone());
                if !have.contains(path) {
                    wanted.push(hash);
                }
            }
            let buckets = if wanted.is_empty() { json!([]) } else { json!([wanted]) };
            async move { ok(json!({ "jwt": if buckets == json!([]) { "complete" } else { "session" }, "buckets": buckets })) }
        }
    };
    let take = {
        let (assets, offered) = (assets.clone(), offered.clone());
        move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
            assert_eq!(headers["authorization"], "Bearer session", "assets go up with the session's token");
            let kind = headers["content-type"].to_str().unwrap().to_string();
            let boundary = format!("--{}", kind.split("boundary=").nth(1).unwrap());
            let text = body.to_vec();
            for part in split(&text, boundary.as_bytes()).into_iter().filter(|p| find(p, b"name=\"").is_some()) {
                let head = String::from_utf8_lossy(&part[..find(part, b"\r\n\r\n").unwrap()]).to_string();
                let hash = head.split("name=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
                let kind = head.split("Content-Type: ").nth(1).unwrap().trim().to_string();
                let at = find(part, b"\r\n\r\n").unwrap() + 4;
                let bytes = data_encoding::BASE64.decode(&part[at..part.len() - 2]).unwrap();
                let path = offered.lock().unwrap()[&hash].clone();
                assets.lock().unwrap().insert(path, (kind, bytes));
            }
            async move { ok(json!({ "jwt": "complete" })) }
        }
    };
    let app = axum::Router::new()
        .route("/accounts", get(move || async move { ok(json!([{ "id": "acct" }])) }))
        .route("/accounts/{a}/workers/scripts/{name}/assets-upload-session", post(session))
        .route("/accounts/{a}/workers/assets/upload", post(take))
        .route("/zones", get(zones))
        .route("/accounts/{a}/workers/subdomain", get(move || async move { ok(json!({ "subdomain": "andre" })) }))
        .route("/accounts/{a}/workers/scripts", get(list))
        .route("/accounts/{a}/workers/scripts/{name}", put(upload))
        .route("/accounts/{a}/workers/scripts/{name}/subdomain", post(subdomain))
        .route("/accounts/{a}/workers/domains", put(domain))
        .layer(axum::extract::DefaultBodyLimit::disable());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new().unwrap().block_on(async move { axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app).await.unwrap() })
    });
    (format!("http://{addr}"), scripts, addresses, assets)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn split<'a>(mut hay: &'a [u8], by: &[u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    while let Some(at) = find(hay, by) {
        out.push(&hay[..at]);
        hay = &hay[at + by.len()..];
    }
    out.push(hay);
    out
}

#[test]
fn a_deploy_to_cloudflare_uploads_each_service_with_its_bindings_and_object() {
    let (api, scripts, addresses, assets) = cloudflare();
    let cf = kernel::Cloudflare { api, token: "token".into(), account: "acct".into() };
    let at = |service: &str, domain: &str| format!("https://{}.{domain}", kernel::worker_name(service));
    let (mut settings, _) = kernel::Settings::new("Test", &at("gateway", "andre.workers.dev"));
    (settings.auth_url, settings.vault_url, settings.schedules_url) = (at("auth", "andre.workers.dev"), at("vault", "andre.workers.dev"), at("schedules", "andre.workers.dev"));
    let modules: Vec<Vec<u8>> = kernel::SERVICES.iter().map(|(_, entry, _)| module(entry)).collect();
    let web = vec![("index.html".to_string(), b"<!doctype html><title>The web client</title>".to_vec()), ("screen/kit.css".to_string(), b":root{}".to_vec())];
    kernel::deploy(&cf, &settings, &modules, &web).unwrap();
    for (service, _, _) in kernel::SERVICES {
        assert_eq!(addresses.lock().unwrap()[&format!("{}.workers.dev", kernel::worker_name(service))], true, "{service} is at its workers.dev address");
    }

    // Each service is one Worker, its Durable Object made the first time, every binding there: the others by name.
    let deployed = scripts.lock().unwrap().clone();
    assert_eq!(deployed.keys().cloned().collect::<Vec<_>>(), ["auth", "gateway", "schedules", "vault"].map(kernel::worker_name));
    for (service, _, class) in kernel::SERVICES {
        let (meta, module) = &deployed[&kernel::worker_name(service)];
        assert_eq!(meta["main_module"], "worker.js");
        assert!(!module.is_empty());
        let names: BTreeSet<String> = meta["bindings"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap().to_string()).collect();
        let assets = (*service == "gateway").then(|| "ASSETS".to_string());
        let want: BTreeSet<String> = settings.bindings(service).into_iter().map(|(n, _, _)| n).chain(class.map(str::to_uppercase)).chain(assets).collect();
        assert_eq!(names, want, "{service}'s bindings");
        for b in meta["bindings"].as_array().unwrap() {
            if b["name"] == "KERNEL_SECRET" || b["name"] == "AUTH_SECRET" || b["name"] == "SCHEDULES_KEY" {
                assert_eq!(b["type"], "secret_text", "{b}");
            }
        }
    }
    // The auth service went up before the vault existed, so it went up again with it bound.
    let auth = &deployed[&kernel::worker_name("auth")].0;
    assert!(auth["bindings"].as_array().unwrap().iter().any(|b| b["type"] == "service" && b["service"] == kernel::worker_name("vault")));
    assert!(auth["migrations"].is_null(), "its Durable Object was made once, the first time");
    assert_eq!(deployed[&kernel::worker_name("vault")].0["migrations"]["new_sqlite_classes"], json!(["Vault"]));

    // The gateway serves the web client's files itself, as they were uploaded.
    let gateway = &deployed[&kernel::worker_name("gateway")].0;
    assert_eq!(gateway["assets"], json!({ "jwt": "complete", "config": { "html_handling": "none", "not_found_handling": "none", "run_worker_first": true } }));
    assert_eq!(assets.lock().unwrap()["/index.html"], ("text/html; charset=utf-8".to_string(), web[0].1.clone()));
    assert_eq!(assets.lock().unwrap()["/screen/kit.css"].0, "text/css; charset=utf-8");

    // At one of the account's domains instead, each service is on its own host there, with workers.dev off.
    (settings.address, settings.auth_url, settings.vault_url, settings.schedules_url) = (at("gateway", "andre.example"), at("auth", "andre.example"), at("vault", "andre.example"), at("schedules", "andre.example"));
    kernel::deploy(&cf, &settings, &modules, &web).unwrap();
    let addresses = addresses.lock().unwrap();
    for (service, _, _) in kernel::SERVICES {
        let name = kernel::worker_name(service);
        assert_eq!((&addresses[&format!("{name}.andre.example")], &addresses[&format!("{name}.workers.dev")]), (&json!(name), &json!(false)));
    }
    drop(addresses);

    // A node behind a Cloudflare Tunnel is the gateway's Workers VPC service binding, not a URL.
    settings.node_url = Some("cloudflare:0191dce4-9ab4-7fce-b660-8e5dec5172da".into());
    kernel::deploy(&cf, &settings, &modules, &web).unwrap();
    let gateway = scripts.lock().unwrap()[&kernel::worker_name("gateway")].0.clone();
    let node: Vec<&Value> = gateway["bindings"].as_array().unwrap().iter().filter(|b| b["name"].as_str().unwrap().starts_with("NODE")).collect();
    assert_eq!(node, [&json!({ "type": "vpc_service", "name": "NODE", "service_id": "0191dce4-9ab4-7fce-b660-8e5dec5172da" })]);

    settings.auth_url = "https://one-auth.someone-else.example".into();
    assert!(kernel::deploy(&cf, &settings, &modules, &web).is_err(), "a host in none of the account's domains is refused");
}

/// Asks `url` as a browser's script would: → (status, headers, body).
fn ask(method: &str, url: &str, headers: &[(&str, &str)], body: Option<Value>) -> (u16, std::collections::BTreeMap<String, String>, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).build().into();
    let mut resp = match body {
        None => {
            let mut req = agent.request_builder_get(method, url);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.call().unwrap()
        }
        Some(body) => {
            let mut req = agent.post(url);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.send_json(body).unwrap()
        }
    };
    let got = resp.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())).collect();
    (resp.status().as_u16(), got, resp.body_mut().read_to_string().unwrap_or_default())
}

trait GetWith {
    fn request_builder_get(&self, method: &str, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody>;
}

impl GetWith for ureq::Agent {
    fn request_builder_get(&self, method: &str, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        match method {
            "HEAD" => self.head(url),
            _ => self.get(url),
        }
    }
}

/// A stand-in node that answers every request with what it was sent, as a page that would run script, sets a cookie
/// and may be framed: → its address.
fn hostile_node() -> String {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut headers = serde_json::Map::new();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap() == 0 || h.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = h.trim().split_once(':') {
                    headers.insert(k.trim().to_lowercase(), json!(v.trim()));
                }
            }
            let body = json!({ "request": line.trim(), "headers": headers }).to_string();
            let mut stream = stream;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\nset-cookie: better-auth.session_token=planted; Path=/\r\naccess-control-allow-origin: *\r\nservice-worker-allowed: /\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    addr
}

#[test]
fn the_gateway_sends_each_path_where_it_belongs_and_nothing_a_node_says_runs_at_its_origin() {
    let node = hostile_node();
    let k = Kernel::start(Some(node));
    let at = |path: &str| format!("{}{path}", k.address);

    // The web client's files: its page at / and wherever it routes itself, the screens' kit, the frame screens load.
    for path in ["/", "/callback", "/vault?id=1&scope=user"] {
        let (status, headers, body) = ask("GET", &at(path), &[], None);
        assert_eq!((status, headers["content-type"].as_str()), (200, "text/html; charset=utf-8"), "{path}");
        assert!(body.contains("The web client"), "{path}: {body}");
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["x-frame-options"], "SAMEORIGIN", "no other site frames the web client");
    }
    let (status, headers, body) = ask("GET", &at("/frame.html"), &[], None);
    assert!(status == 200 && body.contains("A screen's frame"));
    assert!(!headers.contains_key("content-security-policy"), "a screen's frame carries its own policy: {headers:?}");
    assert_eq!(headers["x-frame-options"], "SAMEORIGIN", "only the web client's own page frames it");
    let (status, headers, _) = ask("GET", &at("/screen/kit.css"), &[], None);
    assert_eq!((status, headers["content-type"].as_str()), (200, "text/css; charset=utf-8"));
    assert_eq!(ask("GET", &at("/nothing.js"), &[], None).0, 404, "a missing file isn't the page");

    // Sign-in goes to the auth service, as the install's address; its own routes don't.
    let (status, _, meta) = ask("GET", &at("/.well-known/oauth-authorization-server/api/auth"), &[], None);
    assert_eq!(status, 200, "{meta}");
    assert_eq!(serde_json::from_str::<Value>(&meta).unwrap()["issuer"], at("/api/auth"));
    for (path, says) in [("/sign-in", "Sign In with a Passkey"), ("/invite", "Make a Passkey"), ("/passkeys", "Add a Passkey")] {
        let (status, _, body) = ask("GET", &at(path), &[], None);
        assert!(status == 200 && body.contains(says), "{path}: {status}");
    }
    let (status, _, _) = ask("POST", &at("/internal/invite"), &[], Some(json!({ "name": "Mallory" })));
    assert_eq!(status, 405, "the kernel's own routes aren't reachable at the address");

    // A node gets the token, never the browser's cookies, and what it says is stamped so it can't run here.
    let (status, headers, body) = ask("GET", &at("/v1/screens/board/main?scope=user"), &[("authorization", "Bearer a token"), ("cookie", "better-auth.session_token=mine")], None);
    assert_eq!(status, 200);
    let seen: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(seen["request"], "GET /v1/screens/board/main?scope=user HTTP/1.1");
    assert_eq!(seen["headers"]["authorization"], "Bearer a token");
    assert!(seen["headers"].get("cookie").is_none(), "{seen}");
    assert!(seen["headers"]["forwarded"].as_str().is_some_and(|f| f.starts_with("host=")), "a node knows it came from elsewhere: {seen}");
    assert_eq!(headers["content-security-policy"], "sandbox allow-scripts allow-forms allow-popups allow-downloads");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    for header in ["set-cookie", "access-control-allow-origin", "service-worker-allowed"] {
        assert!(!headers.contains_key(header), "a node's {header} reached the browser: {headers:?}");
    }
    assert_eq!(ask("POST", &at("/v1/call"), &[], Some(json!({}))).0, 200);
    assert_eq!(ask("GET", &at("/mcp"), &[], None).0, 200, "/mcp is a node's");
    assert_eq!(ask("GET", &at("/v1/state"), &[("service-worker", "script")], None).0, 403, "no node's answer becomes a service worker");
}

#[test]
fn a_browser_signs_in_at_the_address_and_uses_a_node_as_its_person_for_what_its_sign_in_opens() {
    // A node with an install, its API where the gateway sends /v1/.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = std::env::temp_dir().join(format!("one-kernel-node-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("server.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": "node" }).to_string()).unwrap();
    let (node, _router, api) = rt.block_on(async {
        let (node, router) = one::node::Node::start(&dir, one::sidecars::built_in()).await.unwrap();
        node.create("Test", "Andre").await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let app = one::api::router(node.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (node, router, api)
    });
    let andre = node.user().unwrap();
    let k = Kernel::start(Some(api.clone()));
    let at = |path: &str| format!("{}{path}", k.address);

    // The install's nodes take sign-ins made at its address, and Andre is let in as who he is in the install.
    rt.block_on(async {
        node.member("setting", json!({ "name": "address", "value": k.address })).await.unwrap();
        node.member("setting", json!({ "name": "kernel_vault", "value": k.vault })).await.unwrap();
    });
    let mut browser = k.browser();
    let (passkey, user) = joins(&k, &mut browser, "Andre", Some(&andre));
    assert_eq!(user, andre, "he joins as who he is in the install");

    let (_, options, _) = browser.get("/api/auth/passkey/generate-authenticate-options");
    let (status, signed) = browser.post("/api/auth/passkey/verify-authentication", json!({ "response": passkey.get(&options, &andre) }));
    assert_eq!(status, 200, "{signed}");

    // The web client signs in as any OAuth client does, at its own origin, and he lets it use his own scope only.
    let mine = format!("user:{andre}");
    let access = web_sign_in(&k, &mut browser, &passkey, json!({ "scopes": [mine] }));
    let bearer = format!("Bearer {access}");
    let with = [("authorization", bearer.as_str())];

    // With it, the node is his: what he sees, and a call in his scope, through the address.
    let (status, headers, body) = ask("GET", &at("/v1/state"), &with, None);
    assert_eq!(status, 200, "{body}");
    assert_eq!(headers["content-security-policy"], "sandbox allow-scripts allow-forms allow-popups allow-downloads");
    let state: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(state["me"]["user"], andre.as_str());
    let called = |input: Value| {
        let (status, _, body) = ask("POST", &at("/v1/call"), &with, Some(json!({ "app": "board", "op": "post", "input": input })));
        (status, serde_json::from_str::<Value>(&body).unwrap())
    };
    let eventually = |what: &str, f: &dyn Fn() -> bool| {
        let started = std::time::Instant::now();
        while !f() {
            assert!(started.elapsed() < Duration::from_secs(45), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(250));
        }
    };
    eventually("the node to be ready", &|| called(json!({ "text": "From the browser" })).0 == 200);
    let (status, _, body) = ask("POST", &at("/v1/call"), &with, Some(json!({ "app": "board", "op": "news", "input": { "format": "json", "since": 0, "scope": "user" } })));
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("From the browser"), "{body}");
    let (status, headers, screen) = ask("GET", &at("/v1/screens/board/main?scope=user"), &with, None);
    assert!(status == 200 && screen.starts_with("<!doctype html>"), "{status} {screen}");
    assert_eq!(headers["content-security-policy"], "sandbox allow-scripts allow-forms allow-popups allow-downloads", "a screen runs in an origin of its own");

    // Only what his sign-in opens: not Everyone's scope, nor what this machine's own programs may do.
    let (status, refused) = called(json!({ "text": "To everyone", "scope": "global" }));
    assert!(status == 400 && refused["error"].as_str().unwrap().contains("no board in global"), "the call reaches only his scope: {refused}");
    let (status, _, body) = ask("POST", &at("/v1/call"), &with, Some(json!({ "app": "board", "op": "post", "input": { "text": "To everyone" }, "scopes": ["global"] })));
    assert!(status == 400 && body.contains("doesn't open global"), "{body}");
    let (status, _, body) = ask("GET", &at("/v1/widgets"), &with, None);
    assert!(status == 400 && body.contains("doesn't open"), "every scope he's in is more than it opens: {body}");
    let (status, _, body) = ask("POST", &at("/v1/settings"), &with, Some(json!({ "name": "address", "value": "https://elsewhere.test" })));
    assert_eq!(status, 403, "{body}");
    let (status, _, body) = ask("POST", &format!("{api}/v1/pair"), &with, Some(json!({})));
    assert_eq!(status, 403, "{body}");

    // A sign-in he let only read reads, and changes nothing.
    let reader = format!("Bearer {}", web_sign_in(&k, &mut browser, &passkey, json!({ "scopes": [mine], "read_only": true })));
    let reading = [("authorization", reader.as_str())];
    let (status, _, body) = ask("POST", &at("/v1/call"), &reading, Some(json!({ "app": "board", "op": "news", "input": { "format": "json", "since": 0, "scope": "user" } })));
    assert!(status == 200 && body.contains("From the browser"), "{body}");
    let (status, _, body) = ask("POST", &at("/v1/call"), &reading, Some(json!({ "app": "board", "op": "post", "input": { "text": "Read only" } })));
    assert!(status == 400 && body.contains("only reads"), "a read-only sign-in posts nothing: {body}");
    let (status, _, body) = ask("POST", &at("/v1/install"), &reading, Some(json!({ "app": "repos", "scope": "user" })));
    assert!(status == 400 && body.contains("only read"), "{body}");

    // Without a sign-in the node takes nothing from the address; with a forged or someone else's, nothing either.
    assert_eq!(ask("GET", &at("/v1/state"), &[], None).0, 403);
    let forged = format!("Bearer {}x", &access[..access.len() - 1]);
    assert_eq!(ask("GET", &at("/v1/state"), &[("authorization", &forged)], None).0, 401);
    let (status, _, body) = ask("GET", &at("/v1/state"), &[("authorization", "Bearer not.a.token")], None);
    assert_eq!(status, 401, "{body}");
}

/// The web client's sign-in at the install's address, as any OAuth client's (PKCE, the passkey's secret at consent),
/// in a browser signed in with that passkey, with what the person lets it have (`{ scopes, apps?, read_only? }`): →
/// its access token.
fn web_sign_in(k: &Kernel, browser: &mut Browser, passkey: &Passkey, consent: Value) -> String {
    let callback = format!("{}/callback", k.address);
    let (status, client) = k.browser().post("/api/auth/oauth2/register", json!({
        "client_name": "The web client", "redirect_uris": [callback], "token_endpoint_auth_method": "none",
        "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"] }));
    assert_eq!(status, 201, "{client}");
    let client_id = client["client_id"].as_str().unwrap().to_string();
    let verifier = B64U.encode(&rand::random::<[u8; 32]>());
    let challenge = { use sha2::Digest; B64U.encode(&sha2::Sha256::digest(verifier.as_bytes())) };
    let (status, _, location) = browser.get(&format!(
        "/api/auth/oauth2/authorize?response_type=code&client_id={client_id}&redirect_uri={}&scope={}&state=s&code_challenge={challenge}&code_challenge_method=S256",
        enc(&callback), enc("openid offline_access")
    ));
    assert_eq!(status, 302);
    let oauth_query = location.unwrap().split_once('?').unwrap().1.to_string();
    let prf = passkey.prf();
    let (status, _) = browser.post("/consent/scopes", json!({ "prf": prf, "credential": B64U.encode(&passkey.id) }));
    assert_eq!(status, 200);
    let mut accept = json!({ "oauth_query": oauth_query, "accept": true, "prf": prf });
    accept.as_object_mut().unwrap().extend(consent.as_object().unwrap().clone());
    let (status, accepted) = browser.post("/consent/accept", accept);
    assert_eq!(status, 200, "{accepted}");
    let code = query_of(accepted["url"].as_str().unwrap())["code"].clone();
    let (status, tokens, _) = k.browser().send("POST", &format!("{}/api/auth/oauth2/token", k.address), None, Some(&[
        ("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &callback), ("client_id", &client_id), ("code_verifier", &verifier)]));
    assert_eq!(status, 200, "{tokens}");
    tokens["access_token"].as_str().unwrap().to_string()
}
