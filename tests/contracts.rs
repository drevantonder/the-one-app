//! Where code in two languages must agree on keys, seals, who's who and what an app's screen may do, a test reads both
//! and checks they do, so neither can drift from the other.

use std::collections::BTreeSet;

use serde_json::{Value, json};

/// The kernel's services (JavaScript, `kernel/shared/crypto.js`) and nodes (`src/kernel.rs`) each open what the
/// other seals: app keys under scope keys, keys to key pairs and under secrets; and derive the same database keys.
#[test]
fn the_kernel_and_nodes_agree_on_keys_and_seals() {
    use data_encoding::BASE64URL_NOPAD as B64U;
    use one::kernel;

    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("kernel");
    let files = ["shared/crypto.js", "shared/token.js", "web/members.js", "test/formats.js"].map(|p| (p.to_string(), std::fs::read(root.join(p)).unwrap()));
    let dir = crate::temp("formats");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let served = kernel::Served { name: "formats".into(), module: kernel::module_from(files.to_vec(), "test/formats.js").unwrap(), modules: vec![], class: None, listen: Some(addr.clone()), bindings: vec![], assets: None, outbound: None };
    let mut child = kernel::start(&kernel::write_config(&dir, &[served]).unwrap(), vec![("formats".into(), listener)]).unwrap();
    let js = |body: Value| -> Value {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        agent.post(format!("http://{addr}")).send_json(body).unwrap().body_mut().read_json().unwrap()
    };
    let b = |v: &Value| B64U.decode(v.as_str().unwrap().as_bytes()).unwrap();

    // An app's key, sealed under its scope's: each opens what the other sealed, and derives the same database key.
    let scope: [u8; 32] = rand::random();
    let app: [u8; 32] = rand::random();
    let opened = js(json!({ "op": "openAppKey", "scopeKey": B64U.encode(&scope), "app": "board", "sealed": B64U.encode(&kernel::seal_app_key(&scope, "board", &app)), "database": "main" }));
    assert_eq!(b(&opened["key"]), app, "JavaScript opens an app's key Rust sealed");
    assert_eq!(b(&opened["database"]), kernel::database_key(&app, "main"), "a database's key");
    let sealed = js(json!({ "op": "sealAppKey", "scopeKey": B64U.encode(&scope), "app": "board", "key": B64U.encode(&app) }));
    assert_eq!(kernel::open_app_key(&scope, "board", &b(&sealed["sealed"])).unwrap(), app, "Rust opens an app's key JavaScript sealed");
    assert!(kernel::open_app_key(&scope, "calendar", &b(&sealed["sealed"])).is_err(), "only as that app's");

    // Sealed to a key pair: JavaScript's pair, sealed in Rust; Rust's pair (the recovery key's twin), sealed in JavaScript.
    let pair = js(json!({ "op": "keyPair" }));
    assert_eq!(kernel::public_of(&b(&pair["secret"]).try_into().unwrap()).to_vec(), b(&pair["public"]), "a key pair's public half");
    let sealed = kernel::seal_to(&b(&pair["public"]).try_into().unwrap(), &scope).unwrap();
    let opened = js(json!({ "op": "openSealed", "secret": pair["secret"], "public": pair["public"], "sealed": B64U.encode(&sealed) }));
    assert_eq!(b(&opened["plain"]), scope, "JavaScript opens what Rust sealed to it");
    let (secret, public) = kernel::twin(&rand::random());
    let sealed = js(json!({ "op": "sealTo", "to": B64U.encode(&public), "plain": B64U.encode(&scope) }));
    assert_eq!(kernel::open_sealed(&secret, &b(&sealed["sealed"])).unwrap(), scope, "Rust opens what JavaScript sealed to it");

    // Sealed under a secret, with the id it's kept by.
    let under = js(json!({ "op": "sealUnder", "secret": "a refresh token, as text", "plain": B64U.encode(&scope) }));
    assert_eq!(under["id"], kernel::id_of(b"a refresh token, as text"), "a copy's id");
    assert_eq!(kernel::open_under(b"a refresh token, as text", &b(&under["sealed"])).unwrap(), scope, "Rust opens a copy JavaScript sealed");
    let sealed = kernel::seal_under(b"a code from the recovery key", &scope);
    let opened = js(json!({ "op": "openUnder", "secret": "a code from the recovery key", "sealed": B64U.encode(&sealed) }));
    assert_eq!(b(&opened["plain"]), scope, "JavaScript opens a copy Rust sealed");

    // Devices' keys (src/keys.rs): canonical JSON, signed requests both ways, objects sealed under an epoch, a call's
    // handoff, join codes, invite proofs and passkeys' device keys.
    {
        use one::keys;
        let seed: [u8; 32] = rand::random();
        let signer = ed25519_dalek::SigningKey::from_bytes(&seed);
        let fields = json!({ "read": ["user-0123456789abcdef/notes/main/1"], "n": 3, "z": { "b": "é\n\"", "a": [] } });
        let request = keys::signed_request(&signer, "one/urls", fields.clone(), 1_000_000);
        let node = ed25519_dalek::SigningKey::from_bytes(&rand::random());
        let to = keys::Endpoint::of(&node).seal;
        let handoff = keys::Handoff { for_: keys::token_id("a token"), until: 9, keys: vec![keys::Handed { scope: "global".into(), epoch: 2, app: "board".into(), database: "main".into(), key: rand::random() }] };
        let epoch_key: [u8; 32] = rand::random();
        let sealed = keys::seal(3, &epoch_key, "global/board/main/1", b"a step");
        let device = ed25519_dalek::SigningKey::from_bytes(&rand::random()).verifying_key().to_bytes();
        let device_seal: [u8; 32] = rand::random();
        let prf: [u8; 32] = rand::random();
        let (my_secret, my_public) = kernel::twin(&rand::random());
        let out = js(json!({
            "op": "keys", "seed": B64U.encode(&seed), "domain": "one/urls", "fields": fields, "at": 1_000_000,
            "request": request, "now": 1_000_000 + keys::FRESH, "epoch": 4, "key": B64U.encode(&epoch_key),
            "name": "global/board/main/1", "plain": B64U.encode(b"from js"), "sealed": B64U.encode(&sealed),
            "to": B64U.encode(&kernel::twin_public(&node.verifying_key().to_bytes()).unwrap()), "handoff": handoff,
            "pair": { "secret": B64U.encode(&my_secret), "public": B64U.encode(&my_public), "sealed": keys::seal_handoff(&my_public, &handoff).unwrap(), "key": data_encoding::HEXLOWER.encode(&device), "seal": data_encoding::HEXLOWER.encode(&device_seal) },
            "token": "a token", "secret": B64U.encode(b"an invite's secret"), "prf": B64U.encode(&prf),
        }));
        assert_eq!(out["canonical"].as_str().unwrap().as_bytes(), keys::canonical(&fields), "canonical JSON");
        assert_eq!(out["signed"], request, "the same request signed the same, Ed25519 being deterministic");
        assert_eq!(out["checked"], json!(data_encoding::HEXLOWER.encode(signer.verifying_key().as_bytes())), "JavaScript checks a request Rust signed");
        assert!(keys::check_request("one/urls", &out["signed"], 1_000_000).is_ok(), "Rust checks one JavaScript signed");
        assert_eq!(keys::open(&epoch_key, "global/board/main/1", &b(&out["sealed"])).unwrap(), b"from js", "Rust opens an object JavaScript sealed");
        assert_eq!(keys::epoch_of(&b(&out["sealed"])).unwrap(), 4);
        assert_eq!(b(&out["opened"]), b"a step", "JavaScript opens one Rust sealed");
        assert_eq!(out["epoch"], 3);
        assert_eq!(keys::open_handoff(&keys::seal_secret(&node), out["handoff"].as_str().unwrap()).unwrap(), handoff, "Rust opens a handoff JavaScript sealed");
        assert_eq!(out["handedBack"], serde_json::to_value(&handoff).unwrap(), "JavaScript opens one Rust sealed");
        assert_eq!(out["tokenId"], keys::token_id("a token"));
        assert_eq!(out["epochHash"], keys::epoch_hash(&epoch_key), "an epoch key's hash");
        assert_eq!(out["joinCode"], keys::join_code(&keys::Endpoint { key: device, seal: device_seal }));
        assert_eq!(out["twin"], data_encoding::HEXLOWER.encode(&kernel::twin_public(&device).unwrap()), "an Ed25519 key's twin to seal to");
        assert_eq!(out["proof"], keys::invite_proof(b"an invite's secret", &keys::Endpoint { key: device, seal: device_seal }));
        let (passkey, seal_secret) = keys::passkey_keys(&prf);
        assert_eq!(out["passkey"]["sign"], data_encoding::HEXLOWER.encode(passkey.verifying_key().as_bytes()), "a passkey's signing key");
        assert_eq!(b(&out["passkey"]["seal"]), seal_secret, "and the secret sealed to");
        assert_eq!(out["passkey"]["sealPublic"], data_encoding::HEXLOWER.encode(&kernel::public_of(&seal_secret)), "and its public half");
        assert_eq!(to, kernel::twin_public(&node.verifying_key().to_bytes()).unwrap(), "a Rust endpoint seals to its key's twin");

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

    // Entries (src/entry.rs): signed alike from the same key and fields, and each checks the other's.
    {
        use one::entry::{Content, Entry, Ext, Hash};
        let seed: [u8; 32] = rand::random();
        let key = ed25519_dalek::SigningKey::from_bytes(&seed);
        let (backlink, p1, p2) = (Hash::from_bytes(rand::random()), Hash::from_bytes(rand::random()), Hash::from_bytes(rand::random()));
        let content = Content::new("node", "group", json!({ "group": "0123456789abcdef", "name": "Café \"Ü\"\n" }));
        let ext = Ext { log: "user:0123456789abcdef".into(), time: 1_700_000_000_123, user: "0123456789abcdef".into(), epoch: 2, previous: vec![p2, p1] };
        let rust = Entry::create(&key, ext, Some((0, backlink)), content);
        let fields = json!({ "log": "user:0123456789abcdef", "seq": 1, "backlink": backlink.to_hex(), "previous": [p2.to_hex(), p1.to_hex()], "time": 1_700_000_000_123u64, "user": "0123456789abcdef", "epoch": 2 });
        let out = js(json!({ "op": "entries", "seed": B64U.encode(&seed), "fields": fields, "body": String::from_utf8(rust.body.clone()).unwrap(), "entry": rust.to_json() }));
        assert_eq!(out["signed"], rust.to_json(), "the same entry signed the same");
        assert_eq!(out["hash"], json!(rust.hash().to_hex()), "and hashed the same");
        let from_js = Entry::from_json(&out["signed"]).expect("Rust checks an entry JavaScript signed");
        assert_eq!((from_js.hash(), from_js.previous().to_vec()), (rust.hash(), vec![p1.min(p2), p1.max(p2)]));
        assert_eq!(out["checked"], json!({ "hash": rust.hash().to_hex(), "user": "0123456789abcdef", "kind": "group" }), "JavaScript checks one Rust signed");
        assert_ne!(out["tampered"], json!("checked"), "but not with another body");
        // What Rust refuses, JavaScript refuses too: a number above 2^53, a log that isn't a scope.
        let made = |log: &str, time: u64| Entry::create(&key, Ext { log: log.into(), time, user: "0123456789abcdef".into(), epoch: 0, previous: vec![] }, None, Content::new("node", "group", json!({}))).to_json();
        let bad = [made("global", 1 << 53), made("nowhere", 1), made("user:0123456789ABCDEF", 1)];
        for e in &bad {
            assert!(Entry::from_json(e).is_err(), "Rust refuses {e}");
        }
        let out = js(json!({ "op": "refused", "entries": bad }));
        for why in out["refused"].as_array().unwrap() {
            assert_ne!(why, "checked", "JavaScript refuses what Rust does");
        }
        assert_eq!(js(json!({ "op": "refused", "entries": [made("global", (1 << 53) - 1)] }))["refused"], json!(["checked"]));
    }

    // Member entries' content (src/members.rs), as the web client builds what a browser signs (kernel/web/members.js).
    {
        use one::entry::Scope;
        use one::members::{self, Kind, Role};
        let endpoint = one::keys::Endpoint { key: rand::random(), seal: rand::random() };
        let (key, seal) = (data_encoding::HEXLOWER.encode(&endpoint.key), data_encoding::HEXLOWER.encode(&endpoint.seal));
        let (user, group) = ("0123456789abcdef", "fedcba9876543210");
        let both = [
            (members::device(&endpoint, user, "Chrome on Mac", &[Kind::Client], None), json!(["device", { "key": key, "seal": seal, "user": user, "name": "Chrome on Mac" }])),
            (members::device(&endpoint, user, "Sam's Chrome", &[Kind::Client], Some(("Sam", Role::Member))), json!(["device", { "key": key, "seal": seal, "user": user, "name": "Sam's Chrome", "newUser": { "name": "Sam", "role": "member" } }])),
            (members::passkey(&endpoint, user, "Passkey", "cred-1"), json!(["passkey", { "key": key, "seal": seal, "user": user, "name": "Passkey", "credential": "cred-1" }])),
            (members::role(user, Role::Admin), json!(["role", { "user": user, "role": "admin" }])),
            (members::remove_user(user, &[(endpoint.key, Some(7))].into()), json!(["removeUser", { "user": user, "seqs": { &key: 7 } }])),
            (members::remove_user(user, &[(endpoint.key, None)].into()), json!(["removeUser", { "user": user, "seqs": { &key: null } }])),
            (members::remove_device(&endpoint.key, Some(7)), json!(["removeDevice", { "key": key, "seq": 7 }])),
            (members::remove_device(&endpoint.key, None), json!(["removeDevice", { "key": key }])),
            (members::setting("name", Some("Home")), json!(["setting", { "name": "name", "value": "Home" }])),
            (members::setting("name", None), json!(["setting", { "name": "name" }])),
            (members::group(group, "Family"), json!(["group", { "group": group, "name": "Family" }])),
            (members::invite(group, user, Role::Member), json!(["invite", { "group": group, "user": user, "role": "member" }])),
            (members::accept(group), json!(["accept", { "group": group }])),
            (members::member(group, user, Some(Role::Admin)), json!(["member", { "group": group, "user": user, "role": "admin" }])),
            (members::member(group, user, None), json!(["member", { "group": group, "user": user }])),
            (members::rename("key", &key, "Work Mac"), json!(["rename", { "what": "key", "id": key, "name": "Work Mac" }])),
            (members::rename("group", group, "Kin"), json!(["rename", { "what": "group", "id": group, "name": "Kin" }])),
            (members::handle("user", user, "sam-2"), json!(["handle", { "what": "user", "id": user, "handle": "sam-2" }])),
            (members::epoch(&Scope::User(user.into()), 2, &[endpoint.key], &seal), json!(["epoch", { "scope": format!("user:{user}"), "n": 2, "to": [key], "hash": seal }])),
            (members::grant(&endpoint.key, &Scope::Group(group.into()), "whatsapp", 0, &Default::default()), json!(["grant", { "endpoint": key, "scope": format!("group:{group}"), "app": "whatsapp", "until": 0 }])),
            (members::grant(&endpoint.key, &Scope::User(user.into()), "whatsapp", 1_800_000_000_000, &[(1, seal.clone()), (2, key.clone())].into()), json!(["grant", { "endpoint": key, "scope": format!("user:{user}"), "app": "whatsapp", "until": 1_800_000_000_000_u64, "keys": { "1": seal, "2": key } }])),
            (members::retire("notes", "admins", "deleted").unwrap(), json!(["retire", { "app": "notes", "owner": "admins", "how": "deleted" }])),
            (members::retire("notes", "admins", "restored").unwrap(), json!(["retire", { "app": "notes", "owner": "admins", "how": "restored" }])),
            (members::install("notes", false), json!(["install", { "app": "notes", "installed": false }])),
            (members::runs(&[endpoint.key]), json!(["runs", { "on": [key] }])),
            (members::runs(&[]), json!(["runs", { "on": [] }])),
            (members::service(&endpoint.key, "shell", false), json!(["service", { "device": key, "name": "shell" }])),
            (members::service(&endpoint.key, "shell", true), json!(["service", { "device": key, "name": "shell", "on": true }])),
        ];
        let out = js(json!({ "op": "members", "calls": both.iter().map(|(_, call)| call.clone()).collect::<Vec<_>>() }));
        for ((rust, call), js) in both.iter().zip(out["contents"].as_array().expect("contents")) {
            assert_eq!(js, &serde_json::to_value(rust).unwrap(), "{call}");
        }
    }

    // What a node is handed with each kind of request (`keys::may_hand`), and how old an epoch (`keys::oldest_needed`):
    // browsers and the MCP Worker hand by the same rule as native apps.
    {
        use one::keys::{Need, app_of, may_hand, oldest_needed};
        let routes = ["call board", "call node", "/v1/screens/board", "/v1/screens/board/main", "/mcp", "/v1/widgets", "/v1/badges", "/v1/vault", "/v1/vault/requests", "/v1/state", "/v1/entries", "/v1/apps", "call "];
        let dbs = [("board", "main"), ("board", "sidecar"), ("notes", "main"), ("board", "other"), ("node", "vault"), ("node", "log"), ("node", "main"), ("node", "blobs")];
        let mut hands: Vec<(String, Need)> = Vec::new();
        for (route, scope, (app, database)) in routes.iter().flat_map(|r| ["user:ab", "global"].map(|s| (r, s))).flat_map(|(r, s)| dbs.iter().map(move |d| (r, s, d))) {
            hands.push((route.to_string(), Need { scope: scope.into(), epoch: 1, app: app.to_string(), database: database.to_string() }));
        }
        let hints: Vec<(Option<String>, u32)> = [None, Some(r#"{"at":9,"copy":0,"epoch":0}"#), Some(r#"{"at":9,"copy":7,"epoch":2}"#), Some(r#"{"at":9,"copy":7,"epoch":5}"#), Some(r#"{"at":9,"copy":7}"#), Some("not a hint"), Some(r#"{"copy":-1,"epoch":2}"#)]
            .into_iter()
            .flat_map(|h| [0, 1, 3].map(|n| (h.map(str::to_string), n)))
            .collect();
        let out = js(json!({ "op": "handed", "hands": hands.iter().map(|(r, n)| json!([r, n])).collect::<Vec<_>>(), "hints": hints.iter().map(|(h, n)| json!([h, n])).collect::<Vec<_>>() }));
        for ((route, need), (js, app)) in hands.iter().zip(out["hands"].as_array().unwrap().iter().zip(out["apps"].as_array().unwrap())) {
            assert_eq!(js.as_bool(), Some(may_hand(route, need)), "{route} handed {}/{}", need.app, need.database);
            assert_eq!(app.as_str(), app_of(route), "{route}'s app");
        }
        for ((hint, newest), js) in hints.iter().zip(out["oldest"].as_array().unwrap()) {
            assert_eq!(js.as_u64(), Some(oldest_needed(hint.as_deref().map(str::as_bytes), *newest) as u64), "{hint:?} at {newest}");
        }
        // And what it says, for what matters most: another app's, or anything but the log with a request that runs none.
        let need = |app: &str, database: &str| Need { scope: "global".into(), epoch: 1, app: app.into(), database: database.into() };
        assert!(may_hand("call board", &need("board", "main")) && !may_hand("call board", &need("notes", "main")));
        assert!(may_hand("/v1/entries", &need("node", "log")) && !may_hand("/v1/entries", &need("board", "main")) && !may_hand("/v1/state", &need("node", "vault")));
        assert!(may_hand("/v1/vault", &need("node", "vault")) && !may_hand("/v1/vault", &need("board", "main")));
        // Apps' code, `global`'s blobs, goes to a node running an app (the serverless node keeps none); a scope's own
        // blobs, its people's files, never.
        let blobs = |scope: &str| Need { scope: scope.into(), epoch: 1, app: "node".into(), database: "blobs".into() };
        assert!(may_hand("call board", &blobs("global")) && may_hand("/v1/screens/board", &blobs("global")) && may_hand("/v1/badges", &blobs("global")));
        assert!(!may_hand("call board", &blobs("user:ab")) && !may_hand("/mcp", &blobs("group:cd")) && !may_hand("/v1/state", &blobs("global")));
        assert_eq!(oldest_needed(Some(br#"{"at":9,"copy":7,"epoch":2}"#), 3), 2, "from the epoch its newest copy is under");
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A client device (one-client: native apps, and browsers as WebAssembly) knows who's who only from the install's own
/// signed entries, replayed as nodes replay them: a device a node lists that no entry of the person's own devices does,
/// a seal key changed on the way, or entries of another install than the one it joined, never reach what it seals to.
#[test]
fn a_client_takes_whos_who_only_from_the_installs_signed_entries() {
    use ed25519_dalek::SigningKey;
    use one::entry::{Entry, Ext, Head, hex, pack};
    use one::keys::Endpoint;
    use one::members::{self, Kind};
    use one_client::members::{Global, view};

    const ANDRE: &str = "a000000000000001";
    let (mac, phone, node) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]), SigningKey::from_bytes(&[3; 32]));
    let write = |key: &SigningKey, prev: Option<Head>, content| Entry::create(key, Ext { log: "global".into(), time: 0, user: ANDRE.into(), epoch: 0, previous: vec![] }, prev, content);
    let genesis = write(&mac, None, members::genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)));
    let listed = write(&mac, Some(genesis.head()), members::device(&Endpoint::of(&phone), ANDRE, "phone", &[Kind::Client], None));
    let runs = write(&mac, Some(listed.head()), members::device(&Endpoint::of(&node), ANDRE, "box", &[Kind::Compute], None));
    let entries = [&genesis, &listed, &runs];

    // The first time, the install's genesis is the one its entries start with, kept from then on.
    let mut client = Global::new(None);
    client.add(&pack(&entries)).unwrap();
    assert_eq!(client.root(), Some(genesis.hash()));
    let k = view(&client.members().unwrap());
    let opens = |k: &serde_json::Value| k["openers"]["global"].as_array().unwrap().iter().map(|o| (o["key"].as_str().unwrap().to_string(), o["seal"].as_str().unwrap().to_string())).collect::<BTreeSet<_>>();
    let seal = |key: &SigningKey| hex(&Endpoint::of(key).seal);
    assert_eq!(opens(&k), BTreeSet::from([(hex(&mac.verifying_key().to_bytes()), seal(&mac)), (hex(&phone.verifying_key().to_bytes()), seal(&phone))]));
    assert_eq!(k["runs_for"][hex(&node.verifying_key().to_bytes())], json!(["global", format!("user:{ANDRE}")]));

    // A device a hacked node lists itself, signed by it rather than by one of Andre's own devices, counts for nothing.
    let mine = SigningKey::from_bytes(&[9; 32]);
    let forged = write(&node, None, members::device(&Endpoint::of(&mine), ANDRE, "phone 2", &[Kind::Client], None));
    client.add(&pack(&[&forged])).unwrap();
    assert_eq!(opens(&view(&client.members().unwrap())), opens(&k), "a device no entry of the person's lists isn't sealed to");

    // A seal key changed in an entry Andre's device signed: refused, all of it.
    let (header, signature, body) = listed.encode();
    let body = String::from_utf8(body).unwrap().replace(&seal(&phone), &hex(&Endpoint::of(&mine).seal)).into_bytes();
    let as_packed = |parts: [Vec<u8>; 3]| {
        let mut out = Vec::new();
        ciborium::into_writer(&ciborium::Value::Array(vec![ciborium::Value::Array(parts.into_iter().map(ciborium::Value::Bytes).collect())]), &mut out).unwrap();
        out
    };
    assert!(Global::new(Some(genesis.hash())).add(&as_packed([header, signature, body])).is_err());

    // Another install's entries: refused once one is kept, and when one's first seen beside another.
    let other = write(&node, None, members::genesis("Home", ANDRE, "Andre", &Endpoint::of(&node), "box", &[Kind::Client], &[7; 32], &"e".repeat(64)));
    assert!(client.add(&pack(&[&other])).is_err());
    assert!(Global::new(None).add(&pack(&[&genesis, &other])).is_err());
    let mut kept = Global::new(Some(other.hash()));
    assert!(kept.add(&pack(&entries)).is_err());
    assert!(kept.members().is_err());
}

/// What a native app keeps of its screens' calls, to show at once next time (one-client's `Answers`), is its person's
/// alone: only what operations that only read answered, sealed under that device's own key, so another person's
/// sign-in on the same computer (a device with its own key, even in the same folder) opens none of it; and it's gone
/// once the device signs out.
#[tokio::test(flavor = "multi_thread")]
async fn what_a_native_app_keeps_of_its_calls_is_its_persons_alone() {
    use ed25519_dalek::SigningKey;
    use one_client::{Answers, Device};

    let dir = crate::temp("answers");
    let (andre, sam) = ([1; 32], [2; 32]);
    let apps = serde_json::to_vec(&serde_json::json!([{ "name": "board", "ops": [{ "name": "topics", "reads": true }, { "name": "post", "reads": false }] }])).unwrap();
    let call = |op: &str, input: serde_json::Value| serde_json::to_vec(&serde_json::json!({ "id": "1", "app": "board", "op": op, "input": input, "client": "mac" })).unwrap();
    let mine = Answers::new(&dir, &SigningKey::from_bytes(&andre));
    mine.learn(&apps);
    mine.keep(&call("topics", serde_json::json!({ "scope": "user", "format": "json" })), br#"{"ok":[{"path":"kitchen-reno"}]}"#);
    mine.keep(&call("post", serde_json::json!({ "text": "hi" })), br#"{"ok":{"n":1}}"#);
    mine.keep(&call("topics", serde_json::json!({ "scope": "global" })), br#"{"error":"no"}"#);

    // The same call, its input's fields in any order, gets what it answered; a write, or a failure, was never kept.
    let (answer, at) = mine.kept("board", "topics", br#"{"format":"json","scope":"user"}"#).unwrap();
    assert_eq!(answer, br#"[{"path":"kitchen-reno"}]"#);
    assert!(at > 0);
    assert!(mine.kept("board", "post", br#"{"text":"hi"}"#).is_none());
    assert!(mine.kept("board", "topics", br#"{"scope":"global"}"#).is_none());
    // Nothing of it is in the clear.
    for file in std::fs::read_dir(dir.join("answers")).unwrap().flatten() {
        assert!(!String::from_utf8_lossy(&std::fs::read(file.path()).unwrap()).contains("kitchen-reno"));
    }

    // Another person's device, in the same folder, opens nothing.
    assert!(Answers::new(&dir, &SigningKey::from_bytes(&sam)).kept("board", "topics", br#"{"format":"json","scope":"user"}"#).is_none());
    // Only a native app's device keeps them; once it starts again, before it's read the apps, it finds them, and once
    // it signs out (`close`), they're gone.
    let cli = Device::new(andre, "https://one.invalid", dir.to_str().unwrap());
    assert!(cli.kept("board", "topics", br#"{"format":"json","scope":"user"}"#).is_none());
    let app = Device::new(andre, "https://one.invalid", dir.to_str().unwrap()).keeping();
    assert!(app.kept("board", "topics", br#"{"format":"json","scope":"user"}"#).is_some());
    app.close().await;
    assert!(app.kept("board", "topics", br#"{"format":"json","scope":"user"}"#).is_none());
    assert!(!dir.join("answers").exists());
}
