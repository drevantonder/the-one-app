//! Real nodes on this machine, reaching each other through the relay of a real kernel's gateway.

use std::time::{Duration, Instant};

use one::node::{CallRequest, Node};
use serde_json::{Value, json};

use crate::temp;

async fn start(name: &str) -> Node {
    let dir = temp(name);
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": name }).to_string()).unwrap();
    Node::start(&dir, one::sidecars::built_in()).await.unwrap()
}

/// A kernel whose gateway's relay the devices of `a`'s install reach each other through, taking only `a` and those it
/// admits: `a` says it's the install's.
async fn relay(a: &Node) -> crate::kernel::Kernel {
    let k = tokio::task::block_in_place(|| crate::kernel::Kernel::for_devices(vec![one::entry::hex(&a.id())]));
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    a.set_setting(&me, "address", Some(&k.address)).await.unwrap();
    k
}

async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(45) {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

/// Waits for a write to be seen on another device, which takes under a second, right after joining too: not until the
/// next fill from the bucket.
async fn soon(mut seen: impl AsyncFnMut() -> bool) {
    let wrote = Instant::now();
    while !seen().await {
        assert!(wrote.elapsed() < Duration::from_secs(1), "a write took over a second to reach another device");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn board(op: &str, input: Value) -> CallRequest {
    CallRequest { app: "board".into(), op: op.into(), input, ..Default::default() }
}

/// The texts of the posts on a device's board of `scope`.
async fn texts(node: &Node, scope: &str) -> Vec<String> {
    match node.call(board("news", json!({ "format": "json", "since": 0, "scope": scope }))).await {
        Ok(posts) => posts.as_array().unwrap().iter().map(|p| p["text"].as_str().unwrap().to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

async fn sees(node: &Node, scope: &str, text: &str) -> bool {
    texts(node, scope).await.iter().any(|t| t == text)
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_share_a_deployed_board() {
    let a = start("a").await;
    let b = start("b").await;

    let recovery = a.create("Test", "Andre").await.unwrap();
    assert_eq!(recovery.len(), 64, "creating gives the recovery key's secret once");
    assert!(a.create("Another", "Andre").await.is_err(), "a white label holds one install");
    let apps = a.apps(&one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None }).await.unwrap();
    let listed = apps.iter().find(|app| app["name"] == "board").expect("the config's board is deployed");
    assert_eq!(listed["owner"], "admins");
    assert!(listed["ops"].as_array().is_some_and(|ops| ops.iter().any(|o| o["name"] == "post")), "{listed}");
    assert_eq!(a.call(board("post", json!({ "text": "from a" }))).await.unwrap(), "Posted as #1 in the top.");

    assert!(a.pair().is_err(), "a computer joins through the install's address, which it has none of yet");
    let _relay = relay(&a).await;
    b.join(&a.pair().unwrap()).await.unwrap();
    assert_eq!(b.user().unwrap(), a.user().unwrap(), "a pairing code from a's user makes another of their devices");
    eventually("b to run the board and see a's post", async || sees(&b, "global", "from a").await).await;
    b.call(board("post", json!({ "text": "from b" }))).await.unwrap();
    soon(async || sees(&a, "global", "from b").await).await;
    assert!(b.join(&a.pair().unwrap()).await.is_err(), "a device in an install joins no other");
    let read = a.call(board("read", json!({}))).await.unwrap();
    let read = read.as_str().unwrap();
    assert!(read.lines().any(|l| l.starts_with("#1 ") && l.ends_with(": from a")) && read.lines().any(|l| l.starts_with("#2 ") && l.ends_with(": from b")), "agents read posts as numbered lines: {read}");

    for i in 0..3 {
        let text = format!("seen soon {i}");
        b.call(board("post", json!({ "text": text }))).await.unwrap();
        soon(async || sees(&a, "global", &text).await).await;
    }

    // A new deploy runs everywhere.
    let source = include_str!("../apps/board/app.js").replace("  ops: {", "  ops: {\n    ping: { description: \"\", input: {}, run: () => \"pong\" },");
    let bundle = one::bundle::pack(&[("app.js".into(), source.clone().into_bytes()), ("src/app.js".into(), source.into_bytes())]).unwrap();
    let blob = a.put_blob(&bundle).unwrap();
    a.append(&one::entry::Scope::Global, one::node::deploy("board", &blob)).await.unwrap();
    eventually("b to run the new deploy", async || b.call(board("ping", json!({}))).await.is_ok_and(|r| r == "pong")).await;
    assert!(sees(&b, "global", "from b").await, "a new deploy keeps the data");
}

fn call(app: &str, op: &str, input: Value) -> CallRequest {
    CallRequest { app: app.into(), op: op.into(), input, ..Default::default() }
}

/// Deploys a plain module as `app`, made by this device's user if it's new, and installs it in their scope.
async fn deploy_module(node: &Node, app: &str, source: &str) {
    let bundle = one::bundle::pack(&[("app.js".into(), source.as_bytes().to_vec()), ("src/app.js".into(), source.as_bytes().to_vec())]).unwrap();
    let blob = node.put_blob(&bundle).unwrap();
    let me = one::node::Caller { key: node.id(), user: node.user().unwrap(), agent: None, token: None };
    if node.source(app, None, None).await.is_err() {
        node.member("app", json!({ "name": app, "owner": format!("user:{}", me.user) })).await.unwrap();
    }
    node.deploy(&me, app, &blob).await.unwrap();
}

/// A device that answers every request with what it got: `{ host, authorization, body }`.
async fn echo() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let router = axum::Router::new().fallback(|headers: axum::http::HeaderMap, body: String| async move {
        let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        axum::Json(json!({ "host": h("host"), "authorization": h("authorization"), "body": body }))
    });
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    port
}

/// Only this machine's own programs use the local API as this machine: over its loopback, with the token in the node's
/// home. A program in a VM or emulator here reaches 127.0.0.1 through QEMU and names any host it likes, but has no
/// token; a connection from elsewhere has none to show. An app's screen opened in a browser at the node's own address
/// is that app's code: whatever a page asks there is refused, and nothing the node answers runs as script there.
#[tokio::test(flavor = "multi_thread")]
async fn only_this_machines_own_programs_use_the_local_api() {
    let a = start("pages").await;
    a.create("Pages", "Andre").await.unwrap();
    deploy_module(&a, "lamps", r#"export default { name: "lamps", ops: {}, screens: { main: { title: "Lamps", html: "<p>Lamps</p>" } } };"#).await;
    let token = a.local_token().to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(one::api::serve(listener, one::api::router(a.clone())));
    // The same API, over a connection from another machine.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let elsewhere = format!("http://{}", listener.local_addr().unwrap());
    let from = |mut req: axum::extract::Request| async move {
        req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([10, 0, 2, 15], 50000))));
        req
    };
    tokio::spawn(async move { axum::serve(listener, one::api::router(a).layer(axum::middleware::map_request(from))).await.unwrap() });
    let own = at.clone();
    tokio::task::spawn_blocking(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let get_at = |at: &str, path: &str, headers: &[(&str, &str)]| {
            let mut req = agent.get(format!("{at}{path}"));
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.call().unwrap()
        };
        let mine = (one::api::LOCAL_TOKEN_HEADER, token.as_str());
        let get = |path: &str, headers: &[(&str, &str)]| get_at(&own, path, &[&[mine][..], headers].concat());
        // This machine's own programs, a shell fetching a screen among them, send the token and no browser's headers.
        assert_eq!(get("/v1/vault", &[]).status(), 200);
        let screen = get("/v1/screens/lamps/main", &[]);
        assert_eq!(screen.status(), 200);
        assert_eq!(screen.headers()["content-security-policy"], "sandbox", "a screen opened in a browser runs no script at the node's address");
        // A program in a VM here: this machine's address, and its own Host, but no token, or a wrong one.
        for headers in [&[][..], &[("host", "127.0.0.1")], &[("host", "localhost:4747")], &[(one::api::LOCAL_TOKEN_HEADER, "00")]] {
            assert_eq!(get_at(&own, "/v1/vault", headers).status(), 403, "{headers:?}");
            assert_eq!(get_at(&own, "/v1/state", headers).status(), 403, "{headers:?}");
        }
        let told: Value = get_at(&own, "/v1/state", &[]).body_mut().read_json().unwrap();
        assert!(told["error"].as_str().unwrap().contains("start it again"), "an older program is told what to do, as JSON it shows: {told}");
        // From another machine, even with the token.
        assert_eq!(get_at(&elsewhere, "/v1/vault", &[mine, ("host", "127.0.0.1")]).status(), 403);
        // What a page at the node's own address asks, a same-origin GET (Sec-Fetch-Site only) or anything with an Origin.
        for headers in [&[("sec-fetch-site", "same-origin")][..], &[("origin", own.as_str())], &[("origin", "null")]] {
            assert_eq!(get("/v1/vault", headers).status(), 403, "{headers:?}");
        }
        let call = agent.post(format!("{own}/v1/call")).header(mine.0, mine.1).header("origin", own.as_str()).header("sec-fetch-site", "same-origin").send_json(json!({ "app": "board", "op": "topics", "input": {} })).unwrap();
        assert_eq!(call.status(), 403);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_ask_for_items_and_never_see_them() {
    let a = start("asks").await;
    a.create("Asks", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: Some("Claude Code".into()), token: None };
    let mine = one::entry::Scope::User(me.user.clone());
    let ask = json!({ "title": "Example key", "fields": { "user": {}, "key": { "secret": true } }, "hosts": ["api.example.com"] });
    let asked = a.request_item(&me, &mine, "board", None, &ask, "Your example.com API key, from its settings page").await.unwrap();
    let id = asked["id"].as_str().unwrap().to_string();
    assert!(asked["link"].as_str().unwrap().contains(&format!("vault?id={id}")), "{asked}");
    assert_eq!(a.state()["vault_requests"], 1, "the shell shows a request waiting");
    let waiting = a.item_requests(&me, None, Some(&id)).await.unwrap();
    assert_eq!(waiting[0]["state"], "waiting");
    assert_eq!(waiting[0]["agent"], "Claude Code");
    assert_eq!(waiting[0]["why"], "Your example.com API key, from its settings page");
    assert_eq!(waiting[0]["matches"], json!([]), "no item has those fields yet");

    let answer = json!({ "fields": { "user": "andre", "key": "s3cret" } });
    a.answer_request(&me, &mine, &id, &answer).await.unwrap();
    let done = a.item_requests(&me, None, Some(&id)).await.unwrap();
    assert_eq!(done[0]["state"], "set", "{done:?}");
    assert!(!format!("{done:?}").contains("s3cret"), "a request never says a secret");
    let items = a.vault(&me, Some(&mine)).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Example key");
    assert_eq!(items[0]["apps"], json!(["board"]), "the app that asked is switched on for it");
    assert_eq!(items[0]["fields"], json!([{ "name": "key", "secret": true }, { "name": "user", "secret": false, "value": "andre" }]));
    assert!(!items[0].to_string().contains("s3cret"), "a secret field is never shown");
    assert!(a.answer_request(&me, &mine, &id, &answer).await.is_err(), "answered once");
    assert_eq!(a.state()["vault_requests"], 0);

    // Another app's request is answered with the item that's there: it's switched on for it.
    deploy_module(&a, "other", r#"export default { name: "other", ops: {} };"#).await;
    let again = a.request_item(&me, &mine, "other", None, &ask, "").await.unwrap();
    let again = again["id"].as_str().unwrap();
    let matches = &a.item_requests(&me, None, Some(again)).await.unwrap()[0]["matches"];
    let item = matches[0]["id"].as_str().unwrap().to_string();
    a.answer_request(&me, &mine, again, &json!({ "item": item })).await.unwrap();
    assert_eq!(a.vault(&me, Some(&mine)).await.unwrap()[0]["apps"], json!(["board", "other"]));

}

#[tokio::test(flavor = "multi_thread")]
async fn apps_use_only_the_items_they_are_switched_on_for() {
    let dir = temp("vault");
    let device = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(device.as_bytes()), "name": "vault" }).to_string()).unwrap();
    let a = Node::start(&dir, one::sidecars::built_in()).await.unwrap();
    a.create("Vault", "Andre").await.unwrap();
    let port = echo().await;
    let module = |name: &str| format!(r#"export default {{
  name: "{name}",
  vault: {{ account: {{ title: "Example account", type: "Example account", accounts: true, hosts: ["127.0.0.1"], fields: {{ email: {{}}, token: {{ secret: true }} }} }} }},
  ops: {{
    send: {{ run: async (ctx, {{ url, auth }}) => {{
      const r = await fetch(url, {{ method: "POST", headers: {{ authorization: auth }}, body: "" }});
      if (!r.ok) throw new Error(await r.text());
      return r.json();
    }} }},
    items: {{ run: (ctx) => ctx.vault.items() }},
    sign_in: {{ run: (ctx, {{ email, token }}) => ctx.vault.create({{ title: email, type: "Example account", fields: {{ email: {{ value: email }}, token: {{ value: token, secret: true }} }}, hosts: ["127.0.0.1"], same: "email" }}) }},
    rehost: {{ run: (ctx, {{ id }}) => {{ ctx.vault.update(id, {{ hosts: ["evil.example.com"] }}); return null; }} }},
    sign_out: {{ run: (ctx, {{ id }}) => {{ ctx.vault.release(id); return null; }} }},
  }},
}};"#);
    deploy_module(&a, "one", &module("one")).await;
    deploy_module(&a, "two", &module("two")).await;
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    let mine = one::entry::Scope::User(me.user.clone());
    for app in ["one", "two"] {
        a.install(&me, app, &mine, true).await.unwrap();
    }
    let send = |app: &str, auth: &str| call(app, "send", json!({ "url": format!("http://127.0.0.1:{port}/"), "auth": auth }));

    // A sign-in makes an item switched on for its app alone, which fills in only its own fields.
    let id = a.call(call("one", "sign_in", json!({ "email": "a@example.com", "token": "t1" }))).await.unwrap();
    let id = id.as_str().unwrap().to_string();
    assert_eq!(a.call(send("one", &format!("Bearer {{{{{id}:token}}}}"))).await.unwrap()["authorization"], "Bearer t1");
    assert_eq!(a.call(send("one", "{{token}}")).await.unwrap()["authorization"], "t1", "by field name, the oldest item that has it");
    assert_eq!(a.call(send("two", &format!("{{{{{id}:token}}}}"))).await.unwrap()["authorization"], format!("{{{{{id}:token}}}}"), "an app not switched on gets nothing");
    let seen = a.call(call("one", "items", json!({}))).await.unwrap();
    assert_eq!(seen[0]["fields"], json!({ "email": { "value": "a@example.com" }, "token": { "secret": true } }), "an app sees plain fields, never a secret");
    assert_eq!(a.call(call("two", "items", json!({}))).await.unwrap(), json!([]));
    assert!(a.call(call("one", "rehost", json!({ "id": id }))).await.is_err(), "an app can't send a secret somewhere else");
    let listed = a.vault(&me, None).await.unwrap();
    assert_eq!(listed[0]["app"], "one", "kept by the app whose sign-in made it");
    assert_eq!(listed[0]["account"], true, "an account, since one asks for accounts with its fields");
    assert_eq!(listed[0]["wanted_by"], json!(["one", "two"]), "both ask for its fields");
    // A request for what an app declares asks for its fields, and offers the item that has them.
    let asked = a.request_item(&me, &mine, "two", Some("account"), &json!({}), "").await.unwrap();
    let asked = &a.item_requests(&me, None, asked["id"].as_str()).await.unwrap()[0];
    assert_eq!((asked["title"].as_str(), asked["matches"][0]["id"].as_str()), (Some("Example account"), Some(id.as_str())));

    // The same account signed in from another app is the same item, now switched on for both.
    let again = a.call(call("two", "sign_in", json!({ "email": "a@example.com", "token": "t2" }))).await.unwrap();
    assert_eq!(again, json!(id));
    assert_eq!(a.vault(&me, None).await.unwrap()[0]["apps"], json!(["one", "two"]));
    assert_eq!(a.call(send("one", "{{token}}")).await.unwrap()["authorization"], "t2");

    // Signing out switches the app off; the item goes once no app uses it.
    a.call(call("one", "sign_out", json!({ "id": id }))).await.unwrap();
    assert_eq!(a.vault(&me, None).await.unwrap()[0]["apps"], json!(["two"]));
    a.call(call("two", "sign_out", json!({ "id": id }))).await.unwrap();
    assert!(a.vault(&me, None).await.unwrap().is_empty());

    // A person's item, switched on for an app by them.
    let made = a.set_item(&me, &mine, json!({ "title": "Key", "fields": { "token": { "value": "k", "secret": true } }, "hosts": ["127.0.0.1"], "apps": ["two"] })).await.unwrap();
    assert_eq!(a.call(send("two", "{{token}}")).await.unwrap()["authorization"], "k");
    let refused = a.call(call("two", "send", json!({ "url": format!("http://localhost:{port}/"), "auth": "{{token}}" }))).await.unwrap_err();
    assert!(format!("{refused:#}").contains("sent only to 127.0.0.1"), "{refused:#}");
    assert!(a.call(call("one", "sign_out", json!({ "id": made }))).await.is_err(), "an app lets go only of what it uses");

    // A sign-in's call fills an item only with the key of its scope's Vault, handed with it: the app's own isn't enough.
    let who = one::node::Caller { token: Some("a sign-in".into()), ..me.clone() };
    let needs = |err: anyhow::Error| err.downcast::<one::node::Needs>().expect("it says what it needs").need;
    let need = needs(a.call_as(send("two", "{{token}}"), who.clone()).await.unwrap_err());
    let vault = |n: &one::keys::Need| n.scope == mine.to_string() && n.app == "node" && n.database == "vault";
    assert!(need.iter().any(vault) && need.iter().any(|n| n.app == "two" && n.database == "main"), "{need:?}");
    let bucket = one::bucket::Place::Folder(dir.join("install").join("bucket"));
    let key_of = |n: &one::keys::Need| {
        let to = one::entry::hex(&one::keys::Endpoint::of(&device).key);
        let epoch: [u8; 32] = one::kernel::open_sealed(&one::keys::seal_secret(&device), &bucket.get(&one::keys::epoch_name(&n.scope, n.epoch, &to)).unwrap().unwrap()).unwrap().try_into().unwrap();
        // An app's key in an epoch is made by the first device to need it, as a client does.
        let name = one::keys::app_key_name(&n.scope, &n.app, n.epoch);
        bucket.create(&name, &one::kernel::seal_app_key(&epoch, &n.app, &rand::random())).unwrap();
        let app = one::kernel::open_app_key(&epoch, &n.app, &bucket.get(&name).unwrap().unwrap()).unwrap();
        one::keys::Handed { key: one::kernel::database_key(&app, &n.database), scope: n.scope.clone(), epoch: n.epoch, app: n.app.clone(), database: n.database.clone() }
    };
    let hand = |keys: Vec<one::keys::Handed>| {
        let until = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + 600_000;
        let handoff = one::keys::Handoff { for_: one::keys::token_id("a sign-in"), until, keys };
        a.hand("a sign-in", &me.user, &one::keys::seal_handoff(&a.endpoint().seal, &handoff).unwrap()).unwrap();
    };
    hand(need.iter().filter(|n| !vault(n)).map(key_of).collect());
    let still = needs(a.call_as(send("two", "{{token}}"), who.clone()).await.unwrap_err());
    assert!(!still.is_empty() && still.iter().all(vault), "only the Vault's key is missing: {still:?}");
    hand(still.iter().map(key_of).collect());
    assert_eq!(a.call_as(send("two", "{{token}}"), who.clone()).await.unwrap()["authorization"], "k");
    hand(needs(a.call_as(send("one", "{{token}}"), who.clone()).await.unwrap_err()).iter().map(key_of).collect());
    assert_eq!(a.call_as(send("one", "{{token}}"), who.clone()).await.unwrap()["authorization"], "{{token}}", "still only for the app switched on");

    // The item is sealed wherever it's kept: no file of the device or its bucket has it in the clear.
    a.set_item(&me, &mine, json!({ "id": made, "fields": { "token": { "value": "s3cret-in-the-vault", "secret": true } } })).await.unwrap();
    assert_eq!(a.call(send("two", "{{token}}")).await.unwrap()["authorization"], "s3cret-in-the-vault");
    for file in walk(&dir) {
        let bytes = std::fs::read(&file).unwrap_or_default();
        assert!(!bytes.windows(19).any(|w| w == b"s3cret-in-the-vault"), "{} has the secret in the clear", file.display());
    }
    // Nothing a person or app reads says it.
    assert!(!json!([a.vault(&me, None).await.unwrap(), a.call(call("two", "items", json!({}))).await.unwrap(), a.state()]).to_string().contains("s3cret-in-the-vault"));
}

/// A device whose key is still in server.json, from before devices were called devices, never quietly becomes a new one.
#[tokio::test(flavor = "multi_thread")]
async fn a_device_keeps_its_key_from_server_json() {
    let dir = temp("server-json");
    std::fs::write(dir.join("server.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": "old" }).to_string()).unwrap();
    let refused = Node::start(&dir, one::sidecars::built_in()).await.err().expect("it doesn't start as a new device");
    assert!(format!("{refused:#}").contains("rename it to device.json"), "{refused:#}");
    assert!(!dir.join("device.json").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn backups_go_to_a_folder_encrypted_and_restore_with_the_recovery_key_alone() {
    let a = start("backup-a").await;
    let b = start("backup-b").await;
    let recovery = a.create("Home", "Andre").await.unwrap();
    let _relay = relay(&a).await;
    b.join(&a.pair().unwrap()).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    eventually("the board to run", async || a.call(board("post", json!({ "text": "before the backup" }))).await.is_ok()).await;
    a.set_item(&me, &one::entry::Scope::User(me.user.clone()), json!({ "title": "Example key", "fields": { "token": { "value": "s3cret-value", "secret": true } }, "hosts": ["api.example.com"] })).await.unwrap();

    // A place that can't be written isn't added.
    let bad = a.add_place(&me, &json!({ "endpoint": "http://127.0.0.1:9", "bucket": "b", "key_id": "k", "secret": "s" })).await.unwrap();
    assert!(bad["id"].is_null() && bad["tried"][0]["error"].is_string(), "{bad}");
    assert!(a.backups().unwrap()["places"].as_array().unwrap().is_empty());

    let dir = temp("backup-disk").join("One Backups");
    let added = a.add_place(&me, &json!({ "path": dir, "title": "T7 Shield" })).await.unwrap();
    assert!(added["id"].is_string() && added["tried"].as_array().unwrap().iter().all(|s| s["error"].is_null()), "{added}");
    eventually("the first backup", async || !a.backups().unwrap()["places"][0]["last"].is_null()).await;
    let shown = a.backups().unwrap();
    assert_eq!(shown["places"][0]["title"], "T7 Shield");
    assert_eq!(shown["places"][0]["last"]["device"], "backup-a", "the folder's own device backs it up: {shown}");
    let first = shown["places"][0]["last"]["time"].clone();
    eventually("b to see it", async || !b.backups().unwrap()["places"][0]["last"].is_null()).await;

    // From nothing but the disk and the recovery key.
    let place = one::bucket::Place::Folder(dir.clone());
    let install = one::backups::installs(&place).unwrap().remove(0);
    let keys = one::backups::open_keys(&place, &install, &one::entry::unhex(&recovery).unwrap()).unwrap();
    assert!(keys.contains_key("global") && keys.contains_key(&format!("user:{}", me.user)), "{:?}", keys.keys());
    let checked = one::backups::check(&place, &install, &keys, None).unwrap();
    assert_eq!(checked["install"], "Home");
    assert_eq!(checked["people"], json!(["Andre"]));
    assert!(checked["apps"].as_array().unwrap().contains(&json!("board")), "{checked}");
    let mine = format!("user:{}", me.user);
    let items = backed_up(&dir, &install, &one::entry::unhex(&recovery).unwrap(), &mine, "node", "vault", "select item from items").await;
    assert!(items.iter().any(|i| i.contains("\"Example key\"")), "the Vault's item came back: {items:?}");
    let restored = one::backups::restore(&place, &install, "global", &keys["global"]).unwrap();
    assert!(restored.entries.iter().any(|e| e.content.kind == "genesis"));
    assert_eq!(backed_up_posts(&dir, &install, &one::entry::unhex(&recovery).unwrap()).await, ["before the backup"]);
    let steps_then = restored.snapshot.databases["board/main"].0;
    let files: Vec<Vec<u8>> = walk(&dir).iter().map(|f| std::fs::read(f).unwrap()).collect();
    assert!(!files.iter().any(|f| f.windows(12).any(|w| w == b"s3cret-value") || f.windows(17).any(|w| w == b"before the backup")), "nothing is readable on the disk");

    // A change is backed up on the next run; only what's new is sent.
    a.call(board("post", json!({ "text": "after the backup" }))).await.unwrap();
    a.back_up_now();
    // A run is done once it says when it last backed up, not once its copy of the bucket has the post: it copies the
    // bucket before writing the snapshot that names the copy, and backs up every scope.
    eventually("the second backup", async || a.backups().unwrap()["places"][0]["last"]["time"] != first).await;
    let again = one::backups::restore(&place, &install, "global", &keys["global"]).unwrap();
    assert!(again.snapshot.databases["board/main"].0 > steps_then);
    assert_eq!(backed_up_posts(&dir, &install, &one::entry::unhex(&recovery).unwrap()).await, ["before the backup", "after the backup"]);
    let checked = a.check_backups().await.unwrap();
    assert_eq!(checked[0]["place"], "T7 Shield", "{checked}");
    assert!(checked[0]["error"].is_null(), "{checked}");
    let shown = a.backups().unwrap();
    let moment = &shown["snapshots"][0];
    assert!(moment["scopes"].as_array().unwrap().contains(&json!("Everyone")) && moment["places"] == json!(["T7 Shield"]), "one snapshot of the install: {shown}");
    assert!(a.state()["backups"].is_null(), "no badge while backups work");

    // A disk that breaks: admins see what's wrong and its fix, on Settings, until a run works again.
    let copy = dir.join(&install).join("bucket");
    std::fs::rename(&copy, dir.join("bucket-aside")).unwrap();
    std::fs::write(&copy, b"not a folder").unwrap();
    // Its failure shows once it's done: nothing writes to the disk until the next run.
    a.call(board("post", json!({ "text": "broken" }))).await.unwrap();
    a.back_up_now();
    eventually("a failed run", async || a.backups().unwrap()["places"][0]["problem"]["text"].as_str().is_some_and(|t| t.contains("couldn't be used") || t.contains("won't let"))).await;
    let problem = a.state()["backups"].clone();
    assert_eq!(problem["fix"], "Check the disk on backup-a is plugged in and has room.", "{problem}");
    assert!(problem["text"].as_str().unwrap().starts_with("T7 Shield "), "{problem}");
    std::fs::remove_file(&copy).unwrap();
    std::fs::rename(dir.join("bucket-aside"), &copy).unwrap();
    a.call(board("post", json!({ "text": "fixed" }))).await.unwrap();
    a.back_up_now();
    eventually("the badge to clear", async || a.state()["backups"].is_null()).await;

    // A folder's item made before devices were called devices names its device `server`, and is still backed up to;
    // the node's other items aren't places.
    let global = one::entry::Scope::Global;
    let old = temp("backup-old-disk").join("One Backups");
    let fields = json!({ "path": { "value": old }, "server": { "value": one::entry::hex(&a.id()) } });
    a.set_item(&me, &global, json!({ "title": "Old disk", "type": one::backups::KIND, "fields": fields, "apps": ["node"] })).await.unwrap();
    let token = json!({ "token": { "value": "t", "secret": true } });
    a.set_item(&me, &global, json!({ "title": "Cloudflare Tunnel", "type": "Tunnel", "fields": token, "hosts": ["example.com"], "apps": ["node"] })).await.unwrap();
    a.back_up_now();
    let place = |title: &str| a.backups().unwrap()["places"].as_array().unwrap().iter().find(|p| p["title"] == title).cloned();
    eventually("the old disk's backup", async || place("Old disk").is_some_and(|p| p["last"]["device"] == "backup-a")).await;
    assert!(place("Cloudflare Tunnel").is_none(), "{}", a.backups().unwrap());

    // A place typed into the sheet on the person's device is a place too.
    let asked = a.request_place(&me, &json!({ "endpoint": "http://127.0.0.1:9", "title": "Sheet bucket" })).await.unwrap();
    let answer = json!({ "fields": { "endpoint": "http://127.0.0.1:9", "bucket": "b", "key_id": "k", "secret": "s" } });
    a.answer_request(&me, &global, asked["id"].as_str().unwrap(), &answer).await.unwrap();
    assert!(place("Sheet bucket").is_some_and(|p| p["kind"] == "s3"), "{}", a.backups().unwrap());
}

/// The board's posts in `global` as a place's copy of the install's bucket holds them, opened with the recovery key
/// alone: each epoch's key sealed to it, each app's key in the epoch sealed under that.
async fn backed_up_posts(dir: &std::path::Path, install: &str, recovery: &[u8; 32]) -> Vec<String> {
    backed_up(dir, install, recovery, "global", "board", "main", "select text from posts order by time").await
}

/// The first column of what `sql` reads from `app`'s `database` in `scope`, as the backup's copy of the bucket has it,
/// opened with the recovery key alone.
async fn backed_up(dir: &std::path::Path, install: &str, recovery: &[u8; 32], scope: &str, app: &str, database: &str, sql: &str) -> Vec<String> {
    let copy = one::bucket::Place::Folder(dir.join(install).join("bucket"));
    let folder = one::bucket::folder(scope);
    let mut keys = one::db::Keys::new();
    for n in copy.list(&format!("{folder}/_epochs")).unwrap().iter().filter_map(|n| n.parse::<u32>().ok()) {
        let epoch = one::backups::unseal(recovery, &copy.get(&one::keys::epoch_name(scope, n, "recovery")).unwrap().unwrap()).unwrap();
        let Ok(Some(sealed)) = copy.get(&one::keys::app_key_name(scope, app, n)) else { continue };
        keys.insert(n, one::kernel::database_key(&one::kernel::open_app_key(&epoch, app, &sealed).unwrap(), database));
    }
    match one::db::Db::open(&copy, &format!("{folder}/{app}/{database}"), keys).await {
        Ok(db) => db.query(sql, &[]).unwrap().into_iter().filter_map(|r| r.values().next().and_then(|v| v.as_str()).map(str::to_string)).collect(),
        Err(_) => Vec::new(),
    }
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        match e.path().is_dir() {
            true => out.extend(walk(&e.path())),
            false => out.push(e.path()),
        }
    }
    out
}

/// A new install's node starts in a home that isn't there yet, as install.sh starts it.
#[test]
fn a_node_starts_in_a_new_home() {
    let root = std::env::temp_dir().join(format!("one-new-home-{}", rand::random::<u64>()));
    let home = root.join("home");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let cli = one::config::CLI.to_uppercase();
    let mut node = std::process::Command::new(env!("CARGO_BIN_EXE_one"))
        .arg("serve")
        .env(format!("{cli}_HOME"), &home)
        .env(format!("{cli}_ADDR"), format!("127.0.0.1:{port}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let answered = loop {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break true;
        }
        if node.try_wait().unwrap().is_some() || started.elapsed() > Duration::from_secs(60) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = node.kill();
    let out = node.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert!(answered, "the node didn't start in a new home:\n{}", String::from_utf8_lossy(&out.stderr));
}

/// Anyone who reaches a node directly (its LAN, its tailnet, a port its router maps), which the relay never sees, gets
/// no call answered: only a pairing code out lets one ask to join, and nothing else.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_answers_only_the_installs_devices() {
    let a = start("strangers").await;
    a.create("Test", "Andre").await.unwrap();
    let _relay = relay(&a).await;
    eventually("a's endpoint", async || a.endpoint_addr().is_some_and(|e| e.ip_addrs().next().is_some())).await;
    let at = a.endpoint_addr().unwrap();
    let alpn = format!("{}/1", one::config::CLI).into_bytes();
    let stranger = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal).relay_mode(iroh::RelayMode::Disabled).bind().await.unwrap();
    let ask = json!({ "kind": "ask", "body": { "call": { "app": "board", "op": "read", "input": {}, "me": { "key": one::entry::hex(&a.id()), "user": a.user().unwrap() } } } }).to_string();
    let asked = async |stranger: &iroh::Endpoint| -> Option<Vec<u8>> {
        let conn = stranger.connect(at.clone(), &alpn).await.ok()?;
        let (mut send, mut recv) = conn.open_bi().await.ok()?;
        send.write_all(ask.as_bytes()).await.ok()?;
        send.finish().ok()?;
        recv.read_to_end(1 << 20).await.ok()
    };
    assert_eq!(asked(&stranger).await, None, "with no pairing code out, a stranger's connection is closed");
    let _code = a.pair().unwrap();
    let answer: Value = serde_json::from_slice(&asked(&stranger).await.expect("a pairing code out lets it ask")).unwrap();
    assert_eq!(answer["error"], "only your One App's devices are answered", "and only to join: {answer}");
}

/// A scope's epoch is sealed only to an endpoint listed as someone's who uses it, and only by someone who uses it.
#[tokio::test(flavor = "multi_thread")]
async fn epochs_are_sealed_only_to_the_scopes_endpoints() {
    let a = start("keys").await;
    a.create("Home", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    eventually("the board to run", async || a.call(board("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let global = one::entry::Scope::Global;
    let mine = a.sealed_keys(&me, &global, Some(&one::entry::hex(&a.id())), None).await.unwrap();
    assert_eq!(mine["epoch"], 1, "{mine}");
    assert!(mine["sealed"]["1"].is_string(), "{mine}");
    let stranger = one::keys::Endpoint::of(&ed25519_dalek::SigningKey::from_bytes(&rand::random()));
    let sealed = data_encoding::BASE64URL_NOPAD.encode(&one::kernel::seal_to(&stranger.seal, &[1; 32]).unwrap());
    let refused = a.seal_key(&me, &global, 1, Some(&one::entry::hex(&stranger.key)), None, &sealed).await.unwrap_err();
    assert!(format!("{refused:#}").contains("isn't one of the install's"), "{refused:#}");
    let again = a.seal_key(&me, &global, 1, Some(&one::entry::hex(&a.id())), None, &sealed).await.unwrap_err();
    assert!(format!("{again:#}").contains("already"), "what's sealed is never replaced: {again:#}");
}

/// A device is approved only by its person's own device, or a newcomer's by the admin whose invite their proof was made
/// with; a device removed opens no epoch made after.
#[tokio::test(flavor = "multi_thread")]
async fn only_a_persons_own_device_or_their_inviter_lets_a_device_in() {
    use one::node::{DeviceRequest, Invited};
    let dir = temp("approve");
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": "mac" }).to_string()).unwrap();
    let a = Node::start(&dir, one::sidecars::built_in()).await.unwrap();
    a.create("Home", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    eventually("the board to run", async || a.call(board("post", json!({ "text": "hi" }))).await.is_ok()).await;
    let global = one::entry::Scope::Global;
    let new_key = || ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let asks = |key: &ed25519_dalek::SigningKey, user: &str, proof: Option<String>| {
        let e = one::keys::Endpoint::of(key);
        DeviceRequest { key: one::entry::hex(&e.key), seal: one::entry::hex(&e.seal), name: "Chrome".into(), user: user.into(), person: String::new(), proof, service: None }
    };
    let opens = async |key: &ed25519_dalek::SigningKey, scope: &one::entry::Scope| -> Vec<u32> {
        let got = a.sealed_keys(&me, scope, Some(&one::entry::hex(&key.verifying_key().to_bytes())), None).await.unwrap();
        let sealed = got["sealed"].as_object().unwrap();
        sealed.iter().filter(|(_, v)| one::kernel::open_sealed(&one::keys::seal_secret(key), &data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap()).is_ok()).map(|(n, _)| n.parse().unwrap()).collect()
    };

    // Andre's new laptop, approved from his machine, opens global.
    let laptop = new_key();
    a.approve_request(&me, &asks(&laptop, &me.user, None), None).await.unwrap();
    assert_eq!(opens(&laptop, &global).await, vec![1]);

    // Sam is let in only with the proof of the secret Andre's invite made.
    let secret = "the-invites-secret";
    let sam = Invited { secret: secret.into(), name: "Sam".into(), user: one::entry::new_id(), role: one::members::Role::Member, until: u64::MAX };
    let sams = new_key();
    let wrong = one::keys::invite_proof(b"another-secret", &one::keys::Endpoint::of(&sams));
    assert!(a.approve_request(&me, &asks(&sams, &sam.user, Some(wrong)), Some(&sam)).await.is_err());
    assert!(!a.is_user(&sam.user), "a request whose proof doesn't match lets no one in");
    let right = one::keys::invite_proof(secret.as_bytes(), &one::keys::Endpoint::of(&sams));
    a.approve_request(&me, &asks(&sams, &sam.user, Some(right)), Some(&sam)).await.unwrap();
    assert!(a.is_user(&sam.user));
    assert_eq!(opens(&sams, &global).await, vec![1]);

    // Sam's next device is his to approve, not Andre's.
    let refused = a.approve_request(&me, &asks(&new_key(), &sam.user, None), None).await.unwrap_err();
    assert!(format!("{refused:#}").contains("one of their own devices"), "{refused:#}");

    // The laptop, removed, opens no epoch made after; Andre's machine and Sam's device do, and the board goes on.
    let removed = a.remove_device(&me, &one::entry::hex(&laptop.verifying_key().to_bytes())).await.unwrap();
    assert!(removed["new_epochs"].as_array().unwrap().iter().any(|e| e["scope"] == "global" && e["epoch"] == 2), "{removed}");
    assert_eq!(opens(&laptop, &global).await, vec![1]);
    for f in walk(&dir).into_iter().filter(|p| p.to_string_lossy().contains("/_epochs/2/")) {
        assert!(one::kernel::open_sealed(&one::keys::seal_secret(&laptop), &std::fs::read(&f).unwrap()).is_err(), "{} opens for the removed laptop", f.display());
    }
    assert_eq!(opens(&sams, &global).await, vec![1, 2]);
    a.call(board("post", json!({ "text": "after" }))).await.unwrap();
}

/// A client device asks its node's local API over iroh (`<cli>/http/1`) as its own person, and as no one else: the
/// key its connection proves is who it is, whatever headers it sends. A key that isn't a client device of the install
/// gets nothing.
#[tokio::test(flavor = "multi_thread")]
async fn client_devices_ask_their_node_over_iroh_as_their_person() {
    let a = start("clients").await;
    a.create("Clients", "Andre").await.unwrap();
    deploy_module(&a, "hi", r#"export default { name: "hi", ops: { hi: { description: "", input: {}, run: (ctx, input) => ({ hi: input.name }) } } };"#).await;
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    let mine_scope = one::entry::Scope::User(me.user.clone());
    a.install(&me, "hi", &mine_scope, true).await.unwrap();
    eventually("hi to run", async || a.call(call("hi", "hi", json!({ "name": "a" }))).await.is_ok()).await;
    let (mine, sams, sam) = (ed25519_dalek::SigningKey::from_bytes(&rand::random()), ed25519_dalek::SigningKey::from_bytes(&rand::random()), one::entry::new_id());
    let listed = |k: &ed25519_dalek::SigningKey| one::keys::Endpoint::of(k);
    let phone = one::node::DeviceRequest { key: one::entry::hex(&listed(&mine).key), seal: one::entry::hex(&listed(&mine).seal), name: "phone".into(), user: me.user.clone(), person: String::new(), proof: None, service: None };
    a.approve_request(&me, &phone, None).await.unwrap();
    a.member("device", json!({ "key": one::entry::hex(&listed(&sams).key), "seal": one::entry::hex(&listed(&sams).seal), "user": sam, "name": "sam's phone", "roles": ["client"], "user_name": "Sam", "role": "member" })).await.unwrap();
    // The relay lets in the install's endpoints, as each device that approves another tells it.
    let k = tokio::task::block_in_place(|| crate::kernel::Kernel::for_devices([a.id(), mine.verifying_key().to_bytes(), sams.verifying_key().to_bytes()].iter().map(one::entry::hex).collect()));
    a.set_setting(&me, "address", Some(&k.address)).await.unwrap();
    eventually("a's endpoint", async || a.endpoint_addr().is_some_and(|e| e.relay_urls().next().is_some())).await;

    let get = |path: &str| one_client::Request { method: "GET".into(), path: path.into(), headers: Vec::new(), body: Vec::new() };
    let json = async |r: one_client::Response| -> (u16, Value) { (r.status, serde_json::from_slice(&r.body.bytes().await.unwrap()).unwrap()) };
    let phone = one_client::Client::start(mine.to_bytes(), &k.address, &[a.id()]).await.unwrap();
    let (status, state) = json(phone.fetch(get("/v1/state")).await.unwrap()).await;
    assert_eq!((status, state["me"]["user"].as_str()), (200, a.user().ok().as_deref()), "{state}");
    let hi = |body: Value, headers: Vec<(String, String)>| one_client::Request { method: "POST".into(), path: "/v1/call".into(), headers: [vec![("content-type".into(), "application/json".into())], headers].concat(), body: body.to_string().into_bytes() };
    let asked = json!({ "app": "hi", "op": "hi", "input": { "name": "phone" } });
    // A call whose data it hasn't handed the keys of says what it needs, and whom to seal them to.
    let (status, need) = json(phone.fetch(hi(asked.clone(), Vec::new())).await.unwrap()).await;
    assert_eq!((status, need["to"].as_str()), (428, Some(one::entry::hex(&a.id()).as_str())), "{need}");
    // The phone opens them as any client does, from what the node keeps sealed, and hands them for its key.
    let b64 = |v: &Value| data_encoding::BASE64URL_NOPAD.decode(v.as_str().unwrap().as_bytes()).unwrap();
    let (scope, phone_hex) = (mine_scope.to_string(), one::entry::hex(&mine.verifying_key().to_bytes()));
    let (_, epochs) = json(phone.fetch(get(&format!("/v1/keys?scope={scope}&to={phone_hex}"))).await.unwrap()).await;
    let epoch: [u8; 32] = one::kernel::open_sealed(&one::keys::seal_secret(&mine), &b64(&epochs["sealed"]["1"])).unwrap().try_into().unwrap();
    // The app's key in the epoch, made by the first to need it: the node keeps whichever is there.
    let made = json!({ "scope": scope, "n": 1, "app": "hi", "sealed": data_encoding::BASE64URL_NOPAD.encode(&one::kernel::seal_app_key(&epoch, "hi", &rand::random())) });
    let (_, app) = json(phone.fetch(one_client::Request { method: "POST".into(), path: "/v1/keys".into(), headers: vec![("content-type".into(), "application/json".into())], body: made.to_string().into_bytes() }).await.unwrap()).await;
    let app_key = one::kernel::open_app_key(&epoch, "hi", &b64(&app["sealed"])).unwrap();
    let handed = one::keys::Handed { scope: scope.clone(), epoch: 1, app: "hi".into(), database: "main".into(), key: one::kernel::database_key(&app_key, "main") };
    let handoff = one::keys::Handoff { for_: phone_hex.clone(), until: one::node::now_ms() + 3_600_000, keys: vec![handed] };
    let seal: [u8; 32] = one::keys::unhex(need["seal"].as_str().unwrap()).unwrap();
    let sealed = one::keys::seal_handoff(&seal, &handoff).unwrap();
    let (status, said) = json(phone.fetch(hi(asked.clone(), vec![(one::api::KEYS_HEADER.into(), sealed.clone())])).await.unwrap()).await;
    assert_eq!((status, &said["ok"]), (200, &json!({ "hi": "phone" })), "{said}");
    let (status, _) = json(phone.fetch(hi(asked.clone(), Vec::new())).await.unwrap()).await;
    assert_eq!(status, 200, "what it handed is kept for its later calls");
    // Many at once, on the one connection.
    let mut many = tokio::task::JoinSet::new();
    for _ in 0..20 {
        let (phone, req) = (phone.clone(), get("/v1/state"));
        many.spawn(async move { phone.fetch(req).await.map(|r| r.status).ok() });
    }
    assert!(many.join_all().await.iter().all(|s| *s == Some(200)));

    // Sam's phone shows the node's own token and a sign-in: it's still Sam's, and asks nothing only this machine may.
    let sams = one_client::Client::start(sams.to_bytes(), &k.address, &[a.id()]).await.unwrap();
    let posing = |path: &str| one_client::Request { headers: vec![(one::api::LOCAL_TOKEN_HEADER.into(), a.local_token().into()), ("authorization".into(), "Bearer x".into())], ..get(path) };
    let (status, state) = json(sams.fetch(posing("/v1/state")).await.unwrap()).await;
    assert_eq!((status, state["me"]["user"].as_str()), (200, Some(sam.as_str())), "{state}");
    assert_eq!(sams.fetch(posing("/v1/storage")).await.unwrap().status, 403, "only this machine's own programs ask that");
    let (status, refused) = json(sams.fetch(hi(asked.clone(), vec![(one::api::KEYS_HEADER.into(), sealed)])).await.unwrap()).await;
    assert_eq!(status, 400, "the keys Andre's phone handed are for its key alone: {refused}");

    // A key the install doesn't list, reaching the node directly as anyone on its network can.
    let stranger = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal).relay_mode(iroh::RelayMode::Disabled).bind().await.unwrap();
    let alpn = format!("{}/http/1", one::config::CLI).into_bytes();
    let answered = async {
        let conn = stranger.connect(a.endpoint_addr().unwrap(), &alpn).await.ok()?;
        let (mut send, mut recv) = conn.open_bi().await.ok()?;
        send.write_all(b"GET /v1/state HTTP/1.1\r\nhost: node\r\n\r\n").await.ok()?;
        send.finish().ok()?;
        recv.read_to_end(1 << 20).await.ok()
    };
    assert_eq!(answered.await, None, "a stranger's connection is closed");
    phone.close().await;
    sams.close().await;
}

/// A service on an endpoint of its own, with no node beside it, answers its person's calls from a node through the
/// relay once one of their devices approves it, checking each caller itself: a stranger that reaches it is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_reaches_a_service_through_the_relay_and_a_stranger_does_not() {
    let a = start("service-node").await;
    a.create("Home", "Andre").await.unwrap();
    let relay = relay(&a).await;
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
    let install = one::service::Install { address: relay.address.clone(), root: a.state()["install"]["root"].as_str().unwrap().to_string() };
    let service = one::service::Service::open(&temp("service"), "echo", Some(install)).unwrap();
    service.start().await.unwrap();
    // The program offering it, as a Go sidecar registers with a node: it says who asked and what.
    service.sidecars().register("echo", json!({ "title": "Echo", "ops": [{ "name": "say" }] }));
    let offering = service.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(call)) = offering.sidecars().next("echo").await {
                offering.sidecars().reply("echo", call.id, json!({ "ok": { "said": call.input["text"], "by": call.caller.user } })).unwrap();
            }
        }
    });
    let asks = one::node::DeviceRequest { key: one::entry::hex(&service.key()), seal: one::entry::hex(&service.endpoint().seal), name: "box".into(), user: String::new(), person: String::new(), proof: None, service: Some("echo".into()) };
    a.approve_service(&me, &asks).await.unwrap();
    assert_eq!(service.owner(), None, "it knows it's approved only once a node brings it the install's entries");

    let say = CallRequest { app: "echo".into(), sidecar: Some("echo".into()), op: "say".into(), input: json!({ "text": "hi" }), ..Default::default() };
    let mut said = Value::Null;
    eventually("the node's call to reach the service", async || {
        said = a.call(say.clone()).await.unwrap_or_default();
        !said.is_null()
    })
    .await;
    assert_eq!(said, json!({ "said": "hi", "by": me.user }));
    assert_eq!(service.owner(), Some(me.user.clone()));

    // A stranger that reaches it directly is answered nothing, calling as Andre too.
    let stranger = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal).relay_mode(iroh::RelayMode::Disabled).bind().await.unwrap();
    let at = service.addr().unwrap();
    let conn = stranger.connect(iroh::EndpointAddr::from_parts(at.id, at.ip_addrs().map(|a| iroh::TransportAddr::Ip(*a))), &one::node::service_alpn()).await.unwrap();
    let ask = json!({ "kind": "ask", "body": { "call": { "op": "say", "input": { "text": "let me in" }, "me": { "key": one::entry::hex(&a.id()), "user": me.user } } } });
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(ask.to_string().as_bytes()).await.unwrap();
    send.finish().unwrap();
    let answer: Value = serde_json::from_slice(&recv.read_to_end(1 << 20).await.unwrap()).unwrap();
    assert_eq!(answer["error"], "only your One App's devices are answered", "{answer}");
}
