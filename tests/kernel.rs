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

/// A vault, and a stand-in auth service that signs any claims.
struct Vault {
    _running: Running,
    url: String,
    auth: String,
    issuer: String,
    secret: String,
}

impl Vault {
    fn start() -> Vault {
        let (vault, auth, sites) = (port(), port(), port());
        let (_, recovery_public) = kernel::twin(&rand::random());
        let secret = B64U.encode(&rand::random::<[u8; 32]>());
        let issuer = "https://auth.test/api/auth".to_string();
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
        ]);
        Vault {
            _running: running,
            url: format!("http://127.0.0.1:{vault}"),
            auth: format!("http://127.0.0.1:{auth}"),
            issuer,
            secret,
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

const ANDRE: &str = "00000000000000a1";
const SAM: &str = "00000000000000b2";

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

    fn get(&mut self, path: &str) -> (u16, Value, Option<String>) {
        self.send("GET", path, None, None)
    }

    fn post(&mut self, path: &str, body: Value) -> (u16, Value) {
        let (status, out, _) = self.send("POST", path, Some(body), None);
        (status, out)
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
}

/// The real kernel services, as an install runs them: people reach them at its address, the gateway's.
struct Kernel {
    _running: Running,
    address: String,
    vault: String,
    secret: String,
    /// The recovery key, as its holder keeps it.
    recovery_key: String,
    settings: kernel::Settings,
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
        Kernel {
            _running: running,
            address: settings.address.clone(),
            vault: settings.vault_url.clone(),
            secret: settings.kernel_secret.clone(),
            recovery_key: recovery,
            settings,
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

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
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
