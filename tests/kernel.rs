//! The kernel servers (`kernel/`) running in the pinned workerd, as compute servers and the auth server use them.

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

/// A workerd running kernel servers, stopped when dropped.
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

/// A vault, and a stand-in auth server that signs any claims, with the recovery key's secret half.
struct Vault {
    _running: Running,
    url: String,
    auth: String,
    issuer: String,
    secret: String,
    recovery: [u8; 32],
}

impl Vault {
    fn start() -> Vault {
        let (vault, auth) = (port(), port());
        let (recovery, recovery_public) = kernel::twin(&rand::random());
        let secret = B64U.encode(&rand::random::<[u8; 32]>());
        let issuer = "https://auth.test/api/auth".to_string();
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
        ]);
        Vault { _running: running, url: format!("http://127.0.0.1:{vault}"), auth: format!("http://127.0.0.1:{auth}"), issuer, secret, recovery }
    }

    /// Asks as the auth server does.
    fn internal(&self, path: &str, body: Value) -> (u16, Value) {
        post(&format!("{}/internal/{path}", self.url), &[("x-kernel-secret", &self.secret)], body)
    }

    /// An access token for `user` from `client`, good for an hour.
    fn token(&self, user: &str, client: &str) -> (String, u64) {
        let exp = now() / 1000 + 3600;
        let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
        let token = agent.post(format!("{}/sign", self.auth)).send_json(json!({ "iss": self.issuer, "sub": user, "azp": client, "client_id": client, "aud": "https://compute.test", "scope": "openid", "exp": exp })).unwrap().body_mut().read_to_string().unwrap();
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
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let url = format!("{}{path}", self.url);
        let mut resp = match method {
            "GET" => agent.get(url).header("authorization", format!("Bearer {token}")).call().unwrap(),
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

    // Without the kernel's secret, nobody but the auth server may ask that.
    let (status, _) = post(&format!("{}/internal/passkey", v.url), &[("x-kernel-secret", "a guess")], json!({ "user": ANDRE, "secret": "a passkey of my own making" }));
    assert_eq!(status, 403);
    // Nor add a passkey to Andre's without one he has.
    let (status, _) = v.internal("passkey", json!({ "user": ANDRE, "secret": "someone else's passkey prf", "from": "not a passkey of andre's" }));
    assert_eq!(status, 403);

    // Claude Code is added at its consent screen, with that passkey, for Andre's own scope only. Until its first token
    // it has no keys; once the auth server swaps its code for tokens, the token opens what it may use.
    let (status, _) = v.internal("grant", json!({ "user": ANDRE, "from": "prf output of the MacBook passkey", "to": "the authorization code", "name": "Claude Code", "client": "claude-code", "scopes": [me] }));
    assert_eq!(status, 200);
    let (token, until) = v.token(ANDRE, "claude-code");
    assert!(vault.keys(&token, &[(&me, "board", None)]).unwrap_err().to_string().contains("signs in again"));
    let (_, moved) = v.internal("rekey", json!({ "from": "the authorization code", "to": "the first refresh token", "token": token, "until": until }));
    assert_eq!(moved["moved"], true);

    // Compute servers check the token offline, as the vault does.
    let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
    let jwks: Value = agent.get(format!("{}/jwks", v.auth)).call().unwrap().body_mut().read_json().unwrap();
    let claims = kernel::verify(&token, &jwks, &v.issuer, now()).unwrap();
    assert_eq!((claims.sub.as_str(), claims.azp.as_str(), claims.for_resource("https://compute.test")), (ANDRE, "claude-code", true));
    assert!(kernel::verify(&token, &jwks, "https://someone-else.test", now()).is_err());
    assert!(kernel::verify(&token, &jwks, &v.issuer, until + 1000).is_err());

    let keys = vault.keys(&token, &[(&me, "board", None), (&me, "board", Some("main"))]).unwrap();
    let board = kernel::app_key(&scopes[&me], "board");
    assert_eq!(keys[0].key, board, "the vault derives an app's key as compute servers do");
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

    // A token the auth server didn't sign opens nothing.
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
fn compute_servers_get_create_only_urls_for_objects_in_scopes_they_may_use() {
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
