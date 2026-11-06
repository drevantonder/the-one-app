//! Where code in two languages must agree on keys, seals, who's who and what an app's screen may do, a test reads both
//! and checks they do, so neither can drift from the other.


use serde_json::{Value, json};

/// The Workers (JavaScript, `cloudflare/shared/crypto.js`) and nodes (`src/cloudflare.rs`) each open what the
/// other seals: app keys under scope keys, keys to key pairs and under secrets; and derive the same database keys.
#[test]
fn the_workers_and_nodes_agree_on_keys_and_seals() {
    use data_encoding::BASE64URL_NOPAD as B64U;
    use one::{cloudflare, keys};

    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("cloudflare");
    let files = ["shared/crypto.js", "shared/token.js", "web/members.js", "test/formats.js"].map(|p| (p.to_string(), std::fs::read(root.join(p)).unwrap()));
    let dir = crate::temp("formats");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let served = cloudflare::Served { name: "formats".into(), module: cloudflare::module_from(files.to_vec(), "test/formats.js").unwrap(), modules: vec![], class: None, listen: Some(addr.clone()), bindings: vec![], assets: None, outbound: None };
    let mut child = cloudflare::start(&cloudflare::write_config(&dir, &[served]).unwrap(), vec![("formats".into(), listener)]).unwrap();
    let js = |body: Value| -> Value {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        agent.post(format!("http://{addr}")).send_json(body).unwrap().body_mut().read_json().unwrap()
    };
    let b = |v: &Value| B64U.decode(v.as_str().unwrap().as_bytes()).unwrap();

    // An app's key, sealed under its scope's: each opens what the other sealed, and derives the same database key.
    let scope: [u8; 32] = rand::random();
    let app: [u8; 32] = rand::random();
    let opened = js(json!({ "op": "openAppKey", "scopeKey": B64U.encode(&scope), "app": "tally", "sealed": B64U.encode(&keys::seal_app_key(&scope, "tally", &app)), "database": "main" }));
    assert_eq!(b(&opened["key"]), app, "JavaScript opens an app's key Rust sealed");
    assert_eq!(b(&opened["database"]), keys::database_key(&app, "tally", "main"), "a database's key");
    let sealed = js(json!({ "op": "sealAppKey", "scopeKey": B64U.encode(&scope), "app": "tally", "key": B64U.encode(&app) }));
    assert_eq!(keys::open_app_key(&scope, "tally", &b(&sealed["sealed"])).unwrap(), app, "Rust opens an app's key JavaScript sealed");
    assert!(keys::open_app_key(&scope, "calendar", &b(&sealed["sealed"])).is_err(), "only as that app's");

    // Sealed to a key pair: JavaScript's pair, sealed in Rust; Rust's pair (the recovery key's twin), sealed in JavaScript.
    let pair = js(json!({ "op": "keyPair" }));
    assert_eq!(keys::public_of(&b(&pair["secret"]).try_into().unwrap()).to_vec(), b(&pair["public"]), "a key pair's public half");
    let sealed = keys::seal_to(&b(&pair["public"]).try_into().unwrap(), &scope).unwrap();
    let opened = js(json!({ "op": "openSealed", "secret": pair["secret"], "public": pair["public"], "sealed": B64U.encode(&sealed) }));
    assert_eq!(b(&opened["plain"]), scope, "JavaScript opens what Rust sealed to it");
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
        let handoff = keys::Handoff { for_: keys::token_id("a token"), until: 9, keys: vec![keys::Handed { scope: "global".into(), epoch: 2, app: "tally".into(), database: "main".into(), key: rand::random() }] };
        let epoch_key: [u8; 32] = rand::random();
        let sealed = keys::seal(3, &epoch_key, "global/tally/main/1", b"a step");
        let device = ed25519_dalek::SigningKey::from_bytes(&rand::random()).verifying_key().to_bytes();
        let device_seal: [u8; 32] = rand::random();
        let prf: [u8; 32] = rand::random();
        let (my_secret, my_public) = keys::twin(&rand::random());
        let out = js(json!({
            "op": "keys", "seed": B64U.encode(&seed), "domain": "one/urls", "fields": fields, "at": 1_000_000,
            "request": request, "now": 1_000_000 + keys::FRESH, "epoch": 4, "key": B64U.encode(&epoch_key),
            "name": "global/tally/main/1", "plain": B64U.encode(b"from js"), "sealed": B64U.encode(&sealed),
            "to": B64U.encode(&keys::twin_public(&node.verifying_key().to_bytes()).unwrap()), "handoff": handoff,
            "pair": { "secret": B64U.encode(&my_secret), "public": B64U.encode(&my_public), "sealed": keys::seal_handoff(&my_public, &handoff).unwrap(), "key": data_encoding::HEXLOWER.encode(&device), "seal": data_encoding::HEXLOWER.encode(&device_seal) },
            "token": "a token", "secret": B64U.encode(b"an invite's secret"), "prf": B64U.encode(&prf),
        }));
        assert_eq!(out["canonical"].as_str().unwrap().as_bytes(), keys::canonical(&fields), "canonical JSON");
        assert_eq!(out["signed"], request, "the same request signed the same, Ed25519 being deterministic");
        assert_eq!(out["checked"], json!(data_encoding::HEXLOWER.encode(signer.verifying_key().as_bytes())), "JavaScript checks a request Rust signed");
        assert!(keys::check_request("one/urls", &out["signed"], 1_000_000).is_ok(), "Rust checks one JavaScript signed");
        assert_eq!(keys::open(&epoch_key, "global/tally/main/1", &b(&out["sealed"])).unwrap(), b"from js", "Rust opens an object JavaScript sealed");
        assert_eq!(keys::epoch_of(&b(&out["sealed"])).unwrap(), 4);
        assert_eq!(b(&out["opened"]), b"a step", "JavaScript opens one Rust sealed");
        assert_eq!(out["epoch"], 3);
        assert_eq!(keys::open_handoff(&keys::seal_secret(&node), out["handoff"].as_str().unwrap()).unwrap(), handoff, "Rust opens a handoff JavaScript sealed");
        assert_eq!(out["handedBack"], serde_json::to_value(&handoff).unwrap(), "JavaScript opens one Rust sealed");
        assert_eq!(out["tokenId"], keys::token_id("a token"));
        assert_eq!(out["epochHash"], keys::epoch_hash(&epoch_key), "an epoch key's hash");
        assert_eq!(out["joinCode"], keys::join_code(&keys::Endpoint { key: device, seal: device_seal }));
        assert_eq!(out["twin"], data_encoding::HEXLOWER.encode(&keys::twin_public(&device).unwrap()), "an Ed25519 key's twin to seal to");
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

    // A step (src/database/steps.rs): signed alike for its position.
    {
        use one::database::content::Content;
        use one::database::steps::Step;
        let seed: [u8; 32] = rand::random();
        let key = ed25519_dalek::SigningKey::from_bytes(&seed);
        let content = Content::new("node", "group", json!({ "group": "0123456789abcdef", "name": "Café \"Ü\"\n" }));
        let step = one::database::steps::sign(&key, &one::database::scopes::Scope::Global, 7, "0123456789abcdef", 2, 1_700_000_000_123, content).unwrap();
        let fields = json!({ "log": "global", "at": 7, "time": 1_700_000_000_123u64, "user": "0123456789abcdef", "epoch": 2 });
        let out = js(json!({ "op": "step", "seed": B64U.encode(&seed), "fields": fields, "body": String::from_utf8(step.body.clone()).unwrap() }));
        assert_eq!(out["signed"], step.to_json(), "the same step signed the same");
        assert!(Step::from_json(&out["signed"]).is_ok(), "and Rust checks it");
    }

    // Who's who as steps (src/database/steps.rs), as a browser reads it: the node's key sealed under an epoch as a
    // browser that makes one seals it, the freeze the recovery key signed and a step a browser signed, sealed as a node
    // puts them in the bucket and read from it as the URL signer's `get` gives them (cloudflare/web/device.js
    // `bucketSteps`), opened and applied by the client (its WebAssembly's `Global`, cloudflare/web/net-worker.js); and
    // from a Welcome, sign-in's word passed on as JavaScript reads it.
    {
        use ed25519_dalek::SigningKey;
        use one::database::steps;
        use one::database::steps::Step;
        use one::database::scopes::{Kind, Scope};
        use one::database::{content, scopes};
        use one::keys::{Endpoint, Hash};
        use one::database::steps::Reader;
        const ANDRE: &str = "a000000000000001";
        let browser_seed: [u8; 32] = rand::random();
        let (mac, browser, recovery) = (SigningKey::from_bytes(&rand::random()), SigningKey::from_bytes(&browser_seed), SigningKey::from_bytes(&rand::random()));
        let (epoch, app): ([u8; 32], [u8; 32]) = (rand::random(), rand::random());
        let root = Hash::from_bytes(rand::random());
        let start = content::first("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &recovery.verifying_key().to_bytes(), &one::keys::epoch_hash(&epoch));
        let first = steps::first_freeze(&recovery, root, &start, ANDRE, 0).unwrap();
        let listed = steps::sign(&mac, &Scope::Global, 2, ANDRE, 1, 0, content::device(&Endpoint::of(&browser), ANDRE, "browser", &[Kind::Client], None)).unwrap();
        let (_, _, body) = steps::sign(&browser, &Scope::Global, 3, ANDRE, 1, 5, content::setting("greeting", Some("hi"))).unwrap().encode();
        let fields = json!({ "log": "global", "at": 3, "time": 5, "user": ANDRE, "epoch": 1 });
        let out = js(json!({ "op": "step", "seed": B64U.encode(&browser_seed), "fields": fields, "body": String::from_utf8(body).unwrap() }));
        let signed = Step::decode(&b(&out["signed"]["header"]), &b(&out["signed"]["signature"]), &b(&out["signed"]["body"])).unwrap();
        let sealed_app = b(&js(json!({ "op": "sealAppKey", "scopeKey": B64U.encode(&epoch), "app": scopes::APP, "key": B64U.encode(&app) }))["sealed"]);
        let keys: one::database::stored::Keys = [(1, one::keys::database_key(&app, one::database::scopes::APP, one::database::steps::NAME))].into();
        let prefix = steps::prefix("global");
        let sealed = |at: u64, step: &Step, copy: Option<&[u8]>| {
            let name = one::database::stored::step_name(&prefix, at);
            json!({ "at": at, "sealed": B64U.encode(&one::database::steps::seal(&keys, &name, &steps::encode(step, copy)).unwrap()) })
        };
        let (s1, s2, s3) = (sealed(1, &first.step, first.copy.as_deref()), sealed(2, &listed, None), sealed(3, &signed, None));
        let junk = json!({ "at": 4, "sealed": B64U.encode(&one::database::steps::seal(&[(1, rand::random())].into(), &one::database::stored::step_name(&prefix, 4), b"not a step").unwrap()) });
        let (epochs, apps) = (std::collections::BTreeMap::from([(1, epoch)]), std::collections::BTreeMap::from([(1, sealed_app)]));
        // As a browser gives them (client/src/wasm.rs `Global`).
        let step_keys = one_client::members::step_keys(&epochs, &apps);
        let add = |r: &mut Reader, given: &serde_json::Value| r.add(&steps::Sealed::from_json(given), &step_keys, Some(&epochs), 0, |_, _, _| {});
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
        assert_eq!(who.setting("greeting").as_deref(), Some("hi"), "the step a browser signed counts");
        // Another network's freeze: nothing to a device of this one.
        let mut elsewhere = Reader::new(Some(Hash::from_bytes(rand::random())));
        add(&mut elsewhere, &read).unwrap();
        assert!(!frozen(&elsewhere), "a freeze that isn't the network it joined");
        // Welcomed: from the freeze and the steps up to the Welcome's, as a node passes them on with `?welcome=`.
        let welcome = steps::Welcome { freeze: 1, at: 3, hash: one::database::stored::state_hash(who.conn()).unwrap() };
        let said = js(json!({ "op": "joining", "asked": [], "said": [[200, { "approved": true, "root": root.to_hex(), "welcome": welcome.to_string() }]] }));
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
    }

    // Member entries' content (src/members.rs), as the web client builds what a browser signs (cloudflare/web/members.js).
    {
        use one::database::scopes::Scope;
        use one::database::content;
        use one::database::scopes::{Kind, Role};
        let endpoint = one::keys::Endpoint { key: rand::random(), seal: rand::random() };
        let (key, seal) = (data_encoding::HEXLOWER.encode(&endpoint.key), data_encoding::HEXLOWER.encode(&endpoint.seal));
        let (user, group) = ("0123456789abcdef", "fedcba9876543210");
        let both = [
            (content::device(&endpoint, user, "Chrome on Mac", &[Kind::Client], None), json!(["device", { "key": key, "seal": seal, "user": user, "name": "Chrome on Mac" }])),
            (
                {
                    let mut c = content::device(&endpoint, user, "phone", &[Kind::Client], None);
                    c.data["device"] = json!(seal.clone());
                    c
                },
                json!(["device", { "key": key, "seal": seal, "user": user, "name": "phone", "device": seal }]),
            ),
            (
                {
                    // Let in by its sign-in, from where it asked.
                    let mut c = content::device(&endpoint, user, "phone", &[Kind::Client], None);
                    c.data["sign_in"] = json!({ "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": "1" } });
                    c.data["place"] = json!("Sydney");
                    c
                },
                json!(["device", { "key": key, "seal": seal, "user": user, "name": "phone", "signIn": { "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": "1" } }, "place": "Sydney" }]),
            ),
            (content::device(&endpoint, user, "Sam's Chrome", &[Kind::Client], Some(("Sam", Role::Member))), json!(["device", { "key": key, "seal": seal, "user": user, "name": "Sam's Chrome", "newUser": { "name": "Sam", "role": "member" } }])),
            (content::passkey(&endpoint, user, "Passkey", "cred-1"), json!(["passkey", { "key": key, "seal": seal, "user": user, "name": "Passkey", "credential": "cred-1" }])),
            (content::role(user, Role::Admin), json!(["role", { "user": user, "role": "admin" }])),
            (content::remove_user(user), json!(["removeUser", { "user": user }])),
            (content::remove_device(&endpoint.key), json!(["removeDevice", { "key": key }])),
            (content::refuse(&endpoint, "MacBook Pro"), json!(["refuse", { "key": key, "seal": seal, "name": "MacBook Pro" }])),
            (content::forget_sign_in(&endpoint.key), json!(["forgetSignIn", { "key": key }])),
            (content::setting("name", Some("Home")), json!(["setting", { "name": "name", "value": "Home" }])),
            (content::setting("name", None), json!(["setting", { "name": "name" }])),
            (content::group(group, "Family"), json!(["group", { "group": group, "name": "Family" }])),
            (content::invite(group, user, Role::Member), json!(["invite", { "group": group, "user": user, "role": "member" }])),
            (content::accept(group), json!(["accept", { "group": group }])),
            (content::member(group, user, Some(Role::Admin)), json!(["member", { "group": group, "user": user, "role": "admin" }])),
            (content::member(group, user, None), json!(["member", { "group": group, "user": user }])),
            (content::rename("key", &key, "Work Mac"), json!(["rename", { "what": "key", "id": key, "name": "Work Mac" }])),
            (content::rename("group", group, "Kin"), json!(["rename", { "what": "group", "id": group, "name": "Kin" }])),
            (content::handle("user", user, "sam-2"), json!(["handle", { "what": "user", "id": user, "handle": "sam-2" }])),
            (content::epoch(&Scope::User(user.into()), 2, &[endpoint.key], &seal), json!(["epoch", { "scope": format!("user:{user}"), "n": 2, "to": [key], "hash": seal }])),
            (content::app_schedules(&endpoint.key, &Scope::Group(group.into()), "whatsapp", 0, &Default::default()), json!(["appSchedules", { "endpoint": key, "scope": format!("group:{group}"), "app": "whatsapp", "until": 0 }])),
            (content::app_schedules(&endpoint.key, &Scope::User(user.into()), "whatsapp", 1_800_000_000_000, &[(1, seal.clone()), (2, key.clone())].into()), json!(["appSchedules", { "endpoint": key, "scope": format!("user:{user}"), "app": "whatsapp", "until": 1_800_000_000_000_u64, "keys": { "1": seal, "2": key } }])),
            (content::retire("notes", "admins", "deleted").unwrap(), json!(["retire", { "app": "notes", "owner": "admins", "how": "deleted" }])),
            (content::retire("notes", "admins", "restored").unwrap(), json!(["retire", { "app": "notes", "owner": "admins", "how": "restored" }])),
            (content::install("notes", false), json!(["install", { "app": "notes", "installed": false }])),
            (content::runs(&[endpoint.key]), json!(["runs", { "on": [key] }])),
            (content::runs(&[]), json!(["runs", { "on": [] }])),
            (content::service(&endpoint.key, "shell", false, None), json!(["service", { "device": key, "name": "shell" }])),
            (content::service(&endpoint.key, "shell", true, None), json!(["service", { "device": key, "name": "shell", "on": true }])),
            (content::service(&endpoint.key, "shell", true, Some(&Scope::Global)), json!(["service", { "device": key, "name": "shell", "on": true, "scope": "global" }])),
        ];
        let out = js(json!({ "op": "members", "calls": both.iter().map(|(_, call)| call.clone()).collect::<Vec<_>>() }));
        for ((rust, call), js) in both.iter().zip(out["contents"].as_array().expect("contents")) {
            assert_eq!(js, &serde_json::to_value(rust).unwrap(), "{call}");
        }
    }

    // What a node is handed with each kind of request (`keys::may_hand`), and how old an epoch (`keys::oldest_needed`):
    // browsers and the remote MCP server hand by the same rule as native apps.
    {
        use one::keys::{Need, app_of, may_hand, oldest_needed};
        let routes = ["call tally", "call node", "/v1/screens/tally", "/v1/screens/tally/main", "/mcp", "/v1/widgets", "/v1/badges", "/v1/vault", "/v1/vault/requests", "/v1/state", "/v1/steps", "/v1/apps", "call "];
        let dbs = [("tally", "main"), ("tally", "sidecar"), ("notes", "main"), ("tally", "other"), ("node", "vault"), ("node", "log"), ("node", "main"), ("node", "blobs")];
        let mut hands: Vec<(String, Need)> = Vec::new();
        for (route, scope, (app, database)) in routes.iter().flat_map(|r| ["user:ab", "global"].map(|s| (r, s))).flat_map(|(r, s)| dbs.iter().map(move |d| (r, s, d))) {
            hands.push((route.to_string(), Need { scope: scope.into(), epoch: 1, app: app.to_string(), database: database.to_string() }));
        }
        let hints: Vec<(&str, Option<String>, u32)> = [None, Some(r#"{"at":9,"copy":0,"epoch":0}"#), Some(r#"{"at":9,"copy":7,"epoch":2}"#), Some(r#"{"at":9,"copy":7,"epoch":5}"#), Some(r#"{"at":9,"copy":7}"#), Some("not a hint"), Some(r#"{"copy":-1,"epoch":2}"#)]
            .into_iter()
            .flat_map(|h| ["log", "main", "blobs"].map(move |d| (d, h)))
            .flat_map(|(d, h)| [0, 1, 3].map(|n| (d, h.map(str::to_string), n)))
            .collect();
        let out = js(json!({ "op": "handed", "hands": hands.iter().map(|(r, n)| json!([r, n])).collect::<Vec<_>>(), "hints": hints.iter().map(|(d, h, n)| json!([d, h, n])).collect::<Vec<_>>() }));
        for ((route, need), (js, app)) in hands.iter().zip(out["hands"].as_array().unwrap().iter().zip(out["apps"].as_array().unwrap())) {
            assert_eq!(js.as_bool(), Some(may_hand(route, need)), "{route} handed {}/{}", need.app, need.database);
            assert_eq!(app.as_str(), app_of(route), "{route}'s app");
        }
        for ((database, hint, newest), js) in hints.iter().zip(out["oldest"].as_array().unwrap()) {
            assert_eq!(js.as_u64(), Some(oldest_needed(database, hint.as_deref().map(str::as_bytes), *newest) as u64), "{database} {hint:?} at {newest}");
        }
        // And what it says, for what matters most: another app's, or anything but the log with a request that runs none.
        let need = |app: &str, database: &str| Need { scope: "global".into(), epoch: 1, app: app.into(), database: database.into() };
        assert!(may_hand("call tally", &need("tally", "main")) && !may_hand("call tally", &need("notes", "main")));
        assert!(may_hand("/v1/steps", &need("node", "log")) && !may_hand("/v1/steps", &need("tally", "main")) && !may_hand("/v1/state", &need("node", "vault")));
        assert!(may_hand("/v1/vault", &need("node", "vault")) && !may_hand("/v1/vault", &need("tally", "main")));
        // Apps' code, `global`'s blobs, goes to a node running an app (the Cloudflare node keeps none); a scope's own
        // blobs, its people's files, never.
        let blobs = |scope: &str| Need { scope: scope.into(), epoch: 1, app: "node".into(), database: "blobs".into() };
        assert!(may_hand("call tally", &blobs("global")) && may_hand("/v1/screens/tally", &blobs("global")) && may_hand("/v1/badges", &blobs("global")) && may_hand("/v1/apps", &blobs("global")));
        assert!(!may_hand("/v1/apps", &need("tally", "main")) && !may_hand("/v1/apps", &blobs("user:ab")));
        assert!(!may_hand("call tally", &blobs("user:ab")) && !may_hand("/mcp", &blobs("group:cd")) && !may_hand("/v1/state", &blobs("global")));
        assert_eq!(oldest_needed("main", Some(br#"{"at":9,"copy":7,"epoch":2}"#), 3), 2, "from the epoch its newest copy is under");
        assert_eq!(oldest_needed("log", Some(br#"{"at":9,"copy":7,"epoch":2}"#), 3), 1, "a scope's steps are read from the first");
        assert_eq!(oldest_needed("log", None, 3), 1, "a scope's steps are read from the first, before any hint");
        assert_eq!(oldest_needed("blobs", None, 3), 1, "a blob is kept under the epoch it was put in");
    }

    // A client device's way in, the same in a browser (cloudflare/web) as in a native app and the CLI (one-client): what a
    // node's 428 may ask it to hand (`keys::asked_for`), what sign-in's word on its request to be let in means
    // (`keys::waited`).
    {
        use one::keys::{MOST_NEEDED, Need, Waited, asked_for, waited};
        let node = "ab".repeat(32);
        let need = |app: &str, database: &str| json!({ "scope": "global", "epoch": 1, "app": app, "database": database });
        let every = |n: usize| (0..n).map(|i| need(&format!("app{i}"), "main")).collect::<Vec<_>>();
        let asked: Vec<(&str, Value)> = vec![
            ("call tally", json!({ "to": node, "need": [need("tally", "main")] })),
            ("call tally", json!({ "to": node, "need": [need("tally", "main"), need("tally", "main"), need("node", "log")] })),
            // The seal key it names isn't read: what's handed is sealed to the one the entries list.
            ("call tally", json!({ "to": node, "seal": "cd".repeat(32), "need": [need("tally", "main")] })),
            ("call tally", json!({ "to": "cd".repeat(32), "need": [need("tally", "main")] })),
            ("call tally", json!({ "need": [need("tally", "main")] })),
            ("call tally", json!({ "to": node, "need": [] })),
            ("call tally", json!({ "to": node, "need": null })),
            ("call tally", json!({ "to": node, "need": [need("notes", "main")] })),
            ("call tally", json!({ "to": node, "need": [need("tally", "main"), { "scope": "global", "epoch": "1", "app": "tally", "database": "main" }] })),
            ("call tally", json!({ "to": node, "need": [{ "scope": "global", "epoch": -1, "app": "tally", "database": "main" }] })),
            ("/v1/badges", json!({ "to": node, "need": every(MOST_NEEDED) })),
            ("/v1/badges", json!({ "to": node, "need": every(MOST_NEEDED + 1) })),
        ];
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
        let out = js(json!({
            "op": "joining",
            "asked": asked.iter().map(|(route, answer)| json!([route, node, answer])).collect::<Vec<_>>(),
            "said": said.iter().map(|(status, body)| json!([status, body])).collect::<Vec<_>>(),
        }));
        for ((route, answer), js) in asked.iter().zip(out["asked"].as_array().unwrap()) {
            let js: Option<Vec<Need>> = serde_json::from_value(js.clone()).unwrap();
            let js = js.map(|mut n| {
                n.sort();
                n
            });
            assert_eq!(js, asked_for(route, &node, answer), "{route}: {answer}");
        }
        assert!(asked_for("call tally", &node, &asked[1].1).is_some_and(|n| n.len() == 2), "a key asked for twice is handed once");
        assert!(asked_for("/v1/badges", &node, &asked[10].1).is_some() && asked_for("/v1/badges", &node, &asked[11].1).is_none());
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

    // The bucket's names as the Workers (the URL signer, the web client) make them, and the refusal of a device that reads less.
    {
        use one::database::stored::{copy_name, newest_name, step_name, tidied_name};
        let prefix = one::database::steps::prefix("user:abc");
        let out = js(json!({ "op": "names", "names": [[prefix, 1], [prefix, 4_000_000_000_000u64]], "prefixes": [["user:abc", "tally", "main"], ["global", "node", "scope"]], "yours": "your One App network" }));
        let rust = |p: &str, n: u64| json!([step_name(p, n), copy_name(p, n), tidied_name(p), newest_name(p)]);
        assert_eq!(out["names"], json!([rust(&prefix, 1), rust(&prefix, 4_000_000_000_000)]), "the names of a database's objects in the bucket");
        assert_eq!(out["prefixes"], json!(["user-abc/tally/main", "global/node/scope"]), "where an app's database of a scope is");
        assert_eq!(out["refusal"], json!(one::protocol::update_refusal("your One App network")), "what a device that reads less is told");
        use one::protocol::code;
        assert_eq!(out["codes"], json!({ "update": code::UPDATE, "notLetIn": code::NOT_LET_IN, "refused": code::REFUSED, "notSignedIn": code::NOT_SIGNED_IN, "restarting": code::RESTARTING, "notInstalled": code::NOT_INSTALLED }), "the codes answers carry");
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
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

/// The Settings screens say who agents are in an item's apps (`src/screen/settings/agents.js`) by the name the node
/// gives them.
#[test]
fn the_screens_name_agents_as_the_vault_does() {
    let js = include_str!("../src/screen/settings/agents.js");
    assert!(js.contains(&format!("name: \"{}\"", one::database::scopes::AGENTS)), "agents.js names agents as scopes::AGENTS does");
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
    use one::app::runtime::Me;
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
    /// waiting.
    fn start_node(at: &str) -> (Arc<Node>, tokio::runtime::Runtime, std::net::SocketAddr) {
        let node = Arc::new(Node { services: Services::default(), registered: tokio::sync::watch::channel(0).0, refused: Mutex::new(Vec::new()) });
        let listener = std::net::TcpListener::bind(at).unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/v1/services/{app}", post(register))
            .route("/v1/services/{app}/calls", get(next))
            .route("/v1/services/{app}/calls/{id}", post(reply))
            .with_state(node.clone());
        let server = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        server.spawn(async move { axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app).await });
        (node, server, addr)
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
            let (mut node, mut server, addr) = start_node("127.0.0.1:0");

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
            (node, server, _) = start_node(&addr.to_string());
            registered(&node, name).await;
            assert!(node.services.has(name), "{name} is listed again");
            let (function, input) = case.ok.clone();
            assert!(ask(&node, name, function, input).await.is_ok(), "{name} answers the node that started again");
            assert!(node.refused.lock().unwrap().is_empty(), "{name} sent without its token: {:?}", node.refused.lock().unwrap());
            tokio::task::block_in_place(|| server.shutdown_timeout(Duration::from_secs(10)));
        }
    }
}
