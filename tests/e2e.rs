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
    Node::start(&dir, one::builtin::all()).await.unwrap()
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
    match node.call(board("news", json!({ "since": 0, "scope": scope }))).await {
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
    assert!(a.create("Another", "Andre").await.is_err(), "a brand holds one install");
    let apps = a.apps(&one::node::Caller { key: a.id(), user: a.user().unwrap() }).await.unwrap();
    let listed = apps.iter().find(|app| app["name"] == "board").expect("the brand's board is deployed");
    assert_eq!(listed["owner"], "admins");
    assert!(listed["ops"].as_array().is_some_and(|ops| ops.iter().any(|o| o["name"] == "post")), "{listed}");
    a.call(board("post", json!({ "text": "from a" }))).await.unwrap();

    b.join(&a.pair(None).unwrap()).await.unwrap();
    assert_eq!(b.user().unwrap(), a.user().unwrap(), "a pairing code from a's user makes another of their servers");
    eventually("b to run the board and see a's post", async || sees(&b, "global", "from a").await).await;
    b.call(board("post", json!({ "text": "from b" }))).await.unwrap();
    assert!(b.join(&a.pair(None).unwrap()).await.is_err(), "a server in an install joins no other");
    eventually("a to see b's post", async || sees(&a, "global", "from b").await).await;

    // A new deploy runs everywhere.
    let source = include_str!("../apps/board/app.js").replace("  ops: {", "  ops: {\n    ping: { description: \"\", input: {}, run: () => \"pong\" },");
    let bundle = one::bundle::pack(&[("app.js".into(), source.clone().into_bytes()), ("src/app.js".into(), source.into_bytes())]).unwrap();
    let blob = a.put_blob(&bundle).unwrap();
    a.append(&one::entry::Scope::Global, one::node::deploy("board", &blob)).await.unwrap();
    eventually("b to run the new deploy", async || b.call(board("ping", json!({}))).await.is_ok_and(|r| r == "pong")).await;
    assert!(sees(&b, "global", "from b").await, "a new deploy replays the stores");

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
    let news = b.call(board("news", json!({ "since": 0 }))).await.unwrap();
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
    eventually("bob to run the board", async || bob.call(board("news", json!({ "since": 0 }))).await.is_ok()).await;
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
    let news = andre.call(board("news", json!({ "since": 0, "scope": "group:f000000000000001" }))).await.unwrap();
    assert_eq!(news[0]["user"], bob.user().unwrap().as_str());

    // A page is a scope's installs: the board is on Everyone's, and on the group's once installed there too.
    let me = one::node::Caller { key: andre.id(), user: andre.user().unwrap() };
    assert_eq!(andre.widgets(me.clone(), Some("global")).await.unwrap().len(), 1);
    assert!(andre.widgets(me.clone(), Some("group:f000000000000001")).await.unwrap().is_empty());
    andre.append(&one::entry::Scope::Group("f000000000000001".into()), one::members::node("install", json!({ "app": "board" }))).await.unwrap();
    let widgets = andre.widgets(me, Some("group:f000000000000001")).await.unwrap();
    assert_eq!(widgets[0]["root"]["c"][1]["text"], "hi family", "a group's page shows its board: {widgets:?}");

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
    let carol_code = andre.pair(Some(("Carol".into(), one::members::Role::Member))).unwrap();
    carol.join(&carol_code).await.unwrap();
    let reply = carol.request("POST", "/v1/call", json!({ "app": "board", "op": "post", "input": { "text": "carol here", "scope": "user" } })).await.unwrap();
    assert!(reply.get("ok").is_some(), "the privileged server serves a phone-only user: {reply}");

    // Bob's call goes elsewhere from a server that doesn't hold what it touches, and runs where it does.
    let bobs_user = format!("user:{}", bob.user().unwrap());
    let req = CallRequest { server: Some(key(&andre)), scopes: Some(vec![bobs_user.clone()]), ..board("post", json!({ "text": "x", "scope": "user" })) };
    let err = bob.call(req).await.unwrap_err();
    assert!(format!("{err:#}").contains("doesn't serve you"), "{err:#}");
    let req = CallRequest { server: Some(key(&cloud)), ..board("post", json!({ "text": "bob via cloud", "scope": "user" })) };
    bob.call(req).await.unwrap();
    eventually("bob's own server to get what cloud wrote for him", async || sees(&bob, "user", "bob via cloud").await).await;

    // An admin opens bob's data on the privileged server: read only, and bob sees it.
    andre.open(&bob.user().unwrap(), &key(&cloud)).await.unwrap();
    let read = CallRequest { server: Some(key(&cloud)), as_user: Some(bob.user().unwrap()), ..board("news", json!({ "since": 0, "scope": "user" })) };
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
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_one")).args(&args).current_dir(&dir).env(format!("{}_ADDR", one::brand::CLI.to_uppercase()), &addr).output().unwrap();
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
    cli(&api, &dir, &["deploy", "hello", "--bundle", &first[..12]]).await.unwrap();
    eventually("the rollback to run", async || a.call(call("hello", "hi", json!({}))).await.is_ok_and(|r| r == "hello &lt;one&gt;")).await;
    let deploys = cli(&api, &dir, &["deploys", "hello"]).await.unwrap();
    assert_eq!(deploys.lines().count(), 3, "{deploys}");
    assert!(deploys.lines().last().unwrap().starts_with(&first[..12]) && deploys.contains("(running)"), "{deploys}");

    // Only someone acting for its owner deploys it.
    bob.join(&a.pair(Some(("Bob".into(), one::members::Role::Member))).unwrap()).await.unwrap();
    eventually("bob to know the app", async || bob.source("hello", None, None).await.is_ok()).await;
    let bob_me = one::node::Caller { key: bob.id(), user: bob.user().unwrap() };
    let refused = bob.deploy(&bob_me, "hello", &first).await.unwrap_err();
    assert!(format!("{refused:#}").contains("only someone acting for hello's owner"), "{refused:#}");
}

/// Deploys a plain module as `app`, made by this server's user if it's new, and installs it in their scope.
async fn deploy_module(node: &Node, app: &str, source: &str) {
    let bundle = one::bundle::pack(&[("app.js".into(), source.as_bytes().to_vec()), ("src/app.js".into(), source.as_bytes().to_vec())]).unwrap();
    let blob = node.put_blob(&bundle).unwrap();
    let me = one::node::Caller { key: node.id(), user: node.user().unwrap() };
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
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap() };
    a.set_secret(&me, &one::entry::Scope::User(me.user.clone()), "keeper", "token", vec!["127.0.0.1".into()], Some("s3cret".into())).await.unwrap();
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

    // Deleted, it's no longer filled in; and only someone who manages a scope sets secrets there.
    a.set_secret(&me, &one::entry::Scope::User(me.user.clone()), "keeper", "token", vec![], None).await.unwrap();
    let got = a.call(send(format!("http://127.0.0.1:{port}/"), "Bearer {{token}}", "")).await.unwrap();
    assert_eq!(got["authorization"], "Bearer {{token}}");
    let other = one::node::Caller { key: a.id(), user: "0123456789abcdef".into() };
    assert!(a.set_secret(&other, &one::entry::Scope::Global, "keeper", "x", vec!["a.com".into()], Some("v".into())).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn apps_install_in_several_scopes_use_each_other_and_fork() {
    let (a, _ra) = start("apps").await;
    a.create("Apps", "Andre").await.unwrap();
    let me = one::node::Caller { key: a.id(), user: a.user().unwrap() };
    let mine = one::entry::Scope::User(me.user.clone());
    a.member("group", json!({ "group": "a000000000000001", "name": "Team" })).await.unwrap();
    let team = one::entry::Scope::Group("a000000000000001".into());
    let notes = r#"export default {
  name: "notes",
  title: "Notes",
  schema: Object.fromEntries(["user", "group", "global"].map((k) => [k, "create table notes (text text)"])),
  ops: {
    add: { run: (ctx, { text, scope }) => { (scope ? ctx.group(scope) : ctx.user).append("note", { text }); return null; } },
    list: { run: async (ctx, { scope }) => (await (scope ? ctx.group(scope) : ctx.user).query("select text from notes")).map((n) => n.text) },
  },
  on: { note: (db, e) => db.exec("insert into notes values (?)", [e.data.text]) },
};"#;
    deploy_module(&a, "notes", notes).await;
    deploy_module(&a, "journal", r#"export default {
  name: "journal",
  uses: { apps: ["notes"], drivers: ["shell"] },
  ops: {
    jot: { run: async (ctx, { text }) => { await ctx.apps.notes.add({ text }); return ctx.apps.notes.list({}); } },
    sneak: { run: (ctx) => ctx.apps.board.read({}) },
    echo: { run: (ctx) => ctx.drivers.shell.run({ command: "echo from a driver" }) },
    keep: { run: async (ctx, { text }) => { const hash = await ctx.blob.put(new TextEncoder().encode(text)); return new TextDecoder().decode(await ctx.blob.get(hash)); } },
    catalog: { run: async (ctx) => (await ctx.catalog()).map((a) => a.name) },
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
    assert!(echoed.to_string().contains("from a driver"), "{echoed}");
    assert_eq!(a.call(call("journal", "keep", json!({ "text": "in a blob" }))).await.unwrap(), "in a blob");
    let names = a.call(call("journal", "catalog", json!({}))).await.unwrap();
    assert!(names.as_array().unwrap().contains(&json!("notes")), "{names}");

    // The same app in another scope: one version, data per scope.
    a.install(&me, "notes", &team, true).await.unwrap();
    a.call(call("notes", "add", json!({ "text": "for the team", "scope": "a000000000000001" }))).await.unwrap();
    assert_eq!(a.call(call("notes", "list", json!({ "scope": "a000000000000001" }))).await.unwrap(), json!(["for the team"]));
    assert_eq!(a.call(call("notes", "list", json!({}))).await.unwrap(), json!(["first"]));
    let mut where_ = installed("notes").await.as_array().unwrap().clone();
    where_.sort_by_key(|v| v.to_string());
    assert_eq!(json!(where_), json!([team.to_string(), mine.to_string()]));

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
