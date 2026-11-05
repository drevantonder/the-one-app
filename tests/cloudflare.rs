//! The install's Workers (`cloudflare/`) running in the pinned workerd, as nodes and the auth service use them.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE64URL_NOPAD as B64U;
use one::cloudflare::{self, Served};
use serde_json::{Value, json};

/// The Workers' files as they are in this checkout, tests and all, with the screens' design kit where build.rs puts it.
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
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("cloudflare");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    let screen = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("src/screen");
    let mut kit = Vec::new();
    walk(&screen, &screen, &mut kit);
    let kit = kit.into_iter().filter_map(|(path, bytes)| match path.strip_prefix("ui/") {
        Some(ui) => ui.strip_prefix("dist/").map(|built| (format!("ui/{built}"), bytes)),
        None => Some((path, bytes)),
    });
    out.extend(kit.map(|(path, bytes)| (format!("web/screen/{path}"), bytes)));
    out
}

fn module(entry: &str) -> Vec<u8> {
    cloudflare::module_from(files(), entry).unwrap()
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

/// How long what's meant to end has to be let in or used first, before the test waits for it to end: seconds on a
/// machine that's busy, which a quiet one doesn't use.
const LASTS: u64 = 4_000;

/// Waits until `ms` (a time as `now` gives it) has passed, as the ending of what lasted until then is seen after it.
async fn past(ms: u64) {
    while now() <= ms + 250 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// A workerd running the Workers, stopped when dropped.
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
    let config = cloudflare::write_config(&dir, &services).unwrap();
    let child = cloudflare::start(&config, listening).unwrap();
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
fn the_workers_own_tests_pass() {
    let dir = crate::temp("workers-tests");
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
    assert!(out.status.success(), "the Workers' tests failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
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

    fn delete(&mut self, path: &str) -> (u16, Value) {
        let (status, out, _) = self.send("DELETE", path, None, None);
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

/// The real Workers, as an install runs them: people reach them at its address, the gateway's.
pub(crate) struct Workers {
    _running: Running,
    pub(crate) address: String,
    /// The stand-in sites the Workers' requests go to (test/sites.js).
    pub(crate) sites: String,
}

impl Workers {
    /// The Workers, their relay taking only `devices` (public keys, hex) and those they admit.
    pub(crate) fn for_devices(devices: Vec<String>) -> Workers {
        Workers::start_as(false, devices)
    }

    /// As `for_devices`, for an install whose people sign in with Google at test.example, the stand-in sites as
    /// Google.
    fn google(devices: Vec<String>) -> Workers {
        Workers::start_as(true, devices)
    }

    fn start_as(google: bool, devices: Vec<String>) -> Workers {
        let gateway = port();
        let mut settings = cloudflare::Settings::new("Test", &format!("http://localhost:{gateway}"));
        settings.ports = [port(), gateway];
        settings.auth_url = format!("http://127.0.0.1:{}", settings.ports[0]);
        // The install's first devices, its admin's machines that run its calls.
        settings.devices = devices.into_iter().map(|key| json!({ "key": key, "role": "compute", "admin": true })).collect();
        settings.genesis = Some(data_encoding::HEXLOWER.encode(&GENESIS));
        settings.bucket = Some(json!({ "endpoint": "https://s3.test", "bucket": "install", "region": "auto", "key_id": "id", "secret": "bucket secret" }));
        let modules = cloudflare::WORKERS.iter().map(|(_, entry, _)| module(entry)).collect();
        let sites = port();
        let web = crate::temp("web");
        std::fs::create_dir_all(web.join("screen")).unwrap();
        std::fs::write(web.join("index.html"), "<!doctype html><title>The web client</title>").unwrap();
        std::fs::write(web.join("frame.html"), "<!doctype html><title>A screen's frame</title>").unwrap();
        std::fs::write(web.join("screen/kit.css"), include_str!("../src/screen/kit.css")).unwrap();
        for (path, bytes) in cloudflare::precompressed(&[("screen/kit.css".into(), include_bytes!("../src/screen/kit.css").to_vec())]).unwrap() {
            std::fs::write(web.join(path), bytes).unwrap();
        }
        let mut served = cloudflare::served_with(&settings, modules, &web, None);
        for s in served.iter_mut().filter(|s| ["signer", "mcp", "push"].contains(&s.name.as_str())) {
            s.outbound = Some("sites".into());
        }
        if google {
            served[0].outbound = Some("sites".into());
            served[0].bindings.extend([("SIGN_IN", "google"), ("JOIN_DOMAIN", "test.example"), ("GOOGLE_CLIENT_ID", "a client id"), ("GOOGLE_CLIENT_SECRET", "a client secret")].map(|(n, v)| (n.into(), v.into())));
        }
        served.push(Served { name: "sites".into(), module: module("test/sites.js"), modules: vec![], class: None, listen: Some(format!("127.0.0.1:{sites}")), bindings: vec![], assets: None, outbound: None });
        let running = run(served);
        Workers { _running: running, address: settings.address, sites: format!("http://127.0.0.1:{sites}") }
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
fn invited(k: &Workers, by: &ed25519_dalek::SigningKey, name: &str, email: Option<&str>) -> (String, String) {
    let invite = cloudflare::invite(&k.address, by, name, None, email).unwrap();
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
    let k = Workers::google(vec![admin_hex]);
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

/// A sign-in for a device names it, and auth passes what it showed on with that device's request to join, unchanged,
/// for the person's devices to check themselves (src/sign_in_proof.rs): a passkey's assertion signs a challenge naming
/// the device and when, which checks; a sign-in for no device, or one for another device, lets nothing in; and with
/// Google, the ID token asked for names the device.
#[test]
fn a_sign_in_for_a_device_names_it_and_auth_passes_it_on() {
    use one::sign_in_proof::{self, Account, Proof};
    let (admin, admin_hex) = endpoint();
    let k = Workers::for_devices(vec![admin_hex.clone()]);
    let (secret, andre) = invited(&k, &admin, "Andre", None);
    let passkey = k.passkey();
    let mut first = k.browser();
    let (status, options, _) = first.get(&format!("/api/auth/passkey/generate-register-options?context={secret}"));
    assert_eq!(status, 200, "{options}");
    let (status, out) = first.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true }));
    assert_eq!(status, 200, "{out}");
    let device = one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()));
    let join = format!("{}{}", data_encoding::HEXLOWER.encode(&device.key), data_encoding::HEXLOWER.encode(&device.seal));
    // Signing in for the device: the passkey signs a challenge naming it.
    let mut new = k.browser();
    let (status, options, _) = new.get(&format!("/api/auth/passkey/generate-authenticate-options?join={join}"));
    assert_eq!(status, 200, "{options}");
    let challenge = B64U.decode(options["challenge"].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(challenge[..challenge.len() - 8], sign_in_proof::challenge(&GENESIS, &device, 0)[..challenge.len() - 8], "the challenge names the device in this install");
    assert_eq!(new.post("/api/auth/passkey/verify-authentication", json!({ "response": passkey.get(&options) })).0, 200);
    assert_eq!(signed_in_as(&mut new).as_deref(), Some(andre.as_str()));
    let asked = |b: &mut Browser, d: &one::keys::Endpoint| {
        let (status, out) = b.post("/api/auth/devices/requests", json!({ "key": data_encoding::HEXLOWER.encode(&d.key), "seal": data_encoding::HEXLOWER.encode(&d.seal), "name": "MacBook Pro" }));
        assert_eq!(status, 200, "{out}");
        let (_, out, _) = b.get(&format!("/api/auth/devices/requests/{}", data_encoding::HEXLOWER.encode(&d.key)));
        out["sign_in"].clone()
    };
    let proof: Proof = serde_json::from_value(asked(&mut new, &device)).expect("its request carries the assertion");
    let install = sign_in_proof::Install::new(&k.address, GENESIS, None);
    let account = sign_in_proof::check(&proof, &device, &install, None, now()).expect("the person's devices check it");
    let Account::Passkey { public_key, .. } = &account else { panic!("{account:?}") };
    let checked = sign_in_proof::vouches(&proof, &device, &install, None, std::slice::from_ref(&account), now());
    assert!(checked.is_ok() && !public_key.is_empty(), "and let it in by it, once it's recorded: {checked:?}");
    let elsewhere = sign_in_proof::Install::new(&k.address, [6; 32], None);
    assert!(sign_in_proof::vouches(&proof, &device, &elsewhere, None, std::slice::from_ref(&account), now()).is_err(), "but not in another install");
    // Turned down, its request goes, and what its sign-in showed with it: asking again, it carries nothing.
    let key = data_encoding::HEXLOWER.encode(&device.key);
    assert_eq!(new.delete(&format!("/api/auth/devices/requests/{key}")).0, 200);
    assert_eq!(asked(&mut new, &device), Value::Null, "a refused device's sign-in is forgotten");
    // Another device asking as them carries none, nor does one signed in for no device.
    assert_eq!(asked(&mut new, &one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()))), Value::Null);
    let mut plain = k.browser();
    assert_eq!(signs_in_with_passkey(&mut plain, &passkey), 200);
    assert_eq!(asked(&mut plain, &one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()))), Value::Null);

    // With Google, the ID token it asks for names the device.
    let g = Workers::google(vec![admin_hex]);
    let mut browser = g.browser();
    let (status, client) = browser.post("/api/auth/oauth2/register", json!({ "client_name": "Chrome on Mac", "redirect_uris": [format!("{}/callback", g.address)], "token_endpoint_auth_method": "none", "grant_types": ["authorization_code"], "response_types": ["code"] }));
    assert_eq!(status, 201, "{client}");
    let query = format!("response_type=code&client_id={}&redirect_uri={}/callback&scope=openid&state=s&join={join}", client["client_id"].as_str().unwrap(), g.address);
    assert_eq!(browser.post("/sign-in/google", json!({ "oauth_query": query })).0, 200);
    let (status, out) = browser.post("/api/auth/sign-in/social", json!({ "provider": "google", "callbackURL": "/", "errorCallbackURL": "/" }));
    assert_eq!(status, 200, "{out}");
    assert_eq!(url_query(out["url"].as_str().unwrap()).get("nonce"), Some(&sign_in_proof::nonce(&GENESIS, &device)), "Google is asked for an ID token naming the device in this install");
}

/// A device is let in by its sign-in only when it signed in itself: one whose sign-in was finished from a link opened
/// elsewhere (the CLI's sign-in code page, or a site that sent the person a link) could have been started by whoever
/// sent it, who'd hold the token it gave. Auth passes that one's proof on as `by_link`, which records what it signed in
/// with when the person taps Approve but lets nothing in by itself; a release before it reads it as nothing.
#[test]
fn a_sign_in_finished_from_a_link_lets_its_device_in_only_by_a_tap() {
    use one::sign_in_proof::{self, Proof};
    use sha2::Digest;
    let (admin, admin_hex) = endpoint();
    let k = Workers::for_devices(vec![admin_hex]);
    let (secret, _) = invited(&k, &admin, "Andre", None);
    let passkey = k.passkey();
    let mut first = k.browser();
    let (_, options, _) = first.get(&format!("/api/auth/passkey/generate-register-options?context={secret}"));
    assert_eq!(first.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true })).0, 200);
    let install = sign_in_proof::Install::new(&k.address, GENESIS, None);
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    // Signs in with the passkey for a new device, through a client coming back to `back`, and asks to join with the
    // token it gave: → what its request carries.
    let carried = |back: &str| -> Value {
        let device = one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()));
        let (key, join) = (data_encoding::HEXLOWER.encode(&device.key), format!("{}{}", data_encoding::HEXLOWER.encode(&device.key), data_encoding::HEXLOWER.encode(&device.seal)));
        let mut b = k.browser();
        let redirect = format!("{}{back}", k.address);
        let (status, client) = b.post("/api/auth/oauth2/register", json!({ "client_name": "A client", "redirect_uris": [redirect], "token_endpoint_auth_method": "none", "grant_types": ["authorization_code"], "response_types": ["code"] }));
        assert_eq!(status, 201, "{client}");
        let client = client["client_id"].as_str().unwrap().to_string();
        let verifier = B64U.encode(&rand::random::<[u8; 32]>());
        let challenge = B64U.encode(&sha2::Sha256::digest(verifier.as_bytes()));
        let query = format!("response_type=code&client_id={client}&redirect_uri={}&scope=openid&state={}&code_challenge={challenge}&code_challenge_method=S256&join={join}", enc(&redirect), "s".repeat(32));
        let (_, _, page) = b.get(&format!("/api/auth/oauth2/authorize?{query}"));
        let oauth_query = page.expect("the authorization goes to the sign-in page").split_once('?').unwrap().1.to_string();
        let (_, options, _) = b.get(&format!("/api/auth/passkey/generate-authenticate-options?join={join}"));
        assert_eq!(b.post("/api/auth/passkey/verify-authentication", json!({ "response": passkey.get(&options) })).0, 200);
        let (status, out) = b.post("/sign-in/accept", json!({ "oauth_query": oauth_query, "passkey": true }));
        assert_eq!(status, 200, "{out}");
        let code = url_query(out["url"].as_str().unwrap())["code"].clone();
        let (status, out, _) = b.send("POST", "/api/auth/oauth2/token", None, Some(&[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect), ("client_id", &client), ("code_verifier", &verifier)]));
        assert_eq!(status, 200, "{out}");
        let token = out["access_token"].as_str().unwrap().to_string();
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let resp = agent.post(format!("{}/api/auth/devices/requests", k.address)).header("authorization", format!("Bearer {token}")).send_json(json!({ "key": key, "seal": data_encoding::HEXLOWER.encode(&device.seal), "name": "MacBook Pro" })).unwrap();
        assert_eq!(resp.status(), 200);
        let mut resp = agent.get(format!("{}/api/auth/devices/requests/{key}", k.address)).header("authorization", format!("Bearer {token}")).call().unwrap();
        let sign_in = resp.body_mut().read_json::<Value>().unwrap()["sign_in"].clone();
        // What the person's devices then do with it, with the passkey recorded as theirs.
        let proof: Proof = serde_json::from_value(sign_in.clone()).expect("a proof");
        let account = sign_in_proof::check(&proof, &device, &install, None, now()).expect("it checks, to be recorded as it's approved by a tap");
        let vouched = sign_in_proof::vouches(&proof, &device, &install, None, &[account], now());
        json!({ "sign_in": sign_in, "lets_in": vouched.is_ok() })
    };
    // The web client signs in in the browser that asks, and is let in by it.
    let here = carried("/callback");
    assert_eq!(here["lets_in"], true, "{here}");
    assert!(here["sign_in"]["by_link"].is_null());
    // The CLI's sign-in link comes back to the sign-in code page, from any browser: it waits for a tap.
    let linked = carried("/sign-in/code/done");
    assert_eq!(linked["lets_in"], false, "{linked}");
    assert!(linked["sign_in"]["by_link"]["passkey"].is_object(), "{linked}");
    // As a site that sent the person a link does.
    assert_eq!(carried("/elsewhere")["lets_in"], false);
}

/// A browser's push subscription, as its push service hands it to the push Worker and decrypts what's pushed to it
/// (RFC 8291): good for one push, as its key pair is.
pub(crate) struct Subscription {
    key: ring::agreement::EphemeralPrivateKey,
    pub(crate) public: Vec<u8>,
    pub(crate) auth: [u8; 16],
}

impl Subscription {
    pub(crate) fn new() -> Subscription {
        let key = ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &ring::rand::SystemRandom::new()).unwrap();
        Subscription { public: key.compute_public_key().unwrap().as_ref().to_vec(), key, auth: rand::random() }
    }

    /// Where the push Worker reaches it, at `endpoint`.
    pub(crate) fn address(&self, endpoint: &str) -> one::push::Address {
        one::push::Address { via: "webpush".into(), address: endpoint.into(), keys: Some(json!({ "p256dh": B64U.encode(&self.public), "auth": B64U.encode(&self.auth) })), ..Default::default() }
    }

    /// What the push service got (`body`, aes128gcm), decrypted: the push's text.
    pub(crate) fn open(self, body: &[u8]) -> Vec<u8> {
        use aes_gcm::aead::{Aead, KeyInit};
        let (salt, id_len) = (&body[..16], body[20] as usize);
        let as_public = &body[21..21 + id_len];
        let hkdf = |salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8]| hkdf::Hkdf::<sha2::Sha256>::new(Some(salt), ikm).expand(info, out).unwrap();
        let shared = ring::agreement::agree_ephemeral(self.key, &ring::agreement::UnparsedPublicKey::new(&ring::agreement::ECDH_P256, as_public), |s| s.to_vec()).unwrap();
        let (mut ikm, mut cek, mut nonce) = ([0u8; 32], [0u8; 16], [0u8; 12]);
        hkdf(&self.auth, &shared, &[b"WebPush: info\0".as_slice(), &self.public, as_public].concat(), &mut ikm);
        hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", &mut cek);
        hkdf(salt, &ikm, b"Content-Encoding: nonce\0", &mut nonce);
        let mut plain = aes_gcm::Aes128Gcm::new(&cek.into()).decrypt(&nonce.into(), &body[21 + id_len..]).unwrap();
        assert_eq!(plain.pop(), Some(2), "one record, the last");
        plain
    }
}

/// What auth pushes about a device asking to join is what the apps read (src/push.rs's `Note`): to its person's other
/// devices, never the one asking, "<device> wants in" with the action `{ join: <code> }` (`keys::join_code`, which the
/// apps' Approve and Not Me send their node) once nothing let it in within a few seconds, and "<device> joined" once
/// one did, each opening Settings › Devices.
#[test]
fn a_device_asking_to_join_is_pushed_to_its_persons_other_devices() {
    use data_encoding::{BASE64, HEXLOWER};
    let (admin, admin_hex) = endpoint();
    let (phone, phone_hex) = endpoint();
    let k = Workers::for_devices(vec![admin_hex, phone_hex.clone()]);
    let (secret, _) = invited(&k, &admin, "Andre", None);
    let passkey = k.passkey();
    let mut andre = k.browser();
    let (_, options, _) = andre.get(&format!("/api/auth/passkey/generate-register-options?context={secret}"));
    assert_eq!(andre.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true })).0, 200);
    let ask = |b: &mut Browser, d: &one::keys::Endpoint| {
        let (status, out) = b.post("/api/auth/devices/requests", json!({ "key": HEXLOWER.encode(&d.key), "seal": HEXLOWER.encode(&d.seal), "name": "MacBook Pro" }));
        assert_eq!(status, 200, "{out}");
    };
    // Their phone joined as theirs, and gets pushes.
    let phone_at = one::keys::Endpoint::of(&phone);
    ask(&mut andre, &phone_at);
    let endpoint = "https://fcm.googleapis.com/fcm/send/the-phone";
    let register = |sub: &Subscription| {
        let body = one::push::register_request(&phone, &phone_at.seal, &sub.address(endpoint), now()).unwrap();
        assert_eq!(post(&format!("{}/push/register", k.address), &[], body).0, 200);
    };
    let pushed = |sub: Subscription| -> Vec<one::push::Note> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let got: Value = agent.get(format!("{}/pushes", k.sites)).call().unwrap().body_mut().read_json().unwrap();
        let mut sub = Some(sub);
        got.as_array().unwrap().iter().filter(|p| p["url"] == endpoint).map(|p| {
            let text = sub.take().expect("one push to the phone").open(&BASE64.decode(p["body"].as_str().unwrap().as_bytes()).unwrap());
            let (note, by) = one::push::open_note(&one::keys::seal_secret(&phone), &B64U.decode(&text).unwrap()).unwrap();
            assert_eq!(by, None, "from the install's Workers");
            note
        }).collect()
    };
    let sub = Subscription::new();
    register(&sub);
    // A new device asks, and nothing lets it in.
    let mac = one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()));
    ask(&mut andre, &mac);
    assert!(pushed(Subscription::new()).is_empty(), "not before nothing let it in");
    std::thread::sleep(std::time::Duration::from_secs(12));
    let notes = pushed(sub);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!((notes[0].title.as_str(), notes[0].app.as_str(), notes[0].screen.as_str()), ("MacBook Pro wants in", "settings", "devices"));
    assert_eq!(notes[0].action, Some(json!({ "join": one::keys::join_code(&mac) })));
    // Approved: it joined.
    let sub = Subscription::new();
    register(&sub);
    assert_eq!(andre.delete(&format!("/api/auth/devices/requests/{}?approved", HEXLOWER.encode(&mac.key))).0, 200);
    let notes = pushed(sub);
    assert_eq!(notes.iter().map(|n| (n.title.as_str(), n.screen.as_str(), n.action.is_none())).collect::<Vec<_>>(), [("MacBook Pro joined", "devices", true)]);
}

/// The install the tests' Workers are for: its first entry's hash, as the device deploying them holds it.
const GENESIS: [u8; 32] = [5; 32];

/// iroh endpoints, as nodes are, through the relay at an install's address and nothing else: its relay protocol is
/// iroh-relay's Rust as WebAssembly in a Durable Object (cloudflare/relay/), which must agree with iroh's own.
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

    /// How long a device may take to be let in, or to reach another through the relay: a limit only for what never
    /// happens, as a busy machine takes seconds for what a quiet one takes milliseconds.
    const LET_IN: Duration = Duration::from_secs(60);
    /// How long to wait to be sure of a refusal. A key the relay won't take never comes online, so there's nothing to
    /// wait for: this can only pass too soon on a busy machine, never fail, and each such key is let in at once by the
    /// positive checks beside it.
    const REFUSED: Duration = Duration::from_secs(2);

    async fn enter(address: &str, key: &SecretKey, ticket: Option<&str>, within: Duration) -> Option<Endpoint> {
        let ep = endpoint(address, key, ticket).await;
        match tokio::time::timeout(within, ep.online()).await {
            Ok(()) => Some(ep),
            Err(_) => {
                ep.close().await;
                None
            }
        }
    }

    /// `key`, let in by the relay, as iroh sees it.
    async fn online(address: &str, key: &SecretKey, ticket: Option<&str>) -> Option<Endpoint> {
        enter(address, key, ticket, LET_IN).await
    }

    /// Whether the relay refuses `key`.
    async fn shut_out(address: &str, key: &SecretKey, ticket: Option<&str>) -> bool {
        match enter(address, key, ticket, REFUSED).await {
            Some(ep) => {
                ep.close().await;
                false
            }
            None => true,
        }
    }

    /// `from` reaches `to` through the relay: a stream that `to` echoes back, whole.
    async fn reaches(address: &str, from: &Endpoint, to: &Endpoint) -> bool {
        carries(address, from, to, LET_IN).await
    }

    /// Whether the relay carries nothing from `from` to `to`.
    async fn cut_off(address: &str, from: &Endpoint, to: &Endpoint) -> bool {
        !carries(address, from, to, REFUSED).await
    }

    async fn carries(address: &str, from: &Endpoint, to: &Endpoint, within: Duration) -> bool {
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
        let echoed = tokio::time::timeout(within, async {
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
        tcp.set_read_timeout(Some(LET_IN)).unwrap();
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

    /// A device is taken out (its key forgotten, its person signed out) only when the relay says so: a node that doesn't
    /// list it, which may not have read its approval yet, says nothing of it, nor does an old key that was in where a
    /// new one signs in.
    #[test]
    fn only_the_relay_says_a_device_was_taken_out() {
        let first = SecretKey::generate();
        let k = super::Workers::for_devices(vec![hex(&first)]);
        let address = k.address.replace("localhost", "127.0.0.1");
        let dir = crate::temp("taken-out");
        let secret: [u8; 32] = rand::random();
        let phone = SecretKey::from_bytes(&secret);
        assert_eq!(tell(&address, &first, json!({ "add": hex(&phone), "role": "client" })), 204);
        // As connecting leaves it, once this key was in.
        std::fs::write(dir.join("joined"), hex(&phone)).unwrap();
        let device = one_client::Device::new(secret, &address, dir.to_str().unwrap());
        let signed_in_again = one_client::Device::new(rand::random(), &address, dir.to_str().unwrap());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            assert!(!device.removed().await, "a device the relay lets in was taken for one taken out");
            assert!(!signed_in_again.removed().await, "a new key where an old one was in was taken for one taken out");
            assert_eq!(tell(&address, &first, json!({ "remove": hex(&phone) })), 204);
            assert!(device.removed().await, "a device the relay took out wasn't taken out");
            assert!(!signed_in_again.removed().await, "a new key was taken out with the old one");
        });
    }

    #[test]
    fn the_relay_carries_only_the_installs_endpoints() {
        let first = SecretKey::generate();
        let k = super::Workers::for_devices(vec![hex(&first)]);
        let address = k.address.replace("localhost", "127.0.0.1");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // The install's first device is on the list, an admin's, and a stranger isn't.
            let a = online(&address, &first, None).await.expect("the install's first device isn't let in");
            let stranger = SecretKey::generate();
            assert!(shut_out(&address, &stranger, None).await, "a stranger's key reached the relay");

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
            assert!(shut_out(&address, &stranger, None).await, "a key taken out reached the relay");

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
            assert!(shut_out(&address, &SecretKey::generate(), Some(&code)).await, "a pairing code let in a second computer");
            assert!(reaches(&address, &c, &a).await, "a pairing code's computer doesn't reach the device that made it");
            assert!(cut_off(&address, &c, &b).await, "a pairing code's computer reached another device");
            // Ended, its computer is out.
            assert_eq!(tell(&address, &phone, json!({ "unpair": sha256(&code) })), 403, "a device ended another's pairing code");
            assert_eq!(tell(&address, &first, json!({ "unpair": sha256(&code) })), 204);
            assert!(cut_off(&address, &c, &a).await, "an ended pairing code's computer still reached the relay");
            c.close().await;
            assert!(shut_out(&address, &joining, Some(&code)).await, "an ended pairing code let its computer in again");
            // Expired, too, while it's connected.
            let code = ticket();
            let until = super::now() + super::LASTS;
            assert_eq!(tell(&address, &first, json!({ "pairing": { "hash": sha256(&code), "until": until } })), 204);
            let d = online(&address, &SecretKey::generate(), Some(&code)).await.expect("a pairing code's computer isn't let in");
            super::past(until).await;
            assert!(cut_off(&address, &d, &a).await, "an expired pairing code's computer still reached the relay");
            d.close().await;

            // Taking a device out takes out the devices it let in.
            let laptop = SecretKey::generate();
            assert_eq!(tell(&address, &phone, json!({ "add": hex(&laptop), "role": "client" })), 204);
            assert_eq!(tell(&address, &first, json!({ "remove": hex(&phone) })), 204);
            assert!(shut_out(&address, &laptop, None).await, "a key a removed device let in still reached the relay");
            assert!(cut_off(&address, &b, &a).await, "a removed device's socket still reached the relay");
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
    /// A client races connections to its nodes and asks the first one up, keeping the others it raced for the next
    /// request. The keys a request hands are sealed as the node it's sent to is picked, to that one alone: never to a
    /// node it only raced a connection to (client/src/lib.rs, `fetch_with`).
    #[test]
    fn a_request_hands_keys_only_to_the_node_it_is_sent_to() {
        use std::sync::{Arc, Mutex};
        let (slow, quick) = (SecretKey::generate(), SecretKey::generate());
        let k = super::Workers::for_devices(vec![hex(&slow), hex(&quick)]);
        let address = k.address.replace("localhost", "127.0.0.1");
        let secret: [u8; 32] = rand::random();
        assert_eq!(tell(&address, &slow, json!({ "add": hex(&SecretKey::from_bytes(&secret)), "role": "client" })), 204);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Each node answers every request, noting the keys it was handed with it (`x-one-keys`); the slow one lets
            // a connection open only a while after it's asked.
            let got: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
            let opened: Arc<Mutex<Vec<String>>> = Arc::default();
            let serve = |key: &SecretKey, wait: Duration| {
                let (key, address, got, opened) = (key.clone(), address.clone(), got.clone(), opened.clone());
                async move {
                    let alpn = format!("{}/http/1", one::config::CLI).into_bytes();
                    let relay = RelayConfig::new(address.parse::<RelayUrl>().unwrap(), None);
                    let ep = Endpoint::builder(presets::Minimal).secret_key(key.clone()).alpns(vec![alpn]).clear_ip_transports().clear_address_lookup().relay_mode(RelayMode::Custom(relay.into())).bind().await.unwrap();
                    tokio::time::timeout(LET_IN, ep.online()).await.expect("a node isn't let in");
                    let serving = ep.clone();
                    tokio::spawn(async move {
                        while let Some(incoming) = serving.accept().await {
                            let (key, got, opened) = (hex(&key), got.clone(), opened.clone());
                            tokio::spawn(async move {
                                tokio::time::sleep(wait).await;
                                let conn = incoming.await.ok()?;
                                opened.lock().unwrap().push(key.clone());
                                while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                                    let asked = recv.read_to_end(1 << 16).await.ok()?;
                                    let asked = String::from_utf8_lossy(&asked).to_string();
                                    let keys = asked.lines().find_map(|l| l.strip_prefix("x-one-keys: ")).unwrap_or("").to_string();
                                    got.lock().unwrap().push((key.clone(), keys));
                                    send.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await.ok()?;
                                    send.finish().ok()?;
                                }
                                Some(())
                            });
                        }
                    });
                    ep
                }
            };
            let _nodes = (serve(&slow, Duration::from_secs(2)).await, serve(&quick, Duration::ZERO).await);
            let client = one_client::Client::start(secret, None, &address, &[*slow.public().as_bytes(), *quick.public().as_bytes()]).await.unwrap();
            let sealed: Arc<Mutex<Vec<String>>> = Arc::default();
            let ask = || {
                let request = one_client::Request { method: "GET".into(), path: "/v1/state".into(), headers: Vec::new(), body: Vec::new().into() };
                client.fetch_with(request, |node| {
                    let node = data_encoding::HEXLOWER.encode(node);
                    sealed.lock().unwrap().push(node.clone());
                    std::future::ready(vec![("x-one-keys".to_string(), format!("sealed to {node}"))])
                })
            };
            // The first in order is slow to connect: the next, raced beside it, is asked, and it alone is handed keys.
            let answered = ask().await.expect("no node answered");
            assert_eq!(answered.node, *quick.public().as_bytes(), "the node raced beside the slow one wasn't asked");
            assert_eq!(*sealed.lock().unwrap(), [hex(&quick)], "keys were sealed to a node the request didn't go to");
            assert_eq!(*got.lock().unwrap(), [(hex(&quick), format!("sealed to {}", hex(&quick)))]);
            // The slow one's connection, raced, opens and is kept: the next request goes to it, the first in order, and
            // only it is handed that request's keys.
            let up = async {
                while !opened.lock().unwrap().contains(&hex(&slow)) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            };
            tokio::time::timeout(LET_IN, up).await.expect("the slow node's raced connection never opened");
            tokio::time::sleep(Duration::from_millis(500)).await;
            let answered = ask().await.expect("no node answered");
            assert_eq!(answered.node, *slow.public().as_bytes(), "the first node in order, connected since, wasn't asked");
            assert_eq!(*sealed.lock().unwrap(), [hex(&quick), hex(&slow)]);
            assert_eq!(got.lock().unwrap()[1..], [(hex(&slow), format!("sealed to {}", hex(&slow)))], "keys reached a node they weren't sealed to");
            client.close().await;
        });
    }

    #[test]
    fn a_session_endpoint_gets_in_only_as_its_device_vouches() {
        let first = SecretKey::generate();
        let k = super::Workers::for_devices(vec![hex(&first)]);
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
            // And which of them are on the relay's network now, which a client races first.
            for body in [by_session.clone(), asked.clone()] {
                let (status, said) = computes(body);
                assert_eq!((status, said), (200, json!({ "computes": [hex(&first)], "online": [hex(&first)] })));
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
            // When each was last here, a session's as the device that vouched for it: told the install's own devices
            // and their sessions, as the computes are, and no one else.
            let seen = |by: &SecretKey| super::post(&format!("{address}/relay/seen"), &[], one::keys::signed_request(&signer(by), "one/relay", json!({ "seen": true }), at()));
            let (status, said) = seen(&phone);
            assert_eq!(status, 200);
            for key in [&first, &phone] {
                assert!(said["seen"][hex(key)].as_u64().is_some_and(|t| t <= super::now()), "{} wasn't seen: {said}", hex(key));
            }
            assert!(said["seen"].get(hex(&session)).is_none(), "a session was seen as itself: {said}");
            assert_eq!(seen(&stranger).0, 403, "a stranger was told who was seen");
            assert_eq!(computes(ask(&session, &certify(&stranger, &session, super::now() + 60_000))).0, 403);

            // Only a client vouches for a session: a compute or a service, admin or not, doesn't.
            let service = SecretKey::generate();
            assert_eq!(tell(&address, &first, json!({ "add": hex(&service), "role": "service" })), 204);
            for by in [&first, &service] {
                let other = SecretKey::generate();
                let vouched = certify(by, &other, super::now() + 60_000);
                assert!(shut_out(&address, &other, Some(&vouched)).await, "a session a compute or service vouched for got in");
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
            assert!(shut_out(&address, &theirs, Some(&foreign)).await, "a stranger's session got in with its proof");
            // Its own session, as one key's newest socket is the one it's reached at, and `s` is still connected.
            let fresh = SecretKey::generate();
            let shown = format!("{}.{}", certify(&phone, &fresh, super::now() + 60_000), proof(&fresh, super::now()));
            let proved = online(&address, &fresh, Some(&shown)).await.expect("a session with its proof isn't let in");
            assert!(reaches(&address, &proved, &a).await, "a session taken in with its proof doesn't reach the install's devices");
            proved.close().await;

            // Nothing for another endpoint, by a key not on the list, or once it has ended.
            assert!(shut_out(&address, &SecretKey::generate(), Some(&cert)).await, "a certificate let in an endpoint it doesn't name");
            assert!(shut_out(&address, &SecretKey::generate(), Some(&certify(&stranger, &stranger, super::now() + 60_000))).await, "a stranger vouched for itself");
            let short = SecretKey::generate();
            let until = super::now() + super::LASTS;
            let brief = online(&address, &short, Some(&certify(&phone, &short, until))).await.expect("a short session isn't let in");
            super::past(until).await;
            assert!(cut_off(&address, &brief, &a).await, "an ended session still reached the relay");
            assert!(shut_out(&address, &short, Some(&certify(&phone, &short, super::now() - 1))).await, "an ended session was let in");
            brief.close().await;

            // Its device taken out, it's out too.
            assert_eq!(tell(&address, &first, json!({ "remove": hex(&phone) })), 204);
            assert!(cut_off(&address, &s, &a).await, "a removed device's session still reached the relay");
            s.close().await;
            assert!(shut_out(&address, &session, Some(&cert)).await, "a removed device's session got in again");
            assert_eq!(computes(ask(&session, &cert)).0, 403, "a removed device's session was told the computes");
            a.close().await;
        });
    }
}
