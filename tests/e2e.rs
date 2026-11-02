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

fn key(node: &Node) -> String {
    one::entry::hex(&node.id())
}

#[tokio::test(flavor = "multi_thread")]
async fn servers_and_a_phone_share_a_deployed_board() {
    let (a, _ra) = start("a").await;
    let (b, _rb) = start("b").await;

    let recovery = a.create("Test", "Andre").await.unwrap();
    assert_eq!(recovery.len(), 64, "creating gives the recovery key's secret once");
    assert!(a.create("Another", "Andre").await.is_err(), "a white label holds one install");
    let apps = a.apps(&one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None }).await.unwrap();
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
    // A phone manages what its user may: installs, groups, secrets.
    let reply = phone.request("POST", "/v1/install", json!({ "app": "board", "scope": "user" })).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    let group = phone.request("POST", "/v1/group", json!({ "name": "Phone's" })).await.unwrap();
    assert!(group["group"].is_string(), "{group}");
    let reply = phone.request("POST", "/v1/secrets", json!({ "scope": "user", "app": "board", "name": "t", "hosts": ["example.com"], "value": "v" })).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    let listed = phone.request("GET", "/v1/secrets", json!({ "scope": "user" })).await.unwrap();
    assert_eq!(listed[0]["name"], "t", "{listed}");
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

#[tokio::test(flavor = "multi_thread")]
async fn users_groups_trust_and_privileged_servers() {
    let (andre, _r1) = start("andre").await;
    let (bob, _r2) = start("bob").await;
    let (cloud, _r3) = start("cloud").await;
    andre.create("Family", "Andre").await.unwrap();
    andre.call(board("post", json!({ "text": "andre's own", "scope": "user" }))).await.unwrap();

    // A new user, on a server of their own.
    let code = andre.pair(Some(("Bob".into(), one::members::Role::Member))).unwrap();
    bob.join(&code).await.unwrap();
    assert_ne!(bob.user().unwrap(), andre.user().unwrap());
    eventually("bob to run the board", async || bob.call(board("news", json!({ "format": "json", "since": 0 }))).await.is_ok()).await;
    assert!(texts(&bob, "user").await.is_empty(), "bob's server never gets andre's own data");
    let a_user = format!("user:{}", andre.user().unwrap());
    let shut = bob.call(CallRequest { scopes: Some(vec![a_user.clone()]), ..board("news", json!({ "scope": a_user })) }).await;
    assert!(shut.is_err(), "nobody reaches another user's scope");

    // A group shares data between them, on both their servers.
    andre.member("group", json!({ "group": "f000000000000001", "name": "Family" })).await.unwrap();
    andre.member("invite", json!({ "group": "f000000000000001", "user": bob.user().unwrap(), "role": "member" })).await.unwrap();
    eventually("bob to see the invite", async || bob.state()["install"]["groups"][0]["invited"].as_array().is_some_and(|i| !i.is_empty())).await;
    bob.member("accept", json!({ "group": "f000000000000001" })).await.unwrap();
    eventually("bob to post in the group", async || bob.call(board("post", json!({ "text": "hi family", "scope": "group:f000000000000001" }))).await.is_ok()).await;
    eventually("andre to see bob's group post", async || sees(&andre, "group:f000000000000001", "hi family").await).await;
    let news = andre.call(board("news", json!({ "format": "json", "since": 0, "scope": "group:f000000000000001" }))).await.unwrap();
    assert_eq!(news[0]["user"], bob.user().unwrap().as_str());

    // A page is a scope's installs: the board is on Everyone's, and on the group's once installed there too.
    let me = one::node::Caller { key: andre.id(), user: andre.user().unwrap(), agent: None };
    let everyone = andre.widgets(me.clone(), Some("global")).await.unwrap();
    assert!(everyone.iter().any(|w| w["app"] == "board"), "{everyone:?}");
    assert!(andre.widgets(me.clone(), Some("group:f000000000000001")).await.unwrap().is_empty());
    andre.append(&one::entry::Scope::Group("f000000000000001".into()), one::members::node("install", json!({ "app": "board" }))).await.unwrap();
    let widgets = andre.widgets(me, Some("group:f000000000000001")).await.unwrap();
    assert_eq!(widgets[0]["root"]["c"][0]["c"][1]["text"], "hi family", "a group's page shows its board: {widgets:?}");
    // It draws each size it lists, the small one its own way.
    assert_eq!(widgets[0]["sizes"]["small"]["c"][1]["text"], "hi family", "{widgets:?}");

    // A phone-only user needs a server that serves them.
    let carol_code = andre.pair(Some(("Carol".into(), one::members::Role::Member))).unwrap();
    let carol = one_client::Client::start(temp("carol"), "Carol's phone").await.unwrap();
    let refused = carol.join(&carol_code).await.unwrap_err();
    assert!(format!("{refused:#}").contains("no server for you yet"), "{refused:#}");

    // A privileged server: owned by the install, it holds everyone's data and serves everyone.
    cloud.join(&andre.pair(None).unwrap()).await.unwrap();
    eventually("cloud to be in", async || cloud.state()["install"]["member"] == true).await;
    andre.member("acl", json!({ "server": key(&cloud), "owners": ["install"], "users": ["install"] })).await.unwrap();
    eventually("cloud to hold bob's group post", async || sees(&cloud, "group:f000000000000001", "hi family").await).await;
    assert!(bob.member("untrust", json!({ "server": key(&cloud) })).await.is_err(), "nobody untrusts a privileged server");
    let (carol_code, carol_id) = andre.pair_local(Some(("Carol".into(), one::members::Role::Member)), None).unwrap();
    carol.join(&carol_code).await.unwrap();
    eventually("cloud to know carol's phone", async || cloud.state()["install"]["clients"].as_array().unwrap().iter().any(|c| c["name"] == "Carol's phone")).await;
    let reply = carol.request("POST", "/v1/call", json!({ "app": "board", "op": "post", "input": { "text": "carol here", "scope": "user" } })).await.unwrap();
    assert!(reply.get("ok").is_some(), "the privileged server serves a phone-only user: {reply}");
    let state = carol.request("GET", "/v1/state", Value::Null).await.unwrap();
    assert_eq!(state["user"]["id"], carol_id.as_str(), "a new user's code says who it admits: {state}");

    // An admin pairs another of someone's machines, as them.
    assert!(bob.pair_local(None, Some("Carol")).is_err(), "only admins pair other people's machines");
    let (code, user) = andre.pair_local(None, Some("Carol")).unwrap();
    assert_eq!(user, carol_id);
    let ipad = one_client::Client::start(temp("carol-ipad"), "Carol's iPad").await.unwrap();
    ipad.join(&code).await.unwrap();
    let state = ipad.request("GET", "/v1/state", Value::Null).await.unwrap();
    assert_eq!(state["user"]["id"], carol_id.as_str(), "{state}");

    // Bob's call goes elsewhere from a server that doesn't hold what it touches, and runs where it does.
    let bobs_user = format!("user:{}", bob.user().unwrap());
    let req = CallRequest { server: Some(key(&andre)), scopes: Some(vec![bobs_user.clone()]), ..board("post", json!({ "text": "x", "scope": "user" })) };
    let err = bob.call(req).await.unwrap_err();
    assert!(format!("{err:#}").contains("doesn't serve you"), "{err:#}");
    let req = CallRequest { server: Some(key(&cloud)), ..board("post", json!({ "text": "bob via cloud", "scope": "user" })) };
    bob.call(req).await.unwrap();
    eventually("bob's own server to get what cloud wrote for him", async || sees(&bob, "user", "bob via cloud").await).await;

    // An admin opens bob's data on the privileged server: read only, and bob sees it.
    andre.open(&bob.user().unwrap(), &key(&cloud), None).await.unwrap();
    let read = CallRequest { server: Some(key(&cloud)), as_user: Some(bob.user().unwrap()), ..board("news", json!({ "format": "json", "since": 0, "scope": "user" })) };
    let posts = andre.call(read.clone()).await.unwrap();
    assert!(posts.as_array().unwrap().iter().any(|p| p["text"] == "bob via cloud"));
    let write = CallRequest { input: json!({ "text": "nope", "scope": "user" }), op: "post".into(), ..read };
    assert!(andre.call(write).await.is_err(), "opened data is read only");
    eventually("bob to see who opened his data", async || bob.state()["install"]["opens"].as_array().is_some_and(|o| !o.is_empty())).await;

    // Leaving the group: bob's server deletes it.
    bob.member("member", json!({ "group": "f000000000000001", "user": bob.user().unwrap(), "role": null })).await.unwrap();
    assert!(texts(&bob, "group:f000000000000001").await.is_empty(), "a server deletes a scope it no longer holds");
}

/// Serves a node's local API on a port of its own, as `one serve` does, for the CLI and git.
async fn serve_api(node: &Node) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let router = one::api::router(node.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

/// Runs the CLI against the node serving `addr`, in `dir`; its output, or what went wrong.
async fn cli(addr: &str, dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let (addr, dir) = (addr.to_string(), dir.to_path_buf());
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_one")).args(&args).current_dir(&dir).env(format!("{}_ADDR", one::config::CLI.to_uppercase()), &addr).output().unwrap();
        match out.status.success() {
            true => Ok(String::from_utf8_lossy(&out.stdout).to_string()),
            false => Err(String::from_utf8_lossy(&out.stderr).to_string()),
        }
    })
    .await
    .unwrap()
}

fn call(app: &str, op: &str, input: Value) -> CallRequest {
    CallRequest { app: app.into(), op: op.into(), input, ..Default::default() }
}

#[tokio::test(flavor = "multi_thread")]
async fn deploys_build_with_bun_keep_their_source_and_roll_back() {
    let (a, _ra) = start("deployer").await;
    let (bob, _rb) = start("bob").await;
    a.create("Deploys", "Andre").await.unwrap();
    let api = serve_api(&a).await;

    // An app with an npm package and a module of its own, in TypeScript.
    let dir = temp("hello").join("hello");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("package.json"), r#"{ "name": "hello", "dependencies": { "escape-html": "1.0.3" } }"#).unwrap();
    std::fs::write(dir.join("greet.ts"), "export const greet = (who: string): string => `hello ${who}`;\n").unwrap();
    let app = |word: &str| {
        format!("import escape from \"escape-html\";\nimport {{ greet }} from \"./greet\";\nexport default {{\n  name: \"hello\",\n  title: \"Hello\",\n  ops: {{ hi: {{ description: \"\", input: {{}}, run: () => escape(greet(\"<{word}>\")) }} }},\n}};\n")
    };
    std::fs::write(dir.join("app.ts"), app("one")).unwrap();
    let out = cli(&api, &dir, &["deploy", "."]).await.unwrap();
    assert!(out.contains("Deployed hello"), "{out}");
    cli(&api, &dir, &["install", "hello"]).await.unwrap();
    eventually("the first deploy to run", async || a.call(call("hello", "hi", json!({}))).await.is_ok_and(|r| r == "hello &lt;one&gt;")).await;

    // Its source is readable as written, with the lockfile Bun made.
    let files = a.source("hello", None, None).await.unwrap();
    let paths: Vec<&str> = files["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert!(paths.contains(&"app.js") && paths.contains(&"src/app.ts") && paths.contains(&"src/greet.ts") && paths.contains(&"src/bun.lock"), "{paths:?}");
    assert!(!paths.iter().any(|p| p.contains("node_modules")), "{paths:?}");
    assert_eq!(a.source("hello", None, Some("src/app.ts")).await.unwrap()["text"], app("one"));
    let shown = cli(&api, &dir, &["source", "hello", "src/greet.ts"]).await.unwrap();
    assert!(shown.contains("export const greet"), "{shown}");

    // A new deploy, then a rollback to the first.
    std::fs::write(dir.join("app.ts"), app("two")).unwrap();
    cli(&api, &dir, &["deploy", "."]).await.unwrap();
    eventually("the second deploy to run", async || a.call(call("hello", "hi", json!({}))).await.is_ok_and(|r| r == "hello &lt;two&gt;")).await;
    let first = files["bundle"].as_str().unwrap().to_string();
    cli(&api, &dir, &["deploy", "hello", "--version", &first[..12]]).await.unwrap();
    eventually("the rollback to run", async || a.call(call("hello", "hi", json!({}))).await.is_ok_and(|r| r == "hello &lt;one&gt;")).await;
    let deploys = cli(&api, &dir, &["deploys", "hello"]).await.unwrap();
    assert_eq!(deploys.lines().count(), 3, "{deploys}");
    assert!(deploys.lines().last().unwrap().starts_with(&first[..12]) && deploys.contains("(running)"), "{deploys}");

    // Only someone acting for its owner deploys it.
    bob.join(&a.pair(Some(("Bob".into(), one::members::Role::Member))).unwrap()).await.unwrap();
    eventually("bob to know the app", async || bob.source("hello", None, None).await.is_ok()).await;
    let bob_me = one::node::Caller { key: bob.id(), user: bob.user().unwrap(), agent: None };
    let refused = bob.deploy(&bob_me, "hello", &first).await.unwrap_err();
    assert!(format!("{refused:#}").contains("only someone acting for hello's owner"), "{refused:#}");
}

/// Deploys a plain module as `app`, made by this server's user if it's new, and installs it in their scope.
async fn deploy_module(node: &Node, app: &str, source: &str) {
    let bundle = one::bundle::pack(&[("app.js".into(), source.as_bytes().to_vec()), ("src/app.js".into(), source.as_bytes().to_vec())]).unwrap();
    let blob = node.put_blob(&bundle).unwrap();
    let me = one::node::Caller { key: node.id(), user: node.user().unwrap(), agent: None };
    if node.source(app, None, None).await.is_err() {
        node.member("app", json!({ "name": app, "owner": format!("user:{}", me.user) })).await.unwrap();
    }
    node.deploy(&me, app, &blob).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_install_moves_its_data_and_every_server_follows() {
    let (a, _ra) = start("move-a").await;
    let (b, _rb) = start("move-b").await;
    a.create("Moving", "Andre").await.unwrap();
    b.join(&a.pair(None).unwrap()).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    assert_eq!(a.storage().unwrap()["kind"], "folder", "a new install keeps its data in a folder on its server");
    eventually("the board to run", async || a.call(board("post", json!({ "text": "before the move" }))).await.is_ok()).await;

    let to = temp("move-bucket");
    let item = json!({ "title": "Storage", "type": "Storage", "fields": { "path": { "value": to.display().to_string() } }, "hosts": [], "apps": ["node"] });
    a.set_item(&me, &one::entry::Scope::Global, item).await.unwrap();
    for n in [&a, &b] {
        eventually("each server to move", async || n.storage().unwrap()["where"] == to.display().to_string().as_str()).await;
    }
    assert!(sees(&b, "global", "before the move").await, "what was there came along");
    b.call(board("post", json!({ "text": "after the move" }))).await.unwrap();
    eventually("a to see it", async || sees(&a, "global", "after the move").await).await;
    assert!(to.join("global/board/main/newest").exists(), "it's kept in the new place");
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

#[tokio::test(flavor = "multi_thread")]
async fn secrets_are_set_never_read_and_go_only_to_their_hosts() {
    let (a, _ra) = start("secrets").await;
    a.create("Secrets", "Andre").await.unwrap();
    let port = echo().await;
    deploy_module(&a, "keeper", r#"export default {
  name: "keeper",
  ops: {
    send: { run: async (ctx, { url, auth, body }) => {
      const r = await fetch(url, { method: "POST", headers: { authorization: auth }, body });
      if (!r.ok) throw new Error(await r.text());
      return r.json();
    } },
    keep: { run: (ctx, { value }) => { ctx.user.secret("kept", value, ["127.0.0.1"]); return null; } },
    status: { run: async (ctx, { url }) => (await fetch(url)).status },
  },
};"#)
    .await;
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    // Its earlier name: the secret `token`, which is an item of its own.
    let secret = |who: &one::node::Caller, scope: &str, name: &str, hosts: Value, value: Value| {
        let (a, who) = (a.clone(), who.clone());
        let body = json!({ "scope": scope, "app": "keeper", "name": name, "hosts": hosts, "value": value });
        async move { a.vault_route(&who, "POST", "/v1/secrets", &body).await }
    };
    secret(&me, "user", "token", json!(["127.0.0.1"]), json!("s3cret")).await.unwrap();
    a.append(&one::entry::Scope::User(me.user.clone()), one::members::node("install", json!({ "app": "keeper" }))).await.unwrap();
    let send = |url: String, auth: &str, body: &str| call("keeper", "send", json!({ "url": url, "auth": auth, "body": body }));

    // Filled in for its host, in headers and bodies.
    let got = a.call(send(format!("http://127.0.0.1:{port}/x"), "Bearer {{token}}", "key={{token}}&other={{nothing}}")).await.unwrap();
    assert_eq!(got["authorization"], "Bearer s3cret");
    assert_eq!(got["body"], "key=s3cret&other={{nothing}}", "a placeholder no secret has stays as it is");
    // Refused for any other host, though it's the same server.
    let refused = a.call(send(format!("http://localhost:{port}/x"), "Bearer {{token}}", "")).await.unwrap_err();
    assert!(format!("{refused:#}").contains("sent only to 127.0.0.1"), "{refused:#}");

    assert_eq!(a.call(call("keeper", "status", json!({ "url": "https://github.com/" }))).await.unwrap(), 200, "https goes out as https (github.com redirects http)");

    // Nothing reads it back: not the list, the state, the apps, nor the app itself.
    let listed = a.secrets(&me, None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["name"], "token");
    assert_eq!(listed[0]["hosts"], json!(["127.0.0.1"]));
    for shown in [json!(listed), a.state(), json!(a.apps(&me).await.unwrap())] {
        assert!(!shown.to_string().contains("s3cret"), "a secret's value never shows: {shown}");
    }

    // A call sets its own app's secret, which it can use but not read.
    a.call(call("keeper", "keep", json!({ "value": "given" }))).await.unwrap();
    let got = a.call(send(format!("http://127.0.0.1:{port}/"), "{{kept}}", "")).await.unwrap();
    assert_eq!(got["authorization"], "given");
    // The list says which a person or agent gave, and which the app's own code keeps.
    let given = |name: &str| a.secrets(&me, None).unwrap().into_iter().find(|s| s["name"] == name).map(|s| s["given"].clone());
    assert_eq!(given("token"), Some(json!(true)));
    assert_eq!(given("kept"), Some(json!(false)));

    // Deleted, it's no longer filled in; and only someone who manages a scope sets secrets there.
    secret(&me, "user", "token", json!([]), Value::Null).await.unwrap();
    let got = a.call(send(format!("http://127.0.0.1:{port}/"), "Bearer {{token}}", "")).await.unwrap();
    assert_eq!(got["authorization"], "Bearer {{token}}");
    let other = one::node::Caller { key: a.id(), user: "0123456789abcdef".into(), agent: None };
    assert!(secret(&other, "global", "x", json!(["a.com"]), json!("v")).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn apps_install_in_several_scopes_use_each_other_and_fork() {
    let (a, _ra) = start("apps").await;
    a.create("Apps", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    let mine = one::entry::Scope::User(me.user.clone());
    a.member("group", json!({ "group": "a000000000000001", "name": "Team" })).await.unwrap();
    let team = one::entry::Scope::Group("a000000000000001".into());
    let notes = r#"export default {
  name: "notes",
  title: "Notes",
  schema: Object.fromEntries(["user", "group", "global"].map((k) => [k, "create table notes (text text)"])),
  ops: {
    add: { run: (ctx, { text, scope }) => { (scope ? ctx.group(scope) : ctx.user).exec("insert into notes values (?)", [text]); return null; } },
    list: { run: async (ctx, { scope }) => (await (scope ? ctx.group(scope) : ctx.user).query("select text from notes")).map((n) => n.text) },
    who: { reads: true, run: (ctx) => ctx.me },
  },
};"#;
    deploy_module(&a, "notes", notes).await;
    deploy_module(&a, "journal", r#"export default {
  name: "journal",
  uses: { apps: ["notes"], sidecars: ["shell"] },
  ops: {
    jot: { run: async (ctx, { text }) => { await ctx.apps.notes.add({ text }); return ctx.apps.notes.list({}); } },
    sneak: { run: (ctx) => ctx.apps.board.read({}) },
    echo: { run: (ctx) => ctx.sidecars.shell.run({ command: "echo from a sidecar" }) },
    keep: { run: async (ctx, { text }) => { const hash = await ctx.blob.put(new TextEncoder().encode(text)); return new TextDecoder().decode(await ctx.blob.get(hash)); } },
    open: { run: async (ctx, { hash }) => new TextDecoder().decode(await ctx.blob.get(hash)) },
    catalog: { run: async (ctx) => (await ctx.catalog()).map((a) => a.name) },
    who: { run: (ctx) => ctx.apps.notes.who({}) },
    share: { run: (ctx, { scope }) => { ctx.install("journal", scope); return null; } },
  },
};"#)
    .await;

    // Installing an app installs what it uses; it calls it as the same user.
    a.install(&me, "journal", &mine, true).await.unwrap();
    let installed = |name: &str| {
        let a = a.clone();
        let (name, me) = (name.to_string(), me.clone());
        async move { a.apps(&me).await.unwrap().into_iter().find(|x| x["name"] == name.as_str()).map(|x| x["installed"].clone()).unwrap_or_default() }
    };
    assert_eq!(installed("notes").await, json!([mine.to_string()]), "journal's install brought notes");
    eventually("journal to run", async || a.call(call("journal", "jot", json!({ "text": "first" }))).await.is_ok_and(|r| r == json!(["first"]))).await;
    let refused = a.call(call("journal", "sneak", json!({}))).await.unwrap_err();
    assert!(format!("{refused:#}").contains("doesn't use board"), "{refused:#}");
    let echoed = a.call(call("journal", "echo", json!({}))).await.unwrap();
    assert!(echoed.to_string().contains("from a sidecar"), "{echoed}");
    assert_eq!(a.call(call("journal", "keep", json!({ "text": "in a blob" }))).await.unwrap(), "in a blob");
    // A call asked on the server's own machine reads a blob put there through the local API, to attach it.
    let put = a.put_blob(b"put locally").unwrap();
    assert_eq!(a.call(call("journal", "open", json!({ "hash": put }))).await.unwrap(), "put locally");
    let names = a.call(call("journal", "catalog", json!({}))).await.unwrap();
    assert!(names.as_array().unwrap().contains(&json!("notes")), "{names}");
    // The agent that asked is credited, and stays on the calls an app makes for it.
    let who = a.call(CallRequest { agent: Some("Claude Code".into()), ..call("journal", "who", json!({})) }).await.unwrap();
    assert_eq!((who["agent"].as_str(), who["user_name"].as_str()), (Some("Claude Code"), Some("Andre")), "{who}");
    assert_eq!(a.call(call("journal", "who", json!({}))).await.unwrap()["agent"], Value::Null, "no agent, none credited");
    let notes_ops = a.apps(&me).await.unwrap().into_iter().find(|x| x["name"] == "notes").unwrap()["ops"].clone();
    assert_eq!(notes_ops.as_array().unwrap().iter().find(|o| o["name"] == "who").unwrap()["reads"], true, "op flags reach the catalog");

    // The same app in another scope: one version, data per scope.
    a.install(&me, "notes", &team, true).await.unwrap();
    a.call(call("notes", "add", json!({ "text": "for the team", "scope": "a000000000000001" }))).await.unwrap();
    assert_eq!(a.call(call("notes", "list", json!({ "scope": "a000000000000001" }))).await.unwrap(), json!(["for the team"]));
    assert_eq!(a.call(call("notes", "list", json!({}))).await.unwrap(), json!(["first"]));
    assert_eq!(installed("notes").await, json!([mine.to_string(), team.to_string()]), "yours first, then groups'");

    // ctx.install, with the caller's rights: an admin may install for everyone.
    a.call(call("journal", "share", json!({ "scope": "global" }))).await.unwrap();
    let global = installed("journal").await;
    assert!(global.as_array().unwrap().contains(&json!("global")), "{global}");
    a.install(&me, "journal", &one::entry::Scope::Global, false).await.unwrap();
    assert!(!installed("journal").await.as_array().unwrap().contains(&json!("global")), "uninstalled");

    // A fork: a new name, its own data, and a link to where it came from.
    a.fork(&me, "notes", "jottings", None).await.unwrap();
    a.install(&me, "jottings", &mine, true).await.unwrap();
    eventually("the fork to run", async || a.call(call("jottings", "list", json!({}))).await.is_ok_and(|r| r == json!([]))).await;
    a.call(call("jottings", "add", json!({ "text": "forked" }))).await.unwrap();
    assert_eq!(a.call(call("notes", "list", json!({}))).await.unwrap(), json!(["first"]), "a fork's data is its own");
    let apps = a.apps(&me).await.unwrap();
    let find = |n: &str| apps.iter().find(|x| x["name"] == n).unwrap().clone();
    assert_eq!(find("jottings")["from"], "notes");
    assert_eq!(find("notes")["forks"], json!(["jottings"]));
    assert_eq!(find("jottings")["title"], "Notes");
}

/// Runs stock git in `dir`.
async fn git(dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let dir = dir.to_path_buf();
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(&dir)
            .envs([("GIT_AUTHOR_NAME", "T"), ("GIT_AUTHOR_EMAIL", "t@example.com"), ("GIT_COMMITTER_NAME", "T"), ("GIT_COMMITTER_EMAIL", "t@example.com"), ("GIT_CONFIG_NOSYSTEM", "1"), ("HOME", dir.to_str().unwrap())])
            .output()
            .unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if out.status.success() { Ok(text) } else { Err(text) }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn repos_clone_push_and_fork_with_stock_git() {
    let (a, _ra) = start("git-a").await;
    let (bob, _rb) = start("git-bob").await;
    a.create("Repos", "Andre").await.unwrap();
    let (api_a, work) = (serve_api(&a).await, temp("git"));
    let url = |addr: &str, repo: &str| format!("http://{addr}/repos/{repo}.git");

    // A new repo, pushed from a local one.
    let src = work.join("src");
    std::fs::create_dir_all(&src).unwrap();
    git(&src, &["init", "-q", "-b", "main"]).await.unwrap();
    std::fs::write(src.join("app.js"), "export default { name: \"hello\" };\n").unwrap();
    git(&src, &["add", "."]).await.unwrap();
    git(&src, &["commit", "-qm", "first"]).await.unwrap();
    git(&src, &["remote", "add", "origin", &url(&api_a, "hello")]).await.unwrap();
    git(&src, &["push", "-q", "origin", "main"]).await.unwrap();

    // Cloned elsewhere, changed and pushed back; fetched by the first.
    git(&work, &["clone", "-q", &url(&api_a, "hello"), "copy"]).await.unwrap();
    let copy = work.join("copy");
    assert_eq!(std::fs::read_to_string(copy.join("app.js")).unwrap(), "export default { name: \"hello\" };\n");
    std::fs::write(copy.join("README"), "hi\n").unwrap();
    git(&copy, &["add", "."]).await.unwrap();
    git(&copy, &["commit", "-qm", "second"]).await.unwrap();
    git(&copy, &["push", "-q"]).await.unwrap();
    git(&src, &["pull", "-q", "origin", "main"]).await.unwrap();
    assert_eq!(std::fs::read_to_string(src.join("README")).unwrap(), "hi\n");

    // A push from behind is refused, unless forced.
    std::fs::write(src.join("app.js"), "// mine\n").unwrap();
    git(&src, &["commit", "-qam", "third"]).await.unwrap();
    git(&src, &["push", "-q", "origin", "main"]).await.unwrap();
    std::fs::write(copy.join("README"), "behind\n").unwrap();
    git(&copy, &["commit", "-qam", "behind"]).await.unwrap();
    let refused = git(&copy, &["push", "-q"]).await.unwrap_err();
    assert!(refused.contains("non-fast-forward") || refused.contains("rejected"), "{refused}");

    // A big push (git sends it in chunks, after asking with an empty one), and a branch made and deleted.
    let noise: String = (0..400_000u64).map(|i| format!("{:x}", i.wrapping_mul(0x9e3779b97f4a7c15) >> 40)).collect::<Vec<_>>().join("\n");
    std::fs::write(src.join("big.txt"), noise).unwrap();
    git(&src, &["add", "."]).await.unwrap();
    git(&src, &["commit", "-qm", "big"]).await.unwrap();
    git(&src, &["push", "-q", "origin", "main"]).await.unwrap();
    git(&src, &["push", "-q", "origin", "main:side"]).await.unwrap();
    git(&src, &["push", "-q", "origin", ":side"]).await.unwrap();

    // Each push's pack was merged into the ones before it, smaller ones first: fewer packs than pushes, and a fresh
    // clone has everything.
    let r = a.call(call("repos", "get", json!({ "repo": "hello" }))).await.unwrap();
    assert!(r["packs"].as_array().unwrap().len() < 3, "packs are merged: {}", r["packs"]);
    git(&work, &["clone", "-q", &url(&api_a, "hello"), "fresh"]).await.unwrap();
    git(&work.join("fresh"), &["fsck", "--strict"]).await.unwrap();
    assert_eq!(git(&work.join("fresh"), &["rev-parse", "HEAD"]).await.unwrap(), git(&src, &["rev-parse", "HEAD"]).await.unwrap());
    git(&src, &["rm", "-q", "big.txt"]).await.unwrap();
    git(&src, &["commit", "-qm", "smaller"]).await.unwrap();
    git(&src, &["push", "-q", "origin", "main"]).await.unwrap();

    // Another user can't push to it: they fork it, push to their fork, and its owner fetches from there.
    bob.join(&a.pair(Some(("Bob".into(), one::members::Role::Member))).unwrap()).await.unwrap();
    let api_bob = serve_api(&bob).await;
    eventually("bob to run repos", async || bob.call(call("repos", "list", json!({}))).await.is_ok_and(|r| r.as_array().is_some_and(|l| !l.is_empty()))).await;
    let bobs = work.join("bobs");
    eventually("bob to clone hello as it is now", async || {
        let _ = std::fs::remove_dir_all(&bobs);
        git(&work, &["clone", "-q", &url(&api_bob, "hello"), "bobs"]).await.is_ok() && std::fs::read_to_string(bobs.join("app.js")).is_ok_and(|t| t == "// mine\n")
    })
    .await;
    std::fs::write(bobs.join("app.js"), "// bob's idea\n").unwrap();
    git(&bobs, &["commit", "-qam", "idea"]).await.unwrap();
    let refused = git(&bobs, &["push", "-q", "origin", "main"]).await.unwrap_err();
    assert!(refused.contains("fork it"), "{refused}");
    bob.call(call("repos", "fork", json!({ "repo": "hello", "name": "hello-bob" }))).await.unwrap();
    git(&bobs, &["push", "-q", &url(&api_bob, "hello-bob"), "main"]).await.unwrap();
    eventually("a to have bob's idea", async || {
        let _ = git(&src, &["fetch", "-q", &url(&api_a, "hello-bob"), "main"]).await;
        git(&src, &["show", "FETCH_HEAD:app.js"]).await.is_ok_and(|t| t == "// bob's idea\n")
    })
    .await;

    // A fork has the branches and history, under a new name.
    a.call(call("repos", "fork", json!({ "repo": "hello", "name": "hello-too" }))).await.unwrap();
    git(&work, &["clone", "-q", &url(&api_a, "hello-too"), "fork"]).await.unwrap();
    assert_eq!(std::fs::read_to_string(work.join("fork/app.js")).unwrap(), "// mine\n");
    let r = a.call(call("repos", "get", json!({ "repo": "hello" }))).await.unwrap();
    assert_eq!(r["forks"], json!(["hello-bob", "hello-too"]));

    // Its pages read the packs: folders, files, history and diffs, as git has them.
    let tree = a.call(call("repos", "tree", json!({ "repo": "hello" }))).await.unwrap();
    let names: Vec<&str> = tree["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["app.js", "README"]);
    assert_eq!(tree["readme"]["text"], "hi\n");
    assert_eq!(tree["commit"]["subject"], "smaller");
    let file = a.call(call("repos", "file", json!({ "repo": "hello", "path": "app.js" }))).await.unwrap();
    assert_eq!(file["text"], "// mine\n");
    let log = a.call(call("repos", "log", json!({ "repo": "hello" }))).await.unwrap();
    let subjects: Vec<&str> = log["commits"].as_array().unwrap().iter().map(|c| c["subject"].as_str().unwrap()).collect();
    assert_eq!(subjects, ["smaller", "big", "third", "second", "first"]);
    let head = log["commits"][0]["sha"].as_str().unwrap();
    assert_eq!(head, git(&src, &["rev-parse", "main"]).await.unwrap().trim());
    let commit = a.call(call("repos", "commit", json!({ "repo": "hello", "sha": head }))).await.unwrap();
    assert_eq!(commit["files"][0]["path"], "big.txt");
    let earlier = log["commits"][2]["sha"].as_str().unwrap();
    let commit = a.call(call("repos", "commit", json!({ "repo": "hello", "sha": earlier }))).await.unwrap();
    assert_eq!(commit["files"][0]["path"], "app.js");
    assert_eq!(commit["files"][0]["hunks"][0]["lines"], json!(["-export default { name: \"hello\" };", "+// mine"]));
    let branches = a.call(call("repos", "branches", json!({ "repo": "hello" }))).await.unwrap();
    assert_eq!(branches.as_array().unwrap().iter().map(|b| b["branch"].as_str().unwrap()).collect::<Vec<_>>(), ["main"]);
    let missing = a.call(call("repos", "file", json!({ "repo": "hello", "path": "nope" }))).await.unwrap_err();
    assert!(format!("{missing:#}").contains("There's no file nope"), "{missing:#}");
    let html = a.screen(one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None }, "repos", "repo", json!({ "repo": "hello" }), None).await.unwrap();
    assert!(html.contains("one.call(\"tree\""), "repos has screens");
    let html = a.screen(one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None }, "repos", "nope", json!({}), None).await.unwrap();
    assert!(html.contains("one.notFound()"), "a screen that isn't there says so");

    // Its owners delete it to Recently Deleted, then restore it or erase it; a fork keeps the packs it shares.
    let refused = bob.call(call("repos", "delete", json!({ "repo": "hello", "confirm": true }))).await.unwrap_err();
    assert!(format!("{refused:#}").contains("deletes, restores or erases"), "{refused:#}");
    a.call(call("repos", "delete", json!({ "repo": "hello", "confirm": true }))).await.unwrap();
    let listed = a.call(call("repos", "list", json!({}))).await.unwrap();
    assert!(listed.as_array().unwrap().iter().any(|r| r["name"] == "hello" && r["deleted"]["time"].is_u64() && r["yours"] == true), "{listed}");
    let gone = git(&work, &["clone", "-q", &url(&api_a, "hello"), "deleted"]).await.unwrap_err();
    assert!(gone.contains("was deleted"), "{gone}");
    assert!(a.call(call("repos", "tree", json!({ "repo": "hello" }))).await.unwrap()["deleted"].is_object(), "its page says so");
    a.call(call("repos", "restore", json!({ "repo": "hello" }))).await.unwrap();
    git(&work, &["clone", "-q", &url(&api_a, "hello"), "restored"]).await.unwrap();
    assert!(a.call(call("repos", "erase", json!({ "repo": "hello", "confirm": true }))).await.is_err(), "deleted before erased");
    a.call(call("repos", "delete", json!({ "repo": "hello", "confirm": true }))).await.unwrap();
    a.call(call("repos", "erase", json!({ "repo": "hello", "confirm": true }))).await.unwrap();
    let listed = a.call(call("repos", "list", json!({}))).await.unwrap();
    assert!(!listed.as_array().unwrap().iter().any(|r| r["name"] == "hello"), "erased, it's gone: {listed}");
    assert!(listed.as_array().unwrap().iter().any(|r| r["name"] == "hello-too" && r["from"].is_null()), "its fork stays, forked from nothing now: {listed}");
    git(&work, &["clone", "-q", &url(&api_a, "hello-too"), "fork-after"]).await.unwrap();
    assert_eq!(std::fs::read_to_string(work.join("fork-after/app.js")).unwrap(), "// mine\n", "the fork's packs stay");
    a.call(call("repos", "create", json!({ "name": "hello" }))).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_app_store_lists_adds_and_shows_code() {
    let (a, _ra) = start("store").await;
    a.create("Store", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    a.member("group", json!({ "group": "b000000000000001", "name": "Team" })).await.unwrap();
    assert_eq!(a.state()["config"]["store"], "apps");
    assert_eq!(a.state()["brand"], a.state()["config"], "shells built before it was config read brand");
    // The install's settings are the white label's until an admin sets one, and null puts it back.
    let default = one::config::SETTINGS.iter().find(|(k, _)| *k == "made_here").map_or(serde_json::Value::Null, |(_, v)| json!(v));
    assert_eq!(a.state()["install"]["settings"]["made_here"], default);
    a.set_setting(&me, "made_here", Some("Made by the team")).await.unwrap();
    assert_eq!(a.state()["install"]["settings"]["made_here"], "Made by the team");
    let html = a.screen(me.clone(), "apps", "main", json!({}), None).await.unwrap();
    assert!(html.contains("\"made_here\":\"Made by the team\""), "screens get it as one.config.made_here");
    a.set_setting(&me, "push", Some("")).await.unwrap();
    assert_eq!(a.state()["config"]["push"], "", "push is off");
    a.set_setting(&me, "made_here", None).await.unwrap();
    assert_eq!(a.state()["install"]["settings"]["made_here"], default);
    let listed = a.call(call("apps", "list", json!({}))).await.unwrap();
    let names: Vec<&str> = listed["apps"].as_array().unwrap().iter().filter(|x| x["installed"].as_array().unwrap().contains(&json!("global"))).map(|x| x["name"].as_str().unwrap()).collect();
    assert!(["board", "repos", "apps", "devices"].iter().all(|n| names.contains(n)), "a new install starts with the config's apps, for everyone: {names:?}");
    let places: Vec<&str> = listed["places"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(places, ["You", "Team", "Everyone"]);

    a.call(call("apps", "add", json!({ "app": "board", "scope": "group:b000000000000001" }))).await.unwrap();
    let listed = a.call(call("apps", "list", json!({}))).await.unwrap();
    let board = listed["apps"].as_array().unwrap().iter().find(|x| x["name"] == "board").unwrap().clone();
    assert_eq!(board["installed_names"], json!(["Team", "Everyone"]), "{board}");
    a.call(call("apps", "remove", json!({ "app": "board", "scope": "group:b000000000000001" }))).await.unwrap();

    let code = a.call(call("apps", "code", json!({ "app": "board" }))).await.unwrap();
    assert!(code["files"].as_array().unwrap().iter().any(|f| f["path"] == "src/app.js"), "{code}");
    let file = a.call(call("apps", "code", json!({ "app": "board", "path": "src/app.js" }))).await.unwrap();
    assert!(file["text"].as_str().unwrap().contains("name: \"board\""));
    for screen in ["main", "app", "file"] {
        let html = a.screen(me.clone(), "apps", screen, json!({ "app": "board", "path": "src/app.js" }), None).await.unwrap();
        assert!(html.contains("one.call("), "{screen}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn built_in_apps_are_listed_added_and_kept_up_to_date() {
    let (a, _ra) = start("built-in").await;
    a.create_with("Built in", "Andre", &["apps"]).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    let app = |listed: &Value, name: &str| listed["apps"].as_array().unwrap().iter().find(|x| x["name"] == name).cloned().unwrap();

    let listed = a.call(call("apps", "list", json!({}))).await.unwrap();
    let board = app(&listed, "board");
    assert_eq!((board["built_in"].clone(), board["added"].clone(), board["title"].clone()), (json!(true), json!(false), json!("Board")), "{board}");
    assert!(board["description"].as_str().is_some_and(|d| d.contains("posts")), "a built-in app's code describes it before it's added: {board}");
    assert_eq!((board["color"].clone(), board["image"].clone()), (json!("#ff9500"), json!(null)), "{board}");
    let (whatsapp, computers) = (app(&listed, "whatsapp"), app(&listed, "agent-computers"));
    let (docs, agents) = (app(&listed, "docs"), app(&listed, "agents"));
    let service = |a: &Value| (a["service"].clone(), a["connector"].clone(), a["connected"].clone());
    let yes = (json!(true), json!(true), json!(true));
    assert_eq!((service(&whatsapp), service(&docs), service(&agents)), (yes.clone(), yes.clone(), yes), "an app with no screen and no widget is a service, under its old names too: {whatsapp} {docs} {agents}");
    let computer = app(&listed, "computer");
    assert_eq!((computers["service"].clone(), computer["service"].clone(), board["service"].clone()), (json!(false), json!(false), json!(false)), "an app with a page isn't one: {computers} {computer} {board}");
    for only in [json!({ "service": true }), json!({ "connector": true }), json!({ "connected": true })] {
        let listed = a.call(call("apps", "list", only.clone())).await.unwrap();
        let names: Vec<_> = listed["apps"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect();
        assert!(names.contains(&"whatsapp".into()) && names.contains(&"agents".into()) && !names.contains(&"agent-computers".into()), "{only} lists only services: {names:?}");
    }
    assert_eq!(whatsapp["uses"], json!({ "apps": [], "sidecars": [] }), "nothing runs it yet: {whatsapp}");
    let drive = app(&listed, "drive");
    assert!(drive["image"].as_str().is_some_and(|i| i.starts_with("data:image/png;base64,")), "icon.png in an app's folder is its picture");
    let everyone = listed["places"].as_array().unwrap().iter().find(|p| p["name"] == "Everyone").unwrap().clone();
    assert_eq!((everyone["may"].clone(), everyone["why"].clone()), (json!(true), json!(null)), "an admin may add for everyone");
    let store = app(&listed, "apps");
    assert_eq!((store["added"].clone(), store["up_to_date"].clone()), (json!(true), json!(true)), "{store}");
    assert!(!a.apps(&me).await.unwrap().iter().any(|x| x["name"] == "board"), "the install's own list has only what it added");
    let code = a.call(call("apps", "code", json!({ "app": "board", "path": "src/app.js" }))).await.unwrap();
    assert!(code["text"].as_str().unwrap().contains("name: \"board\""), "a built-in app's code reads before it's added");

    a.call(call("apps", "add", json!({ "app": "whatsapp", "scope": "user" }))).await.unwrap();
    eventually("whatsapp to run", async || a.call(call("whatsapp", "accounts", json!({}))).await.is_ok_and(|v| v == json!([]))).await;
    let needs = a.call(call("whatsapp", "chats", json!({}))).await.unwrap_err();
    assert!(format!("{needs:#}").contains("WhatsApp needs one of your servers to link your phone to"), "{needs:#}");

    a.call(call("apps", "add", json!({ "app": "board", "scope": "user" }))).await.unwrap();
    let board = app(&a.call(call("apps", "list", json!({}))).await.unwrap(), "board");
    assert_eq!((board["added"].clone(), board["up_to_date"].clone(), board["installed_names"].clone()), (json!(true), json!(true), json!(["You"])), "{board}");
    assert!(board["description"].as_str().is_some_and(|d| d.contains("posts")), "{board}");
    assert_eq!(a.apps(&me).await.unwrap().iter().find(|x| x["name"] == "board").unwrap()["color"], json!("#ff9500"), "a running app's colour is in the list");
    a.call(board_in_user("post", json!({ "text": "hello" }))).await.unwrap();

    // Another deploy is out of date until the release's version is deployed again, as `one update` does.
    deploy_module(&a, "board", "export default { name: \"board\", ops: {} };").await;
    let up_to_date = async |want: bool| a.call(call("apps", "list", json!({}))).await.is_ok_and(|l| app(&l, "board")["up_to_date"] == want);
    eventually("board out of date", async || up_to_date(false).await).await;
    let built_in = a.apps(&me).await.unwrap().iter().find(|x| x["name"] == "board").unwrap()["built_in"].as_str().unwrap().to_string();
    a.deploy(&me, "board", &built_in).await.unwrap();
    eventually("board up to date", async || up_to_date(true).await).await;
}

fn board_in_user(op: &str, mut input: Value) -> CallRequest {
    input["scope"] = json!("user");
    board(op, input)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sidecars_screen_is_served_wherever_it_runs() {
    let (a, _ra) = start("screens-a").await;
    let (b, _rb) = start("screens-b").await;
    a.create("Screens", "Andre").await.unwrap();
    b.join(&a.pair(None).unwrap()).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    b.register_sidecar("lamps", json!({ "title": "Lamps", "ops": [], "screens": [{ "name": "main", "title": "Lamps" }] })).await.unwrap();
    // b's sidecar answers the node's `_screen` calls.
    let sidecar = b.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(c)) = sidecar.sidecars().next("lamps").await {
                let body = format!("<p>{} {}</p>", c.input["name"].as_str().unwrap(), c.input["params"]["room"].as_str().unwrap_or(""));
                let reply = if c.op == "_screen" { json!({ "ok": body }) } else { json!({ "error": "no" }) };
                sidecar.sidecars().reply("lamps", c.id, reply).unwrap();
            }
        }
    });
    eventually("a to list b's sidecar on its user's page", async || {
        let apps = a.apps(&me).await.unwrap();
        apps.iter().any(|x| x["name"] == "lamps" && x["installed"] == json!([format!("user:{}", me.user)]))
    })
    .await;
    let html = a.screen(me, "lamps", "main", json!({ "room": "kitchen" }), None).await.unwrap();
    assert!(html.contains("<p>main kitchen</p>") && html.contains("window.one"), "a serves b's sidecar's screen in the kit and bridge: {html}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_wraps_the_sidecar_of_its_name_on_another_server() {
    let (a, _ra) = start("wraps-a").await;
    let (b, _rb) = start("wraps-b").await;
    a.create("Wraps", "Andre").await.unwrap();
    b.join(&a.pair(None).unwrap()).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    b.register_sidecar("lamps", json!({ "title": "Lamps", "image": "data:image/png;base64,AAAA", "image_kind": "icon", "ops": [{ "name": "on" }] })).await.unwrap();
    let sidecar = b.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(c)) = sidecar.sidecars().next("lamps").await {
                sidecar.sidecars().reply("lamps", c.id, json!({ "ok": format!("{} from {}", c.op, c.from.unwrap_or_default()) })).unwrap();
            }
        }
    });
    eventually("a to reach b's sidecar", async || a.call(call("lamps", "on", json!({}))).await.is_ok_and(|v| v == "on from ")).await;

    // Deployed, an app of the sidecar's name takes its calls, and reaches the sidecar with ctx.sidecars.
    deploy_module(&a, "lamps", r#"export default { name: "lamps", uses: { sidecars: ["lamps"] }, ops: { on: { run: async (ctx) => "app: " + await ctx.sidecars.lamps.on({}) } } };"#).await;
    a.install(&me, "lamps", &one::entry::Scope::User(me.user.clone()), true).await.unwrap();
    eventually("the app to take lamps' calls", async || a.call(call("lamps", "on", json!({}))).await.is_ok_and(|v| v == "app: on from lamps")).await;
    eventually("b to run the app too", async || b.call(call("lamps", "on", json!({}))).await.is_ok_and(|v| v == "app: on from lamps")).await;
    // A call that names the sidecar still reaches it, as sidecars reach each other.
    let direct = CallRequest { sidecar: Some("lamps".into()), ..call("", "on", json!({})) };
    assert_eq!(a.call(direct).await.unwrap(), "on from ");
    let listed = a.apps(&me).await.unwrap();
    assert_eq!(listed.iter().filter(|x| x["name"] == "lamps").count(), 1, "the app stands for its sidecar in the list");
    let lamps = listed.iter().find(|x| x["name"] == "lamps").unwrap();
    assert_eq!((lamps["image"].as_str(), lamps["image_kind"].as_str()), (Some("data:image/png;base64,AAAA"), Some("icon")), "it shows its sidecar's picture: {lamps}");
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_profiles_are_vault_items_whose_key_only_the_agent_computers_sidecar_sees() {
    let (a, _ra) = start("profiles").await;
    a.create_with("Profiles", "Andre", &["agent-computers", "computer"]).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    // The sidecar as the Go one answers: it gets the profile's key filled in, and reports a save to the app.
    a.register_sidecar("agent-computers", json!({ "title": "Agent Computers", "ops": [{ "name": "borrow" }, { "name": "save_profile" }] })).await.unwrap();
    let sidecar = a.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(c)) = sidecar.sidecars().next("agent-computers").await {
                let p = c.input["profile"].clone();
                let reply = match c.op.as_str() {
                    "borrow" => json!({ "computer": "pool-a", "key_length": p["key"].as_str().map(str::len), "blob": p["blob"], "echo": p["key"] }),
                    "save_profile" => {
                        let (node, item) = (sidecar.clone(), p["item"].as_str().unwrap().to_string());
                        let blob = node.put_blob(b"sealed profile").unwrap();
                        let saved = node.call(call("agent-computers", "_saved", json!({ "item": item, "blob": blob, "salt": "5a17", "sites": "outlook.com 2026-11-02" }))).await;
                        json!({ "saved": saved.is_ok() })
                    }
                    _ => json!(null),
                };
                sidecar.sidecars().reply("agent-computers", c.id, json!({ "ok": reply })).unwrap();
            }
        }
    });
    let added = a.call(call("agent-computers", "add_profile", json!({ "profile": "PTC" }))).await.unwrap();
    assert!(a.call(call("agent-computers", "add_profile", json!({ "profile": "ptc" }))).await.is_err(), "one profile of a name");
    let item = a.vault(&me, None).await.unwrap().into_iter().find(|i| i["id"] == added["item"]).unwrap();
    assert_eq!((item["title"].as_str(), item["type"].as_str()), (Some("PTC"), Some("Browser profile")), "{item}");
    assert!(!item.to_string().contains("\"value\"") || item["fields"].as_array().unwrap().iter().all(|f| f["secret"] == true), "its key is never shown: {item}");

    // Agents borrow through Computer Use by the profile's name; the sidecar gets its key, nobody else.
    let borrowed = a.call(call("computer", "borrow", json!({ "for": "Reading mail", "profile": "PTC" }))).await.unwrap();
    assert_eq!((borrowed["key_length"].as_u64(), borrowed["echo"].as_str(), borrowed["blob"].as_str()), (Some(43), Some("••••"), Some("")), "{borrowed}");
    assert!(a.call(call("computer", "borrow", json!({ "for": "x", "profile": "Nope" }))).await.unwrap_err().to_string().contains("No browser profile Nope"));

    // A save is recorded on the item: its blob and salt for the next borrow, and the sites it was used on.
    let saved = a.call(call("agent-computers", "save_profile", json!({ "profile": "PTC" }))).await.unwrap();
    assert_eq!(saved["saved"], true, "{saved}");
    let listed = a.call(call("agent-computers", "profiles", json!({}))).await.unwrap();
    assert_eq!(listed[0]["sites"], "outlook.com 2026-11-02", "{listed}");
    let again = a.call(call("computer", "borrow", json!({ "for": "Reading mail", "profile": "ptc" }))).await.unwrap();
    assert_eq!(again["blob"].as_str().map(str::len), Some(64), "the next borrow starts from the save: {again}");
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_use_a_device_its_owner_let_them_through_computer_use_and_each_session_is_logged() {
    let (a, _ra) = start("device-a").await;
    let (b, _rb) = start("device-b").await;
    a.create_with("Devices", "Andre", &["computer", "agent-computers"]).await.unwrap();
    b.join(&a.pair(None).unwrap()).await.unwrap();
    // b is a Mac whose owner let agents use it: its computer sidecar answers as the Mac app's does.
    b.register_sidecar("computer", json!({ "title": "Computer Use", "ops": [{ "name": "_status" }, { "name": "_use" }] })).await.unwrap();
    let sidecar = b.clone();
    tokio::spawn(async move {
        let mut started = false;
        loop {
            if let Ok(Some(c)) = sidecar.sidecars().next("computer").await {
                let reply = match c.op.as_str() {
                    "_status" => json!({ "on": true, "title": "Andre's Mac", "kind": "mac" }),
                    "_use" => {
                        let first = !std::mem::replace(&mut started, true);
                        json!({ "result": { "content": [{ "type": "text", "text": c.input["tool"] }] }, "title": "Andre's Mac", "started": first.then_some("2026-11-01T09:00:00Z") })
                    }
                    _ => json!(null),
                };
                sidecar.sidecars().reply("computer", c.id, json!({ "ok": reply })).unwrap();
            }
        }
    });
    let listed = async || a.call(call("computer", "list", json!({}))).await;
    eventually("the device among the computers", async || listed().await.is_ok_and(|l| l["computers"].as_array().is_some_and(|d| d.iter().any(|d| d["device"] == true)))).await;
    let device = listed().await.unwrap()["computers"][0].clone();
    assert_eq!((device["title"].clone(), device["state"].clone()), (json!("Andre's Mac"), json!("running")), "listed with no agent computers anywhere: {device}");
    let name = device["name"].as_str().unwrap().to_string();

    for tool in ["list_windows", "click"] {
        let used = a.call(call("computer", "use", json!({ "computer": name, "tool": tool, "args": {} }))).await.unwrap();
        assert_eq!(used["content"][0]["text"], json!(tool), "use reaches the device's sidecar: {used}");
    }
    // Computer Use's page lists them.
    let devices = a.call(call("computer", "_devices", json!({}))).await.unwrap();
    let sessions = devices["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "a session is logged once, when it starts: {devices}");
    assert_eq!((sessions[0]["device"].clone(), sessions[0]["title"].clone()), (json!(name), json!("Andre's Mac")), "{devices}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_servers_owner_removes_a_sidecar_there_and_allows_it_again() {
    let (a, _ra) = start("sidecars-a").await;
    let (b, _rb) = start("sidecars-b").await;
    a.create("Sidecars", "Andre").await.unwrap();
    b.join(&a.pair(None).unwrap()).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    let describe = json!({ "title": "Lamps", "description": "The lights", "version": "abc123", "ops": [{ "name": "on" }] });
    b.register_sidecar("lamps", describe.clone()).await.unwrap();
    let sidecar = b.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(c)) = sidecar.sidecars().next("lamps").await {
                sidecar.sidecars().reply("lamps", c.id, json!({ "ok": "on" })).unwrap();
            }
        }
    });
    let b_name = b.state()["server"]["name"].as_str().unwrap().to_string();
    let on_b = |state: &Value, list: &str| state["install"]["servers"].as_array().unwrap().iter().find(|s| s["name"] == b_name.as_str()).unwrap()[list].clone();
    let lamps = |state: &Value| on_b(state, "installed_sidecars").as_array().unwrap().iter().find(|d| d["name"] == "lamps").cloned();
    eventually("a to see b's sidecar", async || lamps(&a.state()).is_some()).await;
    let listed = lamps(&a.state()).unwrap();
    assert_eq!((listed["name"].clone(), listed["version"].clone(), listed["installed_by"].clone(), listed["users"].clone()), (json!("lamps"), json!("abc123"), json!("Andre"), json!(["Andre"])), "{listed}");
    let shell = on_b(&a.state(), "installed_sidecars").as_array().unwrap().iter().any(|d| d["name"] == "shell" && d["built_in"] == true);
    assert!(shell, "the node's own sidecars are listed as built in");

    // An app over it is a connector whose page says where it runs, and offers its owner to remove it there.
    deploy_module(&a, "lamps", r#"export default { name: "lamps", connector: true, needs: "Needs a lamp", uses: { sidecars: ["lamps"] }, ops: {} };"#).await;
    a.install(&me, "lamps", &one::entry::Scope::User(me.user.clone()), true).await.unwrap();
    let servers = async || a.call(call("apps", "servers", json!({ "app": "lamps" }))).await;
    eventually("the App Store to say where lamps runs", async || servers().await.is_ok_and(|w| w["runs"] == json!([b_name.as_str()]))).await;
    let place = servers().await.unwrap();
    assert_eq!((place["line"].as_str(), place["remove"].clone(), place["ask"].clone()), (Some(format!("Runs on {b_name}").as_str()), json!([b_name.as_str()]), json!(false)), "{place}");

    // Its owner removes it from b: b stops serving it and refuses it until it's allowed again.
    a.call(call("apps", "remove_from", json!({ "app": "lamps", "server": b_name }))).await.unwrap();
    eventually("b to stop serving lamps", async || a.call(CallRequest { sidecar: Some("lamps".into()), ..call("", "on", json!({})) }).await.is_err()).await;
    eventually("the App Store to offer it back", async || servers().await.is_ok_and(|w| w["install"] == json!([b_name.as_str()]) && w["line"] == "Needs a lamp")).await;
    assert!(b.register_sidecar("lamps", describe.clone()).await.is_err_and(|e| format!("{e:#}").contains("removed")), "b refuses it");
    eventually("a to see it removed", async || on_b(&a.state(), "removed_sidecars").as_array().is_some_and(|l| l.len() == 1)).await;
    a.call(call("apps", "install_on", json!({ "app": "lamps", "server": b_name }))).await.unwrap();
    eventually("b to take lamps again", async || b.register_sidecar("lamps", describe.clone()).await.is_ok()).await;
    eventually("a to reach lamps again", async || a.call(CallRequest { sidecar: Some("lamps".into()), ..call("", "on", json!({})) }).await.is_ok()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn schedules_run_once_per_slot_on_one_server_and_switch_off() {
    let (a, _ra) = start("sched-a").await;
    let (b, _rb) = start("sched-b").await;
    a.create("Schedules", "Andre").await.unwrap();
    b.join(&a.pair(None).unwrap()).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    deploy_module(&a, "ticker", r#"export default {
  name: "ticker",
  schema: { global: "create table ticks (by text)" },
  schedules: {
    tick: { description: "Ticks every minute.", op: "tick", every: "1m" },
    broken: { op: "tick", every: "5s" },
  },
  ops: {
    tick: { run: (ctx) => { ctx.global.exec("insert into ticks values (?)", [ctx.me.name]); return null; } },
    ticks: { run: async (ctx) => (await ctx.global.query("select by from ticks")).map((t) => t.by) },
  },
};"#)
    .await;
    a.install(&me, "ticker", &one::entry::Scope::Global, true).await.unwrap();
    let ticker = |n: &Node| {
        let (n, me) = (n.clone(), me.clone());
        async move { n.apps(&me).await.unwrap().into_iter().find(|x| x["name"] == "ticker").unwrap() }
    };
    eventually("b to run ticker", async || b.call(call("ticker", "ticks", json!({}))).await.is_ok()).await;
    let shown = ticker(&a).await;
    let tick = shown["schedules"].as_array().unwrap().iter().find(|s| s["name"] == "tick").unwrap().clone();
    assert_eq!(tick["when"], "every minute");
    assert_eq!(tick["on"], true);
    assert_eq!(tick["runs_as"], "Andre", "it runs as whoever installed it");
    let broken = shown["schedules"].as_array().unwrap().iter().find(|s| s["name"] == "broken").unwrap().clone();
    assert!(broken["invalid"].as_str().unwrap().contains("once a minute"), "{broken}");

    // The next minute's slot runs once, on one of the two servers.
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(100) && a.call(call("ticker", "ticks", json!({}))).await.unwrap().as_array().unwrap().is_empty() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    tokio::time::sleep(Duration::from_secs(25)).await;
    let ticks = a.call(call("ticker", "ticks", json!({}))).await.unwrap();
    let activity = ticker(&b).await["activity"].clone();
    assert!(!activity.as_array().unwrap().is_empty(), "the run shows in the activity: {activity}");
    let slots_run = activity.as_array().unwrap().len();
    assert_eq!(ticks.as_array().unwrap().len(), slots_run, "each slot ran once, on one server: {ticks} {activity}");
    assert_eq!(activity[0]["ok"], true, "{activity}");

    // Switched off, it stops.
    a.switch_schedule(&me, "ticker", "tick", &one::entry::Scope::Global, false).await.unwrap();
    let off = ticker(&a).await["schedules"].as_array().unwrap().iter().find(|s| s["name"] == "tick").unwrap().clone();
    assert_eq!(off["on"], false, "{off}");
    assert!(off["next"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn loop_guards_cap_depth_and_stop_a_looping_app() {
    let (a, _ra) = start("loops").await;
    a.create("Loops", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    deploy_module(&a, "pong", r#"export default { name: "pong", ops: { ping: { run: (ctx) => ctx.from } } };"#).await;
    deploy_module(&a, "loopy", r#"export default {
  name: "loopy",
  uses: { apps: ["loopy", "pong"] },
  ops: {
    deeper: { run: async (ctx, { n }) => n >= 10 ? n : ctx.apps.loopy.deeper({ n: n + 1 }) },
    spin: { run: async (ctx) => { for (let i = 0; i < 700; i++) await ctx.apps.pong.ping({}); return "done"; } },
    from: { run: (ctx) => ctx.apps.pong.ping({}) },
  },
};"#)
    .await;
    a.install(&me, "loopy", &one::entry::Scope::User(me.user.clone()), true).await.unwrap();
    eventually("loopy to run", async || a.call(call("loopy", "from", json!({}))).await.is_ok()).await;
    assert_eq!(a.call(call("loopy", "from", json!({}))).await.unwrap(), "loopy", "the callee sees which app called it");
    let deep = a.call(call("loopy", "deeper", json!({ "n": 0 }))).await.unwrap_err();
    assert!(format!("{deep:#}").contains("at most 3 deep"), "{deep:#}");
    let spun = a.call(call("loopy", "spin", json!({}))).await.unwrap_err();
    assert!(format!("{spun:#}").contains("stopped"), "{spun:#}");
    let again = a.call(call("loopy", "from", json!({}))).await.unwrap_err();
    assert!(format!("{again:#}").contains("looping"), "a stopped app stays stopped: {again:#}");
    // Deployed again, it runs.
    deploy_module(&a, "loopy", r#"export default { name: "loopy", uses: { apps: ["pong"] }, ops: { from: { run: (ctx) => ctx.apps.pong.ping({}) } } };"#).await;
    eventually("loopy to run again", async || a.call(call("loopy", "from", json!({}))).await.is_ok()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn apps_badge_their_icons_and_the_install_pushes_through_the_sidecar_its_setting_names() {
    let (a, _ra) = start("push").await;
    a.create("Push", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    let mine = one::entry::Scope::User(me.user.clone());
    deploy_module(&a, "reminders", r#"export default { name: "reminders",
        schema: { user: "create table due (id text primary key)" },
        async badge(ctx) { return (await ctx.user.query("select count(*) as n from due"))[0].n; },
        ops: { nudge: { async run(ctx) { ctx.user.exec("insert or ignore into due values (?)", [String(ctx.now())]); return ctx.notify({ title: "Pay the rent", screen: "bills" }); } } } };"#).await;
    a.install(&me, "reminders", &mine, true).await.unwrap();

    // With no push sidecar anywhere, the app's call fails there, and its badge still counts.
    eventually("reminders to run without a push sidecar", async || a.call(call("reminders", "nudge", json!({}))).await.is_err_and(|e| format!("{e:#}").contains("push"))).await;
    assert_eq!(a.badges(me.clone(), Some("user")).await.unwrap(), json!({}), "a failed call writes nothing");

    // A push sidecar on this server, serving everyone, hears who pushed and from which app.
    a.register_sidecar("push", json!({ "title": "Push", "serves": "everyone", "ops": [{ "name": "send" }] })).await.unwrap();
    let sidecar = a.clone();
    let (heard, mut hear) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(c)) = sidecar.sidecars().next("push").await {
                sidecar.sidecars().reply("push", c.id, json!({ "ok": { "accepted": 1, "devices": 1, "gone": [] } })).unwrap();
                heard.send(c).unwrap();
            }
        }
    });
    let pushed = a.call(call("reminders", "nudge", json!({}))).await.unwrap();
    assert_eq!(pushed["accepted"], 1, "{pushed}");
    let c = hear.recv().await.unwrap();
    assert_eq!((c.from.as_deref(), c.caller.user.as_str(), c.input["screen"].as_str()), (Some("reminders"), me.user.as_str(), Some("bills")));
    assert_eq!(a.badges(me.clone(), Some("user")).await.unwrap(), json!({ "reminders": 1 }));

    // An agent's or the CLI's push comes from no app.
    a.call(call("push", "send", json!({ "title": "Done" }))).await.unwrap();
    assert_eq!(hear.recv().await.unwrap().from, None);

    // With the install's push off, an app's push isn't sent, and says so.
    a.set_setting(&me, "push", Some("")).await.unwrap();
    let off = a.call(call("reminders", "nudge", json!({}))).await.unwrap();
    assert_eq!(off["sent"], false, "{off}");
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_ask_for_items_and_never_see_them() {
    let (a, _ra) = start("asks").await;
    a.create("Asks", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: Some("Claude Code".into()) };
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

    // Its earlier name asks for one secret field, and is declined.
    let old = a.vault_route(&me, "POST", "/v1/secrets/request", &json!({ "app": "board", "name": "other", "hosts": ["example.org"] })).await.unwrap().unwrap();
    assert!(old["link"].as_str().unwrap().contains("secret?id="), "{old}");
    let old = old["id"].as_str().unwrap();
    assert_eq!(a.item_requests(&me, None, Some(old)).unwrap()[0]["name"], "other", "shells built before ask for it by name");
    a.vault_route(&me, "POST", "/v1/secrets/answer", &json!({ "id": old, "value": null })).await.unwrap();
    assert_eq!(a.item_requests(&me, None, Some(old)).unwrap()[0]["state"], "declined");
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_and_sidecars_use_the_items_switched_on_for_them_without_seeing_them() {
    let (a, _ra) = start("fetch").await;
    a.create("Fetch", "Andre").await.unwrap();
    let person = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    let agent = one::node::Caller { agent: Some("Claude Code".into()), ..person.clone() };
    let mine = one::entry::Scope::User(person.user.clone());
    let port = echo().await;
    let token = json!({ "title": "Example key", "fields": { "token": { "value": "s3cret", "secret": true } }, "hosts": ["127.0.0.1"] });
    let id = a.set_item(&agent, &mine, token).await.unwrap();

    // An agent never lets agents use an item itself: its person does.
    assert!(a.set_item(&agent, &mine, json!({ "id": id, "apps": ["agents"] })).await.is_err());
    let fetch = |url: String| {
        let (a, agent) = (a.clone(), agent.clone());
        async move { a.vault_fetch(&agent, &json!({ "url": url, "headers": { "authorization": "Bearer {{token}}" }, "body": "t={{token}}" })).await }
    };
    let got = fetch(format!("http://127.0.0.1:{port}/x")).await.unwrap();
    assert!(got["body"].as_str().unwrap().contains("Bearer {{token}}"), "not filled before the person let agents use it: {got}");
    a.set_item(&person, &mine, json!({ "id": id, "apps": ["agents"] })).await.unwrap();

    // Filled for its host, and the site's echo of it blotted out.
    let got = fetch(format!("http://127.0.0.1:{port}/x")).await.unwrap();
    assert_eq!(got["status"], 200);
    let echoed: Value = serde_json::from_str(got["body"].as_str().unwrap()).unwrap();
    assert_eq!((echoed["authorization"].as_str(), echoed["body"].as_str()), (Some("Bearer ••••"), Some("t=••••")), "{got}");
    assert!(!got.to_string().contains("s3cret"));
    let refused = fetch(format!("http://localhost:{port}/x")).await.unwrap_err();
    assert!(format!("{refused:#}").contains("sent only to 127.0.0.1"), "{refused:#}");
    // The CLI's, through the local API.
    let api = serve_api(&a).await;
    let headers = r#"{"authorization":"Bearer {{token}}"}"#;
    let out = cli(&api, &temp("fetch-cli"), &["vault", "fetch", &format!("http://127.0.0.1:{port}/x"), "--headers", headers]).await.unwrap();
    assert!(out.contains("Bearer ••••") && !out.contains("s3cret"), "{out}");

    // A sidecar's text is filled for a call of its that waits, from the items of the app of its name, for their hosts.
    a.set_item(&person, &mine, json!({ "title": "Example push", "fields": { "key": { "value": "k3y", "secret": true }, "team": { "value": "T1" } }, "hosts": ["api.example.com"], "apps": ["keeper"] })).await.unwrap();
    a.register_sidecar("keeper", json!({ "title": "Keeper", "ops": [{ "name": "use" }] })).await.unwrap();
    let sidecar = a.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(c)) = sidecar.sidecars().next("keeper").await {
                let fill = |text: &str, host: &str| sidecar.sidecar_fill("keeper", c.id, text, host).map_err(|e| format!("{e:#}"));
                let answer = json!({ "here": fill("{{team}} {{key}}", "api.example.com"), "elsewhere": fill("{{key}}", "evil.example.com"), "theirs": fill("{{token}}", "127.0.0.1") });
                sidecar.sidecars().reply("keeper", c.id, json!({ "ok": answer })).unwrap();
            }
        }
    });
    let got = a.call(call("keeper", "use", json!({}))).await.unwrap();
    assert_eq!(got["here"]["Ok"], "T1 k3y", "{got}");
    assert!(got["elsewhere"]["Err"].as_str().unwrap().contains("sent only to api.example.com"), "{got}");
    assert_eq!(got["theirs"]["Ok"], "{{token}}", "not another app's item");
    assert!(a.sidecar_fill("keeper", 9999, "{{key}}", "api.example.com").is_err(), "only while a call waits");

    // An app's call to a sidecar has the fields of its items filled into its input, a secret one only for a sidecar
    // its item names among its hosts, and blotted out of the answer; a key it asks for is made for it, unseen.
    a.register_sidecar("holder", json!({ "title": "Holder", "ops": [{ "name": "open" }] })).await.unwrap();
    let sidecar = a.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(c)) = sidecar.sidecars().next("holder").await {
                let key = c.input["key"].as_str().unwrap_or_default().to_string();
                sidecar.sidecars().reply("holder", c.id, json!({ "ok": { "length": key.len(), "echo": key } })).unwrap();
            }
        }
    });
    deploy_module(&a, "opener", r#"export default { name: "opener", uses: { sidecars: ["holder"] }, ops: {
  make: { run: (ctx, { hosts }) => ctx.vault.create({ title: "Held", type: "Held", fields: { key: { generate: true } }, hosts }) },
  open: { run: (ctx, { id }) => ctx.sidecars.holder.open({ key: `{{${id}:key}}` }) },
  peek: { run: async () => (await fetch("http://holder/x")).status },
} };"#).await;
    a.install(&person, "opener", &mine, true).await.unwrap();
    let held = a.call(call("opener", "make", json!({ "hosts": ["holder"] }))).await.unwrap();
    let opened = a.call(call("opener", "open", json!({ "id": held }))).await.unwrap();
    assert_eq!((opened["length"].as_u64(), opened["echo"].as_str()), (Some(43), Some("••••")), "{opened}");
    let direct = CallRequest { sidecar: Some("holder".into()), ..call("", "open", json!({ "key": format!("{{{{{}:key}}}}", held.as_str().unwrap()) })) };
    assert_eq!(a.call(direct).await.unwrap()["length"], 24, "not filled for a call no app made");
    let elsewhere = a.call(call("opener", "make", json!({ "hosts": ["api.example.com"] }))).await.unwrap();
    assert!(a.call(call("opener", "open", json!({ "id": elsewhere }))).await.is_err(), "never to a sidecar its item doesn't name");
    assert_eq!(a.call(call("opener", "peek", json!({}))).await.unwrap(), 403, "an app's name is never a site's");
    assert!(!a.vault(&person, Some(&mine)).await.unwrap().iter().any(|i| i["fields"].to_string().contains("value") && i["title"] == "Held"), "nobody sees the key");

    // An app on a person's own page sees Everyone's items switched on for it too, as its placeholders are filled.
    deploy_module(&a, "reader", r#"export default { name: "reader", ops: { items: { run: (ctx) => ctx.vault.items() } } };"#).await;
    a.install(&person, "reader", &mine, true).await.unwrap();
    a.set_item(&person, &one::entry::Scope::Global, json!({ "title": "Everyone's sign-in", "fields": { "client_id": { "value": "c1" } }, "apps": ["reader"] })).await.unwrap();
    let seen = a.call(call("reader", "items", json!({}))).await.unwrap();
    assert_eq!((seen[0]["scope"].as_str(), seen[0]["fields"]["client_id"]["value"].as_str()), (Some("global"), Some("c1")), "{seen}");
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
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
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
async fn work_goes_on_after_a_call_returns_and_forgotten_blobs_go() {
    let (a, _ra) = start("later").await;
    a.create("Later", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    deploy_module(&a, "reader", r#"export default {
  name: "reader",
  schema: { user: "create table reads (text text); create table files (hash text)" },
  ops: {
    read: { run: (ctx) => {
      ctx.waitUntil((async () => { await new Promise((r) => setTimeout(r, 1500)); ctx.user.exec("insert into reads values (?)", ["done later"]); })());
      return "started";
    } },
    reads: { run: async (ctx) => (await ctx.user.query("select text from reads")).map((r) => r.text) },
    keep: { run: async (ctx) => { const hash = await ctx.blob.put(new TextEncoder().encode("a receipt")); ctx.user.exec("insert into files values (?)", [hash], [hash]); return hash; } },
    get: { run: async (ctx, { hash }) => new TextDecoder().decode(await ctx.blob.get(hash)) },
    forget: { run: (ctx, { hash }) => { ctx.user.forget(hash); return null; } },
  },
};"#)
    .await;
    a.install(&me, "reader", &one::entry::Scope::User(me.user.clone()), true).await.unwrap();
    eventually("reader to run", async || a.call(call("reader", "reads", json!({}))).await.is_ok()).await;
    assert_eq!(a.call(call("reader", "read", json!({}))).await.unwrap(), "started", "the call returns at once");
    assert_eq!(a.call(call("reader", "reads", json!({}))).await.unwrap(), json!([]), "the work isn't done yet");
    eventually("the work after the call to write", async || a.call(call("reader", "reads", json!({}))).await.is_ok_and(|r| r == json!(["done later"]))).await;

    let hash = a.call(call("reader", "keep", json!({}))).await.unwrap().as_str().unwrap().to_string();
    assert_eq!(a.call(call("reader", "get", json!({ "hash": hash }))).await.unwrap(), "a receipt");
    a.call(call("reader", "forget", json!({ "hash": hash }))).await.unwrap();
    assert!(a.call(call("reader", "get", json!({ "hash": hash }))).await.is_err(), "a forgotten blob is gone");
    assert!(a.blob(&hash).await.is_err(), "deleted from disk");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_apps_owners_see_who_used_it_its_errors_and_its_ai_cost() {
    let (a, _ra) = start("insights").await;
    a.create("Insights", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    deploy_module(&a, "brain", r#"export default { name: "brain", ops: { think: { run: (ctx) => { ctx.cost({ model: "gemini-flash", usd: 0.002, input: 1200, output: 80 }); return "thought"; } } } };"#).await;
    deploy_module(&a, "claimsy", r#"export default {
  name: "claimsy",
  uses: { apps: ["brain"] },
  ops: {
    ok: { run: () => "fine" },
    boom: { run: () => { throw new Error("the receipt was blank"); } },
    read: { run: (ctx) => ctx.apps.brain.think({}) },
  },
};"#)
    .await;
    a.install(&me, "claimsy", &one::entry::Scope::User(me.user.clone()), true).await.unwrap();
    eventually("claimsy to run", async || a.call(call("claimsy", "ok", json!({}))).await.is_ok()).await;
    a.call(CallRequest { client: Some("cli".into()), ..call("claimsy", "ok", json!({})) }).await.unwrap();
    for _ in 0..2 {
        assert!(a.call(call("claimsy", "boom", json!({}))).await.is_err());
    }
    a.call(call("claimsy", "read", json!({}))).await.unwrap();
    let insights = |n: &Node| {
        let (n, me) = (n.clone(), me.clone());
        async move { n.apps(&me).await.unwrap().into_iter().find(|x| x["name"] == "claimsy").unwrap()["insights"].clone() }
    };
    eventually("the insights to be written", async || {
        let i = insights(&a).await;
        i["errors"].as_array().is_some_and(|e| e.first().is_some_and(|e| e["count"] == 2)) && i["cost"].as_array().is_some_and(|c| !c.is_empty())
    })
    .await;
    let i = insights(&a).await;
    assert_eq!(i["errors"][0]["error"], "the receipt was blank", "{i}");
    assert_eq!(i["errors"][0]["op"], "boom");
    let clients: Vec<&str> = i["usage"].as_array().unwrap().iter().map(|u| u["client"].as_str().unwrap()).collect();
    assert!(clients.contains(&"cli") && clients.contains(&"api"), "who used it, from what: {i}");
    assert_eq!(i["cost"][0]["model"], "gemini-flash", "the AI cost counts against the app that asked: {i}");
    assert_eq!(i["cost"][0]["input"], 1200);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleted_apps_wait_in_recently_deleted_until_restored_or_erased() {
    let (a, _ra) = start("deleted").await;
    a.create("Deleted", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    deploy_module(&a, "scratch", r#"export default { name: "scratch", ops: { hi: { run: () => "hi" } } };"#).await;
    a.install(&me, "scratch", &one::entry::Scope::User(me.user.clone()), true).await.unwrap();
    eventually("scratch to run", async || a.call(call("scratch", "hi", json!({}))).await.is_ok()).await;
    let listed = |n: &Node| {
        let (n, me) = (n.clone(), me.clone());
        async move { n.apps(&me).await.unwrap().into_iter().find(|x| x["name"] == "scratch") }
    };

    a.retire(&me, "scratch", "deleted").await.unwrap();
    let err = a.call(call("scratch", "hi", json!({}))).await.unwrap_err();
    assert!(format!("{err:#}").contains("Recently Deleted"), "{err:#}");
    let shown = listed(&a).await.unwrap();
    assert_eq!(shown["installed"], json!([]), "it leaves every page");
    assert_eq!(shown["deleted"]["yours"], true);
    let blob = a.put_blob(&one::bundle::pack(&[("app.js".into(), b"export default { name: \"scratch\" };".to_vec())]).unwrap()).unwrap();
    assert!(a.deploy(&me, "scratch", &blob).await.is_err(), "a deleted app isn't deployed");

    a.retire(&me, "scratch", "restored").await.unwrap();
    assert_eq!(a.call(call("scratch", "hi", json!({}))).await.unwrap(), "hi", "restored, it runs on the pages it was on");
    let secret = async |scope: one::entry::Scope| a.set_item(&me, &scope, json!({ "title": "Key", "fields": { "key": { "value": "v", "secret": true } }, "hosts": ["example.com"], "apps": ["scratch"] })).await;
    secret(one::entry::Scope::User(me.user.clone())).await.unwrap();
    secret(one::entry::Scope::Global).await.unwrap();
    let kept = || a.secrets(&me, None).unwrap().into_iter().filter(|s| s["app"] == "scratch").count();

    assert!(a.retire(&me, "scratch", "erased").await.is_err(), "deleted before erased");
    a.retire(&me, "scratch", "deleted").await.unwrap();
    assert_eq!(kept(), 2, "deleted, it keeps its secrets, so restoring it works");
    a.retire(&me, "scratch", "erased").await.unwrap();
    assert!(listed(&a).await.is_none(), "erased, it's gone from every list");
    assert_eq!(kept(), 0, "erased, its secrets go from every scope");
    secret(one::entry::Scope::Global).await.unwrap();
    assert_eq!(kept(), 0, "and it's switched on for no item again");
    assert!(a.member("app", json!({ "name": "scratch", "owner": format!("user:{}", me.user) })).await.is_err(), "and its name isn't used again");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_apps_owners_link_where_its_code_is() {
    let (a, _ra) = start("source").await;
    a.create("Source", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
    deploy_module(&a, "scratch", r#"export default { name: "scratch", ops: { hi: { run: () => "hi" } } };"#).await;
    eventually("the App Store to run", async || a.call(call("apps", "list", json!({}))).await.is_ok()).await;
    let source = async |app: &str| a.apps(&me).await.unwrap().into_iter().find(|x| x["name"] == app).unwrap()["source"].clone();
    let set = async |link: &str| a.call(call("apps", "source", json!({ "app": "scratch", "link": link }))).await;

    assert_eq!(source("scratch").await, Value::Null, "an app made here has none until its owners say");
    let board = source("board").await;
    assert!(board.as_str().is_some_and(|l| l.starts_with(&format!("https://github.com/{}/tree/", one::config::REPO)) && l.ends_with("/apps/board")), "a built-in app links to its folder: {board}");
    set("https://example.com/scratch").await.unwrap();
    assert_eq!(source("scratch").await, "https://example.com/scratch");
    set("http://localhost:4747/repos/scratch.git").await.unwrap();
    assert_eq!(source("scratch").await, "http://localhost:4747/repos/scratch.git", "or a repo in Repos");
    assert!(set(&format!("{}://scratch", one::config::CLI)).await.is_err(), "only web addresses");
    assert!(set("javascript:alert(1)").await.is_err(), "only web addresses");
    set("").await.unwrap();
    assert_eq!(source("scratch").await, Value::Null, "an empty link clears it");
    a.call(call("apps", "source", json!({ "app": "board", "link": "" }))).await.unwrap();
    assert_eq!(source("board").await, Value::Null, "a built-in app's owners clear its link too");
}

#[tokio::test(flavor = "multi_thread")]
async fn each_person_arranges_their_pages_and_their_widgets_options() {
    let (andre, _r1) = start("arrange-andre").await;
    let (laptop, _r2) = start("arrange-laptop").await;
    andre.create("Home", "Andre").await.unwrap();
    deploy_module(&andre, "notes", r#"export default {
        name: "notes", schema: { user: "create table notes (topic text, text text)" },
        ops: { add: { run: (ctx, i) => ctx.user.exec("insert into notes values (?, ?)", [i.topic, i.text]) } },
        widgets: { latest: {
            title: "Notes", description: "The newest note", sizes: ["small", "medium"],
            options: {
                topic: { title: "Topic", default: "", choices: async (ctx) => [{ value: "", title: "All" }, ...(await ctx.user.query("select distinct topic from notes")).map((r) => r.topic)] },
                loud: { title: "Loud", default: false },
            },
            async render(ctx) {
                const rows = await ctx.user.query("select text from notes where ? = '' or topic = ?", [ctx.options.topic, ctx.options.topic]);
                return { t: "text", text: `${ctx.size} ${rows.map((r) => r.text).join(",")}${ctx.options.loud ? "!" : ""}` };
            },
        } },
    };"#).await;
    let me = one::node::Caller { key: andre.id(), user: andre.user().unwrap(), agent: None };
    andre.install(&me, "notes", &one::entry::Scope::User(me.user.clone()), true).await.unwrap();
    andre.call(call("notes", "add", json!({ "topic": "work", "text": "ship" }))).await.unwrap();
    andre.call(call("notes", "add", json!({ "topic": "home", "text": "cook" }))).await.unwrap();

    // A page shows every widget its apps draw, with their options' choices worked out from the data.
    let widgets = andre.widgets(me.clone(), Some("user")).await.unwrap();
    let w = widgets.iter().find(|w| w["app"] == "notes").unwrap();
    assert_eq!(w["description"], "The newest note");
    assert_eq!(w["options"][0]["choices"].as_array().unwrap().len(), 3, "{w}");
    assert_eq!(w["options"][1]["default"], false, "{w}");
    assert!(w.get("placed").is_none(), "nothing's placed until someone chooses to: {w}");

    // Arranging a page is the person's own entry: the widget they placed renders with the options they chose.
    let items = json!([{ "app": "notes", "widget": "latest", "id": "w1", "size": "small", "options": { "topic": "work", "loud": true } }, { "app": "notes" }]);
    andre.arrange(&me, "user", items.clone(), json!([])).await.unwrap();
    assert_eq!(andre.state()["arrangements"]["user"]["items"], items);
    let widgets = andre.widgets(me.clone(), Some("user")).await.unwrap();
    let w = widgets.iter().find(|w| w["app"] == "notes").unwrap();
    assert_eq!(w["placed"]["w1"]["sizes"]["small"]["text"], "small ship!", "{w}");
    assert_eq!(w["sizes"]["medium"]["text"], "medium ship,cook", "the gallery's copy keeps its defaults: {w}");
    assert!(andre.arrange(&me, "elsewhere", json!([]), json!([])).await.is_err(), "a page is a scope's");

    // It follows the person to their other devices, and the latest arrangement counts.
    laptop.join(&andre.pair(None).unwrap()).await.unwrap();
    eventually("the laptop to get the arrangement", async || laptop.state()["arrangements"]["user"]["items"] == items).await;
    let theirs = one::node::Caller { key: laptop.id(), user: laptop.user().unwrap(), agent: None };
    laptop.arrange(&theirs, "global", json!([{ "app": "notes" }]), json!(["board"])).await.unwrap();
    eventually("andre to get the laptop's arrangement", async || andre.state()["arrangements"]["global"]["hidden"] == json!(["board"])).await;
}

/// `try` makes an install as `create` makes every new one, with a node of its own beside this machine's, runs commands
/// in it, and `try stop` deletes it.
#[test]
fn try_makes_a_new_install_beside_this_ones() {
    let home = temp("try");
    let var = |name: &str| format!("{}_{name}", one::config::CLI.to_uppercase());
    // This machine's own node, as far as the command knows, isn't running.
    let cli = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_one")).args(args).env(var("HOME"), &home).env(var("ADDR"), "127.0.0.1:9").output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let tried = std::path::PathBuf::from(format!("{}-try", home.display()));
    assert!(cli(&["try"]).contains("Try"));
    let status = cli(&["try", "status"]);
    assert!(status.lines().any(|l| l.starts_with("Try ")), "{status}");
    let apps = cli(&["try", "apps"]);
    for app in one::config::STARTERS {
        assert!(apps.lines().any(|l| l.starts_with(&format!("{app} (")) && l.contains("installed in global")), "{app} isn't installed: {apps}");
    }
    cli(&["try", "board", "post", "hello"]);
    assert!(cli(&["try", "board", "read"]).contains("hello"));
    let pid: i32 = std::fs::read_to_string(tried.join("pid")).unwrap().trim().parse().unwrap();
    cli(&["try", "stop"]);
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "its node still runs");
    assert!(!tried.join("install").exists());
    let _ = std::fs::remove_dir_all(&tried);
}

#[tokio::test(flavor = "multi_thread")]
async fn backups_go_to_a_folder_encrypted_and_restore_with_the_recovery_key_alone() {
    let (a, _ra) = start("backup-a").await;
    let (b, _rb) = start("backup-b").await;
    let recovery = a.create("Home", "Andre").await.unwrap();
    b.join(&a.pair(None).unwrap()).await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap(), agent: None };
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
    eventually("the second backup", async || backed_up_posts(&dir, &install, &keys["global"]).await.len() == 2).await;
    let again = one::backups::restore(&place, &install, "global", &keys["global"]).unwrap();
    assert!(again.snapshot.databases["board/main"].0 > steps_then);
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
    for text in ["broken once", "broken twice"] {
        a.call(board("post", json!({ "text": text }))).await.unwrap();
        a.back_up_now();
        eventually("a failed run", async || a.backups().unwrap()["places"][0]["problem"]["text"].as_str().is_some_and(|t| t.contains("couldn't be used") || t.contains("won't let"))).await;
    }
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
