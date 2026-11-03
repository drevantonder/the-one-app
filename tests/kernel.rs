//! The kernel services (`kernel/`) running in the pinned workerd, as nodes and the auth service use them.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE64URL_NOPAD as B64U;
use one::kernel::{self, Served};
use serde_json::{Value, json};

/// The kernel's files as they are in this checkout, tests and all, with the screens' design kit where build.rs puts it.
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
    let screen = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/screen");
    let mut kit = Vec::new();
    walk(&screen, &screen, &mut kit);
    out.extend(kit.into_iter().map(|(path, bytes)| (format!("web/screen/{path}"), bytes)));
    out
}

fn module(entry: &str) -> Vec<u8> {
    kernel::module_from(files(), entry).unwrap()
}

/// Sockets listening on ports `port` gave out, until `run` gives them to the workerd serving there.
static LISTENING: std::sync::Mutex<Vec<std::net::TcpListener>> = std::sync::Mutex::new(Vec::new());

/// A port of this machine's that stays ours: nothing else, another test's workerd included, can take it.
fn port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    LISTENING.lock().unwrap().push(listener);
    port
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

fn run(services: Vec<Served>) -> Running {
    let dir = crate::temp("workerd");
    let listening = {
        let mut held = LISTENING.lock().unwrap();
        services
            .iter()
            .filter_map(|s| {
                let listen = s.listen.as_ref()?;
                let at = held.iter().position(|l| l.local_addr().unwrap().to_string() == *listen).expect("a service listens on a port from port()");
                Some((s.name.clone(), held.swap_remove(at)))
            })
            .collect()
    };
    let config = kernel::write_config(&dir, &services).unwrap();
    let child = kernel::start(&config, listening).unwrap();
    Running { child, dir }
}

pub(crate) fn post(url: &str, headers: &[(&str, &str)], body: Value) -> (u16, Value) {
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
    let dir = crate::temp("kernel-tests");
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

/// A stand-in auth service that signs any claims.
struct StandIn {
    _running: Running,
    auth: String,
    issuer: String,
}

impl StandIn {
    fn start() -> StandIn {
        let auth = port();
        // Nodes fetch its keys from the address their sign-ins are for, as from the auth service at an install's.
        let running = run(vec![Served { name: "auth".into(), module: module("test/auth.js"), modules: vec![], class: None, listen: Some(format!("127.0.0.1:{auth}")), bindings: vec![], assets: None, outbound: None }]);
        StandIn { _running: running, auth: format!("http://127.0.0.1:{auth}"), issuer: format!("http://127.0.0.1:{auth}/api/auth") }
    }

    /// An access token for `user` from `client`, good for an hour, for the install at `auth`'s address.
    fn token(&self, user: &str, client: &str) -> String {
        self.token_for(user, client, &self.auth)
    }

    /// A token for `aud`.
    fn token_for(&self, user: &str, client: &str, aud: &str) -> String {
        let exp = now() / 1000 + 3600;
        let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
        agent.post(format!("{}/sign", self.auth)).send_json(json!({ "iss": self.issuer, "sub": user, "azp": client, "client_id": client, "aud": aud, "scope": "openid", "exp": exp, "jti": B64U.encode(&rand::random::<[u8; 16]>()) })).unwrap().body_mut().read_to_string().unwrap()
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

/// The URL signer, with a stand-in relay that allows `allowed` (`{ <key hex>: { admin } }`), and the stand-in sites as
/// its bucket.
struct UrlSigner {
    _running: Running,
    url: String,
    sites: String,
}

impl UrlSigner {
    fn start(allowed: Value) -> UrlSigner {
        let (signer, sites) = (port(), port());
        let bucket = json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }).to_string();
        let running = run(vec![
            Served { name: "relay".into(), module: module("test/relay.js"), modules: vec![], class: None, listen: None, bindings: vec![("ALLOWED".into(), allowed.to_string())], assets: None, outbound: None },
            Served { name: "sites".into(), module: module("test/sites.js"), modules: vec![], class: None, listen: Some(format!("127.0.0.1:{sites}")), bindings: vec![], assets: None, outbound: None },
            Served {
                name: "signer".into(),
                module: module("signer/worker.js"),
                modules: vec![],
                class: None,
                listen: Some(format!("127.0.0.1:{signer}")),
                bindings: vec![("RELAY".into(), "service:relay".into()), ("BUCKET".into(), bucket)],
                assets: None,
                outbound: Some("sites".into()),
            },
        ]);
        UrlSigner { _running: running, url: format!("http://127.0.0.1:{signer}"), sites: format!("http://127.0.0.1:{sites}") }
    }

    /// Asks as a device does, signed by `key` at `at`.
    fn ask(&self, key: &ed25519_dalek::SigningKey, fields: Value, at: u64) -> (u16, Value) {
        post(&format!("{}/urls", self.url), &[], one::keys::signed_request(key, "one/urls", fields, at))
    }

    /// A URL it gave, reaching the stand-in bucket here.
    fn here(&self, url: &Value) -> String {
        url.as_str().expect("a URL").replacen("https://s3.test", &self.sites, 1)
    }
}

#[test]
fn the_url_signer_gives_urls_only_to_the_installs_devices_and_removes_only_for_admins() {
    let key = || ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let (device, admin, stranger) = (key(), key(), key());
    let id = |k: &ed25519_dalek::SigningKey| data_encoding::HEXLOWER.encode(k.verifying_key().as_bytes());
    let s = UrlSigner::start(json!({ id(&device): { "admin": false }, id(&admin): { "admin": true } }));
    let db = format!("{}/board/main", one::bucket::folder(&format!("user:{ANDRE}")));
    let (step, next, newest) = (format!("{db}/000000000001"), format!("{db}/000000000002"), format!("{db}/newest"));

    // A device of the install gets, in one trip, URLs to read, HEAD and create (its next positions too), and to replace
    // a newest by its ETag, each with what it must send signed in.
    let (status, urls) = s.ask(&device, json!({ "read": [newest], "head": [newest], "create": [{ "steps": db, "from": 1, "to": 4 }], "replace": [{ "name": newest, "etag": "\"1\"" }] }), now());
    assert_eq!(status, 200, "{urls}");
    assert_eq!(urls["create"].as_object().unwrap().len(), 4);
    assert!(urls["create"][&step].as_str().unwrap().starts_with(&format!("https://s3.test/install/{step}?")));
    assert_eq!(url_query(urls["create"][&step].as_str().unwrap())["X-Amz-SignedHeaders"], "host;if-none-match");
    assert_eq!(url_query(urls["replace"][&newest].as_str().unwrap())["X-Amz-SignedHeaders"], "host;if-match");
    assert_eq!(url_query(urls["read"][&newest].as_str().unwrap())["X-Amz-SignedHeaders"], "host");
    assert!(urls["until"].as_u64().unwrap() > now() + 10 * 60_000, "URLs last minutes, so devices reuse them");

    // A create URL makes an object once, never over one.
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let put = |url: &Value, body: &str| agent.put(s.here(url)).header("if-none-match", "*").send(body).unwrap();
    assert_eq!(put(&urls["create"][&step], "a step").status(), 200);
    assert_eq!(put(&urls["create"][&step], "another step").status(), 412);
    let tag = agent.get(s.here(&s.ask(&device, json!({ "read": [step] }), now()).1["read"][&step])).call().unwrap().headers()["etag"].to_str().unwrap().to_string();

    // It confirms which objects are there with an ETag, in one trip.
    let (status, out) = s.ask(&device, json!({ "confirm": [{ "name": step, "etag": tag }, { "name": next, "etag": tag }, { "name": newest, "etag": "\"9\"" }] }), now());
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["confirm"], json!({ step.clone(): true, next.clone(): false, newest.clone(): false }));

    // Only a newest is replaced; only an admin's device removes; names don't climb out, and there are at most 256.
    assert_eq!(s.ask(&device, json!({ "replace": [{ "name": step, "etag": tag }] }), now()).0, 403);
    assert_eq!(s.ask(&device, json!({ "remove": [step] }), now()).0, 403);
    let (status, out) = s.ask(&admin, json!({ "remove": [step] }), now());
    assert_eq!(status, 200, "{out}");
    assert_eq!(agent.delete(s.here(&out["remove"][&step])).call().unwrap().status(), 204);
    for name in [format!("{db}/../../x"), format!("{db}//x"), "./x".into(), String::new()] {
        assert_eq!(s.ask(&device, json!({ "read": [name] }), now()).0, 400, "{name}");
    }
    let many: Vec<String> = (1..=257).map(|n| format!("{db}/{n:012}")).collect();
    assert_eq!(s.ask(&device, json!({ "read": many }), now()).0, 400);

    // A key that isn't the install's gets nothing; nor does a request signed too long ago, for something else, or
    // changed since it was signed.
    assert_eq!(s.ask(&stranger, json!({ "read": [step] }), now()).0, 403);
    assert_eq!(s.ask(&device, json!({ "read": [step] }), now() - one::keys::FRESH - 5_000).0, 401);
    assert_eq!(post(&format!("{}/urls", s.url), &[], one::keys::signed_request(&device, "one/relay", json!({ "read": [step] }), now())).0, 401);
    let mut changed = one::keys::signed_request(&device, "one/urls", json!({ "read": [step] }), now());
    changed["read"] = json!([newest]);
    assert_eq!(post(&format!("{}/urls", s.url), &[], changed).0, 401);
}

fn url_query(url: &str) -> std::collections::BTreeMap<String, String> {
    let query = url.split_once('?').unwrap().1;
    query.split('&').map(|p| p.split_once('=').unwrap()).map(|(k, v)| (k.to_string(), v.replace("%3B", ";"))).collect()
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
            ("DELETE", ..) => agent.delete(&url).header("origin", &self.origin).header("cookie", &cookie).call().unwrap(),
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

    /// What `navigator.credentials.create` gives, as JSON, for registration options, as the pages send it: no PRF.
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
    fn get(&self, options: &Value) -> Value {
        use ed25519_dalek::Signer;
        use sha2::Digest;
        let mut auth = self.rp_hash();
        auth.push(0x05); // user present and verified
        auth.extend(0u32.to_be_bytes());
        let client = self.client_data("webauthn.get", options["challenge"].as_str().unwrap());
        let signature = self.key.sign(&[auth.as_slice(), &sha2::Sha256::digest(&client)].concat());
        let id = B64U.encode(&self.id);
        json!({ "id": id, "rawId": id, "type": "public-key", "clientExtensionResults": {},
            "response": { "clientDataJSON": B64U.encode(&client), "authenticatorData": B64U.encode(&auth), "signature": B64U.encode(&signature.to_bytes()) } })
    }
}

/// The real kernel services, as an install runs them: people reach them at its address, the gateway's.
pub(crate) struct Kernel {
    _running: Running,
    pub(crate) address: String,
    settings: kernel::Settings,
    /// The stand-in sites, the bucket among them.
    sites: String,
}

const RESOURCE: &str = "https://node.test/mcp";

impl Kernel {
    /// Runs them as `one kernel serve` does, from new settings, with the stand-in sites as the bucket, the gateway
    /// sending `/v1/` to `node_url` and serving a web client of two pages.
    pub(crate) fn start(node_url: Option<String>) -> Kernel {
        Kernel::start_as(node_url, false, vec![], None, None)
    }

    /// As `start`, its relay taking only `devices` (public keys, hex) and those they admit.
    pub(crate) fn for_devices(devices: Vec<String>) -> Kernel {
        Kernel::start_as(None, false, devices, None, None)
    }

    /// As `for_devices`, with the MCP Worker, which runs the serverless node's build `modules`.
    pub(crate) fn with_mcp(devices: Vec<String>, modules: kernel::Modules) -> Kernel {
        Kernel::start_as(None, false, devices, None, Some(modules))
    }

    /// As `for_devices`, with the serverless node of `modules` (`serverless::modules_from`) beside it, ending itself
    /// once it's idled `idle` ms.
    pub(crate) fn with_serverless(devices: Vec<String>, modules: Vec<(String, &'static str, Vec<u8>)>, idle: u64) -> Kernel {
        Kernel::start_as(None, false, devices, Some((modules, idle)), None)
    }

    /// As `for_devices`, for an install whose people sign in with Google at test.example, the stand-in sites as
    /// Google.
    fn google(devices: Vec<String>) -> Kernel {
        Kernel::start_as(None, true, devices, None, None)
    }

    fn start_as(node_url: Option<String>, google: bool, devices: Vec<String>, serverless: Option<(Vec<(String, &'static str, Vec<u8>)>, u64)>, mcp: Option<kernel::Modules>) -> Kernel {
        let gateway = port();
        let mut settings = kernel::Settings::new("Test", &format!("http://localhost:{gateway}"));
        settings.ports = [port(), gateway];
        settings.auth_url = format!("http://127.0.0.1:{}", settings.ports[0]);
        settings.resources = vec![RESOURCE.into()];
        settings.node_url = node_url;
        // The install's first devices, its admin's machines that run its calls.
        settings.devices = devices.into_iter().map(|key| json!({ "key": key, "role": "compute", "admin": true })).collect();
        settings.serverless = serverless.is_some();
        settings.bucket = Some(json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }));
        let modules = kernel::SERVICES.iter().map(|(_, entry, _)| module(entry)).collect();
        let sites = port();
        let web = crate::temp("web");
        std::fs::create_dir_all(web.join("screen")).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html><title>The web client</title>").unwrap();
        std::fs::write(web.join("frame.html"), "<!doctype html><title>A screen's frame</title>").unwrap();
        std::fs::write(web.join("screen/kit.css"), include_str!("../src/screen/kit.css")).unwrap();
        let mut served = kernel::served_with(&settings, modules, &web, mcp);
        for s in served.iter_mut().filter(|s| s.name == "signer" || s.name == "mcp") {
            s.outbound = Some("sites".into());
        }
        if google {
            served[0].outbound = Some("sites".into());
            served[0].bindings.extend([("SIGN_IN", "google"), ("JOIN_DOMAIN", "test.example"), ("GOOGLE_CLIENT_ID", "a client id"), ("GOOGLE_CLIENT_SECRET", "a client secret")].map(|(n, v)| (n.into(), v.into())));
        }
        served.push(Served { name: "sites".into(), module: module("test/sites.js"), modules: vec![], class: None, listen: Some(format!("127.0.0.1:{sites}")), bindings: vec![], assets: None, outbound: None });
        if let Some((modules, idle)) = serverless {
            let mut node = one::serverless::served(&settings, modules);
            node.bindings.push(("IDLE".into(), idle.to_string()));
            served.push(node);
        }
        let running = run(served);
        Kernel { _running: running, address: settings.address.clone(), settings, sites: format!("http://127.0.0.1:{sites}") }
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

/// When an access token expires, in seconds, as it says.
fn tokens_exp(token: &str) -> u64 {
    let claims: Value = serde_json::from_slice(&B64U.decode(token.split('.').nth(1).unwrap().as_bytes()).unwrap()).unwrap();
    claims["exp"].as_u64().unwrap()
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// An endpoint's key, as a device holds it, and its hex, as the relay lists it.
fn endpoint() -> (ed25519_dalek::SigningKey, String) {
    let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let hex = data_encoding::HEXLOWER.encode(key.verifying_key().as_bytes());
    (key, hex)
}

/// What `key`'s device signs for auth's `route`: `fields`, with the route, as devices sign requests.
fn signed(key: &ed25519_dalek::SigningKey, route: &str, mut fields: Value) -> Value {
    fields["route"] = json!(route);
    one::keys::signed_request(key, "one/auth", fields, now())
}

/// A signed GET's or DELETE's request, as its `x-one-request` header carries it.
fn header(request: &Value) -> String {
    B64U.encode(request.to_string().as_bytes())
}

/// An invite `by` an endpoint for `name`, as its device makes one: → its secret and the user id it's for.
fn invited(k: &Kernel, by: &ed25519_dalek::SigningKey, name: &str, email: Option<&str>) -> (String, String) {
    let invite = kernel::invite(&k.address, by, name, None, email).unwrap();
    (invite.link.split_once('#').unwrap().1.to_string(), invite.user)
}

/// Opening an invite link (its secret): they make a passkey, which signs them up and in, as who the invite says.
/// → their passkey and user id.
fn joins_with(k: &Kernel, browser: &mut Browser, secret: &str) -> (Passkey, String) {
    let (status, about) = k.browser().post("/invite/about", json!({ "secret": secret }));
    assert_eq!(status, 200, "{about}");
    let passkey = k.passkey();
    let (status, options, _) = browser.get(&format!("/api/auth/passkey/generate-register-options?context={}", enc(secret)));
    assert_eq!(status, 200, "{options}");
    let (status, made) = browser.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true }));
    assert_eq!(status, 200, "{made}");
    assert_eq!(made["user"]["name"], about["person"]);
    (passkey, made["user"]["id"].as_str().unwrap().to_string())
}

/// Signs in with a passkey in `browser`: → the status.
fn signs_in_with_passkey(browser: &mut Browser, passkey: &Passkey) -> u16 {
    let (status, options, _) = browser.get("/api/auth/passkey/generate-authenticate-options");
    assert_eq!(status, 200, "{options}");
    browser.post("/api/auth/passkey/verify-authentication", json!({ "response": passkey.get(&options) })).0
}

/// Who `browser` is signed in as here, if anyone.
fn signed_in_as(browser: &mut Browser) -> Option<String> {
    browser.get("/api/auth/get-session").1["user"]["id"].as_str().map(str::to_string)
}

#[test]
fn an_invite_lets_one_person_sign_up_once_and_only_before_it_ends() {
    let (admin, admin_hex) = endpoint();
    let k = Kernel::for_devices(vec![admin_hex]);
    let (secret, user) = invited(&k, &admin, "Sam", None);
    let mut sams = k.browser();
    let (_, sam) = joins_with(&k, &mut sams, &secret);
    assert_eq!(sam, user, "they're who the invite says");
    assert_eq!(signed_in_as(&mut sams).as_deref(), Some(sam.as_str()));

    // It's spent: nobody else signs up with it, and Sam isn't invited again.
    let (status, _) = k.browser().post("/invite/about", json!({ "secret": secret }));
    assert_eq!(status, 404);
    let (status, _, _) = k.browser().get(&format!("/api/auth/passkey/generate-register-options?context={}", enc(&secret)));
    assert_eq!(status, 404, "a used invite let someone make a passkey");
    let again = kernel::invite(&k.address, &admin, "Sam", Some(&sam), None).err().unwrap().to_string();
    assert!(again.contains("joined already"), "{again}");

    // One that has ended lets nobody in.
    let secret = B64U.encode(&rand::random::<[u8; 32]>());
    let id = data_encoding::HEXLOWER.encode(&one::keys::sha256(secret.as_bytes()));
    let fields = json!({ "id": id, "name": "Kim", "user": one::entry::new_id(), "until": now() + 1500 });
    let (status, out) = post(&format!("{}/api/auth/invites", k.address), &[], signed(&admin, "POST /api/auth/invites", fields));
    assert_eq!(status, 200, "{out}");
    std::thread::sleep(std::time::Duration::from_millis(2000));
    let (status, out) = k.browser().post("/invite/about", json!({ "secret": secret }));
    assert_eq!(status, 404, "{out}");
    let (status, _, _) = k.browser().get(&format!("/api/auth/passkey/generate-register-options?context={}", enc(&secret)));
    assert_eq!(status, 404, "an ended invite let someone make a passkey");
}

#[test]
fn only_an_admins_device_makes_invites_or_says_who_is_an_admin() {
    let ((admin, admin_hex), (member, member_hex), (stranger, _)) = (endpoint(), endpoint(), endpoint());
    let k = Kernel::for_devices(vec![admin_hex]);
    // The admin's device lets in a member's, which isn't an admin's.
    let tell = one::keys::signed_request(&admin, "one/relay", json!({ "add": member_hex, "role": "client" }), now());
    assert_eq!(post(&format!("{}/relay/tell", k.address), &[], tell).0, 204);
    for key in [&member, &stranger] {
        assert!(kernel::invite(&k.address, key, "Kim", None, None).is_err(), "a device that isn't an admin's made an invite");
    }
    let (secret, kim) = invited(&k, &admin, "Kim", None);
    joins_with(&k, &mut k.browser(), &secret);
    let admin_route = format!("{}/api/auth/people/admin", k.address);
    for key in [&member, &stranger] {
        let (status, out) = post(&admin_route, &[], signed(key, "POST /api/auth/people/admin", json!({ "user": kim, "admin": true })));
        assert_eq!(status, 403, "a device that isn't an admin's made someone one: {out}");
    }

    // A request counts only as signed: not unsigned, nor at another route, nor twice.
    let fields = json!({ "user": kim, "admin": true });
    assert_eq!(post(&admin_route, &[], fields.clone()).0, 403);
    assert_eq!(post(&format!("{}/api/auth/invites", k.address), &[], signed(&admin, "POST /api/auth/people/admin", fields.clone())).0, 403);
    let request = signed(&admin, "POST /api/auth/people/admin", fields);
    let mut changed = request.clone();
    changed["user"] = json!(one::entry::new_id());
    assert_eq!(post(&admin_route, &[], changed).0, 403, "a changed request was taken");
    assert_eq!(post(&admin_route, &[], request.clone()).0, 200);
    assert_eq!(post(&admin_route, &[], request).0, 403, "a request was taken twice");
}

/// Joining with Google, as the invite page does: its secret kept for Google's answer, as `email`. → where the
/// callback goes on to.
fn joins_with_google(browser: &mut Browser, secret: &str, email: &str) -> Option<String> {
    let (status, out) = browser.post("/invite/google", json!({ "secret": secret }));
    assert_eq!(status, 200, "{out}");
    google_callback(browser, "/", &format!("/invite#{secret}"), email)
}

/// Signs in with Google as `email`, coming back to `back` (or `failed`): → where Google's callback goes on to.
fn google_callback(browser: &mut Browser, back: &str, failed: &str, email: &str) -> Option<String> {
    let (status, out) = browser.post("/api/auth/sign-in/social", json!({ "provider": "google", "callbackURL": back, "errorCallbackURL": failed }));
    assert_eq!(status, 200, "{out}");
    let state = url_query(out["url"].as_str().unwrap())["state"].clone();
    let claims = json!({ "iss": "https://accounts.google.com", "sub": email, "email": email, "email_verified": true, "hd": "test.example", "name": email });
    let (_, _, to) = browser.get(&format!("/api/auth/callback/google?code={}&state={state}", B64U.encode(claims.to_string().as_bytes())));
    to
}

#[test]
fn an_admin_signs_in_only_with_a_passkey() {
    let (admin, admin_hex) = endpoint();
    let k = Kernel::google(vec![admin_hex]);
    let (secret, andre) = invited(&k, &admin, "Andre", Some("andre@test.example"));
    // Signing up with Google takes the account the invite is for, and nobody signs up by the domain alone.
    let mut lees = k.browser();
    joins_with_google(&mut lees, &secret, "lee@test.example");
    assert_eq!(signed_in_as(&mut lees), None, "someone else's Google account signed up with the invite");
    let mut kims = k.browser();
    google_callback(&mut kims, "/", "/", "kim@test.example");
    assert_eq!(signed_in_as(&mut kims), None, "someone at the domain joined uninvited");
    let mut andres = k.browser();
    assert_eq!(joins_with_google(&mut andres, &secret, "andre@test.example").as_deref(), Some("/"));
    assert_eq!(signed_in_as(&mut andres).as_deref(), Some(andre.as_str()));
    let mut again = k.browser();
    google_callback(&mut again, "/", "/", "andre@test.example");
    assert_eq!(signed_in_as(&mut again).as_deref(), Some(andre.as_str()), "he signs in with Google again");

    // With no passkey yet, he isn't made an admin, or he couldn't sign in. Just signed in, he adds one.
    let admin_route = format!("{}/api/auth/people/admin", k.address);
    let made_admin = || post(&admin_route, &[], signed(&admin, "POST /api/auth/people/admin", json!({ "user": andre, "admin": true }))).0;
    assert_eq!(made_admin(), 409);
    let passkey = k.passkey();
    let (status, options, _) = andres.get("/api/auth/passkey/generate-register-options");
    assert_eq!(status, 200, "{options}");
    let (status, out) = andres.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options) }));
    assert_eq!(status, 200, "{out}");
    assert_eq!(made_admin(), 200);

    // An admin, Google signs him in no more; his passkey does.
    let mut elsewhere = k.browser();
    google_callback(&mut elsewhere, "/", "/", "andre@test.example");
    assert_eq!(signed_in_as(&mut elsewhere), None, "an admin signed in with Google");
    assert_eq!(signs_in_with_passkey(&mut elsewhere, &passkey), 200);
    assert_eq!(signed_in_as(&mut elsewhere).as_deref(), Some(andre.as_str()));
}

#[test]
fn a_person_sees_only_their_own_join_requests_and_the_installs_devices_see_them_all() {
    let ((a, a_hex), (b, b_hex)) = (endpoint(), endpoint());
    let k = Kernel::for_devices(vec![a_hex, b_hex]);
    let ((kim_secret, _), (lee_secret, _)) = (invited(&k, &a, "Kim", None), invited(&k, &b, "Lee", None));
    let (mut kims, mut lees) = (k.browser(), k.browser());
    joins_with(&k, &mut kims, &kim_secret);
    joins_with(&k, &mut lees, &lee_secret);
    let requests = format!("{}/api/auth/devices/requests", k.address);

    // Each newcomer's first device asks to join, with its proof of their invite's secret.
    let asks = |browser: &mut Browser, secret: &str| {
        let (key, hex) = endpoint();
        let seal = data_encoding::HEXLOWER.encode(&one::keys::Endpoint::of(&key).seal);
        let proof = one::keys::invite_proof(secret.as_bytes(), &one::keys::Endpoint::of(&key));
        let (status, out) = browser.post("/api/auth/devices/requests", json!({ "key": hex, "seal": seal, "name": "Chrome on Mac", "proof": proof }));
        assert_eq!(status, 200, "{out}");
        hex
    };
    let (kim_device, lee_device) = (asks(&mut kims, &kim_secret), asks(&mut lees, &lee_secret));
    let keys = |out: &Value| out["requests"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(keys(&kims.get("/api/auth/devices/requests").1), vec![kim_device.clone()]);
    assert_eq!(keys(&lees.get("/api/auth/devices/requests").1), vec![lee_device.clone()]);
    let by = |key: &ed25519_dalek::SigningKey, method: &str, path: &str| {
        let request = signed(key, &format!("{method} /api/auth/devices/requests{path}"), json!({}));
        let (status, _, body) = ask(method, &format!("{requests}{path}"), &[("x-one-request", &header(&request))], None);
        (status, serde_json::from_str::<Value>(&body).unwrap_or(Value::Null))
    };
    // The install's devices see every one: what they write in approving one counts only from the right person's device.
    assert_eq!(keys(&by(&a, "GET", "").1), vec![kim_device.clone(), lee_device.clone()]);
    assert_eq!(keys(&by(&b, "GET", "").1), vec![kim_device.clone(), lee_device.clone()]);
    let (stranger, _) = endpoint();
    assert_eq!(by(&stranger, "GET", "").0, 403, "a device that isn't the install's sees none");

    // No other person sees or removes one.
    assert_eq!(lees.get(&format!("/api/auth/devices/requests/{kim_device}")).0, 404);
    assert_eq!(lees.send("DELETE", &format!("/api/auth/devices/requests/{kim_device}"), None, None).0, 404);
    assert_eq!(by(&stranger, "DELETE", &format!("/{kim_device}")).0, 403);
    let (status, out) = lees.post("/api/auth/devices/requests", json!({ "key": kim_device, "seal": kim_device, "name": "Not Kim's" }));
    assert_eq!(status, 409, "someone else's device's request was taken over: {out}");
    assert_eq!(k.browser().post("/api/auth/devices/requests", json!({ "key": lee_device, "seal": lee_device, "name": "Nobody's" })).0, 401);

    // Kim's new device sees it waits, until the device that invited her has approved it.
    assert_eq!(kims.get(&format!("/api/auth/devices/requests/{kim_device}")).0, 200);
    assert_eq!(by(&a, "DELETE", &format!("/{kim_device}")).0, 200);
    assert_eq!(kims.get(&format!("/api/auth/devices/requests/{kim_device}")).0, 404);

    // What took or gave keys is gone.
    for path in ["/invite/keys", "/passkeys/add", "/internal/invite"] {
        let (status, _) = k.browser().post(path, json!({}));
        assert!([404, 405].contains(&status), "{path} is still there: {status}");
    }
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
            "DELETE" => self.delete(url),
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
            // Each connection on its own, and one that closes or resets before it's answered is let go: workerd may
            // drop one it opened.
            std::thread::spawn(move || -> std::io::Result<()> {
                let mut reader = BufReader::new(stream.try_clone()?);
                let mut line = String::new();
                reader.read_line(&mut line)?;
                let mut headers = serde_json::Map::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.trim().split_once(':') {
                        headers.insert(k.trim().to_lowercase(), json!(v.trim()));
                    }
                }
                let body = json!({ "request": line.trim(), "headers": headers }).to_string();
                write!(
                    &stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\nset-cookie: better-auth.session_token=planted; Path=/\r\naccess-control-allow-origin: *\r\nservice-worker-allowed: /\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
            });
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
    assert_eq!(headers["cache-control"], "no-cache", "a file not named by its content is asked about each time");
    // Named by its content, a file is kept for good, but only once it's that content: a browser keeps no other for a year.
    let kit = data_encoding::HEXLOWER.encode(&<sha2::Sha256 as sha2::Digest>::digest(include_bytes!("../src/screen/kit.css")));
    let (_, headers, _) = ask("GET", &at(&format!("/screen/kit.css?v={}", &kit[..16])), &[], None);
    assert_eq!(headers["cache-control"], "public, max-age=31536000, immutable");
    let (_, headers, _) = ask("GET", &at("/screen/kit.css?v=0123456789abcdef"), &[], None);
    assert_eq!(headers["cache-control"], "no-cache", "a name that isn't its content");
    assert_eq!(ask("GET", &at("/nothing.js"), &[], None).0, 404, "a missing file isn't the page");

    // Sign-in goes to the auth service, as the install's address; its own routes don't.
    let (status, _, meta) = ask("GET", &at("/.well-known/oauth-authorization-server"), &[], None);
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
    assert_eq!(ask("POST", &at("/mcp"), &[], Some(json!({}))).0, 503, "/mcp is the MCP Worker's, never a node's, and this kernel has none");
    assert_eq!(ask("GET", &at("/v1/state"), &[("service-worker", "script")], None).0, 403, "no node's answer becomes a service worker");
}

/// Settings' screens work for someone signed in, through the gateway, as they do on that person's own Mac: each route
/// acts as them and is checked as theirs, so a member can't do what only admins may, and nobody acts as anyone else.
#[test]
fn a_sign_in_uses_settings_as_its_person_and_only_with_their_rights() {
    let v = StandIn::start();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = crate::temp("node");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": "node" }).to_string()).unwrap();
    let (node, api) = rt.block_on(async {
        let node = one::node::Node::start(&dir, one::sidecars::built_in()).await.unwrap();
        node.create("Test", "Andre").await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let app = one::api::router(node.clone());
        tokio::spawn(one::api::serve(listener, app));
        (node, api)
    });
    // Andre made the install, on this device, his; Sam is a member, with a phone of his own. The device takes sign-ins
    // made at the install's address.
    let (andre, sam) = (node.user().unwrap(), one::entry::new_id());
    let phone = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let sams_phone = one::keys::Endpoint::of(&phone);
    rt.block_on(async {
        node.member("device", json!({ "key": one::entry::hex(&sams_phone.key), "seal": one::entry::hex(&sams_phone.seal), "user": sam, "roles": ["client"], "name": "Sam's phone", "user_name": "Sam", "role": "member" })).await.unwrap();
        node.member("setting", json!({ "name": "address", "value": v.auth })).await.unwrap();
    });
    let (andres, sams) = (v.token(&andre, "web"), v.token(&sam, "web"));
    let ask = |token: &str, method: &str, path: &str, body: Value| ask_at(&api, token, method, path, body);
    let state = |token: &str| ask(token, "GET", "/v1/state", Value::Null).1;
    let items = |token: &str| state(token)["settings_items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let refused = |(status, body): (u16, Value)| status == 400 && body["error"].as_str().is_some_and(|e| !e.is_empty());
    // What Sam's phone signs, after what the device holds, and the device writes (`POST /v1/entries`).
    let signs = |kind: &str, data: Value| {
        let (_, heads) = ask(&sams, "GET", "/v1/entries?scope=global", Value::Null);
        let own = &heads["logs"][one::entry::hex(&sams_phone.key)];
        let backlink = own["hash"].as_str().map(|h| (own["seq"].as_u64().unwrap() as u32, one::entry::Hash::from_hex(h).unwrap()));
        let previous = heads["previous"].as_array().unwrap().iter().map(|h| one::entry::Hash::from_hex(h.as_str().unwrap()).unwrap()).filter(|h| backlink.is_none_or(|b| b.1 != *h)).collect();
        let ext = one::entry::Ext { log: "global".into(), time: 0, user: sam.clone(), epoch: heads["epoch"].as_u64().unwrap() as u32, previous };
        let entry = one::entry::Entry::create(&phone, ext, backlink, one::members::node(kind, data));
        ask(&sams, "POST", "/v1/entries", json!([entry.to_json()]))
    };

    // A remote MCP client's token is for the MCP Worker (`<address>/mcp`), whose device it is: no sign-in here. Nor
    // is naming a client device as a token any way to act as it and use what it was handed, even from this machine's
    // own programs: only a connection that proves its key is.
    let mcps = v.token_for(&andre, "claude", &format!("{}/mcp", v.auth.trim_end_matches('/')));
    assert_eq!(ask(&mcps, "GET", "/v1/state", Value::Null).0, 401, "an MCP client's token signs in at a node");
    let named = |local: bool| {
        let req = ureq::Agent::from(ureq::Agent::config_builder().http_status_as_error(false).build()).get(format!("{api}/v1/apps")).header("authorization", format!("Bearer client.{}", one::entry::hex(&sams_phone.key)));
        let req = if local { req.header(one::api::LOCAL_TOKEN_HEADER, node.local_token()) } else { req };
        req.call().unwrap().status().as_u16()
    };
    assert_eq!((named(true), named(false)), (401, 401), "a header named a client device");

    // Settings shows each their own: Passkeys first, at the install's address (no try install has it, so it's seen only
    // here); only admins see Users, General and Backups.
    assert_eq!(state(&sams)["settings_items"][0]["url"], json!(format!("{}/passkeys", v.auth.trim_end_matches('/'))));
    assert!(!items(&sams).iter().any(|i| ["users", "general", "backups"].contains(&i.as_str())), "{:?}", items(&sams));
    assert!(["users", "general", "backups"].iter().all(|i| items(&andres).iter().any(|n| n == i)), "{:?}", items(&andres));
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let page = |path: &str| agent.get(format!("{api}{path}")).header("authorization", format!("Bearer {sams}")).call().unwrap().body_mut().read_to_string().unwrap();
    let vault = page("/v1/client-screens/vault");
    assert!(vault.contains(&format!("\"user\":\"{sam}\"")), "the page is Sam's: {vault}");
    assert!(page("/v1/client-screens/nothing").contains("one.notFound()"));

    // What only admins may do, Sam can't; Andre can, signed in as on his Mac.
    let setting = json!({ "name": "push", "value": "push" });
    assert!(refused(ask(&sams, "POST", "/v1/settings", setting.clone())), "a member changes no install setting");
    assert!(refused(ask(&sams, "POST", "/v1/role", json!({ "user": sam, "role": "admin" }))), "nor makes himself an admin");
    assert!(refused(ask(&sams, "POST", "/v1/remove", json!({ "user": andre }))), "nor removes anyone");
    assert!(refused(ask(&sams, "POST", "/v1/backups/place", json!({ "path": dir.join("backups"), "title": "Disk" }))), "nor adds backups");
    assert!(refused(ask(&sams, "POST", "/v1/pair", json!({ "user": andre }))), "nor pairs a machine as someone else");
    assert_eq!(ask(&andres, "POST", "/v1/settings", setting).0, 200);
    assert_eq!(state(&andres)["install"]["settings"]["push"], "push");

    // What only a person's own device signs, this device, Andre's, can't sign for Sam: his phone does. A group he makes
    // is his; its members change only as its admins and invitees say.
    let (status, said) = ask(&sams, "POST", "/v1/group", json!({ "name": "Sam's" }));
    assert!(status == 400 && said["error"].as_str().unwrap().contains("isn't one of your devices"), "{said}");
    let group = one::entry::new_id();
    assert_eq!(signs("group", json!({ "group": group, "name": "Sam's" })).0, 200);
    let members = || state(&sams)["install"]["groups"].as_array().unwrap().iter().find(|g| g["id"] == group.as_str()).unwrap()["members"].clone();
    assert_eq!(members(), json!([{ "user": sam, "role": "admin" }]));
    assert!(refused(signs("accept", json!({ "group": group }))), "nobody accepts an invite they weren't sent");
    assert!(refused(ask(&andres, "POST", "/v1/accept", json!({ "group": group }))), "Andre neither");
    assert!(refused(signs("setting", json!({ "name": "made_here", "value": "Sam's" }))), "nor does Sam's phone sign what only admins may");
    assert_eq!(signs("invite", json!({ "group": group, "user": andre, "role": "member" })).0, 200);
    assert!(state(&andres)["settings_items"].as_array().unwrap().iter().any(|i| i["name"] == "requests" && i["badge"] == 1), "his invite waits in Requests");
    assert_eq!(ask(&andres, "POST", "/v1/accept", json!({ "group": group })).0, 200);
    assert_eq!(members().as_array().unwrap().len(), 2);
    assert_eq!(signs("member", json!({ "group": group, "user": andre, "role": null })).0, 200);
    assert_eq!(members().as_array().unwrap().len(), 1);

    // What only this machine's own programs do, no sign-in from elsewhere (through the gateway) does: renaming the
    // device, say.
    let renamed = agent.post(format!("{api}/v1/name")).header("authorization", format!("Bearer {andres}")).header("forwarded", "host=\"one.example\"").send_json(json!({ "name": "elsewhere" })).unwrap();
    assert_eq!(renamed.status(), 403);
    // A sign-in is no device: not the device it reached, so nothing offers it that device's name to change.
    let seen = state(&andres);
    assert_eq!(seen["me"]["key"], "", "{}", seen["me"]);
    assert!(seen["install"]["devices"].as_array().unwrap().iter().all(|s| s["me"] == false), "{}", seen["install"]["devices"]);
    assert!(seen.get("client").is_none());

    // A file is read only by whoever put it or may use a scope that names it: knowing its hash isn't enough. Only
    // this machine's own programs put files, a sign-in's MCP answers among them.
    let get = |token: &str, hash: &str| agent.get(format!("{api}/v1/blobs/{hash}")).header("authorization", format!("Bearer {token}")).header("forwarded", "host=\"one.example\"").call().unwrap().status().as_u16();
    let theirs = node.put_blob(b"an attachment of someone else's").unwrap();
    assert_eq!(get(&sams, &theirs), 400, "a sign-in reads a file nobody gave it");
    let put = |token: &str, forwarded: bool| {
        let req = agent.post(format!("{api}/v1/blobs")).header("authorization", format!("Bearer {token}"));
        let mut resp = if forwarded { req.header("forwarded", "host=\"one.example\"") } else { req.header(one::api::LOCAL_TOKEN_HEADER, node.local_token()) }.send(&b"Sam's file"[..]).unwrap();
        (resp.status().as_u16(), resp.body_mut().read_json::<Value>().ok().and_then(|b| b["blob"].as_str().map(str::to_string)))
    };
    assert_eq!(put(&sams, true).0, 403, "a sign-in from elsewhere puts no files here");
    let (status, sams_file) = put(&sams, false);
    assert_eq!(status, 200);
    let sams_file = sams_file.unwrap();
    assert_eq!(get(&sams, &sams_file), 200, "Sam reads back what he put");
    assert_eq!(get(&andres, &sams_file), 400, "and Andre doesn't, by its hash");

    // Someone removed signs nothing here.
    assert_eq!(ask(&andres, "POST", "/v1/remove", json!({ "user": sam })).0, 200);
    assert_eq!(ask(&sams, "GET", "/v1/entries?scope=global", Value::Null).0, 403, "a removed person is signed in no more, though their token hasn't expired");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A client names itself, so the sign-in page also says where it goes, and no client takes the install's name, nor its
/// apps' unless it comes back to the device as they do: one passkey tap gives a client the person's keys.
#[test]
fn a_client_is_shown_by_where_it_goes_and_none_is_named_as_the_install() {
    let k = Kernel::start(None);
    let register = |name: &str, back: &str| post(&format!("{}/api/auth/oauth2/register", k.address), &[], json!({ "client_name": name, "redirect_uris": [back], "token_endpoint_auth_method": "none", "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"] }));
    for (name, back) in [("Test", "https://elsewhere.example/back"), (" the TEST ", "http://localhost:4000/back"), ("Test on Andre's Mac", "https://elsewhere.example/back")] {
        let (status, out) = register(name, back);
        assert_eq!(status, 400, "{name} going to {back} was taken: {out}");
    }
    let goes = |name: &str, back: &str| {
        let (status, out) = register(name, back);
        assert_eq!(status, 201, "{out}");
        let (_, about, _) = k.browser().get(&format!("/sign-in/about?client_id={}&redirect_uri={}", out["client_id"].as_str().unwrap(), enc(back)));
        (about["client"].as_str().unwrap().to_string(), about["goes"].clone())
    };
    assert_eq!(goes("Claude", "https://claude.ai/api/mcp/auth_callback"), ("Claude".into(), json!("claude.ai")));
    assert_eq!(goes("Claude Code", "http://localhost:33418/callback").1, "an app on this device");
    assert_eq!(goes("Test on Andre's iPhone", "one://signed-in/").1, "an app on this device");
    assert_eq!(goes("Test on a device", &format!("{}/sign-in/code/done", k.address)).1, "where you got your code");
}

/// A client registers itself (anyone may) and its authorization goes to the sign-in page: → the page's query.
fn authorizes(browser: &mut Browser, name: &str, back: &str) -> String {
    let (status, client) = browser.post("/api/auth/oauth2/register", json!({ "client_name": name, "redirect_uris": [back], "token_endpoint_auth_method": "none" }));
    assert_eq!(status, 201, "{client}");
    let client = client["client_id"].as_str().unwrap();
    let (_, _, sign_in) = browser.get(&format!("/api/auth/oauth2/authorize?response_type=code&client_id={client}&redirect_uri={}&scope=openid&state=s&code_challenge=c&code_challenge_method=S256", enc(back)));
    sign_in.expect("authorizing goes to the sign-in page").split_once('?').unwrap().1.to_string()
}

/// Andre signs in with Google in `browser`, coming back to the sign-in page for `query`, as its button does, which
/// starts it there first unless `started` is false.
fn signs_in_with_google(browser: &mut Browser, query: &str, started: bool) {
    if started {
        let (status, out) = browser.post("/sign-in/google", json!({ "oauth_query": query }));
        assert_eq!(status, 200, "{out}");
    }
    let back = format!("/sign-in?{query}");
    assert_eq!(google_callback(browser, &back, &back, "andre@test.example"), Some(back));
}

/// Signing in to a client with Google goes straight on, with no second tap, only for the authorization the sign-in
/// was started for, in that browser: a link to someone else's client, opened just after signing in to another, waits
/// for the person.
#[test]
fn a_google_sign_in_goes_on_only_to_the_client_it_was_started_for() {
    let (admin, admin_hex) = endpoint();
    let k = Kernel::google(vec![admin_hex]);
    let mut andres = k.browser();
    let (secret, _) = invited(&k, &admin, "Andre", Some("andre@test.example"));
    joins_with_google(&mut andres, &secret, "andre@test.example");
    let fresh = |b: &mut Browser, query: &str| b.get(&format!("/sign-in/about?{query}")).1["fresh"].clone();
    let accept = |b: &mut Browser, query: &str| b.post("/sign-in/accept", json!({ "oauth_query": query })).0;
    let ours = authorizes(&mut andres, "Andre's", "http://localhost:9/signed-in");
    signs_in_with_google(&mut andres, &ours, true);
    assert_eq!(fresh(&mut andres, &ours), true);
    assert_eq!(accept(&mut andres, &ours), 200);
    assert_eq!(accept(&mut andres, &ours), 403, "it goes on once");

    // Someone's client, whose link he opens a minute later: signed in with Google just now, all the same.
    let theirs = authorizes(&mut k.browser(), "Someone's", "https://elsewhere.example/back");
    assert_eq!(fresh(&mut andres, &theirs), false);
    assert_eq!(accept(&mut andres, &theirs), 403);
    // Nor once its sender starts a Google sign-in for it in their own browser, nor a page elsewhere in his.
    let mut theirs_started = k.browser();
    assert_eq!(theirs_started.post("/sign-in/google", json!({ "oauth_query": theirs })).0, 200);
    let mut elsewhere = Browser { origin: "https://elsewhere.example".into(), cookies: andres.cookies.clone() };
    let (status, out) = elsewhere.post(&format!("{}/sign-in/google", k.address), json!({ "oauth_query": theirs }));
    assert_eq!(status, 403, "{out}");
    assert_eq!(accept(&mut andres, &theirs), 403);
    // Nor signing in with Google again not started for it.
    signs_in_with_google(&mut andres, &theirs, false);
    assert_eq!(accept(&mut andres, &theirs), 403);
    // Tapping Sign In with Google on its page, he means it.
    signs_in_with_google(&mut andres, &theirs, true);
    assert_eq!(accept(&mut andres, &theirs), 200);
}

#[test]
fn only_a_try_installs_kernel_signs_its_person_in_with_nothing_to_show() {
    let (admin, admin_hex) = endpoint();
    let k = Kernel::for_devices(vec![admin_hex]);
    let (status, out) = k.browser().post("/api/auth/try/sign-in", json!({}));
    assert_eq!(status, 404, "a kernel people use signed someone in with no passkey: {out}");
    let (status, out) = k.browser().post("/api/auth/try/sign-in", json!({ "person": "Andre" }));
    assert_eq!(status, 404, "a kernel people use signed someone in by name: {out}");
    // Nor does an invite link join anyone there without a passkey.
    let (secret, _) = invited(&k, &admin, "Sam", None);
    let (status, out) = k.browser().post("/api/auth/try/join", json!({ "secret": secret }));
    assert_eq!(status, 404, "a kernel people use let someone join with no passkey: {out}");
    // Only `one try` makes such settings, and nothing puts them where anyone but this machine reaches them.
    let mut tried = k.settings.clone();
    tried.try_person = Some(("0123456789abcdef".into(), "Andre".into()));
    let cf = kernel::Cloudflare { api: "http://127.0.0.1:9".into(), token: "a token".into(), account: "an account".into() };
    for refused in [kernel::deploy(&cf, &tried, &[], &[]), one::serverless::deploy(&cf, &tried), kernel::serve(&tried)] {
        assert!(refused.unwrap_err().to_string().contains("try install"));
    }
}

#[test]
fn the_address_serves_only_scripts_that_parse() {
    let settings = kernel::Settings::new("Test", "https://one.example.com");
    kernel::parses(&settings.web()).unwrap();
    let broken = vec![("web.js".to_string(), b"const label = 1;\nconst label = 2;\n".to_vec())];
    assert!(kernel::parses(&broken).is_err(), "a script that doesn't parse is refused");
    let inline = vec![("index.html".to_string(), b"<p>hi</p><script>let a = ;</script>".to_vec())];
    assert!(kernel::parses(&inline).is_err(), "so is an HTML page's inline script");
}

/// A reopen draws with no trip to the address only if every script the page imports is asked for by its content (the
/// gateway keeps those for good): a plain path is a round trip, and a hash that isn't the file's is kept for nothing.
#[test]
fn the_web_client_imports_each_script_by_its_content() {
    let web: std::collections::HashMap<String, Vec<u8>> = kernel::Settings::new("Test", "https://one.example.com").web().into_iter().collect();
    let mut named = 0;
    for (path, bytes) in web.iter().filter(|(path, _)| path.ends_with(".js") && !path.starts_with("client/")) {
        let text = String::from_utf8_lossy(bytes);
        for quoted in text.split('"').skip(1).step_by(2).filter(|q| q.starts_with('/') || q.starts_with("./") || q.starts_with("../")) {
            let (file, v) = quoted.split_once("?v=").unwrap_or((quoted, ""));
            let file = file.trim_start_matches("../").trim_start_matches("./").trim_start_matches('/');
            if !file.ends_with(".js") || !web.contains_key(file) || file == "sw.js" {
                continue;
            }
            let hash = data_encoding::HEXLOWER.encode(&<sha2::Sha256 as sha2::Digest>::digest(&web[file]));
            assert!(!v.is_empty() && hash.starts_with(v) && v.len() >= 12, "{path} imports {quoted}, not {file} by its content ({})", &hash[..16]);
            named += 1;
        }
    }
    assert!(named >= 6, "web.js and the worker import their scripts: {named}");
}

/// The serverless node, the node's Rust as WebAssembly, built as `one update` builds it: what doesn't build, it can't
/// send out. Built in a target folder of its own, kept between runs, so this test's cargo never waits on the one running
/// it: → its modules (`serverless::modules_from`).
pub(crate) fn serverless_node() -> Vec<(String, &'static str, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let target = root.join("target/serverless-test");
    let out = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--release", "-p", "one-serverless", "--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(&target)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success(), "the serverless node doesn't build (it needs rustup target add wasm32-unknown-unknown, and clang):\n{}", String::from_utf8_lossy(&out.stderr));
    let glue = target.join("glue");
    let out = std::process::Command::new("wasm-bindgen").args(["--target", "web", "--out-dir"]).arg(&glue).arg(target.join("wasm32-unknown-unknown/release/one_serverless.wasm")).output().unwrap();
    assert!(out.status.success(), "wasm-bindgen (the version in Cargo.lock) didn't make its glue:\n{}", String::from_utf8_lossy(&out.stderr));
    one::serverless::modules_from(std::fs::read(glue.join("one_serverless.js")).unwrap(), std::fs::read(glue.join("one_serverless_bg.wasm")).unwrap())
}

/// iroh endpoints, as nodes are, through the relay at a kernel's address and nothing else: its relay protocol is
/// iroh-relay's Rust as WebAssembly in a Durable Object (kernel/relay/), which must agree with iroh's own.
mod relay {
    use std::time::Duration;

    use iroh::endpoint::{Endpoint, presets};
    use iroh::{EndpointAddr, RelayConfig, RelayMode, RelayUrl, SecretKey};
    use serde_json::{Value, json};

    const ALPN: &[u8] = b"one/test/0";

    fn hex(key: &SecretKey) -> String {
        data_encoding::HEXLOWER.encode(key.public().as_bytes())
    }

    /// An endpoint reached only through the relay at `address`, showing `ticket` (a pairing code's) if given.
    async fn endpoint(address: &str, key: &SecretKey, ticket: Option<&str>) -> Endpoint {
        let mut relay = RelayConfig::new(address.parse::<RelayUrl>().unwrap(), None);
        if let Some(ticket) = ticket {
            relay = relay.with_auth_token(ticket);
        }
        let builder = Endpoint::builder(presets::Minimal).secret_key(key.clone()).alpns(vec![ALPN.to_vec()]).clear_ip_transports().clear_address_lookup();
        builder.relay_mode(RelayMode::Custom(relay.into())).bind().await.unwrap()
    }

    /// Whether the relay let `key` in, as iroh sees it.
    async fn online(address: &str, key: &SecretKey, ticket: Option<&str>) -> Option<Endpoint> {
        let ep = endpoint(address, key, ticket).await;
        match tokio::time::timeout(Duration::from_secs(2), ep.online()).await {
            Ok(()) => Some(ep),
            Err(_) => {
                ep.close().await;
                None
            }
        }
    }

    /// Whether `from` reaches `to` through the relay: a stream that `to` echoes back, whole.
    async fn reaches(address: &str, from: &Endpoint, to: &Endpoint) -> bool {
        let serving = to.clone();
        let server = tokio::spawn(async move {
            let conn = serving.accept().await?.await.ok()?;
            let (mut send, mut recv) = conn.accept_bi().await.ok()?;
            let got = recv.read_to_end(1 << 20).await.ok()?;
            send.write_all(&got).await.ok()?;
            send.finish().ok()?;
            conn.closed().await;
            Some(())
        });
        let said = vec![7u8; 100_000];
        let at = EndpointAddr::new(to.id()).with_relay_url(address.parse().unwrap());
        let echoed = tokio::time::timeout(Duration::from_secs(2), async {
            let conn = from.connect(at, ALPN).await.ok()?;
            let (mut send, mut recv) = conn.open_bi().await.ok()?;
            send.write_all(&said).await.ok()?;
            send.finish().ok()?;
            let got = recv.read_to_end(1 << 20).await.ok()?;
            conn.close(0u32.into(), b"done");
            Some(got)
        })
        .await;
        server.abort();
        match echoed {
            Ok(Some(got)) => {
                assert_eq!(got, said, "the relay changed what it carried");
                true
            }
            _ => false,
        }
    }

    /// What `key` tells the relay: `fields`, signed as devices sign requests.
    fn tell(address: &str, key: &SecretKey, fields: Value) -> u16 {
        let key = ed25519_dalek::SigningKey::from_bytes(&key.to_bytes());
        let request = one::keys::signed_request(&key, "one/relay", fields, super::now());
        super::post(&format!("{address}/relay/tell"), &[], request).0
    }

    fn sha256(text: &str) -> String {
        use sha2::Digest;
        data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(text.as_bytes()))
    }

    /// iroh's relay frames a socket may be sent first: its challenge, or that it's in.
    const CHALLENGE: u8 = 0;
    const CONFIRMED: u8 = 2;

    /// The kind of the first frame the relay sends a socket that shows `token`, as a browser's WebSocket asks.
    fn first_frame(address: &str, token: &str) -> u8 {
        use std::io::{Read, Write};
        let host = address.trim_start_matches("http://");
        let mut tcp = std::net::TcpStream::connect(host).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(tcp, "GET /relay?token={token} HTTP/1.1\r\nhost: {host}\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\nsec-websocket-protocol: iroh-relay-v2\r\n\r\n").unwrap();
        let mut got = Vec::new();
        let mut byte = [0u8; 1];
        while !got.ends_with(b"\r\n\r\n") {
            tcp.read_exact(&mut byte).unwrap();
            got.push(byte[0]);
        }
        assert!(got.starts_with(b"HTTP/1.1 101"), "{}", String::from_utf8_lossy(&got));
        // A binary frame, short and unmasked, as the relay sends: its payload's first byte is iroh's frame type.
        let mut head = [0u8; 3];
        tcp.read_exact(&mut head).unwrap();
        assert_eq!(head[0], 0x82, "not a binary frame");
        head[2]
    }

    fn ticket() -> String {
        data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>())
    }

    /// Asks on a stream of its own, as nodes ask each other (src/node/net.rs): → the answer on it.
    async fn ask(conn: &iroh::endpoint::Connection, msg: &Value) -> anyhow::Result<Value> {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(msg.to_string().as_bytes()).await?;
        send.finish()?;
        Ok(serde_json::from_slice(&recv.read_to_end(1 << 20).await?)?)
    }

    /// The serverless node is an iroh endpoint with no socket: the relay hands it what's sent to its key, by RPC to its
    /// Durable Object, which runs iroh as WebAssembly with a transport of its own, and passes on what it sends. Native
    /// iroh, iroh in WebAssembly and the relay must agree on every packet for an ask to be answered at all. Since iroh
    /// keeps it awake for good, it ends itself once nobody has asked for a while, its connections with it.
    #[test]
    fn the_serverless_node_answers_through_the_relay_and_ends_itself_when_idle() {
        let modules = super::serverless_node();
        let first = SecretKey::generate();
        let idle = 2_000;
        let k = super::Kernel::with_serverless(vec![hex(&first)], modules, idle);
        let address = k.address.replace("localhost", "127.0.0.1");
        let node: iroh::PublicKey = k.settings.serverless_endpoint().unwrap().parse().unwrap();
        let alpn = format!("{}/1", one::config::CLI).into_bytes();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let a = online(&address, &first, None).await.expect("the install's first device isn't let in");
            let at = EndpointAddr::new(node).with_relay_url(address.parse().unwrap());
            // A call asked as nodes ask: it runs as `/v1/call` runs there, which takes only a sign-in's.
            let call = json!({ "kind": "ask", "body": { "call": { "app": "board", "op": "read", "input": {}, "me": {} } } });
            let said = |answer: &Value| answer["error"].as_str().is_some_and(|e| e.contains("sign in"));
            let woke = std::time::Instant::now();
            let conn = a.connect(at.clone(), &alpn).await.expect("the serverless node wasn't reached");
            let connected = woke.elapsed();
            let answer = ask(&conn, &call).await.unwrap();
            assert!(said(&answer), "the serverless node didn't answer as /v1/call does: {answer}");
            let first_answer = woke.elapsed();
            let asked = std::time::Instant::now();
            assert!(said(&ask(&conn, &call).await.unwrap()));
            eprintln!("serverless node: woken and connected in {connected:?}, first answer in {first_answer:?}, then a round trip in {:?}", asked.elapsed());

            // Once idle, it ends: the connection goes with it, and the next call wakes it again on a new one.
            tokio::time::sleep(Duration::from_millis(idle + 3_000)).await;
            let stale = tokio::time::timeout(Duration::from_secs(5), ask(&conn, &call)).await;
            assert!(!matches!(stale, Ok(Ok(_))), "the serverless node didn't end itself once idle");
            let conn = a.connect(at, &alpn).await.expect("the serverless node didn't wake again");
            assert!(said(&ask(&conn, &call).await.unwrap()));
            a.close().await;
        });
    }

    /// One request a stand-in node was asked over iroh: who asked (the key its connection proved), its line and headers.
    #[derive(Clone, Debug)]
    struct Asked {
        by: String,
        line: String,
        headers: std::collections::BTreeMap<String, String>,
    }

    /// What a stand-in node says with 428 to a tool's call it wasn't handed its database's key for: honestly (the
    /// database, sealed to itself), or a lie, for the device to refuse.
    #[derive(Clone)]
    enum Says {
        Honest,
        SealedTo(one::keys::Endpoint),
        Scope(String),
    }

    /// A stand-in node of key `key`, answering client devices over iroh (`<cli>/http/1`) as a node does: each tool in
    /// `tools` needs one database of `scope`'s (`board_read` the board's), which `/mcp` says with 428, as `says` has it,
    /// until it's handed its key sealed to it; `/v1/state` lists it. It keeps what it was asked, and the handoffs it
    /// opened.
    async fn stand_in_node(address: &str, key: &SecretKey, scope: String, says: std::sync::Arc<std::sync::Mutex<Says>>) -> (Endpoint, std::sync::Arc<std::sync::Mutex<(Vec<Asked>, Vec<one::keys::Handoff>)>>) {
        let alpn = format!("{}/http/1", one::config::CLI).into_bytes();
        let relay = RelayConfig::new(address.parse::<RelayUrl>().unwrap(), None);
        let ep = Endpoint::builder(presets::Minimal).secret_key(key.clone()).alpns(vec![alpn]).clear_ip_transports().clear_address_lookup().relay_mode(RelayMode::Custom(relay.into())).bind().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), ep.online()).await.expect("the stand-in node isn't let in");
        let signing = ed25519_dalek::SigningKey::from_bytes(&key.to_bytes());
        let me = one::keys::Endpoint::of(&signing);
        let seen = std::sync::Arc::new(std::sync::Mutex::new((Vec::new(), Vec::new())));
        let (serving, kept) = (ep.clone(), seen.clone());
        tokio::spawn(async move {
            while let Some(incoming) = serving.accept().await {
                let Ok(conn) = incoming.await else { continue };
                let (kept, scope, says) = (kept.clone(), scope.clone(), says.clone());
                let secret = one::keys::seal_secret(&signing);
                tokio::spawn(async move {
                    let by = data_encoding::HEXLOWER.encode(conn.remote_id().as_bytes());
                    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                        let Ok(raw) = recv.read_to_end(1 << 20).await else { return };
                        let text = String::from_utf8_lossy(&raw).to_string();
                        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
                        let mut lines = head.lines();
                        let line = lines.next().unwrap_or("").to_string();
                        let headers: std::collections::BTreeMap<String, String> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())).collect();
                        let handed = headers.get("x-one-keys").map(|h| one::keys::open_handoff(&secret, h).expect("keys handed weren't sealed to the node asked"));
                        let msg: Value = serde_json::from_str(body).unwrap_or(Value::Null);
                        let database = match msg["params"]["name"].as_str() {
                            Some("board_read") => Some("main"),
                            Some("notes_read") => Some("notes"),
                            _ => None,
                        };
                        let has_keys = {
                            let mut held = kept.lock().unwrap();
                            held.0.push(Asked { by: by.clone(), line: line.clone(), headers: headers.clone() });
                            held.1.extend(handed);
                            database.is_none_or(|d| held.1.iter().any(|h: &one::keys::Handoff| h.keys.iter().any(|k| k.database == d)))
                        };
                        let need = |scope: &str, to: &one::keys::Endpoint| json!({ "error": "it needs keys", "need": [{ "scope": scope, "epoch": 1, "app": "board", "database": database }], "to": hex_of(&to.key), "seal": hex_of(&to.seal) });
                        let (status, out) = match line.split(' ').nth(1).unwrap_or("") {
                            "/mcp" if !has_keys => match says.lock().unwrap().clone() {
                                Says::Honest => (428, need(&scope, &me)),
                                Says::SealedTo(other) => (428, need(&scope, &other)),
                                Says::Scope(other) => (428, need(&other, &me)),
                            },
                            "/mcp" => (200, json!({ "jsonrpc": "2.0", "id": msg["id"], "result": { "content": [{ "type": "text", "text": "the board" }] } })),
                            "/v1/state" => (200, json!({ "install": { "devices": [{ "key": hex_of(&me.key), "order": 0 }] } })),
                            _ => (404, json!({ "error": "not here" })),
                        };
                        let out = out.to_string();
                        let said = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{out}", out.len());
                        let _ = send.write_all(said.as_bytes()).await;
                        let _ = send.finish();
                    }
                });
            }
        });
        (ep, seen)
    }

    /// Writes the install's `global` log and Andre's keys to the stand-in bucket at `sites`, as his device would have by
    /// the time it approved the MCP client's (`device`, `seal`): the genesis (his `node`, client and compute), the MCP
    /// client's device, Sam's box (compute that runs none of Andre's calls), and his own scope's first epoch, each epoch
    /// sealed to the MCP client's device. → the key of the board's database `database` in his scope.
    fn andres_install(sites: &str, node: &SecretKey, sams_box: &SecretKey, user: &str, device: &str, seal: &str) -> impl Fn(&str) -> [u8; 32] {
        use one::entry::{Entry, Ext};
        use one::members::{Kind, Role};
        let signing = ed25519_dalek::SigningKey::from_bytes(&node.to_bytes());
        let sams = ed25519_dalek::SigningKey::from_bytes(&sams_box.to_bytes());
        let (global, own, log_app, board) = (rand::random::<[u8; 32]>(), rand::random::<[u8; 32]>(), rand::random::<[u8; 32]>(), rand::random::<[u8; 32]>());
        let mcp = one::keys::Endpoint { key: one::keys::unhex(device).unwrap(), seal: one::keys::unhex(seal).unwrap() };
        let scope = one::entry::Scope::User(user.into());
        let mut prev = None;
        let mut entry = |content| {
            let e = Entry::create(&signing, Ext { log: "global".into(), time: 1, user: user.into(), epoch: 0, previous: Vec::new() }, prev, content);
            prev = Some(e.head());
            e
        };
        let entries = vec![
            entry(one::members::genesis("Test", user, "Andre", &one::keys::Endpoint::of(&signing), "box", &[Kind::Client, Kind::Compute], &rand::random(), &one::keys::epoch_hash(&global))),
            entry(one::members::device(&mcp, user, "Claude", &[Kind::Client], None)),
            entry(one::members::device(&one::keys::Endpoint::of(&sams), &one::entry::new_id(), "Sam's box", &[Kind::Client, Kind::Compute], Some(("Sam", Role::Member)))),
            entry(one::members::epoch(&scope, 1, &[mcp.key], &one::keys::epoch_hash(&own))),
        ];
        let prefix = one::log::prefix("global");
        let step = one::db::step_name(&prefix, 1);
        let log_keys = one::db::Keys::from([(1, one::kernel::database_key(&log_app, one::log::NAME))]);
        let objects = [
            (one::keys::epoch_name("global", 1, device), one::kernel::seal_to(&mcp.seal, &global).unwrap()),
            (one::keys::app_key_name("global", one::members::APP, 1), one::kernel::seal_app_key(&global, one::members::APP, &log_app)),
            (step.clone(), one::log::seal(&log_keys, &step, &one::entry::pack(&entries.iter().collect::<Vec<_>>())).unwrap()),
            (one::keys::epoch_name(&scope.to_string(), 1, device), one::kernel::seal_to(&mcp.seal, &own).unwrap()),
            (one::keys::app_key_name(&scope.to_string(), "board", 1), one::kernel::seal_app_key(&own, "board", &board)),
        ];
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        for (name, bytes) in objects {
            assert_eq!(agent.put(format!("{sites}/install/{name}")).header("if-none-match", "*").send(&bytes[..]).unwrap().status(), 200, "{name}");
        }
        move |database: &str| one::kernel::database_key(&board, database)
    }

    fn hex_of(key: &[u8; 32]) -> String {
        data_encoding::HEXLOWER.encode(key)
    }

    /// A remote MCP client (ChatGPT, Claude.ai) signs in at the MCP Worker's token endpoint, which makes it a device its
    /// person approves: until they do, it's told how; then its MCP requests reach a node over iroh as that device,
    /// handing the keys they need sealed to that node and bound to the device's key, and nothing else: no token. A
    /// refresh keeps the same device; the old refresh token, used again, gets nothing.
    #[test]
    fn a_remote_mcp_client_is_a_device_that_hands_a_node_its_keys_and_nothing_else() {
        use sha2::Digest;
        let modules = super::serverless_node();
        let node = SecretKey::generate();
        let signing = ed25519_dalek::SigningKey::from_bytes(&node.to_bytes());
        let k = super::Kernel::with_mcp(vec![hex(&node)], modules);
        let at = |p: &str| format!("{}{p}", k.address);
        let (secret, user) = super::invited(&k, &signing, "Andre", None);
        let mut browser = k.browser();
        super::joins_with(&k, &mut browser, &secret);

        // An MCP client with no sign-in is told where to sign in, and finds the MCP Worker's token endpoint there.
        let (status, headers, _) = super::ask("POST", &at("/mcp"), &[], Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" })));
        assert_eq!(status, 401);
        assert!(headers["www-authenticate"].contains(&at("/.well-known/oauth-protected-resource/mcp")), "{headers:?}");
        let (_, resource, _) = browser.get("/.well-known/oauth-protected-resource/mcp");
        assert_eq!(resource["authorization_servers"][0], at("/api/auth"));
        let (_, meta, _) = browser.get("/.well-known/oauth-authorization-server/api/auth");
        assert_eq!((meta["issuer"].as_str(), meta["token_endpoint"].as_str()), (Some(at("/api/auth").as_str()), Some(at("/mcp/token").as_str())), "{meta}");

        // It signs in, as OAuth's code flow does, the person tapping their passkey.
        let back = "https://claude.ai/api/mcp/auth_callback";
        let (status, client) = browser.post("/api/auth/oauth2/register", json!({ "client_name": "Claude", "redirect_uris": [back], "token_endpoint_auth_method": "none", "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"] }));
        assert_eq!(status, 201, "{client}");
        let client = client["client_id"].as_str().unwrap().to_string();
        let verifier = "a code verifier, long enough for PKCE's forty-three";
        let challenge = data_encoding::BASE64URL_NOPAD.encode(&sha2::Sha256::digest(verifier.as_bytes()));
        let (_, _, sign_in) = browser.get(&format!("/api/auth/oauth2/authorize?response_type=code&client_id={client}&redirect_uri={}&scope=openid+profile+offline_access&state=s&code_challenge={challenge}&code_challenge_method=S256&resource={}", super::enc(back), super::enc(&k.address)));
        let query = sign_in.expect("authorizing goes to the sign-in page").split_once('?').unwrap().1.to_string();
        let (status, accepted) = browser.post("/sign-in/accept", json!({ "oauth_query": query, "passkey": true }));
        assert_eq!(status, 200, "{accepted}");
        let code = accepted["url"].as_str().unwrap().split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let token = |form: &[(&str, &str)]| -> (u16, Value) {
            let mut r = agent.post(at("/mcp/token")).send_form(form.iter().copied()).unwrap();
            (r.status().as_u16(), r.body_mut().read_json().unwrap_or(Value::Null))
        };
        // It asks for a token for the install's address, as a sign-in at its nodes: it's given one for the MCP Worker
        // alone, which nodes refuse, as it isn't one of the install's own apps.
        let (status, tokens) = token(&[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", back), ("client_id", &client), ("code_verifier", verifier), ("resource", &k.address)]);
        assert_eq!(status, 200, "{tokens}");
        let access = tokens["access_token"].as_str().unwrap().to_string();
        let claims: Value = serde_json::from_slice(&data_encoding::BASE64URL_NOPAD.decode(access.split('.').nth(1).unwrap().as_bytes()).unwrap()).unwrap();
        let aud: Vec<Value> = claims["aud"].as_array().cloned().unwrap_or_else(|| vec![claims["aud"].clone()]);
        assert!(aud.contains(&json!(at("/mcp"))) && !aud.contains(&json!(k.address)), "{claims}");
        let mcp = |access: &str, body: Value| super::post(&at("/mcp"), &[("authorization", &format!("Bearer {access}")), ("accept", "application/json, text/event-stream")], body);

        // Its device asked to join, and until a device of Andre's approves it, it says how, by its code.
        let (_, asked, _) = browser.get("/api/auth/devices/requests");
        let request = &asked["requests"][0];
        assert_eq!(request["name"], "Claude (claude.ai)", "its name is shown with where its sign-in goes: {asked}");
        let device = request["key"].as_str().unwrap().to_string();
        let seal = request["seal"].as_str().unwrap().to_string();
        let join = one::keys::join_code(&one::keys::Endpoint { key: one::keys::unhex(&device).unwrap(), seal: one::keys::unhex(&seal).unwrap() });
        let (status, said) = mcp(&access, json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "board_read", "arguments": {} } }));
        let text = said["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
        assert!(status == 200 && text.contains("Approve Claude") && text.contains(&format!("{} {}", &join[..4], &join[4..])), "{said}");

        // Approved: Andre's device lets it in at the relay, lists it, and seals it his scopes' keys. Sam's box is
        // compute too, but runs none of Andre's calls.
        let address = k.address.replace("localhost", "127.0.0.1");
        assert_eq!(tell(&address, &node, json!({ "add": device, "role": "client" })), 204);
        let sams_box = SecretKey::generate();
        assert_eq!(tell(&address, &node, json!({ "add": hex(&sams_box), "role": "compute" })), 204);
        let key_of = andres_install(&k.sites, &node, &sams_box, &user, &device, &seal);
        let scope = format!("user:{user}");
        let call = |id: u64, tool: &str| json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": tool, "arguments": {} } });
        let text = |out: &Value| out["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Only Sam's box is on: it isn't asked, as it doesn't run Andre's calls, so it's handed nothing.
            let says = std::sync::Arc::new(std::sync::Mutex::new(Says::Honest));
            let (sams_ep, sams_seen) = stand_in_node(&address, &sams_box, scope.clone(), says.clone()).await;
            let (status, out) = tokio::task::block_in_place(|| mcp(&access, call(3, "board_read")));
            assert!(status == 200 && text(&out).contains("None of your devices is on"), "{out}");
            assert!(sams_seen.lock().unwrap().0.is_empty(), "a node that doesn't run Andre's calls was asked: {:?}", sams_seen.lock().unwrap().0);

            // Andre's node is on, but its 428 asks for keys sealed to another endpoint (Sam's box), or of a scope it
            // doesn't run for him: it's handed nothing.
            let (ep, seen) = stand_in_node(&address, &node, scope.clone(), says.clone()).await;
            *says.lock().unwrap() = Says::SealedTo(one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&sams_box.to_bytes())));
            let (status, out) = tokio::task::block_in_place(|| mcp(&access, call(4, "board_read")));
            assert!(status == 200 && text(&out).contains("sealed to another"), "{out}");
            *says.lock().unwrap() = Says::Scope(format!("user:{}", one::entry::new_id()));
            let (status, out) = tokio::task::block_in_place(|| mcp(&access, call(5, "board_read")));
            assert!(status == 200 && text(&out).contains("doesn't run for you"), "{out}");
            assert!(seen.lock().unwrap().1.is_empty(), "a node lying in its 428 was handed keys");
            assert!(sams_seen.lock().unwrap().0.is_empty() && sams_seen.lock().unwrap().1.is_empty());
            sams_ep.close().await;

            // Honest, it's handed the board's keys, opened from the bucket by the device itself.
            *says.lock().unwrap() = Says::Honest;
            let asked = tokio::task::block_in_place(|| {
                let first = std::time::Instant::now();
                let (status, out) = mcp(&access, call(6, "board_read"));
                assert_eq!((status, text(&out).as_str()), (200, "the board"), "{out}");
                let cold = first.elapsed();
                let mut warm = Vec::new();
                for id in 7..17 {
                    let t = std::time::Instant::now();
                    let (status, _) = mcp(&access, call(id, "board_read"));
                    assert_eq!(status, 200);
                    warm.push(t.elapsed());
                }
                warm.sort();
                eprintln!("a remote MCP call through the MCP Worker: first {cold:?} (428, opening its keys, again), then {:?} median, {:?} at most", warm[warm.len() / 2], warm[warm.len() - 1]);
                // Another tool's call hands only what that tool needs.
                let (status, out) = mcp(&access, call(17, "notes_read"));
                assert_eq!((status, text(&out).as_str()), (200, "the board"), "{out}");
                seen.lock().unwrap().clone()
            });
            let (requests, handed) = asked;
            // Every request came over iroh from the device's own key, with no token or cookie: only MCP's headers and
            // the keys, which were sealed to this node, for that key, until the access token expires.
            assert!(requests.iter().all(|r| r.by == device), "{requests:?}");
            for r in &requests {
                for h in ["authorization", "cookie", "x-one-local-token", "forwarded"] {
                    assert!(!r.headers.contains_key(h), "{h} reached the node: {r:?}");
                }
            }
            let exp = super::tokens_exp(&access);
            fn databases(h: &one::keys::Handoff) -> Vec<(&str, u32, &str, &str, [u8; 32])> {
                h.keys.iter().map(|k| (k.scope.as_str(), k.epoch, k.app.as_str(), k.database.as_str(), k.key)).collect()
            }
            assert_eq!(handed.len(), 2, "keys were handed once for each tool: {requests:?}");
            for h in &handed {
                assert_eq!((h.for_.as_str(), h.until), (device.as_str(), exp * 1000));
            }
            assert_eq!(databases(&handed[0]), vec![(scope.as_str(), 1, "board", "main", key_of("main"))]);
            assert_eq!(databases(&handed[1]), vec![(scope.as_str(), 1, "board", "notes", key_of("notes"))], "a tool's call hands only what it needs");
            let mcps = requests.iter().filter(|r| r.line.starts_with("POST /mcp ")).count();
            assert_eq!(mcps, 16, "each MCP request reached the node once, and each tool's first once more once it had the keys: {requests:?}");

            // A refresh keeps the same device; the old refresh token, used again, gets nothing.
            let refresh = tokens["refresh_token"].as_str().unwrap().to_string();
            let (status, again) = tokio::task::block_in_place(|| token(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &client)]));
            assert_eq!(status, 200, "{again}");
            let fresh = again["access_token"].as_str().unwrap().to_string();
            let (status, out) = tokio::task::block_in_place(|| mcp(&fresh, call(20, "board_read")));
            assert_eq!((status, text(&out).as_str()), (200, "the board"), "{out}");
            assert!(seen.lock().unwrap().0.iter().all(|r| r.by == device), "a refresh made a new device");
            let (status, _) = tokio::task::block_in_place(|| token(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &client)]));
            assert_ne!(status, 200, "a refresh token was used twice");
            ep.close().await;
        });
    }

    #[test]
    fn the_relay_carries_only_the_installs_endpoints() {
        let first = SecretKey::generate();
        let k = super::Kernel::for_devices(vec![hex(&first)]);
        let address = k.address.replace("localhost", "127.0.0.1");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // The install's first device is on the list, an admin's, and a stranger isn't.
            let a = online(&address, &first, None).await.expect("the install's first device isn't let in");
            let stranger = SecretKey::generate();
            assert!(online(&address, &stranger, None).await.is_none(), "a stranger's key reached the relay");

            // A device it adds is let in, and the two reach each other through it.
            let phone = SecretKey::generate();
            assert_eq!(tell(&address, &first, json!({ "add": hex(&phone), "role": "client" })), 204);
            let b = online(&address, &phone, None).await.expect("a key the first device added isn't let in");
            assert!(reaches(&address, &a, &b).await, "two of the install's devices don't reach each other");

            // Only an admin's device adds an admin's, changes a key that's in, or takes out a key another let in; the
            // last admin stays; a device takes itself out.
            assert_eq!(tell(&address, &phone, json!({ "add": hex(&stranger), "role": "client", "admin": true })), 403);
            assert_eq!(tell(&address, &phone, json!({ "add": hex(&first), "role": "client" })), 403, "a device made an admin's key its own");
            assert_eq!(tell(&address, &phone, json!({ "remove": hex(&first) })), 403);
            assert_eq!(tell(&address, &first, json!({ "remove": hex(&first) })), 403, "the last admin was taken out");
            assert_eq!(tell(&address, &first, json!({ "add": hex(&first), "role": "client" })), 403, "the last admin was made a member");
            assert_eq!(tell(&address, &stranger, json!({ "add": hex(&stranger), "role": "client" })), 403, "a stranger told the relay");
            let request = one::keys::signed_request(&ed25519_dalek::SigningKey::from_bytes(&first.to_bytes()), "one/relay", json!({ "add": hex(&stranger), "role": "client" }), super::now());
            let mut forged = request.clone();
            forged["add"] = json!(hex(&SecretKey::generate()));
            assert_eq!(super::post(&format!("{address}/relay/tell"), &[], forged).0, 403, "a changed tell was taken");
            assert_eq!(super::post(&format!("{address}/relay/tell"), &[], request.clone()).0, 204);
            assert_eq!(super::post(&format!("{address}/relay/tell"), &[], request).0, 403, "a tell was taken twice");
            let big = one::keys::signed_request(&ed25519_dalek::SigningKey::from_bytes(&first.to_bytes()), "one/relay", json!({ "add": hex(&phone), "role": "client", "pad": "x".repeat(5000) }), super::now());
            assert_eq!(super::post(&format!("{address}/relay/tell"), &[], big).0, 403, "a big tell was read");
            assert_eq!(tell(&address, &stranger, json!({ "remove": hex(&stranger) })), 204);
            assert!(online(&address, &stranger, None).await.is_none(), "a key taken out reached the relay");

            // A client adds only clients: a compute or a service only an admin's device or a compute adds, and a key's
            // role, once it's in, only an admin's changes.
            for role in ["compute", "service"] {
                assert_eq!(tell(&address, &phone, json!({ "add": hex(&SecretKey::generate()), "role": role })), 403, "a client added a {role}");
            }
            let machine = SecretKey::generate();
            assert_eq!(tell(&address, &first, json!({ "add": hex(&machine), "role": "compute" })), 204);
            assert_eq!(tell(&address, &machine, json!({ "add": hex(&SecretKey::generate()), "role": "compute" })), 204, "a compute didn't add the computer it paired");
            assert_eq!(tell(&address, &machine, json!({ "add": hex(&SecretKey::generate()), "role": "service" })), 204, "a compute didn't add its person's service");
            assert_eq!(tell(&address, &phone, json!({ "add": hex(&machine), "role": "client" })), 403, "a client changed a compute's role");
            assert_eq!(tell(&address, &machine, json!({ "add": hex(&phone), "role": "compute" })), 403, "a compute made a client a compute");

            // A pairing code lets in one computer, which reaches only the device that made it until it's added.
            let code = ticket();
            assert_eq!(tell(&address, &first, json!({ "pairing": { "hash": sha256(&code), "until": super::now() + 600_000 } })), 204);
            let joining = SecretKey::generate();
            let c = online(&address, &joining, Some(&code)).await.expect("a pairing code's computer isn't let in");
            assert!(online(&address, &SecretKey::generate(), Some(&code)).await.is_none(), "a pairing code let in a second computer");
            assert!(reaches(&address, &c, &a).await, "a pairing code's computer doesn't reach the device that made it");
            assert!(!reaches(&address, &c, &b).await, "a pairing code's computer reached another device");
            // Ended, its computer is out.
            assert_eq!(tell(&address, &phone, json!({ "unpair": sha256(&code) })), 403, "a device ended another's pairing code");
            assert_eq!(tell(&address, &first, json!({ "unpair": sha256(&code) })), 204);
            assert!(!reaches(&address, &c, &a).await, "an ended pairing code's computer still reached the relay");
            c.close().await;
            assert!(online(&address, &joining, Some(&code)).await.is_none(), "an ended pairing code let its computer in again");
            // Expired, too, while it's connected.
            let code = ticket();
            assert_eq!(tell(&address, &first, json!({ "pairing": { "hash": sha256(&code), "until": super::now() + 2_000 } })), 204);
            let d = online(&address, &SecretKey::generate(), Some(&code)).await.expect("a pairing code's computer isn't let in");
            tokio::time::sleep(Duration::from_millis(2_500)).await;
            assert!(!reaches(&address, &d, &a).await, "an expired pairing code's computer still reached the relay");
            d.close().await;

            // Taking a device out takes out the devices it let in.
            let laptop = SecretKey::generate();
            assert_eq!(tell(&address, &phone, json!({ "add": hex(&laptop), "role": "client" })), 204);
            assert_eq!(tell(&address, &first, json!({ "remove": hex(&phone) })), 204);
            assert!(online(&address, &laptop, None).await.is_none(), "a key a removed device let in still reached the relay");
            assert!(!reaches(&address, &b, &a).await, "a removed device's socket still reached the relay");
            for ep in [a, b] {
                ep.close().await;
            }
        });
        // iroh's probes, which a browser's endpoint reads too.
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let ping = agent.get(format!("{address}/ping")).call().unwrap();
        assert_eq!((ping.status().as_u16(), ping.headers().get("access-control-allow-origin").map(|v| v.to_str().unwrap())), (200, Some("*")));
        let portal = agent.get(format!("{address}/generate_204")).header("x-iroh-challenge", "abc_1.2").call().unwrap();
        assert_eq!((portal.status().as_u16(), portal.headers().get("x-iroh-response").map(|v| v.to_str().unwrap())), (204, Some("response abc_1.2")));
    }

    /// A browser's endpoint is a key of its session's, which its device vouches for (keys::certify): the relay lets it
    /// in as that device, only for the key the certificate names, until its `until`, and while the device is listed.
    #[test]
    fn a_session_endpoint_gets_in_only_as_its_device_vouches() {
        let first = SecretKey::generate();
        let k = super::Kernel::for_devices(vec![hex(&first)]);
        let address = k.address.replace("localhost", "127.0.0.1");
        let phone = SecretKey::generate();
        let signer = |k: &SecretKey| ed25519_dalek::SigningKey::from_bytes(&k.to_bytes());
        let certify = |by: &SecretKey, endpoint: &SecretKey, until: u64| one::keys::certify(&signer(by), endpoint.public().as_bytes(), until, super::now());
        let computes = |body: Value| super::post(&format!("{address}/relay/computes"), &[], body);
        // What a session asks the computes with: a request its own key signs, with its certificate.
        let ask = |session: &SecretKey, cert: &str| one::keys::signed_request(&signer(session), "one/relay", json!({ "computes": true, "certificate": cert }), super::now());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let a = online(&address, &first, None).await.expect("the install's first device isn't let in");
            assert_eq!(tell(&address, &first, json!({ "add": hex(&phone), "role": "client" })), 204);
            assert_eq!(tell(&address, &first, json!({ "add": hex(&SecretKey::generate()) })), 403, "an add that doesn't say its role");

            let session = SecretKey::generate();
            let cert = certify(&phone, &session, super::now() + 60_000);
            let s = online(&address, &session, Some(&cert)).await.expect("a session its device vouched for isn't let in");
            assert!(reaches(&address, &s, &a).await, "a session's endpoint doesn't reach the install's devices");
            // Its first nodes to ask: the computes, for it or a device on the list, and no one else.
            let asked = one::keys::signed_request(&signer(&phone), "one/relay", json!({ "computes": true }), super::now());
            let by_session = ask(&session, &cert);
            for body in [by_session.clone(), asked.clone()] {
                let (status, said) = computes(body);
                assert_eq!((status, said), (200, json!({ "computes": [hex(&first)] })));
            }
            // Each request is taken once; a certificate alone, or one signed by a key it doesn't name, gets nothing; a
            // clock that's off isn't told it's out (403), so it keeps the nodes it knows.
            for replayed in [by_session, asked] {
                assert_eq!(computes(replayed).0, 400, "a computes request was taken twice");
            }
            assert_eq!(computes(json!({ "certificate": cert })).0, 400, "a certificate alone was told the computes");
            assert_eq!(computes(ask(&SecretKey::generate(), &cert)).0, 400, "someone else's key asked with a session's certificate");
            let late = one::keys::signed_request(&signer(&session), "one/relay", json!({ "computes": true, "certificate": cert }), super::now() - 120_000);
            assert_eq!(computes(late).0, 400);
            let stranger = SecretKey::generate();
            assert_eq!(computes(one::keys::signed_request(&signer(&stranger), "one/relay", json!({ "computes": true }), super::now())).0, 403, "a stranger was told the computes");
            assert_eq!(computes(ask(&session, &certify(&stranger, &session, super::now() + 60_000))).0, 403);

            // Only a client vouches for a session: a compute or a service, admin or not, doesn't.
            let service = SecretKey::generate();
            assert_eq!(tell(&address, &first, json!({ "add": hex(&service), "role": "service" })), 204);
            for by in [&first, &service] {
                let other = SecretKey::generate();
                let vouched = certify(by, &other, super::now() + 60_000);
                assert!(online(&address, &other, Some(&vouched)).await.is_none(), "a session a compute or service vouched for got in");
                assert_eq!(computes(ask(&other, &vouched)).0, 403, "a session a compute or service vouched for was told the computes");
            }

            // With its key's proof it's taken in at once, with no challenge, once: shown again, stale, by another key or
            // for another's certificate, it's challenged as any key is (and a stranger's then refused).
            let proof = |key: &SecretKey, at: u64| {
                let nonce = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>());
                data_encoding::BASE64URL_NOPAD.encode(&one::keys::canonical(&one::keys::signed_request(&signer(key), "one/socket", json!({ "nonce": nonce }), at)))
            };
            let shown = format!("{cert}.{}", proof(&session, super::now()));
            assert_eq!(first_frame(&address, &shown), CONFIRMED, "a session with its proof was challenged");
            assert_eq!(first_frame(&address, &shown), CHALLENGE, "a proof was taken twice");
            assert_eq!(first_frame(&address, &format!("{cert}.{}", proof(&session, super::now() - 20_000))), CHALLENGE, "a proof older than ten seconds was taken");
            assert_eq!(first_frame(&address, &format!("{cert}.{}", proof(&SecretKey::generate(), super::now()))), CHALLENGE, "another key's proof was taken");
            let theirs = SecretKey::generate();
            let foreign = format!("{}.{}", certify(&stranger, &theirs, super::now() + 60_000), proof(&theirs, super::now()));
            assert_eq!(first_frame(&address, &foreign), CHALLENGE, "a stranger's session was taken in");
            assert!(online(&address, &theirs, Some(&foreign)).await.is_none(), "a stranger's session got in with its proof");
            // Its own session, as one key's newest socket is the one it's reached at, and `s` is still connected.
            let fresh = SecretKey::generate();
            let shown = format!("{}.{}", certify(&phone, &fresh, super::now() + 60_000), proof(&fresh, super::now()));
            let proved = online(&address, &fresh, Some(&shown)).await.expect("a session with its proof isn't let in");
            assert!(reaches(&address, &proved, &a).await, "a session taken in with its proof doesn't reach the install's devices");
            proved.close().await;

            // Nothing for another endpoint, by a key not on the list, or once it has ended.
            assert!(online(&address, &SecretKey::generate(), Some(&cert)).await.is_none(), "a certificate let in an endpoint it doesn't name");
            assert!(online(&address, &SecretKey::generate(), Some(&certify(&stranger, &stranger, super::now() + 60_000))).await.is_none(), "a stranger vouched for itself");
            let short = SecretKey::generate();
            let brief = online(&address, &short, Some(&certify(&phone, &short, super::now() + 2_000))).await.expect("a short session isn't let in");
            tokio::time::sleep(Duration::from_millis(2_500)).await;
            assert!(!reaches(&address, &brief, &a).await, "an ended session still reached the relay");
            assert!(online(&address, &short, Some(&certify(&phone, &short, super::now() - 1))).await.is_none(), "an ended session was let in");
            brief.close().await;

            // Its device taken out, it's out too.
            assert_eq!(tell(&address, &first, json!({ "remove": hex(&phone) })), 204);
            assert!(!reaches(&address, &s, &a).await, "a removed device's session still reached the relay");
            s.close().await;
            assert!(online(&address, &session, Some(&cert)).await.is_none(), "a removed device's session got in again");
            assert_eq!(computes(ask(&session, &cert)).0, 403, "a removed device's session was told the computes");
            a.close().await;
        });
    }
}
