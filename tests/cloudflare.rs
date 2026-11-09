//! The network's Workers (`cloudflare/`) running in the pinned workerd, as nodes and auth use them.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE64URL_NOPAD as B64U;
use one::cloudflare::{self, Served};
use serde_json::{Value, json};

/// The Workers' files as they are in this checkout, tests and all, with the screens' design kit and onboarding's pages where build.rs puts them.
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
    out.push(("onboarding/pages.js".into(), std::fs::read(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../src/onboarding/pages.js"))).unwrap()));
    out
}

fn module(entry: &str) -> Vec<u8> {
    cloudflare::module_from(files(), entry).unwrap()
}

/// Sockets listening on ports `port` gave out, until `run` gives them to the workerd serving there.
static LISTENING: std::sync::Mutex<Vec<std::net::TcpListener>> = std::sync::Mutex::new(Vec::new());

/// A port of this machine's that stays ours: nothing else, another test's workerd included, can take it.
pub(crate) fn port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    LISTENING.lock().unwrap().push(listener);
    port
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

pub(crate) struct Running {
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

pub(crate) fn run(services: Vec<Served>) -> Running {
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

/// What the address serves as the web client is parsed before anything deploys, so a script that doesn't (a merge's
/// markers left in it) fails here, not `one-dev update workers`.
#[test]
fn the_web_clients_scripts_parse() {
    one::cloudflare::parses(&one::web::files()).unwrap();
}

/// The page's policy allows its one inline script (start.js) by its hash: of the text the page serves, whatever that
/// script says, so a path in it isn't renamed after hashing and refused by the browser.
#[test]
fn the_pages_inline_script_is_the_one_its_policy_allows() {
    use sha2::Digest;
    let files = one::web::files();
    let page = String::from_utf8(files.iter().find(|(p, _)| p == "index.html").unwrap().1.clone()).unwrap();
    let body = page.split("<script>").nth(1).unwrap().split("</script>").next().unwrap();
    let hash = data_encoding::BASE64.encode(&sha2::Sha256::digest(body.as_bytes()));
    assert!(page.contains(&format!("'sha256-{hash}'")), "inline script hash {hash} not in policy");
}

#[test]
fn the_workers_own_tests_pass() {
    let dir = crate::temp("workers-tests");
    std::fs::write(dir.join("tests.js"), module("test/tests.js")).unwrap();
    std::fs::write(
        dir.join("config.capnp"),
        format!(
            "using Workerd = import \"/workerd/workerd.capnp\";\nconst config :Workerd.Config = (services = [(name = \"tests\", worker = (modules = [(name = \"tests.js\", esModule = embed \"tests.js\")], compatibilityDate = \"{}\"))]);\n",
            one::app::runtime::COMPATIBILITY_DATE
        ),
    )
    .unwrap();
    let out = std::process::Command::new(one::app::runtime::workerd().unwrap()).arg("test").arg(dir.join("config.capnp")).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "the Workers' tests failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

fn url_query(url: &str) -> std::collections::BTreeMap<String, String> {
    let query = url.split_once('?').unwrap().1;
    query.split('&').map(|p| p.split_once('=').unwrap()).map(|(k, v)| (k.to_string(), v.replace("%3B", ";"))).collect()
}

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

/// The real Workers, as a network runs them: users reach them at its address, the gateway's.
pub(crate) struct Workers {
    _running: Running,
    pub(crate) address: String,
    /// The stand-in sites the Workers' requests go to (test/sites.js).
    pub(crate) sites: String,
    /// Where the relay answers a test alone, which no gateway passes on.
    relay_test: String,
}

impl Workers {
    /// The Workers, their relay taking only `devices` (public keys, hex) and those they admit.
    pub(crate) fn for_devices(devices: Vec<String>) -> Workers {
        Workers::start_as(None, false, devices)
    }

    /// As `for_devices`, for a network whose users sign in with Google at test.example, the stand-in sites as
    /// Google.
    fn google(devices: Vec<String>) -> Workers {
        Workers::start_as(Some("test.example"), true, devices)
    }

    /// As `google`, for a network with a provider and no domain.
    fn provider(devices: Vec<String>) -> Workers {
        Workers::start_as(None, true, devices)
    }

    fn start_as(domain: Option<&str>, google: bool, devices: Vec<String>) -> Workers {
        let gateway = port();
        let mut settings = cloudflare::Settings::new("Test", &format!("http://localhost:{gateway}"));
        settings.ports = [port(), gateway];
        settings.auth_url = format!("http://127.0.0.1:{}", settings.ports[0]);
        // The network's first devices, its admin's machines that run its calls.
        settings.devices = devices.into_iter().map(|key| json!({ "key": key, "role": "compute", "admin": true })).collect();
        settings.root = Some(data_encoding::HEXLOWER.encode(&ROOT));
        settings.bucket = Some(json!({ "endpoint": "https://s3.test", "bucket": "network", "region": "auto", "key_id": "id", "secret": "bucket secret" }));
        let sites = port();
        let web = crate::temp("web");
        std::fs::write(web.join("index.html"), "<!doctype html><title>The web client</title>").unwrap();
        std::fs::write(web.join("frame.html"), "<!doctype html><title>A screen's frame</title>").unwrap();
        let mut served = cloudflare::served_with(&settings, &web, None).unwrap();
        for s in served.iter_mut().filter(|s| ["signer", "mcp", "push"].contains(&s.name.as_str())) {
            s.outbound = Some("sites".into());
        }
        // `TEST` gives auth and the relay a clock a test moves and the relay a log of what it refused: a network's
        // bindings come from its settings, so no deployed one has it.
        for s in served.iter_mut().filter(|s| ["auth", "relay"].contains(&s.name.as_str())) {
            s.bindings.push(("TEST".into(), "1".into()));
        }
        let relay_test = port();
        served.iter_mut().find(|s| s.name == "relay").unwrap().listen = Some(format!("127.0.0.1:{relay_test}"));
        if google {
            let sign_in = served.iter_mut().find(|s| s.name == "auth").unwrap();
            sign_in.outbound = Some("sites".into());
            let providers = json!([{ "id": "google", "name": "Google", "issuer": "https://accounts.google.com", "domain_claim": "hd", "domain_hint": "hd", "client_id": "a client id", "client_secret": "a client secret" }]);
            sign_in.bindings.push(("PROVIDERS".to_string(), providers.to_string()));
            sign_in.bindings.extend(domain.map(|d| ("JOIN_DOMAIN".to_string(), d.to_string())));
        }
        served.push(Served { name: "sites".into(), module: module("test/sites.js"), modules: vec![], class: None, listen: Some(format!("127.0.0.1:{sites}")), bindings: vec![], assets: None, outbound: None });
        let running = run(served);
        Workers { _running: running, address: settings.address, sites: format!("http://127.0.0.1:{sites}"), relay_test: format!("http://127.0.0.1:{relay_test}") }
    }

    /// The gateway's address, as endpoints reach it.
    fn relay_address(&self) -> String {
        self.address.replace("localhost", "127.0.0.1")
    }

    fn browser(&self) -> Browser {
        Browser { origin: self.address.clone(), cookies: Default::default() }
    }

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

/// What `key`'s device signs for sign-in's `route`: `fields`, with the route, as devices sign requests.
fn signed(key: &ed25519_dalek::SigningKey, route: &str, mut fields: Value) -> Value {
    fields["route"] = json!(route);
    one::keys::signed_request(key, "one/auth", fields, now())
}

/// A device's request to join, as `by`, one of the network's devices, reads it.
fn request_as_read_by(k: &Workers, by: &ed25519_dalek::SigningKey, device: &[u8; 32]) -> Value {
    let key = data_encoding::HEXLOWER.encode(device);
    let asked = data_encoding::BASE64URL_NOPAD.encode(signed(by, &format!("GET /api/auth/devices/requests/{key}"), json!({})).to_string().as_bytes());
    ureq::agent().get(format!("{}/api/auth/devices/requests/{key}", k.address)).header("x-one-request", asked).call().unwrap().body_mut().read_json().unwrap()
}

/// An invite `by` an endpoint for `name`, as its device makes one: → its secret and the user id it's for.
fn invited(k: &Workers, by: &ed25519_dalek::SigningKey, name: &str, email: Option<&str>) -> (String, String) {
    let secret = B64U.encode(&rand::random::<[u8; 32]>());
    let user = one::database::scopes::new_id();
    let until = now() + 86_400_000;
    let mut fields = json!({ "id": data_encoding::HEXLOWER.encode(&one::keys::sha256(secret.as_bytes())), "name": name, "user": user, "until": until });
    if let Some(email) = email {
        fields["email"] = json!(email);
    }
    let (status, out) = post(&format!("{}/api/auth/invites", k.address), &[], signed(by, "POST /api/auth/invites", fields));
    assert_eq!(status, 200, "{out}");
    (secret, user)
}

fn signs_in_with_passkey(browser: &mut Browser, passkey: &Passkey) -> u16 {
    let (status, options, _) = browser.get("/api/auth/passkey/generate-authenticate-options");
    assert_eq!(status, 200, "{options}");
    browser.post("/api/auth/passkey/verify-authentication", json!({ "response": passkey.get(&options) })).0
}

fn signed_in_as(browser: &mut Browser) -> Option<String> {
    browser.get("/api/auth/get-session").1["user"]["id"].as_str().map(str::to_string)
}

/// Joining with Google, as the invite page does: its secret kept for Google's answer, as `email`. → where the
/// callback goes on to.
fn joins_with_google(browser: &mut Browser, secret: &str, email: &str) -> Option<String> {
    let (status, out) = browser.post("/invite/provider", json!({ "secret": secret }));
    assert_eq!(status, 200, "{out}");
    google_callback(browser, "/", &format!("/invite#{secret}"), email)
}

/// Signs in with Google as `email`, coming back to `back` (or `failed`): → where Google's callback goes on to.
fn google_callback(browser: &mut Browser, back: &str, failed: &str, email: &str) -> Option<String> {
    provider_callback(browser, back, failed, json!({ "sub": email, "email": email, "email_verified": true, "hd": "test.example", "name": email }))
}

/// Signs in with the stand-in provider, its ID token saying `claims`: → where its callback goes on to.
fn provider_callback(browser: &mut Browser, back: &str, failed: &str, claims: Value) -> Option<String> {
    let (status, out) = browser.post("/api/auth/sign-in/social", json!({ "provider": "google", "callbackURL": back, "errorCallbackURL": failed }));
    assert_eq!(status, 200, "{out}");
    let state = url_query(out["url"].as_str().unwrap())["state"].clone();
    let (_, _, to) = browser.get(&format!("/api/auth/callback/google?code={}&state={state}", B64U.encode(claims.to_string().as_bytes())));
    to
}

/// The network's domain is read from the provider's signed token: a token saying another domain, or one the provider
/// didn't sign, signs no one in, whatever the page was steered to.
#[test]
fn the_domain_is_read_from_the_providers_signed_token() {
    let (admin, admin_hex) = endpoint();
    let k = Workers::google(vec![admin_hex]);
    let (secret, sam) = invited(&k, &admin, "Sam", Some("sam@test.example"));
    let claims = |hd: &str, forged: bool| json!({ "sub": "sam", "email": "sam@test.example", "email_verified": true, "hd": hd, "name": "Sam", "forged": forged });
    let mut browser = k.browser();
    assert_eq!(browser.post("/invite/provider", json!({ "secret": secret })).0, 200);
    let (_, out) = browser.post("/api/auth/sign-in/social", json!({ "provider": "google", "callbackURL": "/", "errorCallbackURL": "/x" }));
    assert_eq!(url_query(out["url"].as_str().unwrap()).get("hd").map(String::as_str), Some("test.example"), "the provider's page is steered to the domain");
    let back = provider_callback(&mut browser, "/", "/x", claims("elsewhere.example", false));
    assert!(back.is_some_and(|to| to.contains("error=")), "an account at another domain was let through");
    assert_eq!(signed_in_as(&mut browser), None);
    provider_callback(&mut browser, "/", "/x", claims("test.example", true));
    assert_eq!(signed_in_as(&mut browser), None, "a token its provider didn't sign counted");
    assert_eq!(provider_callback(&mut browser, "/", "/x", claims("test.example", false)).as_deref(), Some("/"));
    assert_eq!(signed_in_as(&mut browser).as_deref(), Some(sam.as_str()));
}

#[test]
fn an_admin_signs_in_like_anyone_else() {
    let (admin, admin_hex) = endpoint();
    let k = Workers::google(vec![admin_hex]);
    let (secret, andre) = invited(&k, &admin, "Andre", Some("andre@test.example"));
    // Signing up with Google takes the account the invite is for, and nobody signs up by the domain alone.
    let mut lees = k.browser();
    joins_with_google(&mut lees, &secret, "lee@test.example");
    assert_ne!(signed_in_as(&mut lees).as_deref(), Some(andre.as_str()), "someone else's Google account took up the invite");
    // At the domain, anyone signs in, which only lets them ask: nothing about them is in the network until someone says yes.
    let mut kims = k.browser();
    google_callback(&mut kims, "/", "/", "kim@test.example");
    assert!(signed_in_as(&mut kims).is_some_and(|kim| kim != andre), "someone at the domain may ask");
    let mut andres = k.browser();
    assert_eq!(joins_with_google(&mut andres, &secret, "andre@test.example").as_deref(), Some("/"));
    assert_eq!(signed_in_as(&mut andres).as_deref(), Some(andre.as_str()));
    let mut again = k.browser();
    google_callback(&mut again, "/", "/", "andre@test.example");
    assert_eq!(signed_in_as(&mut again).as_deref(), Some(andre.as_str()), "they sign in with Google again");

    // Having added a passkey, the same user signs in with either.
    let passkey = k.passkey();
    let (status, options, _) = andres.get("/api/auth/passkey/generate-register-options");
    assert_eq!(status, 200, "{options}");
    let (status, out) = andres.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options) }));
    assert_eq!(status, 200, "{out}");
    let mut elsewhere = k.browser();
    assert_eq!(signs_in_with_passkey(&mut elsewhere, &passkey), 200);
    assert_eq!(signed_in_as(&mut elsewhere).as_deref(), Some(andre.as_str()));
}

/// An invite naming an email needs no link: that account signs in as who the invite says, and no other does; and what a
/// request's sign-in showed, an ID token being a bearer's, reaches only an admin's device or the user's own.
#[test]
fn an_invited_email_signs_in_and_only_who_may_answer_reads_its_proof() {
    use data_encoding::HEXLOWER;
    let (admin, admin_hex) = endpoint();
    let (member, member_hex) = endpoint();
    let k = Workers::google(vec![admin_hex]);
    assert_eq!(post(&format!("{}/relay/tell", k.address), &[], one::keys::signed_request(&admin, "one/relay", json!({ "add": member_hex, "role": "client" }), now())).0, 204);
    let (_, sam) = invited(&k, &admin, "Sam", Some("sam@test.example"));
    let mut sams = k.browser();
    google_callback(&mut sams, "/", "/", "sam@test.example");
    assert_eq!(signed_in_as(&mut sams).as_deref(), Some(sam.as_str()), "signed in as who the invite named");

    let device = one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()));
    let (status, out) = sams.post("/api/auth/devices/requests", json!({ "key": HEXLOWER.encode(&device.key), "seal": HEXLOWER.encode(&device.seal), "name": "Sam's phone", "reads": one::database::READS }));
    assert_eq!(status, 200, "{out}");
    let proof = "a.proof.kept";
    assert_eq!(post(&format!("{}/api/auth/test", k.address), &[], json!({ "keep_proof": { "key": HEXLOWER.encode(&device.key), "user": sam, "proof": { "id_token": proof } } })).0, 200);
    let listed_to = |key: &ed25519_dalek::SigningKey| -> Value {
        let asked = B64U.encode(signed(key, "GET /api/auth/devices/requests", json!({})).to_string().as_bytes());
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let mut got = agent.get(format!("{}/api/auth/devices/requests", k.address)).header("x-one-request", asked).call().unwrap();
        got.body_mut().read_json::<Value>().unwrap()["requests"][0].clone()
    };
    assert_eq!(listed_to(&admin)["sign_in"]["id_token"], proof);
    assert!(listed_to(&member).get("sign_in").is_none(), "another user's device read someone's ID token");
    assert_eq!(listed_to(&member)["name"], "Sam's phone", "though it sees the request");
}

/// Only a request's user's own device, or an admin's, answers it: any other device of the network marks it approved or
/// forgets it no more, whether auth knows whose it is or not; and auth knows whose a device is only from who may say.
#[test]
fn only_its_users_own_device_or_an_admins_answers_a_request() {
    use data_encoding::HEXLOWER;
    let (admin, admin_hex) = endpoint();
    let (stranger, stranger_hex) = endpoint();
    let (sams_box, sams_box_hex) = endpoint();
    let k = Workers::google(vec![admin_hex]);
    for hex in [&stranger_hex, &sams_box_hex] {
        assert_eq!(post(&format!("{}/relay/tell", k.address), &[], one::keys::signed_request(&admin, "one/relay", json!({ "add": hex, "role": "client" }), now())).0, 204);
    }
    let (_, sam) = invited(&k, &admin, "Sam", Some("sam@test.example"));
    let (_, lee) = invited(&k, &admin, "Lee", Some("lee@test.example"));
    let mut sams = k.browser();
    google_callback(&mut sams, "/", "/", "sam@test.example");
    let device = one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()));
    let key = HEXLOWER.encode(&device.key);
    assert_eq!(sams.post("/api/auth/devices/requests", json!({ "key": key, "seal": HEXLOWER.encode(&device.seal), "name": "Sam's phone", "reads": one::database::READS })).0, 200);

    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let answer = |by: &ed25519_dalek::SigningKey, query: &str| -> u16 {
        let asked = B64U.encode(signed(by, &format!("DELETE /api/auth/devices/requests/{key}"), json!({})).to_string().as_bytes());
        agent.delete(format!("{}/api/auth/devices/requests/{key}{query}", k.address)).header("x-one-request", asked).call().unwrap().status().as_u16()
    };
    let theirs = |by: &ed25519_dalek::SigningKey, key: &str, user: &str| post(&format!("{}/api/auth/devices/theirs", k.address), &[], signed(by, "POST /api/auth/devices/theirs", json!({ "key": key, "user": user }))).0;

    assert_eq!(answer(&sams_box, "?approved"), 403, "a device auth knows no user of answered someone's request");
    assert_eq!(answer(&stranger, ""), 403, "and forgot it");
    assert_eq!(theirs(&stranger, &stranger_hex, &sam), 403, "a device said it's someone's, without being theirs or an admin's");
    assert_eq!(theirs(&admin, &stranger_hex, &lee), 200);
    assert_eq!(answer(&stranger, "?approved"), 403, "another user's device answered it");
    assert_eq!(theirs(&admin, &sams_box_hex, &sam), 200);
    assert_eq!(answer(&sams_box, "?approved"), 200, "their own device answers it");
    assert_eq!(theirs(&sams_box, &stranger_hex, &sam), 200, "and says what else is theirs, as it pairs a computer");

    // Whoever signed in can name any key: it doesn't change whose one already is.
    let mut lees = k.browser();
    google_callback(&mut lees, "/", "/", "lee@test.example");
    let (status, _) = lees.post("/api/auth/devices/requests", json!({ "key": sams_box_hex, "seal": HEXLOWER.encode(&device.seal), "name": "Mine now", "reads": one::database::READS }));
    assert_eq!(status, 200);
    let second = one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()));
    let second_key = HEXLOWER.encode(&second.key);
    assert_eq!(sams.post("/api/auth/devices/requests", json!({ "key": second_key, "seal": HEXLOWER.encode(&second.seal), "name": "Sam's laptop", "reads": one::database::READS })).0, 200);
    let asked = B64U.encode(signed(&sams_box, &format!("DELETE /api/auth/devices/requests/{second_key}"), json!({})).to_string().as_bytes());
    assert_eq!(agent.delete(format!("{}/api/auth/devices/requests/{second_key}", k.address)).header("x-one-request", asked).call().unwrap().status().as_u16(), 200, "still Sam's");
}

/// A passkey belongs to an account. An invite that names a provider's account can't be taken up by making a passkey, and
/// someone with no email is never matched by one a provider says.
#[test]
fn a_passkey_belongs_to_an_account_and_no_email_is_matched() {
    let (admin, admin_hex) = endpoint();
    let domain = Workers::google(vec![admin_hex.clone()]);
    let (secret, _) = invited(&domain, &admin, "Lee", Some("lee@test.example"));
    let mut lees = domain.browser();
    assert_eq!(lees.get(&format!("/api/auth/passkey/generate-register-options?context={secret}")).0, 403, "a passkey took up an invite for a provider's account");

    let k = Workers::provider(vec![admin_hex]);
    let (secret, sam) = invited(&k, &admin, "Sam", None);
    let (mut sams, passkey) = (k.browser(), k.passkey());
    let (status, options, _) = sams.get(&format!("/api/auth/passkey/generate-register-options?context={secret}"));
    assert_eq!(status, 200, "{options}");
    assert_eq!(options["user"]["name"], "Sam");
    assert_eq!(sams.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true })).0, 200);
    assert_eq!(signed_in_as(&mut sams).as_deref(), Some(sam.as_str()));
    let (_, options, _) = sams.get("/api/auth/passkey/generate-register-options");
    assert_eq!(options["user"]["name"], "Sam", "a passkey added later is named for its user, not a stand-in email");
    // A provider's account saying it has that stand-in signs in as no one.
    let mut other = k.browser();
    let email = format!("{sam}@people.invalid");
    provider_callback(&mut other, "/", "/", json!({ "sub": "stranger", "email": email, "email_verified": true, "name": "Stranger" }));
    assert_eq!(signed_in_as(&mut other), None, "a provider's account took over someone with no email");
    assert_eq!(signs_in_with_passkey(&mut other, &passkey), 200);
    assert_eq!(signed_in_as(&mut other).as_deref(), Some(sam.as_str()), "the passkey still signs them in");
}

/// One more identity is linked from a session just signed in with what they already had, however different its email:
/// its token names the device, for that device to check and record, and it signs them in afterwards; and it's only theirs.
#[test]
fn a_further_identity_is_linked_only_from_someone_signed_in() {
    let (admin, admin_hex) = endpoint();
    let k = Workers::provider(vec![admin_hex]);
    let (secret, sam) = invited(&k, &admin, "Sam", None);
    let (mut sams, passkey) = (k.browser(), k.passkey());
    let (_, options, _) = sams.get(&format!("/api/auth/passkey/generate-register-options?context={secret}"));
    assert_eq!(sams.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true })).0, 200);
    let device = one::keys::Endpoint::of(&admin);
    let join = format!("{}{}", data_encoding::HEXLOWER.encode(&device.key), data_encoding::HEXLOWER.encode(&device.seal));
    let claims = json!({ "sub": "sams-google", "email": "sam@anywhere.example", "email_verified": true, "name": "Sam" });

    let link = |b: &mut Browser, claims: Value| {
        let (status, out) = b.post(&format!("/api/auth/link-social?join={join}"), json!({ "provider": "google", "callbackURL": format!("/sign-in/link?join={join}&linked=google"), "errorCallbackURL": format!("/sign-in/link?join={join}") }));
        assert_eq!(status, 200, "{out}");
        let query = url_query(out["url"].as_str().unwrap());
        assert_eq!(query.get("nonce"), Some(&one::onboarding::sign_in_proof::nonce(&ROOT, &device)), "its token will name the device");
        b.get(&format!("/api/auth/callback/google?code={}&state={}", B64U.encode(claims.to_string().as_bytes()), query["state"])).2
    };
    assert_eq!(sams.get(&format!("/sign-in/link?join={join}")).0, 200);
    assert!(link(&mut sams, claims.clone()).is_some_and(|to| to.ends_with("&linked=google")));
    assert_eq!(sams.post("/sign-in/link/done", json!({ "join": join, "provider": "google" })).0, 200);

    // What the device takes to check and record.
    let shown = |key: &ed25519_dalek::SigningKey| {
        let asked = B64U.encode(serde_json::to_string(&signed(key, "GET /api/auth/devices/proof", json!({}))).unwrap().as_bytes());
        let mut resp = ureq::agent().get(format!("{}/api/auth/devices/proof", k.address)).header("x-one-request", asked).call().unwrap();
        (resp.status().as_u16(), resp.body_mut().read_json::<Value>().unwrap())
    };
    let (status, out) = shown(&admin);
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["proofs"][0]["user"], json!(sam));
    assert!(out["proofs"][0]["proof"]["id_token"].is_string(), "{out}");

    // It signs them in now, in any browser.
    let mut elsewhere = k.browser();
    provider_callback(&mut elsewhere, "/", "/", claims.clone());
    assert_eq!(signed_in_as(&mut elsewhere).as_deref(), Some(sam.as_str()));

    // Someone else, also just signed in, can't link it to themselves.
    let (secret, _) = invited(&k, &admin, "Lee", None);
    let (mut lees, lee_passkey) = (k.browser(), k.passkey());
    let (_, options, _) = lees.get(&format!("/api/auth/passkey/generate-register-options?context={secret}"));
    assert_eq!(lees.post("/api/auth/passkey/verify-registration", json!({ "response": lee_passkey.create(&options), "createSession": true })).0, 200);
    assert!(link(&mut lees, claims).is_some_and(|to| to.contains("error=")), "someone else's identity was linked");

    // And no one signed out links anything.
    let (status, _) = k.browser().post("/api/auth/link-social", json!({ "provider": "google", "callbackURL": "/" }));
    assert!(status == 401 || status == 403, "{status}");
}

/// A sign-in for a device names it, and auth passes what it showed on with that device's request to join, unchanged,
/// for the user's devices to check themselves (src/onboarding/sign_in_proof.rs): a passkey's assertion signs a challenge naming
/// the device and when, which checks; a sign-in for no device, or one for another device, lets nothing in; and with
/// Google, the ID token asked for names the device.
#[test]
fn a_sign_in_for_a_device_names_it_and_auth_passes_it_on() {
    use one::onboarding::sign_in_proof::{self, Account, Proof};
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
    assert_eq!(challenge[..challenge.len() - 8], sign_in_proof::challenge(&ROOT, &device, 0)[..challenge.len() - 8], "the challenge names the device in this network");
    assert_eq!(new.post("/api/auth/passkey/verify-authentication", json!({ "response": passkey.get(&options) })).0, 200);
    assert_eq!(signed_in_as(&mut new).as_deref(), Some(andre.as_str()));
    let asked = |w: &Workers, b: &mut Browser, d: &one::keys::Endpoint| {
        let (status, out) = b.post("/api/auth/devices/requests", json!({ "key": data_encoding::HEXLOWER.encode(&d.key), "seal": data_encoding::HEXLOWER.encode(&d.seal), "name": "MacBook Pro", "platform": "mac", "reads": one::database::READS }));
        assert_eq!(status, 200, "{out}");
        let (_, out, _) = b.get(&format!("/api/auth/devices/requests/{}", data_encoding::HEXLOWER.encode(&d.key)));
        assert_eq!(out["platform"], "mac", "what a device says it is reaches the devices that approve it");
        assert!(out.get("sign_in").is_none(), "a sign-in alone read what its request carries: {out}");
        request_as_read_by(w, &admin, &d.key)["sign_in"].clone()
    };
    let proof: Proof = serde_json::from_value(asked(&k, &mut new, &device)).expect("its request carries the assertion");
    let network = sign_in_proof::Network::new(&k.address, ROOT, None);
    let account = sign_in_proof::check(&proof, &device, &network, None).expect("the user's devices check it");
    let Account::Passkey { public_key, .. } = &account else { panic!("{account:?}") };
    let checked = sign_in_proof::vouches(&proof, &device, &network, None, std::slice::from_ref(&account));
    assert!(checked.is_ok() && !public_key.is_empty(), "and it's theirs, once it's recorded: {checked:?}");
    let elsewhere = sign_in_proof::Network::new(&k.address, [6; 32], None);
    assert!(sign_in_proof::vouches(&proof, &device, &elsewhere, None, std::slice::from_ref(&account)).is_err(), "but not in another network");
    // Turned down, its request goes, and what its sign-in showed with it: asking again, it carries nothing.
    let key = data_encoding::HEXLOWER.encode(&device.key);
    assert_eq!(new.delete(&format!("/api/auth/devices/requests/{key}")).0, 200);
    assert_eq!(asked(&k, &mut new, &device), Value::Null, "a refused device's sign-in is forgotten");
    // Another device asking as them carries none, nor does one signed in for no device.
    assert_eq!(asked(&k, &mut new, &one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()))), Value::Null);
    let mut plain = k.browser();
    assert_eq!(signs_in_with_passkey(&mut plain, &passkey), 200);
    assert_eq!(asked(&k, &mut plain, &one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()))), Value::Null);

    // With Google, the ID token it asks for names the device.
    let g = Workers::google(vec![admin_hex]);
    let mut browser = g.browser();
    let (status, client) = browser.post("/api/auth/oauth2/register", json!({ "client_name": "Chrome on Mac", "redirect_uris": [format!("{}/callback", g.address)], "token_endpoint_auth_method": "none", "grant_types": ["authorization_code"], "response_types": ["code"] }));
    assert_eq!(status, 201, "{client}");
    let (status, out) = browser.post(&format!("/api/auth/sign-in/social?join={join}"), json!({ "provider": "google", "callbackURL": "/", "errorCallbackURL": "/" }));
    assert_eq!(status, 200, "{out}");
    let google = url_query(out["url"].as_str().unwrap());
    assert_eq!(google.get("nonce"), Some(&sign_in_proof::nonce(&ROOT, &device)), "Google is asked for an ID token naming the device in this network");
    // Its token goes with that device's request, and with no other's.
    let (_, invited_as) = invited(&g, &admin, "Kim", Some("kim@test.example"));
    let claims = json!({ "sub": "kim", "email": "kim@test.example", "email_verified": true, "hd": "test.example", "name": "Kim", "nonce": google["nonce"] });
    browser.get(&format!("/api/auth/callback/google?code={}&state={}", B64U.encode(claims.to_string().as_bytes()), google["state"]));
    assert_eq!(signed_in_as(&mut browser).as_deref(), Some(invited_as.as_str()));
    assert!(asked(&g, &mut browser, &device)["id_token"].is_string(), "the device it names carries it");
    assert_eq!(asked(&g, &mut browser, &one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()))), Value::Null, "another carries none");
    // An ID token handed to auth directly signs no one in, nor links, whoever holds one.
    let (status, out) = browser.post("/api/auth/sign-in/social", json!({ "provider": "google", "idToken": { "token": "a.b.c" } }));
    assert_eq!(status, 403, "{out}");
    let (status, out) = browser.post("/api/auth/link-social", json!({ "provider": "google", "idToken": { "token": "a.b.c" } }));
    assert_eq!(status, 403, "{out}");
}

/// A sign-in finished from a link opened elsewhere (the CLI's sign-in code page, or a site that sent the user a link)
/// could have been started by whoever sent it, who'd hold the token it gave. Auth passes that one's proof on as `by_link`,
/// which still shows who signed in, but is never enough for an invite to let it in with no tap.
#[test]
fn a_sign_in_finished_from_a_link_says_so() {
    use one::onboarding::sign_in_proof::{self, Proof};
    use sha2::Digest;
    let (admin, admin_hex) = endpoint();
    let k = Workers::for_devices(vec![admin_hex]);
    let (secret, _) = invited(&k, &admin, "Andre", None);
    let passkey = k.passkey();
    let mut first = k.browser();
    let (_, options, _) = first.get(&format!("/api/auth/passkey/generate-register-options?context={secret}"));
    assert_eq!(first.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true })).0, 200);
    let network = sign_in_proof::Network::new(&k.address, ROOT, None);
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
        let (status, out) = b.post("/api/auth/oauth2/consent", json!({ "accept": true, "oauth_query": oauth_query }));
        assert_eq!(status, 200, "{out}");
        let code = url_query(out["url"].as_str().unwrap())["code"].clone();
        // A sign-in made before an authorization was asked for gives it nothing, whoever holds that session.
        let (_, _, again) = b.get(&format!("/api/auth/oauth2/authorize?{query}"));
        let (_, out) = b.post("/api/auth/oauth2/consent", json!({ "accept": true, "oauth_query": again.expect("it asks for a sign-in").split_once('?').unwrap().1 }));
        assert!(url_query(out["url"].as_str().unwrap()).get("code").is_none(), "{out}");
        let (status, out, _) = b.send("POST", "/api/auth/oauth2/token", None, Some(&[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect), ("client_id", &client), ("code_verifier", &verifier)]));
        assert_eq!(status, 200, "{out}");
        let token = out["access_token"].as_str().unwrap().to_string();
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let resp = agent.post(format!("{}/api/auth/devices/requests", k.address)).header("authorization", format!("Bearer {token}")).send_json(json!({ "key": key, "seal": data_encoding::HEXLOWER.encode(&device.seal), "name": "MacBook Pro", "reads": one::database::READS })).unwrap();
        assert_eq!(resp.status(), 200);
        let mut resp = agent.get(format!("{}/api/auth/devices/requests/{key}", k.address)).header("authorization", format!("Bearer {token}")).call().unwrap();
        assert!(resp.body_mut().read_json::<Value>().unwrap().get("sign_in").is_none(), "a sign-in alone read what its request carries");
        let sign_in = request_as_read_by(&k, &admin, &device.key)["sign_in"].clone();
        // What the user's devices then do with it, with the passkey recorded as theirs.
        let proof: Proof = serde_json::from_value(sign_in.clone()).expect("a proof");
        sign_in_proof::check(&proof, &device, &network, None).expect("it checks, to be recorded as it's approved by a tap");
        json!({ "sign_in": sign_in, "goes_straight_in": matches!(proof, Proof::IdToken(_) | Proof::Passkey { .. }) })
    };
    // The web client signs in in the browser that asks.
    let here = carried("/callback");
    assert_eq!(here["goes_straight_in"], true, "{here}");
    assert!(here["sign_in"]["by_link"].is_null());
    // The CLI's sign-in link comes back to the sign-in code page, from any browser: it waits for a tap.
    let linked = carried("/sign-in/code/done");
    assert_eq!(linked["goes_straight_in"], false, "{linked}");
    assert!(linked["sign_in"]["by_link"]["passkey"].is_object(), "{linked}");
    // As a site that sent the user a link does.
    assert_eq!(carried("/elsewhere")["goes_straight_in"], false);
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

/// What sign-in pushes about a device asking to join is what the apps read (src/push.rs's `Note`): to its user's other
/// devices, never the one asking, "<device> wants in" with the action `{ join: <code> }` (`keys::join_code`, which the
/// apps' Approve and Not Me send their node) as it asks, and "<device> joined" once
/// one let it in, each opening Settings › Devices. Their devices are those that asked to join as them here, and those that
/// registered for pushes naming them, as a device that joined before sign-in kept whose devices asked does.
#[test]
fn a_device_asking_to_join_is_pushed_to_its_users_other_devices() {
    use data_encoding::{BASE64, HEXLOWER};
    let (admin, admin_hex) = endpoint();
    let (phone, phone_hex) = endpoint();
    let (old, old_hex) = endpoint();
    let k = Workers::for_devices(vec![admin_hex, phone_hex.clone(), old_hex]);
    let (secret, user) = invited(&k, &admin, "Andre", None);
    let passkey = k.passkey();
    let mut andre = k.browser();
    let (_, options, _) = andre.get(&format!("/api/auth/passkey/generate-register-options?context={secret}"));
    assert_eq!(andre.post("/api/auth/passkey/verify-registration", json!({ "response": passkey.create(&options), "createSession": true })).0, 200);
    let ask = |b: &mut Browser, d: &one::keys::Endpoint| {
        let (status, out) = b.post("/api/auth/devices/requests", json!({ "key": HEXLOWER.encode(&d.key), "seal": HEXLOWER.encode(&d.seal), "name": "MacBook Pro", "reads": one::database::READS }));
        assert_eq!(status, 200, "{out}");
    };
    let approved = |key: &str| {
        let asked = B64U.encode(signed(&admin, &format!("DELETE /api/auth/devices/requests/{key}"), json!({})).to_string().as_bytes());
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        agent.delete(format!("{}/api/auth/devices/requests/{key}?approved", k.address)).header("x-one-request", asked).call().unwrap().status().as_u16()
    };
    // Their phone joined as theirs, and gets pushes.
    let phone_at = one::keys::Endpoint::of(&phone);
    ask(&mut andre, &phone_at);
    assert_eq!(approved(&phone_hex), 200);
    let endpoint = "https://fcm.googleapis.com/fcm/send/the-phone";
    let register = |sub: &Subscription| {
        let body = one::push::register_request(&phone, &phone_at.seal, &sub.address(endpoint), None, now()).unwrap();
        assert_eq!(post(&format!("{}/push/register", k.address), &[], body).0, 200);
    };
    // What was pushed to each device, by its endpoint, with its subscription and key, from what was pushed since last asked.
    let pushed_to = |devices: Vec<(&str, Subscription, &ed25519_dalek::SigningKey)>| -> Vec<Vec<one::push::Note>> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let got: Value = agent.get(format!("{}/pushes", k.sites)).call().unwrap().body_mut().read_json().unwrap();
        devices.into_iter().map(|(endpoint, sub, key)| {
            let mut sub = Some(sub);
            got.as_array().unwrap().iter().filter(|p| p["url"] == endpoint).map(|p| {
                let text = sub.take().expect("one push to the device").open(&BASE64.decode(p["body"].as_str().unwrap().as_bytes()).unwrap());
                let (note, by) = one::push::open_note(&one::keys::seal_secret(key), &B64U.decode(&text).unwrap()).unwrap();
                assert_eq!(by, None, "from the network's Workers");
                note
            }).collect()
        }).collect()
    };
    let pushed = |sub: Subscription| pushed_to(vec![(endpoint, sub, &phone)]).remove(0);
    let sub = Subscription::new();
    register(&sub);
    // Their Mac, which joined before, never asked here, and registers naming them.
    let old_endpoint = "https://fcm.googleapis.com/fcm/send/the-mac";
    let old_sub = Subscription::new();
    let old_at = one::keys::Endpoint::of(&old);
    let body = one::push::register_request(&old, &old_at.seal, &old_sub.address(old_endpoint), Some(&user), now()).unwrap();
    assert_eq!(post(&format!("{}/push/register", k.address), &[], body).0, 200);
    // A new device asks.
    let mac = one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()));
    ask(&mut andre, &mac);
    let [notes, to_old]: [Vec<one::push::Note>; 2] = pushed_to(vec![(endpoint, sub, &phone), (old_endpoint, old_sub, &old)]).try_into().unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(to_old.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["MacBook Pro wants in"], "the Mac that only registered is pushed too");
    assert_eq!((notes[0].title.as_str(), notes[0].app.as_str(), notes[0].screen.as_str()), ("MacBook Pro wants in", "settings", "devices"));
    assert_eq!(notes[0].action, Some(json!({ "join": one::keys::join_code(&mac) })));
    // Approved: it joined.
    let sub = Subscription::new();
    register(&sub);
    assert_eq!(approved(&HEXLOWER.encode(&mac.key)), 200);
    let notes = pushed(sub);
    assert_eq!(notes.iter().map(|n| (n.title.as_str(), n.screen.as_str(), n.action.is_none())).collect::<Vec<_>>(), [("MacBook Pro joined", "devices", true)]);
}

/// The network the tests' Workers are for: its root, as the device deploying them holds it.
const ROOT: [u8; 32] = [5; 32];

/// iroh endpoints, as nodes are, through the relay at a network's address and nothing else: its relay protocol is
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

    async fn online(address: &str, key: &SecretKey, ticket: Option<&str>) -> Option<Endpoint> {
        let ep = endpoint(address, key, ticket).await;
        if tokio::time::timeout(LET_IN, ep.online()).await.is_ok() {
            return Some(ep);
        }
        ep.close().await;
        None
    }

    /// What the relay's test hooks (its `TEST` binding) say: see `Relay.testing`. A wait for what the relay will say
    /// is bounded by `LET_IN`, as it's for what happens.
    async fn testing(k: &super::Workers, told: Value) -> Value {
        let url = format!("{}/relay/test", k.relay_test);
        tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(LET_IN)).build().into();
            agent.post(&url).send_json(told).unwrap().body_mut().read_json().unwrap()
        })
        .await
        .unwrap()
    }

    async fn said(k: &super::Workers, key: &str, what: &str) -> u64 {
        testing(k, json!({ "count": [key, what] })).await.as_u64().unwrap()
    }

    /// The relay says `what` of `key` once more than `after` times: a wait for what it does, which a refusal is, not
    /// for what it doesn't.
    async fn says(k: &super::Workers, key: &str, what: &str, after: u64) {
        testing(k, json!({ "wait": [key, what, after] })).await;
    }

    fn id_of(ep: &Endpoint) -> String {
        data_encoding::HEXLOWER.encode(ep.id().as_bytes())
    }

    /// Whether the relay denies `key` at its proof, as it says it does; an endpoint it let in would be online first.
    async fn shut_out(k: &super::Workers, key: &SecretKey, ticket: Option<&str>) -> bool {
        let (address, id) = (k.relay_address(), hex(key));
        let before = said(k, &id, "denied").await;
        let ep = endpoint(&address, key, ticket).await;
        let denied = tokio::select! {
            () = ep.online() => false,
            () = says(k, &id, "denied", before) => true,
        };
        ep.close().await;
        denied
    }

    /// The status the relay answers a socket that shows `ticket` with, before it asks for any proof.
    fn upgrade_status(address: &str, ticket: &str) -> u16 {
        use std::io::{Read, Write};
        let host = address.trim_start_matches("http://");
        let mut tcp = std::net::TcpStream::connect(host).unwrap();
        tcp.set_read_timeout(Some(LET_IN)).unwrap();
        write!(tcp, "GET /relay?token={ticket} HTTP/1.1\r\nhost: {host}\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\nsec-websocket-protocol: iroh-relay-v2\r\n\r\n").unwrap();
        let mut got = [0u8; 12];
        tcp.read_exact(&mut got).unwrap();
        String::from_utf8_lossy(&got[9..12]).parse().unwrap()
    }

    async fn cut_off(k: &super::Workers, ep: &Endpoint) -> bool {
        testing(k, json!({ "connected": [id_of(ep)] })).await == json!([])
    }

    async fn forbidden(k: &super::Workers, from: &Endpoint, to: &Endpoint) -> bool {
        let (address, id) = (k.relay_address(), id_of(from));
        let before = said(k, &id, "forbidden").await;
        tokio::select! {
            _ = carries(&address, from, to) => false,
            () = says(k, &id, "forbidden", before) => true,
        }
    }

    /// `from` reaches `to` through the relay: a stream that `to` echoes back, whole.
    async fn reaches(address: &str, from: &Endpoint, to: &Endpoint) -> bool {
        tokio::time::timeout(LET_IN, carries(address, from, to)).await.unwrap_or(false)
    }

    async fn carries(address: &str, from: &Endpoint, to: &Endpoint) -> bool {
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
        let echoed = async {
            let conn = from.connect(at, ALPN).await.ok()?;
            let (mut send, mut recv) = conn.open_bi().await.ok()?;
            send.write_all(&said).await.ok()?;
            send.finish().ok()?;
            let got = recv.read_to_end(1 << 20).await.ok()?;
            conn.close(0u32.into(), b"done");
            Some(got)
        }
        .await;
        server.abort();
        match echoed {
            Some(got) => {
                assert_eq!(got, said, "the relay changed what it carried");
                true
            }
            None => false,
        }
    }

    /// What `key` tells the relay: `fields`, signed as devices sign requests.
    /// When to sign what's told the relay: now, but each at a millisecond of its own, as the relay takes each `(by, at)`
    /// once, and a test tells it faster than a user would.
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

    /// A device is taken out (its key forgotten, its user signed out) only when the relay says so: a node that doesn't
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
    fn the_relay_carries_only_the_networks_endpoints() {
        let first = SecretKey::generate();
        let k = super::Workers::for_devices(vec![hex(&first)]);
        let address = k.address.replace("localhost", "127.0.0.1");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // The network's first device is on the list, an admin's, and a stranger isn't.
            let a = online(&address, &first, None).await.expect("the network's first device isn't let in");
            let stranger = SecretKey::generate();
            assert!(shut_out(&k, &stranger, None).await, "a stranger's key reached the relay");

            // A device it adds is let in, and the two reach each other through it.
            let phone = SecretKey::generate();
            assert_eq!(tell(&address, &first, json!({ "add": hex(&phone), "role": "client" })), 204);
            let b = online(&address, &phone, None).await.expect("a key the first device added isn't let in");
            assert!(reaches(&address, &a, &b).await, "two of the network's devices don't reach each other");

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
            assert!(shut_out(&k, &stranger, None).await, "a key taken out reached the relay");

            // A client adds only clients: a compute or a service only an admin's device or a compute adds, and a key's
            // role, once it's in, only an admin's changes.
            for role in ["compute", "service"] {
                assert_eq!(tell(&address, &phone, json!({ "add": hex(&SecretKey::generate()), "role": role })), 403, "a client added a {role}");
            }
            let machine = SecretKey::generate();
            assert_eq!(tell(&address, &first, json!({ "add": hex(&machine), "role": "compute" })), 204);
            assert_eq!(tell(&address, &machine, json!({ "add": hex(&SecretKey::generate()), "role": "compute" })), 204, "a compute didn't add the computer it paired");
            assert_eq!(tell(&address, &machine, json!({ "add": hex(&SecretKey::generate()), "role": "service" })), 204, "a compute didn't add its user's service");
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
            assert!(shut_out(&k, &SecretKey::generate(), Some(&code)).await, "a pairing code let in a second computer");
            assert!(reaches(&address, &c, &a).await, "a pairing code's computer doesn't reach the device that made it");
            assert!(forbidden(&k, &c, &b).await, "a pairing code's computer reached another device");
            // Ended, its computer is out.
            assert_eq!(tell(&address, &phone, json!({ "unpair": sha256(&code) })), 403, "a device ended another's pairing code");
            assert_eq!(tell(&address, &first, json!({ "unpair": sha256(&code) })), 204);
            assert!(cut_off(&k, &c).await, "an ended pairing code's computer still reached the relay");
            c.close().await;
            assert_eq!(upgrade_status(&address, &code), 403, "an ended pairing code let its computer in again");
            // Taking a device out takes out the devices it let in.
            let laptop = SecretKey::generate();
            assert_eq!(tell(&address, &phone, json!({ "add": hex(&laptop), "role": "client" })), 204);
            assert_eq!(tell(&address, &first, json!({ "remove": hex(&phone) })), 204);
            assert!(shut_out(&k, &laptop, None).await, "a key a removed device let in still reached the relay");
            assert!(cut_off(&k, &b).await, "a removed device's socket still reached the relay");
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

    /// A pairing code's computer and a session's endpoint, let in, are cut off as their code and certificate end, and
    /// don't get in again: the relay's clock is moved on (`TEST`), not waited for.
    #[test]
    fn what_ends_with_time_cuts_its_endpoint_off() {
        let (first, phone) = (SecretKey::generate(), SecretKey::generate());
        let k = super::Workers::for_devices(vec![hex(&first)]);
        let address = k.address.replace("localhost", "127.0.0.1");
        let signer = |k: &SecretKey| ed25519_dalek::SigningKey::from_bytes(&k.to_bytes());
        assert_eq!(tell(&address, &first, json!({ "add": hex(&phone), "role": "client" })), 204);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let a = online(&address, &first, None).await.expect("the network's first device isn't let in");
            let code = ticket();
            let lasts = super::now() + 600_000;
            assert_eq!(tell(&address, &first, json!({ "pairing": { "hash": sha256(&code), "until": lasts } })), 204);
            let d = online(&address, &SecretKey::generate(), Some(&code)).await.expect("a pairing code's computer isn't let in");
            let short = SecretKey::generate();
            let cert = one::keys::certify(&signer(&phone), short.public().as_bytes(), lasts, super::now());
            let brief = online(&address, &short, Some(&cert)).await.expect("a session isn't let in");
            assert!(reaches(&address, &d, &a).await, "a pairing code's computer doesn't reach the device that made it");
            assert!(reaches(&address, &brief, &a).await, "a session's endpoint doesn't reach the network's devices");
            assert!(!cut_off(&k, &d).await && !cut_off(&k, &brief).await);

            assert_eq!(testing(&k, json!({ "online": [id_of(&a)] })).await, json!([id_of(&a)]));
            testing(&k, json!({ "advance": 600_001 })).await;
            // A socket nothing's come from for that long, as a laptop whose lid closed leaves, isn't online.
            assert_eq!(testing(&k, json!({ "online": [id_of(&a)] })).await, json!([]));
            assert!(!cut_off(&k, &a).await, "a quiet socket was closed");
            assert!(cut_off(&k, &d).await, "an expired pairing code's computer is still let in");
            assert!(cut_off(&k, &brief).await, "an ended session is still let in");
            assert_eq!(upgrade_status(&address, &code), 403, "an expired pairing code let its computer in again");
            assert!(shut_out(&k, &short, Some(&cert)).await, "an ended session was let in again");
            for ep in [a, d, brief] {
                ep.close().await;
            }
        });
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
            // a connection open only once the test lets it.
            let got: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
            let serve = |key: &SecretKey, gate: Option<Arc<tokio::sync::Notify>>| {
                let (key, address, got) = (key.clone(), address.clone(), got.clone());
                async move {
                    let alpn = format!("{}/http/1", one::config::CLI).into_bytes();
                    let relay = RelayConfig::new(address.parse::<RelayUrl>().unwrap(), None);
                    let ep = Endpoint::builder(presets::Minimal).secret_key(key.clone()).alpns(vec![alpn]).clear_ip_transports().clear_address_lookup().relay_mode(RelayMode::Custom(relay.into())).bind().await.unwrap();
                    tokio::time::timeout(LET_IN, ep.online()).await.expect("a node isn't let in");
                    let serving = ep.clone();
                    tokio::spawn(async move {
                        while let Some(incoming) = serving.accept().await {
                            let (key, got, gate) = (hex(&key), got.clone(), gate.clone());
                            tokio::spawn(async move {
                                if let Some(gate) = gate {
                                    gate.notified().await;
                                }
                                let conn = incoming.await.ok()?;
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
            let gate = Arc::new(tokio::sync::Notify::new());
            let _nodes = (serve(&slow, Some(gate.clone())).await, serve(&quick, None).await);
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
            // The slow one's connection, raced, is kept as it opens: a request to it alone waits for that one, and only
            // it is handed that request's keys.
            gate.notify_one();
            client.set_nodes(&[*slow.public().as_bytes()]).unwrap();
            let answered = ask().await.expect("no node answered");
            assert_eq!(answered.node, *slow.public().as_bytes(), "the node asked wasn't the one asked");
            assert_eq!(*sealed.lock().unwrap(), [hex(&quick), hex(&slow)]);
            assert_eq!(got.lock().unwrap()[1..], [(hex(&slow), format!("sealed to {}", hex(&slow)))], "keys reached a node they weren't sealed to");
            client.close().await;
        });
    }

    #[test]
    fn a_device_that_reads_less_is_told_to_update_by_the_relay() {
        let first = SecretKey::generate();
        let k = super::Workers::for_devices(vec![hex(&first)]);
        let address = k.address.replace("localhost", "127.0.0.1");
        let phone = SecretKey::generate();
        let signer = |k: &SecretKey| ed25519_dalek::SigningKey::from_bytes(&k.to_bytes());
        let computes = |by: &SecretKey, fields: Value| super::post(&format!("{address}/relay/computes"), &[], one::keys::signed_request(&signer(by), "one/relay", fields, at()));
        assert_eq!(tell(&address, &first, json!({ "add": hex(&phone), "role": "client" })), 204);
        for older in [json!({ "computes": true }), json!({ "computes": true, "reads": one::database::READS - 1 })] {
            let (status, said) = computes(&phone, older);
            assert_eq!(status, 426, "a device that reads less than this build wasn't told to update");
            assert!(said["error"].as_str().is_some_and(|e| e.starts_with("Update this device")), "{said}");
        }
        assert_eq!(computes(&phone, json!({ "computes": true, "reads": one::database::READS })).0, 200);
        // A browser's session runs the code the address serves, the newest: it isn't asked.
        let session = SecretKey::generate();
        let cert = one::keys::certify(&signer(&phone), session.public().as_bytes(), super::now() + 60_000, super::now());
        assert_eq!(computes(&session, json!({ "computes": true, "certificate": cert })).0, 200);
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
            let a = online(&address, &first, None).await.expect("the network's first device isn't let in");
            assert_eq!(tell(&address, &first, json!({ "add": hex(&phone), "role": "client" })), 204);
            assert_eq!(tell(&address, &first, json!({ "add": hex(&SecretKey::generate()) })), 403, "an add that doesn't say its role");

            let session = SecretKey::generate();
            let cert = certify(&phone, &session, super::now() + 60_000);
            let s = online(&address, &session, Some(&cert)).await.expect("a session its device vouched for isn't let in");
            assert!(reaches(&address, &s, &a).await, "a session's endpoint doesn't reach the network's devices");
            // Its first nodes to ask: the computes, for it or a device on the list, and no one else.
            let asked = one::keys::signed_request(&signer(&phone), "one/relay", json!({ "computes": true, "reads": one::database::READS }), at());
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
            assert_eq!(computes(one::keys::signed_request(&signer(&stranger), "one/relay", json!({ "computes": true, "reads": one::database::READS }), at())).0, 403, "a stranger was told the computes");
            // When each was last here, a session's as the device that vouched for it: told the network's own devices
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
                assert!(shut_out(&k, &other, Some(&vouched)).await, "a session a compute or service vouched for got in");
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
            assert!(shut_out(&k, &theirs, Some(&foreign)).await, "a stranger's session got in with its proof");
            // Its own session, as one key's newest socket is the one it's reached at, and `s` is still connected.
            let fresh = SecretKey::generate();
            let shown = format!("{}.{}", certify(&phone, &fresh, super::now() + 60_000), proof(&fresh, super::now()));
            let proved = online(&address, &fresh, Some(&shown)).await.expect("a session with its proof isn't let in");
            assert!(reaches(&address, &proved, &a).await, "a session taken in with its proof doesn't reach the network's devices");
            proved.close().await;

            // Nothing for another endpoint, by a key not on the list, or once it has ended.
            assert!(shut_out(&k, &SecretKey::generate(), Some(&cert)).await, "a certificate let in an endpoint it doesn't name");
            assert!(shut_out(&k, &SecretKey::generate(), Some(&certify(&stranger, &stranger, super::now() + 60_000))).await, "a stranger vouched for itself");
            assert_eq!(upgrade_status(&address, &certify(&phone, &SecretKey::generate(), super::now() - 1)), 403, "an ended session was let in");

            // Its device taken out, it's out too.
            assert_eq!(tell(&address, &first, json!({ "remove": hex(&phone) })), 204);
            assert!(cut_off(&k, &s).await, "a removed device's session still reached the relay");
            s.close().await;
            assert!(shut_out(&k, &session, Some(&cert)).await, "a removed device's session got in again");
            assert_eq!(computes(ask(&session, &cert)).0, 403, "a removed device's session was told the computes");
            a.close().await;
        });
    }
}
