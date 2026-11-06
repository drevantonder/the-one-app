//! Where code in two languages must agree on keys, seals, who's who and what an app's screen may do, a test reads both
//! and checks they do, so neither can drift from the other. Each agreement is a test of its own.

use data_encoding::BASE64URL_NOPAD as B64U;
use one::keys;
use serde_json::{Value, json};

/// A workerd running `cloudflare/test/formats.js`, which does with the Workers' JavaScript what a test asks, for the test
/// to compare with what Rust does.
struct Formats {
    addr: String,
    child: std::process::Child,
    dir: std::path::PathBuf,
}

impl Formats {
    fn start() -> Self {
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("cloudflare");
        let mut files: Vec<(String, Vec<u8>)> = ["shared/crypto.js", "shared/token.js", "test/formats.js"].map(|p| (p.to_string(), std::fs::read(root.join(p)).unwrap())).to_vec();
        files.push(("screen/agents.js".into(), std::fs::read(root.join("../src/screen/settings/agents.js")).unwrap()));
        let dir = crate::temp("formats");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let served = one::cloudflare::Served { name: "formats".into(), module: one::cloudflare::module_from(files, "test/formats.js").unwrap(), modules: vec![], class: None, listen: Some(addr.clone()), bindings: vec![], assets: None, outbound: None };
        let child = one::cloudflare::start(&one::cloudflare::write_config(&dir, &[served]).unwrap(), vec![("formats".into(), listener)]).unwrap();
        Formats { addr, child, dir }
    }

    /// `body`'s `op` run in JavaScript: → what it gave.
    fn js(&self, body: Value) -> Value {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        agent.post(format!("http://{}", self.addr)).send_json(body).unwrap().body_mut().read_json().unwrap()
    }
}

impl Drop for Formats {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn b(v: &Value) -> Vec<u8> {
    B64U.decode(v.as_str().unwrap().as_bytes()).unwrap()
}

/// The Workers (JavaScript, `cloudflare/shared/crypto.js`) and nodes each open what the other seals: to key pairs and
/// under secrets.
#[test]
fn the_workers_and_nodes_seal_keys_for_each_other() {
    let formats = Formats::start();
    let js = |body: Value| formats.js(body);

    // Sealed to a key pair (the recovery key's twin): Rust opens what JavaScript sealed to it (push's notes).
    let scope: [u8; 32] = rand::random();
    let (secret, public) = keys::twin(&rand::random());
    let sealed = js(json!({ "op": "sealTo", "to": B64U.encode(&public), "plain": B64U.encode(&scope) }));
    assert_eq!(keys::open_sealed(&secret, &b(&sealed["sealed"])).unwrap(), scope, "Rust opens what JavaScript sealed to it");

    // Sealed under a secret, with the id it's kept by.
    let under = js(json!({ "op": "sealUnder", "secret": "a refresh token, as text", "plain": B64U.encode(&scope) }));
    assert_eq!(under["id"], keys::id_of(b"a refresh token, as text"), "a copy's id");
    assert_eq!(keys::open_under(b"a refresh token, as text", &b(&under["sealed"])).unwrap(), scope, "Rust opens a copy JavaScript sealed");
    let sealed = keys::seal_under(b"a code from the recovery key", &scope);
    let opened = js(json!({ "op": "openUnder", "secret": "a code from the recovery key", "sealed": B64U.encode(&sealed) }));
    assert_eq!(b(&opened["plain"]), scope, "JavaScript opens a copy Rust sealed");
}

/// Devices' keys (`src/keys.rs`): canonical JSON, signed requests both ways, join codes, invite proofs,
/// passkeys' device keys and a session endpoint's certificate.
#[test]
fn a_device_signs_and_proves_as_the_nodes_read_it() {
    let formats = Formats::start();
    let js = |body: Value| formats.js(body);

    let seed: [u8; 32] = rand::random();
    let signer = ed25519_dalek::SigningKey::from_bytes(&seed);
    let fields = json!({ "read": ["user-0123456789abcdef/notes/main/1"], "n": 3, "z": { "b": "é\n\"", "a": [] } });
    let request = keys::signed_request(&signer, "one/urls", fields.clone(), 1_000_000);
    let node = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let to = keys::Endpoint::of(&node).seal;
    let device = ed25519_dalek::SigningKey::from_bytes(&rand::random()).verifying_key().to_bytes();
    let device_seal: [u8; 32] = rand::random();
    let prf: [u8; 32] = rand::random();
    let out = js(json!({
        "op": "keys", "seed": B64U.encode(&seed), "domain": "one/urls", "fields": fields, "at": 1_000_000,
        "request": request, "now": 1_000_000 + keys::FRESH,
        "pair": { "key": data_encoding::HEXLOWER.encode(&device), "seal": data_encoding::HEXLOWER.encode(&device_seal) },
        "secret": B64U.encode(b"an invite's secret"), "prf": B64U.encode(&prf),
    }));
    assert_eq!(out["canonical"].as_str().unwrap().as_bytes(), keys::canonical(&fields), "canonical JSON");
    assert_eq!(out["signed"], request, "the same request signed the same, Ed25519 being deterministic");
    assert_eq!(out["checked"], json!(data_encoding::HEXLOWER.encode(signer.verifying_key().as_bytes())), "JavaScript checks a request Rust signed");
    assert!(keys::check_request("one/urls", &out["signed"], 1_000_000).is_ok(), "Rust checks one JavaScript signed");
    assert_eq!(out["joinCode"], keys::join_code(&keys::Endpoint { key: device, seal: device_seal }));
    assert_eq!(out["proof"], keys::invite_proof(b"an invite's secret", &keys::Endpoint { key: device, seal: device_seal }));
    let asking = keys::Endpoint { key: device, seal: device_seal };
    assert_eq!(out["nonce"], one::sign_in_proof::nonce(&device_seal, &asking), "the OIDC nonce sign-in asks for a device in its network");
    assert_eq!(b(&out["challenge"]), one::sign_in_proof::challenge(&device_seal, &asking, 1_800_000_000_123), "and the challenge its passkey signs");
    let (passkey, seal_secret) = keys::passkey_keys(&prf);
    assert_eq!(out["passkey"]["sign"], data_encoding::HEXLOWER.encode(passkey.verifying_key().as_bytes()), "a passkey's signing key");
    assert_eq!(b(&out["passkey"]["seal"]), seal_secret, "and the secret sealed to");
    assert_eq!(out["passkey"]["sealPublic"], data_encoding::HEXLOWER.encode(&keys::public_of(&seal_secret)), "and its public half");
    assert_eq!(to, keys::twin_public(&node.verifying_key().to_bytes()).unwrap(), "a Rust endpoint seals to its key's twin");

    // A session endpoint's certificate: made the same, and each takes and refuses the same ones.
    let endpoint: [u8; 32] = rand::random();
    let at = 1_700_000_000_000u64;
    let made = keys::certify(&signer, &endpoint, at + keys::SESSION, at);
    let other = keys::signed_request(&signer, "one/relay", json!({ "endpoint": data_encoding::HEXLOWER.encode(&endpoint), "until": at + 1 }), at);
    let mut tampered: Value = serde_json::from_slice(&B64U.decode(made.as_bytes()).unwrap()).unwrap();
    tampered["until"] = json!(at + 2);
    let check = [
        made.clone(),
        keys::certify(&signer, &endpoint, at + keys::SESSION + 1, at),
        keys::certify(&signer, &endpoint, at + 10, at),
        keys::certify(&signer, &endpoint, at + keys::SESSION, at + keys::FRESH + 200),
        // Signed by a clock a second fast, for a day from then: longer than a day from now.
        keys::certify(&signer, &endpoint, at + 1_000 + keys::SESSION, at + 1_000),
        B64U.encode(&keys::canonical(&other)),
        B64U.encode(&keys::canonical(&tampered)),
        "not base64url!".into(),
    ];
    let now = at + 100;
    let out = js(json!({ "op": "certificates", "seed": B64U.encode(&seed), "endpoint": data_encoding::HEXLOWER.encode(&endpoint), "until": at + keys::SESSION, "at": at, "check": check, "now": now }));
    assert_eq!(out["made"], json!(made), "the same certificate made the same");
    for (cert, js) in check.iter().zip(out["checked"].as_array().unwrap()) {
        let rust = keys::check_certificate(cert, now);
        assert_eq!(js.get("error").is_none(), rust.is_ok(), "{cert}: Rust {rust:?}, JavaScript {js}");
        if let Ok(s) = rust {
            assert_eq!(js, &json!({ "by": data_encoding::HEXLOWER.encode(&s.by), "endpoint": data_encoding::HEXLOWER.encode(&s.endpoint), "until": s.until }));
        }
    }
    assert!(out["checked"].as_array().unwrap().iter().skip(1).all(|c| c.get("error").is_some()), "only the first is good");
}

/// Who's who as steps (`src/database/steps.rs`), as a browser reads it: the node's key sealed under an epoch, the
/// freeze the recovery key signed and a step a browser signed, sealed as a node
/// puts them in the bucket and read from it as the URL signer's `get` gives them (`cloudflare/web/net-worker.js`),
/// opened and applied by the client's keyring; and from a Welcome, sign-in's word passed on as JavaScript reads it.
#[test]
fn a_browser_reads_whos_who_as_a_node_puts_it_in_the_bucket() -> anyhow::Result<()> {
    let formats = Formats::start();
    let js = |body: Value| formats.js(body);

    use ed25519_dalek::SigningKey;
    use one::database::steps;
    use one::database::steps::Step;
    use one::database::scopes::{Kind, Scope};
    use one::database::{content, scopes};
    use one::keys::{Endpoint, Hash};
    use one::database::steps::Reader;
    const ANDRE: &str = "a000000000000001";
    let (mac, browser, recovery) = (SigningKey::from_bytes(&rand::random()), SigningKey::from_bytes(&rand::random()), SigningKey::from_bytes(&rand::random()));
    let (epoch, app): ([u8; 32], [u8; 32]) = (rand::random(), rand::random());
    let root = Hash::from_bytes(rand::random());
    let start = content::first("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &recovery.verifying_key().to_bytes(), &one::keys::epoch_hash(&epoch));
    let first = steps::first_freeze(&recovery, root, &start, ANDRE, 0).unwrap();
    let listed = steps::sign(&mac, &Scope::Global, 2, ANDRE, 1, 0, content::device(&Endpoint::of(&browser), ANDRE, "browser", &[Kind::Client], None)).unwrap();
    let signed = steps::sign(&browser, &Scope::Global, 3, ANDRE, 1, 5, content::setting("greeting", Some("hi"))).unwrap();
    let sealed_app = one::keys::seal_app_key(&epoch, scopes::APP, &app);
    let keys: one::database::stored::Keys = [(1, one::keys::database_key(&app, one::database::scopes::APP, one::database::steps::NAME))].into();
    let prefix = steps::prefix("global");
    let sealed = |at: u64, step: &Step, copy: Option<&[u8]>| {
        let name = one::database::stored::step_name(&prefix, at);
        json!({ "at": at, "sealed": B64U.encode(&one::database::steps::seal(&keys, &name, &steps::encode(step, copy)).unwrap()) })
    };
    let (s1, s2, s3) = (sealed(1, &first.step, first.copy.as_deref()), sealed(2, &listed, None), sealed(3, &signed, None));
    let junk = json!({ "at": 4, "sealed": B64U.encode(&one::database::steps::seal(&[(1, rand::random())].into(), &one::database::stored::step_name(&prefix, 4), b"not a step").unwrap()) });
    let epochs = std::collections::BTreeMap::from([(1, epoch)]);
    let step_keys: one::database::stored::Keys = [(1, one::keys::open_database_key(&epoch, scopes::APP, steps::NAME, &sealed_app)?)].into();
    let add = |r: &mut Reader, given: &serde_json::Value| r.add(&steps::Sealed::from_json(given), &step_keys, Some(&epochs), 0, |_, _, _| Ok(()));
    let frozen = |r: &Reader| r.truth().is_some();
    // A freeze an admin's device signed is nothing.
    let mut client = Reader::new(Some(root));
    let planted = steps::sign(&mac, &Scope::Global, 1, ANDRE, 1, 0, first.step.content.clone()).unwrap();
    add(&mut client, &json!({ "steps": [sealed(1, &planted, first.copy.as_deref())] })).unwrap();
    assert!(!frozen(&client) && client.at() == 1, "it's passed over");
    let mut client = Reader::new(Some(root));
    let read = json!({ "steps": [s1, s2, s3, junk] });
    add(&mut client, &read).unwrap();
    assert!(frozen(&client) && client.at() == 4, "frozen, the browser's step taken and the junk passed over");
    let who = &client.truth().unwrap().db;
    assert_eq!(who.setting("greeting")?.as_deref(), Some("hi"), "the step a browser signed counts");
    // Another network's freeze: nothing to a device of this one.
    let mut elsewhere = Reader::new(Some(Hash::from_bytes(rand::random())));
    add(&mut elsewhere, &read).unwrap();
    assert!(!frozen(&elsewhere), "a freeze that isn't the network it joined");
    // Welcomed: from the freeze and the steps up to the Welcome's, as a node passes them on with `?welcome=`.
    let welcome = steps::Welcome { freeze: 1, at: 3, hash: one::database::stored::state_hash(who.conn()).unwrap() };
    let said = js(json!({ "op": "joining", "said": [[200, { "approved": true, "root": root.to_hex(), "welcome": welcome.to_string() }]] }));
    let given = json!({ "welcome": said["said"][0]["welcome"], "freeze": s1, "at": 3, "steps": [s2, s3] });
    let mut new = Reader::new(Some(root));
    add(&mut new, &given).unwrap();
    assert!(frozen(&new) && new.at() == 3, "a new device starts where it was welcomed");
    assert_eq!(one::database::stored::state_hash(new.truth().unwrap().db.conn()).unwrap(), welcome.hash);
    let mut other = given.clone();
    other["welcome"] = json!(steps::Welcome { hash: "0".repeat(64), ..welcome.clone() }.to_string());
    let mut new = Reader::new(Some(root));
    assert!(add(&mut new, &other).is_err() && !frozen(&new), "nor from a Welcome its steps don't come to: it reads from the freeze, which it wasn't given");
    // Junk at the first position, under an epoch no one made, and anything after it: passed over, not waited for.
    let name = one::database::stored::step_name(&prefix, 1);
    let junk = json!({ "at": 1, "sealed": B64U.encode(&one::keys::seal(0xFFFF_FFFF, &rand::random(), &name, b"junk")) });
    let mut fresh = Reader::new(Some(root));
    add(&mut fresh, &json!({ "steps": [junk, sealed(2, &first.step, first.copy.as_deref())] })).unwrap();
    assert_eq!(fresh.at(), 2, "a browser reads past junk");
    Ok(())
}

/// What sign-in's word on a device's request to be let in means (`keys::waited`): the web client reads it as native apps
/// and the CLI do.
#[test]
fn a_client_reads_sign_ins_word_as_native_apps_do() {
    let formats = Formats::start();
    let js = |body: Value| formats.js(body);

    use one::keys::{Waited, waited};
    let root = "e".repeat(64);
    let said: Vec<(u16, Value)> = vec![
        (401, json!({})),
        (404, json!({})),
        (200, json!({ "approved": true, "root": root })),
        (200, json!({ "approved": true, "root": root.to_uppercase() })),
        (200, json!({ "approved": true, "root": 7 })),
        (200, json!({ "approved": true })),
        (200, json!({ "approved": true, "root": root, "welcome": format!("3.250.{}", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!("3.3.{}", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!("4.3.{}", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!("03.250.{}", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!("0.250.{}", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!("+3.250.{}", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!("3.9007199254740992.{}", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!("3.9007199254740991.{}", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!("3.250.{}", "A".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!(" 3.250.{}", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": format!("3.250.{}.1", "a".repeat(64)) })),
        (200, json!({ "approved": true, "welcome": 3 })),
        (200, json!({ "approved": false })),
        (200, json!({ "approved": "yes" })),
        (204, json!({ "approved": true })),
        (500, json!({ "approved": true })),
        (0, Value::Null),
    ];
    let out = js(json!({ "op": "joining", "said": said.iter().map(|(status, body)| json!([status, body])).collect::<Vec<_>>() }));
    for ((status, body), js) in said.iter().zip(out["said"].as_array().unwrap()) {
        let rust = match waited(*status, body) {
            Waited::SignedOut => json!({ "signedOut": true }),
            Waited::Gone => json!({ "gone": true }),
            Waited::Approved { root, welcome } => {
                let mut v = json!({ "approved": true });
                if let Some(g) = root {
                    v["root"] = json!(g);
                }
                if let Some(w) = welcome {
                    v["welcome"] = json!(w);
                }
                v
            }
            Waited::Waiting => json!({}),
        };
        assert_eq!(js, &rust, "{status} {body}");
    }
    assert_eq!(waited(200, &said[2].1), Waited::Approved { root: Some(root.clone()), welcome: None }, "an approval names the network it's in");
    let welcomed = waited(200, &said[6].1);
    assert!(matches!(&welcomed, Waited::Approved { welcome: Some(w), .. } if one::database::steps::Welcome::parse(w).is_some_and(|w| (w.freeze, w.at, w.copy()) == (3, 250, Some(200)))), "and where who's who was read to: {welcomed:?}");
    assert_eq!(said.iter().filter(|(s, b)| matches!(waited(*s, b), Waited::Approved { welcome: Some(_), .. })).count(), 3, "only a welcome that's one is taken");
}

/// The bucket's names as the Workers (the URL signer, the web client) make them, the refusal of a device that reads less,
/// and the codes answers carry.
#[test]
fn the_workers_name_the_buckets_objects_as_nodes_do() {
    let formats = Formats::start();
    let js = |body: Value| formats.js(body);

    use one::database::stored::{copy_name, newest_name, step_name, tidied_name};
    let prefix = one::database::steps::prefix("user:abc");
    let out = js(json!({ "op": "names", "names": [[prefix, 1], [prefix, 4_000_000_000_000u64]], "prefixes": [["user:abc", "tally", "main"], ["global", "node", "scope"]], "yours": "your One App network" }));
    let rust = |p: &str, n: u64| json!([step_name(p, n), copy_name(p, n), tidied_name(p), newest_name(p)]);
    assert_eq!(out["names"], json!([rust(&prefix, 1), rust(&prefix, 4_000_000_000_000)]), "the names of a database's objects in the bucket");
    assert_eq!(out["prefixes"], json!(["user-abc/tally/main", "global/node/scope"]), "where an app's database of a scope is");
    assert_eq!(out["refusal"], json!(one::protocol::update_refusal("your One App network")), "what a device that reads less is told");
    use one::protocol::code;
    assert_eq!(out["codes"], json!({ "update": code::UPDATE, "notLetIn": code::NOT_LET_IN, "refused": code::REFUSED, "notSignedIn": code::NOT_SIGNED_IN, "restarting": code::RESTARTING, "notAdded": code::NOT_ADDED, "taken": code::TAKEN, "replayed": code::REPLAYED, "already": code::ALREADY, "notListed": code::NOT_LISTED, "pairingUsed": code::PAIRING_USED, "badPairingCode": code::BAD_PAIRING_CODE, "alreadyInNetwork": code::ALREADY_IN_NETWORK, "unreachable": code::UNREACHABLE, "noClient": code::NO_CLIENT, "notWaiting": code::NOT_WAITING, "limit": code::LIMIT, "noProfile": code::NO_PROFILE }), "the codes answers carry");
}

/// The Settings screens say who agents are in an item's apps (`src/screen/settings/agents.js`) by the name the node
/// gives them.
#[test]
fn the_screens_name_agents_as_the_vault_does() {
    let formats = Formats::start();
    assert_eq!(formats.js(json!({ "op": "agents" }))["name"], json!(one::database::scopes::AGENTS));
}

/// Push (src/push.rs, cloudflare/push/): a device registers its address with the push Worker as Rust signs it, a node's
/// send of a note sealed to that device goes out from the Worker as the browser's push service takes it (RFC 8291,
/// VAPID with the key the Workers made, the note as base64url text), and what arrives opens, only with the device's key, as the note the node
/// sealed. Only the network's devices register or send, each request once.
#[test]
fn a_note_reaches_only_the_device_it_was_sealed_to() {
    use data_encoding::{BASE64, BASE64URL_NOPAD as B64U, HEXLOWER};
    use ed25519_dalek::SigningKey;
    use one::push::{self, Note};

    let node = SigningKey::from_bytes(&rand::random());
    let phone = SigningKey::from_bytes(&rand::random());
    let k = crate::cloudflare::Workers::for_devices([&node, &phone].map(|key| HEXLOWER.encode(key.verifying_key().as_bytes())).to_vec());
    let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let post = |path: &str, body: serde_json::Value| crate::cloudflare::post(&format!("{}{path}", k.address), &[], body);

    // A browser's subscription: its key pair and secret, as the push service would hand them to the Worker.
    let subscription = crate::cloudflare::Subscription::new();
    let endpoint = "https://fcm.googleapis.com/fcm/send/the-phone";
    let address = subscription.address(endpoint);
    let phone_seal = one::keys::Endpoint::of(&phone).seal;
    let register = push::register_request(&phone, &phone_seal, &address, None, now()).unwrap();
    assert_eq!(post("/push/register", register.clone()), (200, json!({ "registered": true })), "the network's device registers");
    assert_eq!(post("/push/register", register).0, 403, "a request is taken once");
    let stranger = SigningKey::from_bytes(&rand::random());
    assert_eq!(post("/push/register", push::register_request(&stranger, &phone_seal, &address, None, now()).unwrap()).0, 403, "only the network's devices register");

    // The node seals a note to the phone and sends it.
    let note = Note { id: "0123456789abcdef".into(), title: "Tally".into(), body: "Sam counted".into(), app: "tally".into(), screen: "main".into(), ..Default::default() };
    let sealed = push::seal_note(&node, &phone_seal, &note, now()).unwrap();
    let unregistered = HEXLOWER.encode(node.verifying_key().as_bytes());
    let to = HEXLOWER.encode(phone.verifying_key().as_bytes());
    let fields = json!({ "route": "POST /push/send", "notes": [{ "to": to, "note": B64U.encode(&sealed) }, { "to": unregistered, "note": B64U.encode(&sealed) }] });
    let (status, said) = post("/push/send", one::keys::signed_request(&node, push::DOMAIN, fields, now()));
    assert_eq!((status, &said["to"][&to], &said["to"][&unregistered]), (200, &json!({ "sent": true }), &json!({ "none": true })), "it went to the device that registered: {said}");

    // What the browser's push service got: VAPID with the network's key, and the note, encrypted to the subscription.
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let pushes: serde_json::Value = agent.get(format!("{}/pushes", k.sites)).call().unwrap().body_mut().read_json().unwrap();
    assert_eq!(pushes.as_array().unwrap().len(), 1);
    let got = &pushes[0];
    assert_eq!((got["url"].as_str(), got["headers"]["content-encoding"].as_str()), (Some(endpoint), Some("aes128gcm")));
    let vapid: serde_json::Value = agent.get(format!("{}/push/key", k.address)).call().unwrap().body_mut().read_json().unwrap();
    assert!(got["headers"]["authorization"].as_str().unwrap().ends_with(&format!(", k={}", vapid["vapid"].as_str().unwrap())), "signed with the network's VAPID key");
    let plain = subscription.open(&BASE64.decode(got["body"].as_str().unwrap().as_bytes()).unwrap());
    let plain = B64U.decode(&plain).expect("Web Push carries the note as base64url text");
    assert_eq!(plain, sealed, "the Worker carried the sealed note as it was");
    let (opened, by) = push::open_note(&one::keys::seal_secret(&phone), &plain).unwrap();
    assert_eq!((opened, by), (note, Some(HEXLOWER.encode(node.verifying_key().as_bytes()))), "the phone opens the note, from the node");
    assert!(push::open_note(&one::keys::seal_secret(&node), &plain).is_err(), "no other device opens it");

    // A device that stops registering gets nothing more.
    assert_eq!(post("/push/unregister", push::unregister_request(&phone, now())).0, 200);
    let fields = json!({ "route": "POST /push/send", "notes": [{ "to": to, "note": B64U.encode(&sealed) }] });
    assert_eq!(post("/push/send", one::keys::signed_request(&node, push::DOMAIN, fields, now())).1["to"][&to], json!({ "none": true }));
}

/// The browser's service worker opens a note as `push::open_note` does (`openNote` in `cloudflare/shared/crypto.js`): one a
/// device signed shows with who signed it, one the network's own Workers send is unsigned, and one whose signature is
/// wrong, or isn't signed by who it says, is refused.
#[test]
fn the_service_worker_opens_notes_as_rust_does() {
    use data_encoding::HEXLOWER;
    use ed25519_dalek::SigningKey;
    use one::push::{self, Note};

    let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let formats = Formats::start();
    let open = |secret: &[u8; 32], public: &[u8; 32], plain: &[u8]| {
        let sealed = keys::seal_to(public, plain).unwrap();
        formats.js(json!({ "op": "openNote", "secret": B64U.encode(secret), "public": B64U.encode(public), "sealed": B64U.encode(&sealed) }))["note"].clone()
    };
    let (secret, public) = keys::twin(&rand::random());
    let node = SigningKey::from_bytes(&rand::random());
    let by = HEXLOWER.encode(node.verifying_key().as_bytes());
    let note = Note { id: "0123456789abcdef".into(), title: "Café \"Ü\"".into(), body: "é…".into(), app: "tally".into(), screen: "main".into(), params: Some(serde_json::from_value(json!({ "b": 1, "a": ["x"] })).unwrap()), ..Default::default() };

    let sealed = push::seal_note(&node, &public, &note, now()).unwrap();
    let got = formats.js(json!({ "op": "openNote", "secret": B64U.encode(&secret), "public": B64U.encode(&public), "sealed": B64U.encode(&sealed) }))["note"].clone();
    let (opened, from) = push::open_note(&secret, &sealed).unwrap();
    assert_eq!((got["title"].as_str(), got["by"].as_str()), (Some(opened.title.as_str()), from.as_deref()), "a note a device signed: {got}");
    assert_eq!(got["by"], json!(by));

    let signed = keys::signed_request(&node, push::SIGNED_NOTE, serde_json::to_value(&note).unwrap(), now());
    let unsigned = open(&secret, &public, &serde_json::to_vec(&note).unwrap());
    assert_eq!(unsigned["title"], json!(note.title), "a note from the network's Workers is unsigned: {unsigned}");
    let mut forged = signed.clone();
    forged["title"] = json!("Your device needs approving");
    assert!(open(&secret, &public, &serde_json::to_vec(&forged).unwrap())["error"].is_string(), "a note changed after it was signed");
    let mut other = signed;
    other["by"] = json!(HEXLOWER.encode(SigningKey::from_bytes(&rand::random()).verifying_key().as_bytes()));
    assert!(open(&secret, &public, &serde_json::to_vec(&other).unwrap())["error"].is_string(), "a note signed by another than it says");
    assert!(push::open_note(&secret, &keys::seal_to(&public, &serde_json::to_vec(&forged).unwrap()).unwrap()).is_err(), "Rust refuses the same");
}

/// A service answers its node in the words of `service/wire.json`, in whatever language it's written: registering what's
/// about its machine, taking calls and answering with `ok`, `error` or `elsewhere`, registering again when its node
/// forgets it, always with the token in its home. The node here is the real registry (`one::services`) behind its routes.
/// Rust's Shell and Go's WhatsApp run as programs; the Swift helpers are held to the same file by `swift test` in
/// `service/swift`, which needs a Mac.
mod service_protocol {
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::extract::{Path as UrlPath, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use one::state::Me;
    use one::services::{Call, Elsewhere, Services};
    use serde_json::{Value, json};

    const TOKEN: &str = "a token in the service's home";

    struct Node {
        services: Services,
        /// How many times a service registered, and the requests that came without the token.
        registered: tokio::sync::watch::Sender<usize>,
        refused: Mutex<Vec<String>>,
    }

    type Shared = State<Arc<Node>>;

    fn answered(node: &Node, headers: &HeaderMap, what: &str) -> Option<Response> {
        if headers.get("x-one-local-token").and_then(|t| t.to_str().ok()) == Some(TOKEN) {
            return None;
        }
        node.refused.lock().unwrap().push(what.to_string());
        Some((StatusCode::UNAUTHORIZED, Json(json!({ "error": "send the local token" }))).into_response())
    }

    async fn register(State(node): Shared, headers: HeaderMap, UrlPath(app): UrlPath<String>, Json(describe): Json<Value>) -> Response {
        if let Some(refused) = answered(&node, &headers, "register") {
            return refused;
        }
        node.services.register(&app, describe);
        node.registered.send_modify(|n| *n += 1);
        Json(json!({})).into_response()
    }

    async fn next(State(node): Shared, headers: HeaderMap, UrlPath(app): UrlPath<String>) -> Response {
        if let Some(refused) = answered(&node, &headers, "poll") {
            return refused;
        }
        match node.services.next(&app).await {
            Ok(Some(call)) => Json(call).into_response(),
            Ok(None) => StatusCode::NO_CONTENT.into_response(),
            Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{err:#}") }))).into_response(),
        }
    }

    async fn reply(State(node): Shared, headers: HeaderMap, UrlPath((app, id)): UrlPath<(String, u64)>, Json(body): Json<Value>) -> Response {
        if let Some(refused) = answered(&node, &headers, "reply") {
            return refused;
        }
        match node.services.reply(&app, id, body) {
            Ok(()) => Json(json!({})).into_response(),
            Err(err) => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("{err:#}") }))).into_response(),
        }
    }

    /// A node's registry behind its routes, on a runtime of its own so that it can stop, as a node does, with its calls
    /// waiting. It serves on a copy of `port`, which the test keeps open, so a node started again has the same address
    /// and no other test's listener takes it in between.
    fn start_node(port: &std::net::TcpListener) -> (Arc<Node>, tokio::runtime::Runtime) {
        let node = Arc::new(Node { services: Services::default(), registered: tokio::sync::watch::channel(0).0, refused: Mutex::new(Vec::new()) });
        let listener = port.try_clone().unwrap();
        listener.set_nonblocking(true).unwrap();
        let app = Router::new()
            .route("/v1/services/{app}", post(register))
            .route("/v1/services/{app}/calls", get(next))
            .route("/v1/services/{app}/calls/{id}", post(reply))
            .with_state(node.clone());
        let server = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        server.spawn(async move { axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app).await });
        (node, server)
    }

    /// Waits for a service to register with `node`.
    async fn registered(node: &Node, name: &str) {
        let mut registered = node.registered.subscribe();
        tokio::time::timeout(Duration::from_secs(60), registered.wait_for(|n| *n >= 1)).await.unwrap_or_else(|_| panic!("{name} never registered")).unwrap();
    }

    struct Running(Child);

    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn me() -> Me {
        serde_json::from_value(json!({ "key": "k1", "user": "u1", "name": "mac", "role": null, "user_name": "Ann", "handle": "ann", "agent": "Claude Code" })).unwrap()
    }

    fn wire() -> Value {
        serde_json::from_slice(&std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../service/wire.json")).unwrap()).unwrap()
    }

    /// A service's program, its way of being asked, and what each answer to the calls below should be.
    struct Case {
        name: &'static str,
        program: PathBuf,
        ok: (&'static str, Value),
        error: (&'static str, Value, &'static str),
        elsewhere: Option<(&'static str, Value)>,
        prefer: Value,
    }

    fn rust_and_go() -> Vec<Case> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        // Go builds the way a release does; with none installed, the build says how to get it.
        let platforms = one::build::PLATFORMS.iter().filter(|p| p.target == one::config::TARGET);
        let out = crate::temp("go-service");
        let built = one::build::build_service(&root.join("apps/whatsapp"), "contract-test", platforms, &out).unwrap_or_else(|err| panic!("WhatsApp's service is in Go, which these tests hold to the protocol: {err:#}")).unwrap();
        let go = Command::new(one::build::go()).args(["test", "./..."]).current_dir(root.join("apps/whatsapp/service")).output().unwrap_or_else(|err| panic!("Go isn't installed ({err}): see go.dev/doc/install, or `mise use -g go`"));
        assert!(go.status.success(), "WhatsApp's own tests (go test) failed:\n{}{}", String::from_utf8_lossy(&go.stdout), String::from_utf8_lossy(&go.stderr));
        // Shell is built beside the program, which `cargo test` builds for its own tests but not the services.
        assert!(Command::new(one::build::cargo()).args(["build", "-q", "-p", "one-shell"]).current_dir(&root).status().unwrap().success(), "Shell's service didn't build");
        vec![
            Case {
                name: "shell",
                program: Path::new(env!("CARGO_BIN_EXE_one")).with_file_name("one-shell"),
                ok: ("run", json!({ "command": "printf hi" })),
                error: ("run", json!({}), "give a command"),
                elsewhere: None,
                prefer: json!({ "run": "here" }),
            },
            Case {
                name: "whatsapp",
                program: built.made[0].1.clone(),
                ok: ("_pairing", json!({})),
                error: ("accounts", json!(["not an object"]), "cannot unmarshal"),
                elsewhere: Some(("chats", json!({}))),
                prefer: Value::Null,
            },
        ]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn services_in_rust_and_go_speak_the_protocol_their_node_reads() {
        let fixture = wire();
        // The node sends a call as the file says, and replies as the file says reach it as what they are.
        let sent = Call { id: 7, function: "events".into(), input: fixture["call"]["input"].clone(), caller: me(), from: Some("calendar".into()) };
        assert_eq!(serde_json::to_value(&sent).unwrap(), fixture["call"], "wire.json's call is what the node sends");
        let allowed: Vec<&str> = fixture["registers"].as_array().unwrap().iter().filter_map(Value::as_str).collect();

        for case in rust_and_go() {
            let name = case.name;
            let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = port.local_addr().unwrap();
            let (mut node, mut server) = start_node(&port);

            let home = crate::temp(&format!("service-{name}"));
            std::fs::write(home.join("local-token"), format!("{TOKEN}\n")).unwrap();
            let data = home.join("data");
            let _running = Running(
                Command::new(&case.program)
                    .env("ONE_ADDR", addr.to_string())
                    .env("ONE_HOME", &home)
                    .env(format!("{}_DATA", name.to_uppercase()), &data)
                    .env("HOME", &home)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap_or_else(|err| panic!("{name}'s program ({}): {err}", case.program.display())),
            );

            // It says only what's about its machine.
            registered(&node, name).await;
            let said = node.services.describe().pop().unwrap();
            let fields: Vec<&str> = said.as_object().unwrap().keys().map(String::as_str).filter(|k| !matches!(*k, "name" | "runs")).collect();
            assert!(fields.iter().all(|k| allowed.contains(k)), "{name} registered {fields:?}: wire.json allows {allowed:?}");
            assert!(said["version"].is_string(), "{name} says its version");
            assert_eq!(said["prefer"], case.prefer, "{name}'s functions about its own machine");

            // It answers a call, a refusal and, in Go, one that belongs on another device.
            async fn ask(node: &Node, name: &str, function: &str, input: Value) -> anyhow::Result<one::services::Answer> {
                node.services.call(name, function, input, me(), None).await
            }
            let (function, input) = case.ok.clone();
            assert!(ask(&node, name, function, input).await.is_ok(), "{name} answers {function}");
            let (function, input, why) = case.error.clone();
            let err = ask(&node, name, function, input).await.unwrap_err();
            assert!(err.downcast_ref::<Elsewhere>().is_none() && format!("{err:#}").contains(why), "{name} answers a call it can't run with its error: {err:#}");
            if let Some((function, input)) = case.elsewhere.clone() {
                let err = ask(&node, name, function, input).await.unwrap_err();
                assert!(err.downcast_ref::<Elsewhere>().is_some(), "{name} says a call goes on to another device: {err:#}");
            }
            if name == "shell" {
                let out = ask(&node, name, "run", json!({ "command": "printf hi" })).await.unwrap();
                assert_eq!(out.ok["stdout"], "hi", "{name} reads the call's input");
            }

            assert!(node.refused.lock().unwrap().is_empty(), "{name} sent without its token: {:?}", node.refused.lock().unwrap());

            // It registers again with a node that starts again, which forgot it.
            tokio::task::block_in_place(|| server.shutdown_timeout(Duration::from_secs(10)));
            (node, server) = start_node(&port);
            registered(&node, name).await;
            assert!(node.services.has(name), "{name} is listed again");
            let (function, input) = case.ok.clone();
            assert!(ask(&node, name, function, input).await.is_ok(), "{name} answers the node that started again");
            assert!(node.refused.lock().unwrap().is_empty(), "{name} sent without its token: {:?}", node.refused.lock().unwrap());
            tokio::task::block_in_place(|| server.shutdown_timeout(Duration::from_secs(10)));
        }
    }
}

/// What a widget's tree can hold, which an app makes and every client draws by itself: the iPhone's, Android's and
/// Windows' widgets and the screens' own. A type or a coloured tone one names and another doesn't shows as nothing, or plain
/// text, on some devices only.
#[test]
fn every_renderer_of_a_widgets_tree_draws_the_same_types_and_tones() {
    const TYPES: &[&str] = &["col", "row", "text", "time", "stat", "gauge", "progress", "spark", "chart", "list", "item", "image", "icon", "dot", "link", "divider", "button", "toggle"];
    const TONES: &[&str] = &["muted", "warn", "bad"];
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
    let all = |dir: &str, ext: &str| {
        let mut text = String::new();
        for f in std::fs::read_dir(root.join(dir)).unwrap().flatten().filter(|f| f.path().extension().is_some_and(|e| e == ext)) {
            text += &std::fs::read_to_string(f.path()).unwrap();
        }
        text
    };
    let renderers = [
        ("the iPhone's", read("client/native/iphone/Widget/WidgetTree.swift")),
        ("Android's", all("client/native/android/app/src/main/kotlin/app/client/widget", "kt")),
        ("Windows'", read("client/native/windows/WidgetCard.cs")),
        ("the screens'", read("src/screen/ui/src/components/widget.tsx")),
    ];
    let mut missing = Vec::new();
    for (who, source) in &renderers {
        for name in TYPES.iter().chain(TONES) {
            // Quoted, as each draws a type or a tone by its name.
            if !source.contains(&format!("\"{name}\"")) && !source.contains(&format!("{name}:")) {
                missing.push(format!("{who} widgets don't name `{name}`"));
            }
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}
