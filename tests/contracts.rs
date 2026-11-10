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
        let mut files: Vec<(String, Vec<u8>)> = ["shared/crypto.js", "shared/token.js", "shared/http.js", "telemetry/worker.js", "telemetry/usage.js", "test/formats.js"].map(|p| (p.to_string(), std::fs::read(root.join(p)).unwrap())).to_vec();
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

/// What an earlier build sealed (`tests/sealed/`), which the Workers read: copies under a token, and notes.
#[test]
fn the_workers_open_what_an_earlier_build_sealed() {
    let formats = Formats::start();
    for w in crate::sealed::fixtures() {
        for s in w["seal_under"].as_array().unwrap() {
            let opened = formats.js(json!({ "op": "openUnder", "secret": s["secret"], "sealed": s["sealed"] }));
            assert_eq!(opened["plain"], s["plain"], "{}", s["what"]);
        }
        let n = &w["note"];
        let hex = |v: &Value| B64U.encode(&data_encoding::HEXLOWER.decode(v.as_str().unwrap().as_bytes()).unwrap());
        let opened = formats.js(json!({ "op": "openNote", "secret": hex(&n["secret"]), "public": hex(&n["public"]), "sealed": n["sealed"] }));
        assert_eq!(opened["note"], n["note"], "a note");
    }
}

/// Devices' keys (`src/keys.rs`): canonical JSON, signed requests both ways, join codes,
/// and a session endpoint's certificate.
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
    let out = js(json!({
        "op": "keys", "seed": B64U.encode(&seed), "domain": "one/urls", "fields": fields, "at": 1_000_000,
        "request": request, "now": 1_000_000 + keys::FRESH,
        "pair": { "key": data_encoding::HEXLOWER.encode(&device), "seal": data_encoding::HEXLOWER.encode(&device_seal) },
    }));
    assert_eq!(out["canonical"].as_str().unwrap().as_bytes(), keys::canonical(&fields), "canonical JSON");
    assert_eq!(out["signed"], request, "the same request signed the same, Ed25519 being deterministic");
    assert_eq!(out["checked"], json!(data_encoding::HEXLOWER.encode(signer.verifying_key().as_bytes())), "JavaScript checks a request Rust signed");
    assert!(keys::check_request("one/urls", &out["signed"], 1_000_000).is_ok(), "Rust checks one JavaScript signed");
    assert_eq!(out["joinCode"], keys::join_code(&keys::Endpoint { key: device, seal: device_seal }));
    let asking = keys::Endpoint { key: device, seal: device_seal };
    assert_eq!(out["nonce"], one::onboarding::sign_in_proof::nonce(&device_seal, &asking), "the OIDC nonce sign-in asks for a device in its network");
    assert_eq!(b(&out["challenge"]), one::onboarding::sign_in_proof::challenge(&device_seal, &asking, 1_800_000_000_123), "and the challenge its passkey signs");
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
    let keys: one::database::stored::Keys = [(1, one::keys::database_key(&app, one::database::scopes::APP, one::database::scopes::CoreDb::Scope.name()))].into();
    let prefix = steps::prefix("global");
    let sealed = |at: u64, step: &Step, copy: Option<&[u8]>| {
        let name = one::database::stored::step_name(&prefix, at);
        json!({ "at": at, "sealed": B64U.encode(&one::database::steps::seal(&keys, &name, &steps::encode(step, copy)).unwrap()) })
    };
    let (s1, s2, s3) = (sealed(1, &first.step, first.copy.as_deref()), sealed(2, &listed, None), sealed(3, &signed, None));
    let junk = json!({ "at": 4, "sealed": B64U.encode(&one::database::steps::seal(&[(1, rand::random())].into(), &one::database::stored::step_name(&prefix, 4), b"not a step").unwrap()) });
    let epochs = std::collections::BTreeMap::from([(1, epoch)]);
    let step_keys: one::database::stored::Keys = [(1, one::keys::open_database_key(&epoch, scopes::APP, scopes::CoreDb::Scope.name(), &sealed_app)?)].into();
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
    let keys::Waited::Approved { welcome: Some(said), .. } = keys::waited(200, &json!({ "approved": true, "root": root.to_hex(), "welcome": welcome.to_string() })) else { panic!("an approval's welcome is taken") };
    let given = json!({ "welcome": said, "freeze": s1, "at": 3, "steps": [s2, s3] });
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

/// What sign-in's word on a device's request to be let in means (`keys::waited`), which every client acts on
/// (onet/src/join.rs): an approval names only a network and a welcome that are one.
#[test]
fn a_client_takes_only_a_network_and_a_welcome_that_are_one() {
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
    assert_eq!(waited(200, &said[2].1), Waited::Approved { root: Some(root.clone()), welcome: None }, "an approval names the network it's in");
    let welcomed = waited(200, &said[6].1);
    assert!(matches!(&welcomed, Waited::Approved { welcome: Some(w), .. } if one::database::steps::Welcome::parse(w).is_some_and(|w| (w.freeze, w.at, w.copy()) == (3, 250, Some(200)))), "and where who's who was read to: {welcomed:?}");
    assert_eq!(said.iter().filter(|(s, b)| matches!(waited(*s, b), Waited::Approved { welcome: Some(_), .. })).count(), 3, "only a welcome that's one is taken");
}

/// The bucket's names as the Workers (the URL signer, the web client) make them, the refusal of a device that reads less,
/// and the codes answers carry.
#[test]
fn auth_and_devices_find_a_providers_keys_at_the_same_place() {
    use one::onboarding::sign_in_proof::{Network, discovery};
    let formats = Formats::start();
    for issuer in ["https://login.example", "https://login.example/", "https://login.example/tenant/v2.0"] {
        let network = Network::new("https://one.example", [1; 32], Some(&format!("{issuer} ours")));
        assert_eq!(formats.js(json!({ "op": "discoveryUrl", "issuer": issuer }))["url"].as_str(), discovery(issuer, &network).as_deref(), "{issuer}");
    }
}

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
    assert_eq!(out["codes"], json!({ "update": code::UPDATE, "notLetIn": code::NOT_LET_IN, "refused": code::REFUSED, "notThisMachine": code::NOT_THIS_MACHINE, "notSignedIn": code::NOT_SIGNED_IN, "restarting": code::RESTARTING, "stopped": code::STOPPED, "notAdded": code::NOT_ADDED, "appDeleted": code::APP_DELETED, "taken": code::TAKEN, "replayed": code::REPLAYED, "already": code::ALREADY, "notListed": code::NOT_LISTED, "pairingUsed": code::PAIRING_USED, "badPairingCode": code::BAD_PAIRING_CODE, "alreadyInNetwork": code::ALREADY_IN_NETWORK, "unreachable": code::UNREACHABLE }), "the codes answers carry");
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

/// Auth's pages drawn by hand (`src/onboarding/render.js`) call the network what this build does (`config::YOURS`, from
/// build.rs), from the same config.toml.
#[test]
fn auth_pages_drawn_by_hand_name_the_network_as_the_build_does() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let dir = crate::temp("drawn");
    let out = std::process::Command::new(one::build::bun().unwrap())
        .arg(root.join("src/onboarding/render.js"))
        .arg(root.join(option_env!("CONFIG").unwrap_or("config.toml")))
        .arg(&dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(std::fs::read_to_string(dir.join("invite.html")).unwrap().contains(&format!("Join {}", one::config::NAME)));
}

/// A row the telemetry Worker lets through has exactly the columns Basin's stream was made with: one that doesn't is taken
/// and dropped in silence, and a stream can't change once it's made.
#[test]
fn telemetrys_tables_are_the_ones_basin_is_set_up_with() {
    let tables = Formats::start().js(json!({ "op": "telemetryTables" }));
    let ours: Value = one::cloudflare::basin::TABLES.iter().map(|(table, columns)| (table.to_string(), json!(columns.iter().map(|(name, _, required)| json!([name, *required])).collect::<Vec<_>>()))).collect::<serde_json::Map<_, _>>().into();
    assert_eq!(tables, ours);
}

/// The Insights screen and the `insights` tool add a histogram's P95 the same way: JavaScript's in the screen, Rust's for
/// agents.
#[test]
fn insights_on_the_screen_and_for_agents_agree_on_a_p95() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let rows = serde_json::json!([{ "buckets": [0, 20, 20, 10, 10, 5, 5, 10, 10, 0, 5, 5, 3, 0, 0, 2] }, { "buckets": [0, 0, 0, 0, 0, 0, 1, 2, 3] }, {}]);
    let script = format!("const {{ quantile }} = await import({:?}); console.log(JSON.stringify([0, 0.5, 0.95, 1].map((q) => quantile({rows}, q))))", root.join("src/screen/settings/quantile.js").display());
    let out = std::process::Command::new(one::build::bun().unwrap()).args(["-e", &script]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let screen: Vec<f64> = serde_json::from_slice(&out.stdout).unwrap();
    let rows: Vec<&serde_json::Value> = rows.as_array().unwrap().iter().collect();
    let agents: Vec<f64> = [0.0, 0.5, 0.95, 1.0].iter().map(|q| one::insights::view::quantile(&rows, *q).unwrap()).collect();
    assert_eq!(screen, agents);
    let worker = Formats::start().js(json!({ "op": "telemetryBounds" }));
    let bounds: Vec<f64> = one::insights::BOUNDS.iter().map(|b| *b as f64).chain([60000.0]).collect();
    assert_eq!(bounds.as_slice(), one::insights::view::BOUNDS.as_slice(), "a histogram's bounds are the same on a device and where they're read");
    assert_eq!(worker, json!(one::insights::BOUNDS), "and in the telemetry Worker");
}

/// A service answers its node in the words of `service/wire.json`, in whatever language it's written: started as its
/// endpoint starts it, registering what's about its machine, taking calls and answering with `ok`, `error` or
/// `elsewhere`, registering again when its endpoint forgets it, always with the token in its home. What it talks to is
/// the endpoint's own registry and routes (`one::services::Registry`). Rust's Shell and Go's WhatsApp run as programs;
/// the Swift helpers are held to the same file by `swift test` in `service/swift`, which needs a Mac.
mod service_protocol {
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;
    use std::time::Duration;

    use one::services::{Call, Elsewhere, Registry};
    use one::state::Me;
    use serde_json::{Value, json};

    const TOKEN: &str = "a token in the service's home";

    /// A service's registry behind its routes, on a runtime of its own so that it can stop, as an endpoint does, with
    /// its calls waiting. It serves on a copy of `port`, which the test keeps open, so one started again has the same
    /// address and no other test's listener takes it in between.
    fn start(name: &str, port: &std::net::TcpListener) -> (Arc<Registry>, tokio::runtime::Runtime) {
        let registry = Registry::new(name, TOKEN);
        let listener = port.try_clone().unwrap();
        listener.set_nonblocking(true).unwrap();
        let routes = registry.routes();
        let server = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        server.spawn(async move { axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), routes).await });
        (registry, server)
    }

    /// Waits for a service to register.
    async fn registered(registry: &Registry, name: &str) {
        tokio::time::timeout(Duration::from_secs(60), registry.services().registered(name)).await.unwrap_or_else(|_| panic!("{name} never registered"));
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
        /// A call it answers, and a field of its answer (a JSON pointer) with what it says there.
        ok: (&'static str, Value, &'static str, Value),
        /// What broke, and what's the caller's to change: each a call, and what it says of it.
        error: (&'static str, Value, &'static str),
        refused: (&'static str, Value, &'static str),
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
        assert!(one::build::cargo().args(["build", "-q", "-p", "one-shell"]).current_dir(&root).status().unwrap().success(), "Shell's service didn't build");
        vec![
            Case {
                name: "shell",
                program: Path::new(env!("CARGO_BIN_EXE_one")).with_file_name("one-shell"),
                ok: ("run", json!({ "command": "printf hi" }), "/stdout", json!("hi")),
                error: ("nope", json!({}), "has no function"),
                refused: ("run", json!({}), "give a command"),
                elsewhere: None,
                prefer: json!({ "run": "here" }),
            },
            Case {
                name: "whatsapp",
                program: built.made[0].1.clone(),
                ok: ("_pairing", json!({}), "", json!({})),
                error: ("_take", json!({}), "names no move"),
                refused: ("accounts", json!(["not an object"]), "cannot unmarshal"),
                elsewhere: Some(("chats", json!({}))),
                prefer: Value::Null,
            },
        ]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn services_in_rust_and_go_speak_the_protocol_their_node_reads() {
        let fixture = wire();
        // The node sends a call as the file says, and replies as the file says reach it as what they are.
        let sent = Call { id: 7, function: "events".into(), input: fixture["call"]["input"].clone(), caller: me(), from: Some("calendar".into()), traceparent: fixture["call"]["traceparent"].as_str().map(str::to_string) };
        assert_eq!(serde_json::to_value(&sent).unwrap(), fixture["call"], "wire.json's call is what the node sends");
        let allowed: Vec<&str> = fixture["registers"].as_array().unwrap().iter().filter_map(Value::as_str).collect();

        for case in rust_and_go() {
            let name = case.name;
            let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = port.local_addr().unwrap();
            let (mut registry, mut server) = start(name, &port);

            let home = crate::temp(&format!("service-{name}"));
            std::fs::write(home.join(one::api::LOCAL_TOKEN), format!("{TOKEN}\n")).unwrap();
            let data = home.join("data");
            let _running = Running(
                Command::new(&case.program)
                    .envs(one::services::program_env(name, addr, &home, &data))
                    .env("HOME", &home)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap_or_else(|err| panic!("{name}'s program ({}): {err}", case.program.display())),
            );

            // It says only what's about its machine.
            registered(&registry, name).await;
            let said = registry.services().describe().pop().unwrap();
            let fields: Vec<&str> = said.as_object().unwrap().keys().map(String::as_str).filter(|k| !matches!(*k, "name" | "runs")).collect();
            assert!(fields.iter().all(|k| allowed.contains(k)), "{name} registered {fields:?}: wire.json allows {allowed:?}");
            assert!(said["version"].is_string(), "{name} says its version");
            assert_eq!(said["prefer"], case.prefer, "{name}'s functions about its own machine");

            // It answers a call, a refusal and, in Go, one that belongs on another device.
            async fn ask(registry: &Registry, name: &str, function: &str, input: Value) -> anyhow::Result<one::services::Answer> {
                registry.services().call(name, function, input, me(), None, None).await
            }
            let (function, input, field, says) = case.ok.clone();
            let out = ask(&registry, name, function, input.clone()).await.unwrap_or_else(|err| panic!("{name} answers {function}: {err:#}"));
            assert_eq!(out.ok.pointer(field), Some(&says), "{name} reads the call's input and answers it");
            for ((function, input, why), code) in [(case.error.clone(), None), (case.refused.clone(), Some("refused"))] {
                let err = ask(&registry, name, function, input).await.unwrap_err();
                assert!(err.downcast_ref::<Elsewhere>().is_none() && format!("{err:#}").contains(why), "{name} answers {function} with why: {err:#}");
                assert_eq!(one::protocol::Coded::of(&err).as_deref(), code, "{name} says what kind of error it is, as wire.json's replies do");
            }
            if let Some((function, input)) = case.elsewhere.clone() {
                let err = ask(&registry, name, function, input).await.unwrap_err();
                assert!(err.downcast_ref::<Elsewhere>().is_some(), "{name} says a call goes on to another device: {err:#}");
            }

            // It registers again with an endpoint that starts again, which forgot it.
            tokio::task::block_in_place(|| server.shutdown_timeout(Duration::from_secs(10)));
            (registry, server) = start(name, &port);
            registered(&registry, name).await;
            let (function, input, ..) = case.ok.clone();
            assert!(ask(&registry, name, function, input).await.is_ok(), "{name} answers the endpoint that started again");
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

/// A widget too slow to draw beside the rest of its page comes marked `late`, from either node, and is then asked for
/// alone with `kind=<app>.<widget>`: a client that doesn't take it in shows "Still drawing." for good, and a node that
/// doesn't send it holds back the whole page again.
#[test]
fn both_nodes_mark_a_late_widget_and_every_client_asks_for_it_alone() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
    let mut missing = Vec::new();
    let mut names = |who: &str, source: &str, wanted: &[&str]| {
        for name in wanted.iter().filter(|name| !source.contains(**name)) {
            missing.push(format!("{who} doesn't name `{name}`"));
        }
    };
    names("the node's widgets", &read("src/node/screens.rs"), &["[\"late\"] = json!(true)"]);
    names("the node's route", &read("src/node/widgets/says.rs"), &["kind: Option<String>"]);
    names("the Cloudflare node", &read("cloudflare/node/worker.js"), &["late: true", "searchParams.get(\"kind\")", one::protocol::path::OPENAPI]);
    names("the pages", &read("src/screen/home/lib/model.js"), &["w.late", "&kind="]);
    names("the native widgets", &read("client/native/src/widgets.rs"), &["w[\"late\"]", "&kind="]);
    names("a chat's view", &read("src/mcp/view.html"), &["w.late", "widgets_show"]);
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

/// Every `state.<field>` the screens, the web client and the Workers read of `GET /api/state` is a field the node says
/// (`one::state::State`): a field renamed or dropped there shows as a screen that reads nothing, which no one sees fail.
/// The sample has a value in every field, so a field added to the Rust is added here (it won't compile otherwise).
#[test]
fn what_the_screens_read_of_the_state_is_what_the_node_says() {
    use one::database::scopes::{Kind, Role};
    use one::state::*;
    let part = || Part { name: "apps".into(), on: true, title: Some("Apps".into()), fixed: true, doing: Some("Starting".into()), problem: Some("It stopped".into()), sandbox: Some("why".into()), user: true };
    let sample = State {
        config: Config { name: "The One App".into(), cli: "one".into(), store: "apps".into(), install: Some("curl".into()) },
        me: Viewer { key: "k".into(), name: "n".into(), user: "u".into() },
        device: Answering { key: "k".into(), name: "n".into(), ready: true, guide: Some("g".into()) },
        user: Some(User { id: "u".into(), name: "n".into(), role: Role::Admin }),
        network: Some(Network {
            name: "n".into(),
            root: Some("r".into()),
            user: "u".into(),
            role: Some(Role::Admin),
            recovery: Some("r".into()),
            epochs: [("global".to_string(), Epoch { n: 1, hashes: Default::default() })].into(),
            runs: Some(vec![]),
            recovery_used: Some(RecoveryUsed { time: 1, user: "u".into() }),
            recovery_set: 1,
            member: true,
            users: vec![NetworkUser { id: "u".into(), name: "n".into(), handle: "h".into(), role: Role::Admin, me: true, recovery_holder: false }],
            email_invites: Vec::new(),
            groups: vec![Group { id: "g".into(), name: "n".into(), handle: "h".into(), role: Some(Role::Admin), members: vec![Member { user: "u".into(), role: Role::Admin }], invited: vec![] }],
            devices: vec![Device {
                key: "k".into(),
                seal: "s".into(),
                name: "n".into(),
                user: "u".into(),
                roles: [Kind::Compute].into(),
                since: 1,
                device: Some("d".into()),
                kind: "laptop".into(),
                priority: one::rank::Priority::Last,
                order: 0,
                serves_me: true,
                runs_for: vec![],
                off_services: vec![Switched { name: "n".into(), time: 1 }],
                me: true,
                now: DeviceNow {
                    here: true,
                    online: true,
                    far: true,
                    build: Some("b".into()),
                    target: Some("t".into()),
                    services: vec![],
                    installed_services: vec![InstalledService { name: "n".into(), title: "t".into(), icon: Some("i".into()), color: Some("c".into()), domain: Some("d".into()), image: Some("i".into()), image_kind: Some("k".into()), description: Some("d".into()), version: Some("v".into()), built_in: true, installed_by: Some("u".into()), installed: Some(1), users: vec![] }],
                    parts: Some(vec![part()]),
                    load: Some(Default::default()),
                    kind: None,
                    latest: Some("l".into()),
                    updating: Some("u".into()),
                },
            }],
            devices_at: 1,
            clients: vec![Client { key: "k".into(), seal: "s".into(), name: "n".into(), user: "u".into(), roles: [Kind::Client].into(), platform: None, passkey: Some("p".into()), device: Some("d".into()), since: 1, place: Some("p".into()), me: true, sign_in: Some(one::onboarding::sign_in_proof::Account::Passkey { credential: "c".into(), public_key: "k".into() }), linked: Vec::new() }],
            app_schedules: vec![AppSchedule { endpoint: "e".into(), scope: "s".into(), app: "a".into(), until: 1, by: "u".into(), keys: Default::default() }],
            steps: 1,
            settings: [("address".to_string(), "a".to_string())].into(),
            cloudflare_node: Some(CloudflareNode { priority: one::rank::Priority::Last, key: "k".into(), runs_for: vec![], parts: vec![part()] }),
        }),
        pages: vec![Page { scope: "user".into(), title: "You".into(), subtitle: "".into(), hidden: false }],
        vault_requests: 1,
        arrangements: Default::default(),
        backups: Some(json!({})),
        settings_items: vec![SettingsItem { name: "n".into(), title: "t".into(), icon: "i".into(), color: "c".into(), url: Some("u".into()), badge: Some(1), app: Some("a".into()) }],
        agent_prompt: "p".into(),
        client: Some(ClientHere { key: "k".into(), name: "n".into() }),
    };
    let said = serde_json::to_value(&sample).unwrap();
    assert_eq!(serde_json::from_value::<State>(said.clone()).map(|s| serde_json::to_value(s).unwrap()).unwrap(), said, "the state reads back as it's said");
    // The fields of each object the readers go into.
    let (user, me, device, config, client) = (&said["user"], &said["me"], &said["device"], &said["config"], &said["client"]);
    let names = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<std::collections::BTreeSet<_>>();
    let (top, network) = (names(&said), names(&said["network"]));
    let objects = [("user", names(user)), ("me", names(me)), ("device", names(device)), ("config", names(config)), ("client", names(client)), ("network", network.clone())];

    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let mut sources = vec![root.join("cloudflare/node/worker.js"), root.join("cloudflare/mcp/worker.js")];
    for dir in ["cloudflare/web", "src/screen/home", "src/screen/settings"] {
        let mut stack = vec![root.join(dir)];
        while let Some(dir) = stack.pop() {
            for f in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = f.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "js" || e == "jsx") {
                    sources.push(path);
                }
            }
        }
    }
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut unknown = Vec::new();
    for path in &sources {
        let source = std::fs::read_to_string(path).unwrap();
        for (at, _) in source.match_indices("state") {
            let (before, line_start) = (&source[..at], source[..at].rfind('\n').map_or(0, |n| n + 1));
            // A word of its own, not the browser's `history.state` nor in a comment.
            if before.chars().next_back().is_some_and(word) || before.ends_with("history.") || source[line_start..].trim_start().starts_with("//") {
                continue;
            }
            // `state.a`, `state?.a`, then `.b` for what's an object: a, b.
            let mut chain = Vec::new();
            let mut rest = &source[at + 5..];
            while chain.len() < 2 {
                let Some(r) = rest.strip_prefix("?.").or_else(|| rest.strip_prefix('.')) else { break };
                let len = r.find(|c: char| !word(c)).unwrap_or(r.len());
                if len == 0 {
                    break;
                }
                chain.push(&r[..len]);
                rest = &r[len..];
            }
            let line = source[..at].matches('\n').count() + 1;
            let Some(first) = chain.first() else { continue };
            if !top.contains(*first) {
                unknown.push(format!("{}:{line}: state.{first}", path.strip_prefix(root).unwrap().display()));
            } else if let (Some(second), Some((_, fields))) = (chain.get(1), objects.iter().find(|(n, _)| n == first)) {
                if !fields.contains(*second) {
                    unknown.push(format!("{}:{line}: state.{first}.{second}", path.strip_prefix(root).unwrap().display()));
                }
            }
        }
    }
    assert!(unknown.is_empty(), "these read what the node's state doesn't say (src/state.rs):\n{}", unknown.join("\n"));
}

/// Every path of the node's API that the Workers, the screens, the web client and the native apps ask is one of its routes
/// (`api::routes`) or one a device answers itself (`one::settings::answered`): a path misspelled or dropped from one
/// fails as a 404, which no one sees until it's used. Each is in the OpenAPI document too, but a client's keys and signed
/// steps, which it asks as itself, and what the document lists is what the node serves.
#[test]
fn every_path_asked_of_a_node_is_one_it_serves() {
    // Not an app's own requests (`/{app}/{*path}`), which take any path that isn't the API's.
    let routes: Vec<Vec<String>> = one::api::routes().who().iter().map(|(path, _)| path.trim_start_matches('/').split('/').map(str::to_string).collect::<Vec<_>>()).filter(|route| !route[0].starts_with('{')).collect();
    // `{app}` and `{*path}` take what's there; so does what a page or format fills in (`${id}`, `{}`).
    let matches = |routes: &[Vec<String>], asked: &[&str]| {
        routes.iter().any(|route| {
            let tail = route.last().is_some_and(|s| s.starts_with("{*"));
            (route.len() == asked.len() || (tail && asked.len() >= route.len() - 1)) && route.iter().zip(asked).all(|(r, a)| r.starts_with('{') || r == a || a.contains("${") || a.contains('{'))
        })
    };
    const UNLISTED: [&str; 7] = ["/api/keys", "/api/steps", "/api/steps/global", "/api/openapi.json", "/api/holds", "/api/holds/shared", "/api/holds/write"];
    let document = one::openapi::document(&[], true);
    for (path, who, _) in one::says::listed() {
        assert!(one::api::routes().who().iter().any(|(p, w)| *p == path && *w == who), "src/says.rs says {path} is asked by {who:?}, which no route of the node (src/api.rs) is");
    }
    let listed: Vec<Vec<String>> = document["paths"].as_object().unwrap().keys().map(|path| path.trim_start_matches('/').split('/').map(str::to_string).collect()).collect();
    for path in document["paths"].as_object().unwrap().keys() {
        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        assert!(matches(&routes, &segments) || one::settings::answered(path), "the OpenAPI document lists {path}, which the node doesn't serve");
    }
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let mut files = Vec::new();
    for dir in ["cloudflare", "src/screen", "onet/src", "client/native/src", "apps"] {
        let mut stack = vec![root.join(dir)];
        while let Some(dir) = stack.pop() {
            let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if matches!(name, "node_modules" | "dist" | "target" | "test" | "tests" | "relay" | "service") {
                continue;
            }
            for f in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = f.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| matches!(e.to_str(), Some("js" | "jsx" | "rs"))) {
                    files.push(path);
                }
            }
        }
    }
    let (mut unserved, mut unlisted) = (Vec::new(), Vec::new());
    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        for (n, line) in source.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for (at, _) in line.match_indices("/api/") {
                // A literal: the quote before it is its own start.
                let start = line[..at].chars().next_back();
                if !matches!(start, Some('"' | '\'' | '`')) {
                    continue;
                }
                let end = line[at..].find(['"', '\'', '`', '?', ' ', '\\']).map_or(line.len(), |e| at + e);
                let literal = &line[at..end];
                // Auth's, at the gateway, not a node's.
                if literal.starts_with("/api/auth") {
                    continue;
                }
                let asked: Vec<&str> = literal.trim_start_matches('/').split('/').collect();
                // A start of a path, which more is added to.
                let place = format!("{}:{}: {}", path.strip_prefix(root).unwrap().display(), n + 1, &line[at..end]);
                if !literal.ends_with('/') && !matches(&routes, &asked) && !one::settings::answered(literal) {
                    unserved.push(place.clone());
                }
                if !literal.ends_with('/') && !UNLISTED.contains(&literal) && !matches(&listed, &asked) {
                    unlisted.push(place);
                }
            }
        }
    }
    assert!(unserved.is_empty(), "these paths aren't routes of the node (src/api.rs):\n{}", unserved.join("\n"));
    assert!(unlisted.is_empty(), "these paths aren't in the OpenAPI document (src/openapi.rs): add the route to its feature's `says.rs` (src/says.rs):\n{}", unlisted.join("\n"));
}

/// What isn't an app's own request (`protocol::path::ROOTS`) is every first segment the API's routes have: a route under
/// another would be taken for an app's by every client that hands keys, and by git.
#[test]
fn the_apis_paths_are_never_taken_for_an_apps() {
    for (path, _) in one::api::routes().who() {
        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        if segments.len() > 1 && !segments[0].starts_with('{') {
            assert!(one::protocol::path::ROOTS.contains(&segments[0]), "{path} would be an app's: add {} to protocol::path::ROOTS", segments[0]);
            assert!(segments[..2] != ["api", "auth"], "{path}: /api/auth/ is auth's at the gateway, so no node would ever be asked it");
        }
    }
}

/// An app runs the same on a machine's node and on the Cloudflare node: the shim it runs in, asked as each node asks it
/// (over HTTP with its asks going out through the node, and by RPC with the node answering them), gives the same answer,
/// the same error and code, the same code on an error the node answers its ask with, and writes the node reads.
#[test]
fn an_app_answers_the_same_on_a_machine_and_on_the_cloudflare_node() {
    const APP: &str = r#"
        export default {
          name: "probe",
          uses: { apps: ["other"] },
          tools: {
            reads: { run: async (ctx) => ctx.user.query("SELECT 1 AS n") },
            writes: { run: async (ctx, input) => {
              ctx.user.exec("INSERT INTO notes (text) VALUES (?)", [input.text]);
              ctx.user.check("SELECT 1", [], "changed");
              ctx.user.forget("ab".repeat(32));
              ctx.cost({ model: "m", usd: 0.5, input: 3, output: 4 });
              await ctx.vault.create({ title: "T", fields: { user: { value: "ann" } }, hosts: ["x.com"], same: "user" });
              ctx.vault.update("v1", { title: "U" });
              ctx.vault.release("v1");
              ctx.add("other", "user", true, "linux");
              ctx.retire("other", "deleted");
              ctx.source("other", "https://example.com");
              ctx.service("other", "linux", true, "global");
              ctx.schedule("other", "daily", "user", false);
              return "wrote";
            } },
            fails: { run: async () => { throw Object.assign(new Error("sign in first"), { code: "not_signed_in" }); } },
            asks: { run: async (ctx) => {
              try {
                return await ctx.apps.other.anything();
              } catch (err) {
                return { error: err.message, code: err.code ?? null };
              }
            } },
          },
        };
    "#;
    let runner = format!(
        r#"
        import {{ RpcTarget, WorkerEntrypoint }} from "cloudflare:workers";
        const SHIM = {shim};
        const APP = {app};
        const CODE = {code};
        // The node, answering what the app asks the same however it asks.
        function answer(name, body) {{
          if (name === "query") return Response.json({{ ok: [{{ n: 1 }}] }});
          if (name === "vault.match") return Response.json({{ id: "v0" }});
          if (name === "call") return new Response("not yours to call", {{ status: 400, headers: {{ [CODE]: "refused" }} }});
          return new Response(`no ask ${{name}}`, {{ status: 400 }});
        }}
        // A machine's node: the app's asks reach it as requests to one.internal.
        export class Node extends WorkerEntrypoint {{
          async fetch(req) {{
            return answer(new URL(req.url).pathname.slice(1), await req.text());
          }}
        }}
        // The Cloudflare node: it hands the app this to ask.
        class Asks extends RpcTarget {{
          ask(name, body) {{
            return answer(name, body);
          }}
        }}
        export default {{
          async fetch(req, env) {{
            const app = env.LOADER.get("probe", () => ({{ compatibilityDate: {date}, compatibilityFlags: ["nodejs_als"], mainModule: "shim.js", modules: {{ "shim.js": SHIM, "app.js": APP }}, globalOutbound: env.NODE }}));
            const tool = new URL(req.url).searchParams.get("tool");
            const body = {{ tool, input: {{ text: "hi" }}, me: {{ user: "u1" }}, now: 1, scopes: [{{ id: "user:u1", name: "Ann" }}], read_only: false, token: "t" }};
            const machine = await (await app.getEntrypoint().fetch("http://app/call", {{ method: "POST", body: JSON.stringify(body) }})).json();
            const cloudflare = await app.getEntrypoint("Call").run("call", null, body, new Asks());
            return Response.json({{ machine, cloudflare }});
          }},
        }};
        "#,
        shim = json!(one::call::shim()),
        app = json!(APP),
        code = json!(one::protocol::header::CODE),
        date = json!(one::app::runtime::COMPATIBILITY_DATE),
    );
    let at = format!("127.0.0.1:{}", crate::cloudflare::port());
    let _running = crate::cloudflare::run(vec![one::cloudflare::Served {
        name: "runner".into(),
        module: runner.into_bytes(),
        modules: vec![],
        class: None,
        listen: Some(at.clone()),
        bindings: vec![("LOADER".into(), "loader:".into()), ("NODE".into(), "service:runner:Node".into())],
        assets: None,
        outbound: None,
    }]);
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let ask = |tool: &str| -> Value { serde_json::from_str(&agent.get(format!("http://{at}/?tool={tool}")).call().unwrap().body_mut().read_to_string().unwrap()).unwrap() };
    for (tool, expected) in [
        ("reads", json!({ "ok": [{ "n": 1 }], "appends": [], "later": false })),
        ("fails", json!({ "error": "sign in first", "code": "not_signed_in" })),
        ("asks", json!({ "ok": { "error": "not yours to call", "code": "refused" }, "appends": [], "later": false })),
    ] {
        let mut both = ask(tool);
        for side in ["machine", "cloudflare"] {
            if let Some(thrown) = both[side].as_object_mut().and_then(|o| o.remove("exception")) {
                assert_eq!(thrown["type"], "Error", "what's thrown, by type and where, on {side}");
                assert!(thrown["frames"].is_array(), "{thrown}");
            }
        }
        assert_eq!(both["machine"], expected, "{tool} on a machine");
        assert_eq!(both["cloudflare"], expected, "{tool} on the Cloudflare node");
    }
    let both = ask("writes");
    assert_eq!(both["machine"], both["cloudflare"], "the same writes on both");
    let appends = one::call::appends(&both["machine"]["appends"]).unwrap_or_else(|err| panic!("the node reads what the shim hands it: {err:#}\n{}", both["machine"]["appends"]));
    assert_eq!(appends.len(), 12, "every write the app made: {appends:?}");
}

/// What the shim asks the node is what `one::call::Ask` names, and the Cloudflare node answers exactly those it says it
/// does (a machine's node answers each, which the compiler holds it to).
#[test]
fn the_cloudflare_node_answers_the_asks_it_says_it_does() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let names: std::collections::BTreeMap<&str, bool> = one::call::Ask::names().collect();
    let quoted = |text: &str, before: &str| -> std::collections::BTreeSet<String> { text.split(before).skip(1).filter_map(|rest| Some(rest.split_once('"')?.0.to_string())).collect() };
    let shim = std::fs::read_to_string(root.join("src/app/runtime/shim.js")).unwrap();
    let asked: std::collections::BTreeSet<String> = quoted(&shim, "ask(\"").union(&quoted(&shim, "post(\"")).cloned().collect();
    let unknown: Vec<&String> = asked.iter().filter(|a| !names.contains_key(a.as_str())).collect();
    assert!(unknown.is_empty(), "the shim asks what src/call.rs doesn't name: {unknown:?}");
    let worker = std::fs::read_to_string(root.join("cloudflare/node/worker.js")).unwrap();
    let ask = worker.split("  async ask(name, body) {").nth(1).expect("the Cloudflare node's ForCall.ask").split("\n  }\n").next().unwrap();
    let answered = quoted(ask, "case \"");
    let said: std::collections::BTreeSet<String> = names.iter().filter(|(_, on)| **on).map(|(n, _)| n.to_string()).collect();
    assert_eq!(answered, said, "the asks the Cloudflare node answers, and those src/call.rs says it does");
}

/// A web bundle says the shell it needs by a number (`one::web::SHELL`): what the shell gives the page, its commands and
/// `window.__oneNative`, is pinned to each number, so changing them without raising it fails here, before a bundle that
/// needs them reaches an app that hasn't got them.
#[test]
fn what_the_shell_gives_the_page_is_pinned_to_its_number() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("client/native/src");
    let mut hash = blake3::Hasher::new();
    for file in ["commands.rs", "native.js"] {
        hash.update(&std::fs::read(root.join(file)).unwrap());
    }
    let hash = hash.finalize().to_hex().to_string();
    const PINNED: &[(u32, &str)] = &[(4, "c373913c50e9158b6ce2056b8ad061f3f5670cff5b9af88a9155f11b89f8d99b"), (3, "7ae08c822ecc7bc95c15dcd268d6bbaaadc6e014d550d40304bb25b7c3db8742"), (2, "2f046373196f89532b94512e4e9c5f197bb76f8c418dffdc717bde2658cb5248"), (1, "44f63548b850ab33724d60f9da788d1885423d02901add2710901371cc37064c")];
    let raise = format!("what the shell gives the page (client/native/src/commands.rs, native.js) changed: raise one::web::SHELL and pin ({}, \"{hash}\")", one::web::SHELL + 1);
    assert!(PINNED.iter().any(|(n, h)| *n == one::web::SHELL && *h == hash), "{raise}");
}

/// The iPhone's build scripts (`client/native/apple.sh`) make its bundle id as build.rs does (`config::IPHONE`): from the
/// same config.toml, which can name it.
#[test]
fn the_iphone_build_scripts_name_the_app_as_the_build_does() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let out = std::process::Command::new("bash")
        .args(["-c", ". client/native/apple.sh && echo $CONFIG_IPHONE"])
        .current_dir(root)
        .env("CONFIG", root.join(option_env!("CONFIG").unwrap_or("config.toml")))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), one::config::IPHONE);
}

/// What a screen's bridge reports of what threw is its type, where, and what it said: a frame stays only a location, however
/// a message is dressed (lines like a stack's, a URL's query on a frame), and what the node keeps of the message is cleaned.
#[test]
fn a_screens_error_crosses_from_the_bridge_and_what_it_said_is_cleaned() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let bridge = std::fs::read_to_string(root.join("src/screen/bridge.js")).unwrap();
    let from = bridge.find("  function described(err) {").expect("bridge.js says what threw");
    let body = &bridge[from..from + bridge[from..].find("\n  }\n").unwrap() + 4];
    let cases = serde_json::json!([
        { "message": "anna@example.org", "name": "TypeError", "stack": "TypeError: anna@example.org\n    at run (https://x.example/a.js?token=SECRET:12:3)\n    at b (about:srcdoc:2:5)" },
        { "message": "line one\n    at SECRET (leak.js:9:9)", "name": "Weird", "stack": "Error: line one\n    at SECRET (leak.js:9:9)\n    at real (app.js:1:2)" },
        { "message": "from safari", "name": "Error", "stack": "run@https://x.example/b.js?q=SECRET:4:5\n@about:srcdoc:6:7" },
        { "message": "in a screen", "name": "Error", "stack": "Error: in a screen\n    at explode (about:srcdoc:12:30)\n    at bridge (about:srcdoc:3:5)", "screen": { "name": "main", "at": "Error\n    at about:srcdoc:10:1" } },
    ]);
    let script = format!("{body}\nconsole.log(JSON.stringify({cases}.map((c) => {{ globalThis.__oneScreen = c.screen; return described(Object.assign(new Error(c.message), {{ name: c.name, stack: c.stack }})) }})))");
    let at = std::env::temp_dir().join(format!("bridge-thrown-{}.js", std::process::id()));
    std::fs::write(&at, script).unwrap();
    let out = std::process::Command::new(one::build::bun().unwrap()).arg(&at).output().unwrap();
    std::fs::remove_file(&at).unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let said = String::from_utf8(out.stdout).unwrap();
    let said: Vec<Value> = serde_json::from_str(&said).unwrap();
    let kept: Vec<_> = said.iter().map(|e| one::insights::Exception::from_json(e).unwrap()).collect();
    let stored = kept.iter().map(|e| one::insights::Attrs::default().exception(e).json().to_string()).collect::<String>();
    assert!(!stored.contains("anna") && !stored.contains("SECRET:") && !stored.contains("example"), "{stored}");
    assert!(kept.iter().all(|e| e.frames.iter().all(|f| !f.contains("SECRET") && !f.contains("leak"))), "{kept:?}");
    assert_eq!(one::insights::message::cleaned(kept[0].message.as_deref().unwrap()), "<email>");
    assert_eq!(kept[0].kind, "TypeError");
    assert_eq!(kept[0].frames, ["a.js:12:3", "srcdoc:2:5"]);
    assert_eq!(kept[1].kind, "Error", "a name of the program's own is `Error`");
    assert_eq!(kept[1].frames, ["app.js:1:2"]);
    assert_eq!(kept[2].frames, ["b.js:4:5", "srcdoc:6:7"]);
    assert_eq!(kept[3].frames, ["main.screen.js:3:30", "srcdoc:3:5"], "a frame in the screen's own module is a line of it, and its source map's to resolve; anything before it isn't");
}

/// Cloudflare's export of the Workers' traces is held to what a program's OTLP is on a device: nothing but a plain name, the
/// ids, timing and status of each span, and what an exception was, where, and its plain words; and it only ever keeps the
/// Workers that open nothing. Every request is counted, kept or not, a WebSocket message never, and a trace is kept whole
/// only if it failed, was slow, or is one in 20 of the requests.
#[test]
fn a_workers_traces_are_clamped_as_a_programs_are() {
    let formats = Formats::start();
    let traced: Vec<String> = serde_json::from_value(formats.js(json!({ "op": "telemetryTraced" }))).unwrap();
    assert_eq!(traced, one::insights::TRACED, "the telemetry Worker keeps the Workers the deploy exports, never its own");
    let start = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() * 1_000_000;
    let span = |name: &str, trace: char, status: i64, parent: Option<&str>| json!({ "name": name, "traceId": trace.to_string().repeat(32), "spanId": "1".repeat(16), "parentSpanId": parent, "startTimeUnixNano": start.to_string(), "endTimeUnixNano": (start + 500_000_000).to_string(), "status": { "code": status }, "kind": 2, "attributes": [{ "key": "url.full", "value": { "stringValue": "https://x/anna@example.org" } }, { "key": "http.response.status_code", "value": { "intValue": "500" } }] });
    let of = |worker: &str, spans: Vec<Value>| json!({ "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": worker } }] }, "scopeSpans": [{ "spans": spans }] });
    let mut failed = span("GET /users/anna@example.org", 'a', 2, None);
    failed["events"] = json!([{ "name": "exception", "attributes": [{ "key": "exception.type", "value": { "stringValue": "TypeError" } }, { "key": "exception.message", "value": { "stringValue": "internal error; reference = l1kprhha71iut801 for anna@example.org" } }, { "key": "exception.stacktrace", "value": { "stringValue": "    at async worker_default.ask (worker.js:474:18)" } }] }]);
    // Trace `0…` is one in 20; trace `d…` isn't.
    let request = json!({ "resourceSpans": [of("one-gateway", vec![failed, span("GET", 'd', 0, None), span("GET", '0', 0, None)]), of("one-relay", vec![span("hibernatableWebSocket", 'e', 0, None)]), of("one-telemetry", vec![span("fetch", 'c', 2, None)]), of("one-mcp", vec![span("fetch", 'b', 2, None)]), of("one-node", vec![span("fetch", 'b', 2, None)])] });
    let read = formats.js(json!({ "op": "telemetryWorkerSpans", "request": request, "cli": "one" }));
    let rows = read["spans"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "the gateway's failure and the one request in 20: not another request, a WebSocket message, nor what mcp, node or telemetry did: {rows:?}");
    let stored = rows[0].to_string();
    assert!(!stored.contains("anna") && !stored.contains("example") && !stored.contains("url.full") && !stored.contains("l1kp"), "{stored}");
    let attrs: Value = serde_json::from_str(rows[0]["attrs"].as_str().unwrap()).unwrap();
    assert_eq!((rows[0]["name"].as_str(), rows[0]["service"].as_str(), rows[0]["duration"].as_u64(), rows[0]["status"].as_str()), (Some("span"), Some("worker.gateway"), Some(500), Some("error")));
    assert_eq!((&attrs["error.type"], &attrs["http.response.status_code"], &attrs["one.sampled"]), (&json!(one::insights::exception_type("TypeError")), &json!(500), &json!(1)));
    assert_eq!((rows[0]["message"].as_str(), attrs["one.fingerprint"].as_str()), (Some("internal error; reference = <value> for <value>"), Some("TypeError|worker.js:474:18|")), "an exception's plain words, and what the Errors list groups it by");
    assert_eq!((rows[1]["trace"].as_str(), serde_json::from_str::<Value>(rows[1]["attrs"].as_str().unwrap()).unwrap()["one.sampled"].as_u64()), (Some("0".repeat(32).as_str()), Some(20)), "a sampled request stands for the 20 it's one of");
    let counted: Vec<(String, String, String, u64)> = read["metrics"].as_array().unwrap().iter().map(|m| { let a: Value = serde_json::from_str(m["attrs"].as_str().unwrap()).unwrap(); (m["service"].as_str().unwrap().to_string(), a["span.name"].as_str().unwrap().to_string(), a["status.code"].as_str().unwrap().to_string(), m["count"].as_u64().unwrap()) }).collect();
    let want = |s: &str, n: &str, st: &str, c: u64| (s.to_string(), n.to_string(), st.to_string(), c);
    assert_eq!(counted, [want("worker.gateway", "span", "STATUS_CODE_ERROR", 1), want("worker.gateway", "GET", "STATUS_CODE_UNSET", 2)], "every request is counted, kept or not, and a WebSocket message isn't one");
    let program = one::insights::otlp::items(&json!({ "resourceSpans": [{ "scopeSpans": [{ "spans": [span("GET /users/anna@example.org", 'a', 2, None)] }] }] }));
    assert_eq!((program[0].name.as_str(), program[0].ms, program[0].ok, program[0].status_code), ("span", 500, false, Some(500)), "the same span reads the same on a device");
    let named: Vec<String> = serde_json::from_value(formats.js(json!({ "op": "telemetryExceptions" }))).unwrap();
    assert!(named.iter().all(|n| one::insights::exception_type(n) == n) && one::insights::exception_type("Nope") == "Error", "a Worker's exception is named as a program's is: {named:?}");
}

/// What the network uses of Cloudflare is asked of its analytics by its own resources alone, grouped by nothing that's
/// about a request, and recorded as the service only an admin reads; and every price is of a metric that's recorded.
#[test]
fn cloudflare_usage_is_only_the_networks_and_priced() {
    let formats = Formats::start();
    let usage = json!({ "account": "a", "scripts": { "one-gateway": "one-gateway" }, "buckets": { "b": "b" }, "namespaces": { "n1": "one-relay_Relay" }, "pipelines": { "p1": "one_spans_pipeline" }, "warehouses": { "a_b": "b" } });
    let groups = |dimensions: Value, sum: Value| json!({ "dimensions": dimensions, "sum": sum, "max": { "payloadSize": 7, "metadataSize": 1 }, "count": 4 });
    let answer = json!({
        "workersInvocationsAdaptive": [groups(json!({ "scriptName": "one-gateway" }), json!({ "requests": 3, "cpuTimeUs": 2000 })), groups(json!({ "scriptName": "someone-elses" }), json!({ "requests": 9, "cpuTimeUs": 9 }))],
        "r2OperationsAdaptiveGroups": [groups(json!({ "bucketName": "b", "actionType": "GetObject" }), json!({ "requests": 5 })), groups(json!({ "bucketName": "b", "actionType": "PutObject" }), json!({ "requests": 2 }))],
        "r2StorageAdaptiveGroups": [groups(json!({ "bucketName": "b" }), json!({}))],
        "durableObjectsInvocationsAdaptiveGroups": [groups(json!({ "namespaceId": "n1", "type": "hibernation" }), json!({ "requests": 6 }))],
        "durableObjectsPeriodicGroups": [groups(json!({ "namespaceId": "n1" }), json!({ "activeTime": 2_000_000 }))],
        "pipelinesOperatorAdaptiveGroups": [groups(json!({ "pipelineId": "p1" }), json!({ "bytesIn": 10 }))],
        "pipelinesSinkAdaptiveGroups": [groups(json!({ "pipelineId": "p1" }), json!({ "uncompressedBytesWritten": 8 }))],
        "r2sqlOperationsAdaptiveGroups": [groups(json!({ "bucket": "b" }), json!({ "r2BytesRead": 30 }))],
        "r2CatalogDataOperationsAdaptiveGroups": [groups(json!({ "warehouseName": "a_b" }), json!({}))],
    });
    let said = formats.js(json!({ "op": "telemetryUsage", "usage": usage, "answer": answer }));
    let query = said["query"].as_str().unwrap();
    let resources = ["scriptName", "bucketName", "actionType", "namespaceId", "type", "pipelineId", "bucket", "warehouseName"];
    for grouped in query.split("dimensions { ").skip(1).flat_map(|d| d.split(" }").next().unwrap().split_whitespace()) {
        assert!(resources.contains(&grouped), "usage is grouped by the network's resources alone, never {grouped}");
    }
    assert!(query.contains(r#"scriptName_in: ["one-gateway"]"#) && query.contains(r#"namespaceId_in: ["n1"]"#), "only the network's own: {query}");
    let rows = said["rows"].as_array().unwrap();
    assert!(rows.iter().all(|r| r["service"] == one::insights::cost::CLOUDFLARE && !r.to_string().contains("someone-elses")), "{rows:?}");
    let attrs = |r: &Value| serde_json::from_str::<Value>(r["attrs"].as_str().unwrap()).unwrap();
    for price in one::insights::cost::PRICES {
        assert!(rows.iter().any(|r| r["name"] == price.metric && (price.counts)(&attrs(r)) > 0.0), "{} is priced as nothing that's recorded", price.metric);
    }
    let cpu = rows.iter().find(|r| r["name"] == "cloudflare.workers.cpu_time").unwrap();
    assert_eq!((cpu["sum"].as_f64(), cpu["unit"].as_str(), attrs(cpu)), (Some(2.0), Some("ms"), json!({ "cloudflare.product": "workers", "cloudflare.resource": "one-gateway" })));
}

/// The body a device sends `POST /relay/computes`, what it's answered, and the signed ask it hears are frozen, as the
/// signed record is: a device left behind must still be able to tell the relay what it's on and hear that it's been
/// asked to update, whatever else has changed.
#[test]
fn the_relay_keeps_its_word_to_a_device_left_behind() {
    use iroh::SecretKey;
    let signer = |k: &SecretKey| ed25519_dalek::SigningKey::from_bytes(&k.to_bytes());
    let hex = |k: &SecretKey| data_encoding::HEXLOWER.encode(k.public().as_bytes());
    let (admin, device) = (SecretKey::generate(), SecretKey::generate());
    let k = crate::cloudflare::Workers::for_devices(vec![hex(&admin)]);
    let address = k.address.replace("localhost", "127.0.0.1");
    let clock = std::sync::atomic::AtomicU64::new(one::node::now_ms());
    let at = || clock.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tell = |by: &SecretKey, fields: Value| crate::cloudflare::post(&format!("{address}/relay/tell"), &[], keys::signed_request(&signer(by), "one/relay", fields, at()));
    let computes = |by: &SecretKey, fields: Value| crate::cloudflare::post(&format!("{address}/relay/computes"), &[], keys::signed_request(&signer(by), "one/relay", fields, at()));
    let hex_device = hex(&device);

    assert_eq!(tell(&admin, json!({ "add": hex_device, "role": "client" })).0, 204);
    let version = "abcdef123456 1700000000 x86_64-unknown-linux-gnu";
    let (status, said) = computes(&device, json!({ "computes": true, "reads": one::database::READS, "version": version }));
    assert_eq!((status, said.as_object().unwrap().keys().cloned().collect::<Vec<_>>()), (200, ["computes", "online", "versions", "workers"].map(String::from).to_vec()));
    assert_eq!(said["versions"][&hex_device], version);
    // What a deploy says of the Workers: the device they're deployed from, and each part's version in a version's words.
    assert_eq!(said["workers"], json!({ "home": hex(&admin), "workers": crate::cloudflare::WORKERS_ARE, "web": "bbbbbbbbbbbb 1700000000 web", "node": null }));

    // The ask is left whole, signed by the asker, and heard once, even by a device too old to be told anything else.
    let ask = keys::signed_request(&signer(&admin), "one/relay", json!({ "update": hex_device }), at());
    assert_eq!(crate::cloudflare::post(&format!("{address}/relay/tell"), &[], ask.clone()).0, 204);
    let older = "000000000000 1600000000 x86_64-unknown-linux-gnu";
    let (status, said) = computes(&device, json!({ "computes": true, "reads": 0, "version": older }));
    assert_eq!(status, 426, "{said}");
    assert_eq!(said["code"], "update", "{said}");
    assert_eq!(said["asked"], ask, "the ask as it was signed");
    let (by, fields) = keys::check_request("one/relay", &said["asked"], at()).unwrap();
    assert_eq!((data_encoding::HEXLOWER.encode(&by), fields["update"].clone()), (hex(&admin), json!(hex_device)));

    // Its version was noted before it was told it's too old, and the ask is gone.
    let (status, said) = computes(&admin, json!({ "computes": true, "reads": one::database::READS }));
    assert_eq!(status, 200, "{said}");
    assert_eq!(said["versions"][&hex_device], older, "kept before the reads check");
    assert!(said.get("asked").is_none());
    assert!(computes(&device, json!({ "computes": true, "reads": one::database::READS, "version": version })).1.get("asked").is_none(), "heard once");
    assert_eq!(tell(&device, json!({ "update": data_encoding::HEXLOWER.encode(&[9u8; 32]) })).0, 403, "an ask for no one on the list");

    // An ask to deploy the Workers again is left for the device they're deployed from, whoever leaves it, and heard once, whole.
    let deploy = keys::signed_request(&signer(&device), "one/relay", json!({ "cloudflare": true }), at());
    assert_eq!(crate::cloudflare::post(&format!("{address}/relay/tell"), &[], deploy.clone()).0, 204);
    let home = json!({ "computes": true, "reads": one::database::READS, "version": version });
    let (status, said) = computes(&admin, home.clone());
    assert_eq!((status, &said["asked"]), (200, &deploy), "{said}");
    let (by, fields) = keys::check_request("one/relay", &said["asked"], at()).unwrap();
    assert_eq!((data_encoding::HEXLOWER.encode(&by), fields["cloudflare"].clone()), (hex_device.clone(), json!(true)));
    assert!(computes(&admin, home).1.get("asked").is_none(), "heard once");
}

/// A release for Windows adds its own line to the signed record. Signed once, as a record with that line is: every device
/// elsewhere goes on reading the record and takes only its own line, and one on Windows finds its own.
#[test]
fn a_windows_line_in_the_record_leaves_every_other_reader_as_it_was() {
    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILc1vQtY8RWeBKRF1BZhK7L6CpWUhLdZE9wrxPTJihkp";
    const RECORD: &str = "node/x86_64-unknown-linux-gnu abc123 1700000000 0000000000000000000000000000000000000000000000000000000000000001 3 -
node/x86_64-pc-windows-gnu def456 1700000100 0000000000000000000000000000000000000000000000000000000000000002 4 -
not a line a reader knows
-----BEGIN SSH SIGNATURE-----
U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAgtzW9C1jxFZ4EpEXUFmErsvoKlZ
SEt1kT3CvE9MmKGSkAAAAOcmVsZWFzZS1sYXRlc3QAAAAAAAAABnNoYTUxMgAAAFMAAAAL
c3NoLWVkMjU1MTkAAABAUeB9U2B9jrEiyXjrLWnEvrqlbLS56VB86VjOfE1OkDAZmMhXQd
+uZ1kk/Nq2ixRFKuLKAomwCijyssLtuhcEBg==
-----END SSH SIGNATURE-----
";
    let latest = one::latest::Latest::read(KEY, RECORD).unwrap();
    assert_eq!(latest.things.len(), 2, "a line it doesn't understand is ignored");
    assert_eq!(latest.get("node/x86_64-unknown-linux-gnu").unwrap().commit, "abc123");
    assert_eq!(latest.for_target("x86_64-unknown-linux-gnu").unwrap().commit, "abc123", "Linux takes its own line");
    assert_eq!(latest.for_target(one::build::WINDOWS).unwrap().commit, "def456", "Windows takes its own");
    assert!(latest.for_target("aarch64-unknown-linux-gnu").is_none(), "a platform with no line has nothing, as before");
}
