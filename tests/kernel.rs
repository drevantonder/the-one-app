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
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("kernel");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    let screen = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("src/screen");
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
}

impl Kernel {
    /// The kernel, its relay taking only `devices` (public keys, hex) and those they admit.
    pub(crate) fn for_devices(devices: Vec<String>) -> Kernel {
        Kernel::start_as(false, devices)
    }

    /// As `for_devices`, for an install whose people sign in with Google at test.example, the stand-in sites as
    /// Google.
    fn google(devices: Vec<String>) -> Kernel {
        Kernel::start_as(true, devices)
    }

    fn start_as(google: bool, devices: Vec<String>) -> Kernel {
        let gateway = port();
        let mut settings = kernel::Settings::new("Test", &format!("http://localhost:{gateway}"));
        settings.ports = [port(), gateway];
        settings.auth_url = format!("http://127.0.0.1:{}", settings.ports[0]);
        // The install's first devices, its admin's machines that run its calls.
        settings.devices = devices.into_iter().map(|key| json!({ "key": key, "role": "compute", "admin": true })).collect();
        settings.bucket = Some(json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }));
        let modules = kernel::SERVICES.iter().map(|(_, entry, _)| module(entry)).collect();
        let sites = port();
        let web = crate::temp("web");
        std::fs::create_dir_all(web.join("screen")).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html><title>The web client</title>").unwrap();
        std::fs::write(web.join("frame.html"), "<!doctype html><title>A screen's frame</title>").unwrap();
        std::fs::write(web.join("screen/kit.css"), include_str!("../src/screen/kit.css")).unwrap();
        for (path, bytes) in kernel::precompressed(&[("screen/kit.css".into(), include_bytes!("../src/screen/kit.css").to_vec())]).unwrap() {
            std::fs::write(web.join(path), bytes).unwrap();
        }
        let mut served = kernel::served_with(&settings, modules, &web, None);
        for s in served.iter_mut().filter(|s| s.name == "signer" || s.name == "mcp") {
            s.outbound = Some("sites".into());
        }
        if google {
            served[0].outbound = Some("sites".into());
            served[0].bindings.extend([("SIGN_IN", "google"), ("JOIN_DOMAIN", "test.example"), ("GOOGLE_CLIENT_ID", "a client id"), ("GOOGLE_CLIENT_SECRET", "a client secret")].map(|(n, v)| (n.into(), v.into())));
        }
        served.push(Served { name: "sites".into(), module: module("test/sites.js"), modules: vec![], class: None, listen: Some(format!("127.0.0.1:{sites}")), bindings: vec![], assets: None, outbound: None });
        let running = run(served);
        Kernel { _running: running, address: settings.address }
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

/// An invite `by` an endpoint for `name`, as its device makes one: → its secret and the user id it's for.
fn invited(k: &Kernel, by: &ed25519_dalek::SigningKey, name: &str, email: Option<&str>) -> (String, String) {
    let invite = kernel::invite(&k.address, by, name, None, email).unwrap();
    (invite.link.split_once('#').unwrap().1.to_string(), invite.user)
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
    /// When to sign what's told the relay: now, but each at a millisecond of its own, as the relay takes each `(by, at)`
    /// once, and a test tells it faster than a person would.
    fn at() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
        static LAST: AtomicU64 = AtomicU64::new(0);
        loop {
            let last = LAST.load(SeqCst);
            let at = super::now().max(last + 1);
            if LAST.compare_exchange(last, at, SeqCst, SeqCst).is_ok() {
                return at;
            }
        }
    }

    fn tell(address: &str, key: &SecretKey, fields: Value) -> u16 {
        let key = ed25519_dalek::SigningKey::from_bytes(&key.to_bytes());
        let request = one::keys::signed_request(&key, "one/relay", fields, at());
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
            let request = one::keys::signed_request(&ed25519_dalek::SigningKey::from_bytes(&first.to_bytes()), "one/relay", json!({ "add": hex(&stranger), "role": "client" }), at());
            let mut forged = request.clone();
            forged["add"] = json!(hex(&SecretKey::generate()));
            assert_eq!(super::post(&format!("{address}/relay/tell"), &[], forged).0, 403, "a changed tell was taken");
            assert_eq!(super::post(&format!("{address}/relay/tell"), &[], request.clone()).0, 204);
            assert_eq!(super::post(&format!("{address}/relay/tell"), &[], request).0, 403, "a tell was taken twice");
            let big = one::keys::signed_request(&ed25519_dalek::SigningKey::from_bytes(&first.to_bytes()), "one/relay", json!({ "add": hex(&phone), "role": "client", "pad": "x".repeat(5000) }), at());
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
            // A device that let a key in adds it again as it is (approving it again, to finish): done, nothing changed.
            let laptop = SecretKey::generate();
            assert_eq!(tell(&address, &phone, json!({ "add": hex(&laptop), "role": "client" })), 204);
            assert_eq!(tell(&address, &phone, json!({ "add": hex(&laptop), "role": "client" })), 204, "a device approved again wasn't taken");
            assert_eq!(tell(&address, &machine, json!({ "add": hex(&laptop), "role": "client" })), 403, "another device changed who let a key in");

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
        let ask = |session: &SecretKey, cert: &str| one::keys::signed_request(&signer(session), "one/relay", json!({ "computes": true, "certificate": cert }), at());
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
            let asked = one::keys::signed_request(&signer(&phone), "one/relay", json!({ "computes": true }), at());
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
            assert_eq!(computes(one::keys::signed_request(&signer(&stranger), "one/relay", json!({ "computes": true }), at())).0, 403, "a stranger was told the computes");
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
