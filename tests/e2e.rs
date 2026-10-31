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
