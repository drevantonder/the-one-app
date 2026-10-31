//! Two real nodes and a phone client, talking over iroh (needs the internet for discovery and relays).

use std::time::{Duration, Instant};

use one::node::Node;
use serde_json::{Value, json};

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("one-e2e-{name}-{}", rand::random::<u32>()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

async fn texts(node: &Node, space: [u8; 32]) -> Vec<String> {
    let posts = node.call(space, "board", "news", json!({ "since": 0 })).await.unwrap();
    posts.as_array().unwrap().iter().map(|p| p["text"].as_str().unwrap().to_string()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_and_a_phone_share_a_board() {
    let (a, _ra) = Node::start(&temp("a"), one::builtin::all()).await.unwrap();
    let (b, _rb) = Node::start(&temp("b"), one::builtin::all()).await.unwrap();

    let space = a.create_space("Test").await.unwrap();
    a.call(space, "board", "post", json!({ "text": "from a" })).await.unwrap();

    b.join(&a.invite(space, true).unwrap()).await.unwrap();
    eventually("b to receive a's post", async || b.call(space, "board", "news", json!({ "since": 0 })).await.is_ok_and(|p| p.as_array().is_some_and(|p| !p.is_empty()))).await;
    b.call(space, "board", "post", json!({ "text": "from b" })).await.unwrap();
    eventually("a to receive b's post", async || texts(&a, space).await.contains(&"from b".to_string())).await;

    let phone = one_client::Client::start(temp("phone"), "Phone").await.unwrap();
    phone.join(&a.invite(space, true).unwrap()).await.unwrap();
    let state = phone.request("GET", "/v1/state", Value::Null).await.unwrap();
    assert_eq!(state["device"]["name"], "Phone");
    let page = phone.request("GET", "/v1/screen", json!({ "app": "board", "screen": "main" })).await.unwrap();
    let html = page["html"].as_str().unwrap();
    assert!(html.contains("window.one") && html.contains("one.call(\"topics\")"), "a phone gets the board's screen with the bridge");
    let reply = phone.request("POST", "/v1/call", json!({ "app": "board", "op": "post", "input": { "text": "from the phone" } })).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    eventually("b to receive the phone's post", async || texts(&b, space).await.contains(&"from the phone".to_string())).await;
    let news = b.call(space, "board", "news", json!({ "since": 0 })).await.unwrap();
    let by_phone = news.as_array().unwrap().iter().find(|p| p["text"] == "from the phone").unwrap();
    assert_eq!(by_phone["name"], "Phone", "posts are credited to the device that asked");
    phone.request("POST", "/v1/device", json!({ "name": "Andre's iPhone" })).await.unwrap();
    let state = phone.request("GET", "/v1/state", Value::Null).await.unwrap();
    assert_eq!(state["device"]["name"], "Andre's iPhone", "an admin renames a phone that asks");

    a.remove_device(space, one::entry::unhex(&phone.id()).unwrap()).await.unwrap();
    let devices = a.state()["spaces"][0]["devices"].as_array().unwrap().len();
    assert_eq!(devices, 2);
    assert!(phone.request("GET", "/v1/state", Value::Null).await.is_err(), "a removed device is refused");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_answers_calls_from_another_device() {
    let (a, _ra) = Node::start(&temp("pa"), one::builtin::all()).await.unwrap();
    let (b, _rb) = Node::start(&temp("pb"), one::builtin::all()).await.unwrap();
    let space = a.create_space("Test").await.unwrap();
    b.join(&a.invite(space, true).unwrap()).await.unwrap();
    eventually("b to become a member", async || b.state()["spaces"][0]["member"] == true).await;

    // b runs a provider for "echo", as a Swift or Go program would over the local API.
    let echo = json!({ "title": "Echo", "ops": [{ "name": "say", "description": "", "input": {} }], "widgets": [] });
    b.provide("echo", echo.clone()).await.unwrap();
    let entries = b.state()["spaces"][0]["entries"].clone();
    b.provide("echo", echo).await.unwrap();
    assert_eq!(b.state()["spaces"][0]["entries"], entries, "registering again announces nothing new");
    eventually("a to list b's provider", async || a.apps().await.unwrap().iter().any(|app| app["name"] == "echo" && app["runs"] == "here")).await;
    let serving = b.clone();
    tokio::spawn(async move {
        while let Ok(Some(call)) = serving.providers().next("echo").await {
            let reply = json!({ "ok": { "heard": call.input["text"], "from": call.caller.name }, "entries": [{ "kind": "said", "data": call.input }] });
            serving.providers().reply("echo", call.id, reply).unwrap();
        }
    });

    let reply = a.call_on(b.id(), space, "echo", "say", json!({ "text": "hi" })).await.unwrap();
    assert_eq!(reply["heard"], "hi");
    assert_eq!(reply["from"], a.state()["device"]["name"]);
    assert!(a.call_on(b.id(), space, "nothing", "here", json!({})).await.is_err());
}
