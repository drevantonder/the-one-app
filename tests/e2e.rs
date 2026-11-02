//! Real nodes and phone clients on this machine, talking over iroh (needs the internet for discovery and relays).

use std::time::{Duration, Instant};

use one::node::{CallRequest, Node};
use serde_json::{Value, json};

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("one-e2e-{name}-{}", rand::random::<u32>()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn start(name: &str) -> (Node, iroh::protocol::Router) {
    let dir = temp(name);
    std::fs::write(dir.join("server.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": name }).to_string()).unwrap();
    Node::start(&dir, one::sidecars::built_in()).await.unwrap()
}

async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(45) {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for {what}");
}

fn board(op: &str, input: Value) -> CallRequest {
    CallRequest { app: "board".into(), op: op.into(), input, ..Default::default() }
}

/// The texts of the posts on a server's board of `scope`.
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
async fn servers_and_a_phone_share_a_deployed_board() {
    let (a, _ra) = start("a").await;
    let (b, _rb) = start("b").await;

    let recovery = a.create("Test", "Andre").await.unwrap();
    assert_eq!(recovery.len(), 64, "creating gives the recovery key's secret once");
    assert!(a.create("Another", "Andre").await.is_err(), "a white label holds one install");
    let apps = a.apps(&one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None }).await.unwrap();
    let listed = apps.iter().find(|app| app["name"] == "board").expect("the config's board is deployed");
    assert_eq!(listed["owner"], "admins");
    assert!(listed["ops"].as_array().is_some_and(|ops| ops.iter().any(|o| o["name"] == "post")), "{listed}");
    assert_eq!(a.call(board("post", json!({ "text": "from a" }))).await.unwrap(), "Posted as #1 in the top.");

    b.join(&a.pair(None).unwrap()).await.unwrap();
    assert_eq!(b.user().unwrap(), a.user().unwrap(), "a pairing code from a's user makes another of their servers");
    eventually("b to run the board and see a's post", async || sees(&b, "global", "from a").await).await;
    b.call(board("post", json!({ "text": "from b" }))).await.unwrap();
    assert!(b.join(&a.pair(None).unwrap()).await.is_err(), "a server in an install joins no other");
    eventually("a to see b's post", async || sees(&a, "global", "from b").await).await;
    let read = a.call(board("read", json!({}))).await.unwrap();
    let read = read.as_str().unwrap();
    assert!(read.lines().any(|l| l.starts_with("#1 ") && l.ends_with(": from a")) && read.lines().any(|l| l.starts_with("#2 ") && l.ends_with(": from b")), "agents read posts as numbered lines: {read}");

    // A new deploy runs everywhere.
    let source = include_str!("../apps/board/app.js").replace("  ops: {", "  ops: {\n    ping: { description: \"\", input: {}, run: () => \"pong\" },");
    let bundle = one::bundle::pack(&[("app.js".into(), source.clone().into_bytes()), ("src/app.js".into(), source.into_bytes())]).unwrap();
    let blob = a.put_blob(&bundle).unwrap();
    a.append(&one::entry::Scope::Global, one::node::deploy("board", &blob)).await.unwrap();
    eventually("b to run the new deploy", async || b.call(board("ping", json!({}))).await.is_ok_and(|r| r == "pong")).await;
    assert!(sees(&b, "global", "from b").await, "a new deploy keeps the data");

    let phone = one_client::Client::start(temp("phone"), "Phone").await.unwrap();
    phone.join(&a.pair(None).unwrap()).await.unwrap();
    let state = phone.request("GET", "/v1/state", Value::Null).await.unwrap();
    assert_eq!(state["me"]["name"], "Phone");
    let page = phone.request("GET", "/v1/screen", json!({ "app": "board", "screen": "main", "scope": "global" })).await.unwrap();
    let html = page["html"].as_str().unwrap();
    assert!(html.contains("window.one") && html.contains("one.call(\"topics\""), "a phone gets the board's screen with the bridge");
    let reply = phone.request("POST", "/v1/call", json!({ "app": "board", "op": "post", "input": { "text": "from the phone" } })).await.unwrap();
    assert!(reply.get("error").is_none() && reply.get("elsewhere").is_none(), "{reply}");
    eventually("b to see the phone's post", async || sees(&b, "global", "from the phone").await).await;
    let news = b.call(board("news", json!({ "format": "json", "since": 0 }))).await.unwrap();
    let by_phone = news.as_array().unwrap().iter().find(|p| p["text"] == "from the phone").unwrap();
    assert_eq!(by_phone["name"], "Phone", "posts are credited to the client that asked");
    phone.request("POST", "/v1/name", json!({ "name": "Andre's iPhone" })).await.unwrap();
    let state = phone.request("GET", "/v1/state", Value::Null).await.unwrap();
    assert_eq!(state["me"]["name"], "Andre's iPhone", "a phone renames itself");
    // A phone manages what its user may: installs, groups, the Vault.
    let reply = phone.request("POST", "/v1/install", json!({ "app": "board", "scope": "user" })).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    let group = phone.request("POST", "/v1/group", json!({ "name": "Phone's" })).await.unwrap();
    assert!(group["group"].is_string(), "{group}");
    let reply = phone.request("POST", "/v1/vault", json!({ "scope": "user", "title": "t", "fields": { "t": { "value": "v", "secret": true } }, "hosts": ["example.com"], "apps": ["board"] })).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    let listed = phone.request("GET", "/v1/vault", json!({ "scope": "user" })).await.unwrap();
    assert_eq!(listed[0]["title"], "t", "{listed}");
    let shown = a.state()["install"]["clients"][0].clone();
    assert!(shown["asked"].is_u64(), "a phone shows when it last asked: {shown}");

    // The phone fails over: with a gone, b answers.
    _ra.shutdown().await.unwrap();
    let reply = phone.request("POST", "/v1/call", json!({ "app": "board", "op": "post", "input": { "text": "while a is away" } })).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    assert!(sees(&b, "global", "while a is away").await, "b ran it");

    b.member("remove", json!({ "key": phone.id() })).await.unwrap();
    assert!(phone.request("GET", "/v1/state", Value::Null).await.is_err(), "a removed client is refused");
}

fn call(app: &str, op: &str, input: Value) -> CallRequest {
    CallRequest { app: app.into(), op: op.into(), input, ..Default::default() }
}

/// Deploys a plain module as `app`, made by this server's user if it's new, and installs it in their scope.
async fn deploy_module(node: &Node, app: &str, source: &str) {
    let bundle = one::bundle::pack(&[("app.js".into(), source.as_bytes().to_vec()), ("src/app.js".into(), source.as_bytes().to_vec())]).unwrap();
    let blob = node.put_blob(&bundle).unwrap();
    let me = one::node::Caller { key: node.id(), user: node.user().unwrap(), agent: None, token: None };
    if node.source(app, None, None).await.is_err() {
        node.member("app", json!({ "name": app, "owner": format!("user:{}", me.user) })).await.unwrap();
    }
    node.deploy(&me, app, &blob).await.unwrap();
}

/// A server that answers every request with what it got: `{ host, authorization, body }`.
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

/// An app's screen opened in a browser at the node's own address is that app's code, not this machine's own programs:
/// whatever a page asks there is refused, and nothing the node answers runs as script at its address.
#[tokio::test(flavor = "multi_thread")]
async fn no_page_uses_the_local_api_as_this_machine() {
    let (a, _ra) = start("pages").await;
    a.create("Pages", "Andre").await.unwrap();
    deploy_module(&a, "lamps", r#"export default { name: "lamps", ops: {}, screens: { main: { title: "Lamps", html: "<p>Lamps</p>" } } };"#).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, one::api::router(a)).await.unwrap() });
    let own = at.clone();
    tokio::task::spawn_blocking(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let get = |path: &str, headers: &[(&str, &str)]| {
            let mut req = agent.get(format!("{own}{path}"));
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.call().unwrap()
        };
        // This machine's own programs, a shell fetching a screen among them, send no browser's headers.
        assert_eq!(get("/v1/vault", &[]).status(), 200);
        let screen = get("/v1/screens/lamps/main", &[]);
        assert_eq!(screen.status(), 200);
        assert_eq!(screen.headers()["content-security-policy"], "sandbox", "a screen opened in a browser runs no script at the node's address");
        // What a page at the node's own address asks, a same-origin GET (Sec-Fetch-Site only) or anything with an Origin.
        for headers in [&[("sec-fetch-site", "same-origin")][..], &[("origin", own.as_str())], &[("origin", "null")]] {
            assert_eq!(get("/v1/vault", headers).status(), 403, "{headers:?}");
        }
        let call = agent.post(format!("{own}/v1/call")).header("origin", own.as_str()).header("sec-fetch-site", "same-origin").send_json(json!({ "app": "board", "op": "topics", "input": {} })).unwrap();
        assert_eq!(call.status(), 403);
        // What the person opens themselves, a link to a file the node keeps, say, is theirs.
        assert_eq!(get("/v1/state", &[("sec-fetch-site", "none")]).status(), 200);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_ask_for_items_and_never_see_them() {
    let (a, _ra) = start("asks").await;
    a.create("Asks", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: Some("Claude Code".into()), token: None };
    let mine = one::entry::Scope::User(me.user.clone());
    let ask = json!({ "title": "Example key", "fields": { "user": {}, "key": { "secret": true } }, "hosts": ["api.example.com"] });
    let asked = a.request_item(&me, &mine, "board", None, &ask, "Your example.com API key, from its settings page").await.unwrap();
    let id = asked["id"].as_str().unwrap().to_string();
    assert!(asked["link"].as_str().unwrap().contains(&format!("vault?id={id}")), "{asked}");
    assert_eq!(a.state()["vault_requests"], 1, "the shell shows a request waiting");
    let waiting = a.item_requests(&me, None, Some(&id)).unwrap();
    assert_eq!(waiting[0]["state"], "waiting");
    assert_eq!(waiting[0]["agent"], "Claude Code");
    assert_eq!(waiting[0]["why"], "Your example.com API key, from its settings page");
    assert_eq!(waiting[0]["matches"], json!([]), "no item has those fields yet");

    let answer = json!({ "fields": { "user": "andre", "key": "s3cret" } });
    a.answer_request(&me, &mine, &id, &answer).await.unwrap();
    let done = a.item_requests(&me, None, Some(&id)).unwrap();
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
    let matches = &a.item_requests(&me, None, Some(again)).unwrap()[0]["matches"];
    let item = matches[0]["id"].as_str().unwrap().to_string();
    a.answer_request(&me, &mine, again, &json!({ "item": item })).await.unwrap();
    assert_eq!(a.vault(&me, Some(&mine)).await.unwrap()[0]["apps"], json!(["board", "other"]));

}

#[tokio::test(flavor = "multi_thread")]
async fn apps_use_only_the_items_they_are_switched_on_for() {
    let (a, _ra) = start("vault").await;
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
    let asked = &a.item_requests(&me, None, asked["id"].as_str()).unwrap()[0];
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
}

#[tokio::test(flavor = "multi_thread")]
async fn backups_go_to_a_folder_encrypted_and_restore_with_the_recovery_key_alone() {
    let (a, _ra) = start("backup-a").await;
    let (b, _rb) = start("backup-b").await;
    let recovery = a.create("Home", "Andre").await.unwrap();
    b.join(&a.pair(None).unwrap()).await.unwrap();
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
    assert_eq!(shown["places"][0]["last"]["server"], "backup-a", "the folder's own server backs it up: {shown}");
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
    assert!(checked["vault"].as_array().unwrap().contains(&json!("Example key")), "{checked}");
    let restored = one::backups::restore(&place, &install, "global", &keys["global"]).unwrap();
    assert!(restored.entries.iter().any(|e| e.content.kind == "genesis"));
    assert_eq!(backed_up_posts(&dir, &install, &keys["global"]).await, ["before the backup"]);
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
    assert_eq!(backed_up_posts(&dir, &install, &keys["global"]).await, ["before the backup", "after the backup"]);
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
}

/// The board's posts in `global` as a place's copy of the install's bucket holds them, opened with the scope's key.
async fn backed_up_posts(dir: &std::path::Path, install: &str, keys: &[[u8; 32]]) -> Vec<String> {
    let copy = one::bucket::Place::Folder(dir.join(install).join("bucket"));
    let Ok(Some(sealed)) = copy.get(&one::kernel::app_key_name("global", "board")) else { return Vec::new() };
    for key in keys {
        let Ok(app) = one::kernel::open_app_key(key, "board", &sealed) else { continue };
        let key = one::kernel::database_key(&app, "main");
        if let Ok(db) = one::db::Db::open(&copy, "global/board/main", key).await {
            return db.query("select text from posts order by time", &[]).unwrap().into_iter().map(|r| r["text"].as_str().unwrap().to_string()).collect();
        }
    }
    Vec::new()
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
