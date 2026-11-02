//! Where code in two languages must agree (the node sends, the Mac and iPhone apps, the Go sidecars, the screens'
//! bridge and the chat view read), a test reads both and checks they do, so neither can drift from the other.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

fn read(path: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The quoted word after each `before` in `text`, and any more quoted words listed after it with commas
/// (`case "save", "preview":`). `before` ends where its quote would start; spaces may come between.
fn quoted(text: &str, before: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (i, _) in text.match_indices(before) {
        if text[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let mut rest = text[i + before.len()..].trim_start();
        while let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let Some(end) = rest[1..].find(q) else { break };
            let word = &rest[1..1 + end];
            if word.is_empty() || !word.chars().all(|c| c.is_alphanumeric() || c == '_') {
                break;
            }
            out.insert(word.to_string());
            rest = rest[2 + end..].trim_start();
            match rest.strip_prefix(',') {
                Some(more) => rest = more.trim_start(),
                None => break,
            }
        }
    }
    out
}

/// A word made of letters, digits and `_`, at the start of `text`.
fn word(text: &str) -> &str {
    &text[..text.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(text.len())]
}

/// A field a Swift `Decodable` struct reads: its JSON key, its Swift type, and whether decoding fails without it.
#[derive(Debug)]
struct Field {
    key: String,
    ty: String,
    required: bool,
}

/// `var name: Type` (or `let`), with no body or default: a stored property, its name and type.
fn property(line: &str) -> Option<(String, String)> {
    let line = line.split("//").next().unwrap().trim();
    let rest = line.strip_prefix("var ").or_else(|| line.strip_prefix("let "))?;
    let (name, ty) = rest.split_once(':')?;
    (!ty.contains(['{', '=']) && word(name.trim()) == name.trim()).then(|| (name.trim().to_string(), ty.trim().to_string()))
}

/// The `Decodable` structs in Swift source, by name (nested ones as `Outer.Inner`), each with the fields it reads.
fn swift_structs(src: &str) -> BTreeMap<String, Vec<Field>> {
    // Each struct being read: its name, the depth of its body, its properties, its CodingKeys, and the keys a custom
    // init(from:) decodes with a plain `try` (so must find).
    struct Open {
        name: String,
        depth: i32,
        props: Vec<(String, String)>,
        keys: Option<Vec<(String, String)>>,
        init: Option<BTreeSet<String>>,
    }
    let (mut stack, mut out, mut depth) = (Vec::<Open>::new(), BTreeMap::new(), 0);
    let (mut in_keys, mut keys_text, mut in_init) = (false, String::new(), false);
    for line in src.lines() {
        let at = depth;
        let t = line.trim_start().trim_start_matches("private ").trim_start_matches("fileprivate ");
        let opens = t.strip_prefix("struct ").filter(|r| r.split('{').next().unwrap().contains("Codable") || r.split('{').next().unwrap().contains("Decodable"));
        if let Some(rest) = opens {
            let outer = stack.last().map(|o| format!("{}.", o.name)).unwrap_or_default();
            // A struct on one line keeps its properties between its braces, separated by `;`.
            let props = match (line.find('{'), line.rfind('}')) {
                (Some(a), Some(b)) if a < b => line[a + 1..b].split(';').filter_map(property).collect(),
                _ => vec![],
            };
            stack.push(Open { name: format!("{outer}{}", word(rest)), depth: at + 1, props, keys: None, init: None });
        } else if let Some(top) = stack.last_mut().filter(|o| o.depth == at) {
            if t.starts_with("enum CodingKeys") || t.starts_with("enum Key:") {
                in_keys = true;
                keys_text.clear();
            } else if t.contains("init(from decoder") {
                in_init = true;
                top.init = Some(BTreeSet::new());
            } else if let Some(p) = property(line) {
                top.props.push(p);
            }
        }
        if in_keys {
            keys_text.push_str(line.split_once("CodingKey").map(|(_, rest)| rest).unwrap_or(line));
            keys_text.push(' ');
        }
        if in_init && let Some(plain) = stack.last_mut().and_then(|o| o.init.as_mut()) {
            for piece in line.split("try").skip(1).filter(|p| p.starts_with(' ') && p.contains(".decode(")) {
                if let Some((_, key)) = piece.split_once("forKey: .") {
                    plain.insert(word(key).to_string());
                }
            }
        }
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if in_keys && depth <= stack.last().map(|o| o.depth).unwrap_or(0) {
            in_keys = false;
            let pairs = keys_text.split(['{', '}', ',']).map(|w| w.trim().trim_start_matches("case ").trim()).filter(|w| !w.is_empty());
            let pairs = pairs.map(|w| match w.split_once('=') {
                Some((name, key)) => (name.trim().to_string(), key.trim().trim_matches('"').to_string()),
                None => (w.to_string(), w.to_string()),
            });
            stack.last_mut().unwrap().keys = Some(pairs.collect());
        }
        if in_init && depth <= stack.last().map(|o| o.depth).unwrap_or(0) {
            in_init = false;
        }
        while stack.last().is_some_and(|o| depth < o.depth) {
            let o = stack.pop().unwrap();
            let fields = o
                .props
                .iter()
                .filter_map(|(name, ty)| {
                    let key = match &o.keys {
                        Some(keys) => keys.iter().find(|(n, _)| n == name)?.1.clone(),
                        None => name.clone(),
                    };
                    let required = match &o.init {
                        Some(plain) => plain.contains(name),
                        None => !ty.ends_with('?'),
                    };
                    Some(Field { key, ty: ty.trim_end_matches('?').to_string(), required })
                })
                .collect();
            out.insert(o.name, fields);
        }
    }
    out
}

/// What's wrong with `value` as Swift's `ty` would decode it, from inside struct `within`: each problem by its path.
fn check(structs: &BTreeMap<String, Vec<Field>>, within: &str, ty: &str, value: &Value, path: &str, problems: &mut Vec<String>) {
    let ty = ty.trim().trim_end_matches('?');
    let named = |t: &str| {
        let mut outer = within.to_string();
        loop {
            let name = if outer.is_empty() { t.to_string() } else { format!("{outer}.{t}") };
            if structs.contains_key(&name) {
                return Some(name);
            }
            if outer.is_empty() {
                return None;
            }
            outer = outer.rsplit_once('.').map(|(o, _)| o.to_string()).unwrap_or_default();
        }
    };
    let wrong = |what: &str| format!("{path}: Swift reads {ty}, the node sent {what}");
    if let Some(inner) = ty.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        match inner.split_once(':') {
            Some((_, of)) => match value.as_object() {
                Some(map) => map.iter().for_each(|(k, v)| check(structs, within, of, v, &format!("{path}.{k}"), problems)),
                None => problems.push(wrong(&value.to_string())),
            },
            None => match value.as_array() {
                Some(list) => list.iter().enumerate().for_each(|(i, v)| check(structs, within, inner, v, &format!("{path}[{i}]"), problems)),
                None => problems.push(wrong(&value.to_string())),
            },
        }
        return;
    }
    let ok = match ty {
        "String" => value.is_string(),
        "Int" => value.is_i64() || value.is_u64(),
        "Double" => value.is_number(),
        "Bool" => value.is_boolean(),
        _ => match named(ty) {
            Some(name) => {
                for f in &structs[&name] {
                    match value.get(&f.key).filter(|v| !v.is_null()) {
                        Some(v) => check(structs, &name, &f.ty, v, &format!("{path}.{}", f.key), problems),
                        None if f.required => problems.push(format!("{path}.{}: Swift's {name} needs it, the node left it out", f.key)),
                        None => {}
                    }
                }
                true
            }
            // A type of its own (JSONValue, an enum) decodes itself.
            None => true,
        },
    };
    if !ok {
        let sent = value.to_string();
        problems.push(wrong(&sent[..sent.len().min(80)]));
    }
}

async fn start(name: &str) -> (one::node::Node, iroh::protocol::Router) {
    let dir = std::env::temp_dir().join(format!("one-contracts-{name}-{}", rand::random::<u32>()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("server.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": name }).to_string()).unwrap();
    one::node::Node::start(&dir, one::sidecars::built_in()).await.unwrap()
}

/// GETs or POSTs the node's local API, as the Mac app does.
async fn api(addr: &str, method: &str, path: &str, body: Value) -> Value {
    let url = format!("http://{addr}{path}");
    let method = method.to_string();
    tokio::task::spawn_blocking(move || {
        let mut reply = match method.as_str() {
            "GET" => ureq::get(&url).call(),
            _ => ureq::post(&url).send_json(&body),
        }
        .unwrap_or_else(|e| panic!("{url}: {e}"));
        reply.body_mut().read_json::<Value>().unwrap()
    })
    .await
    .unwrap()
}

/// Every payload the Mac and iPhone apps decode from the local API decodes with the structs they decode it into: each
/// field they need is there, and each field they read has the type they read it as. A shell built before still
/// decodes what a newer server sends, so a field leaves the payload only once no shell needs it.
#[tokio::test(flavor = "multi_thread")]
async fn the_apple_apps_decode_what_the_node_sends() {
    let swift = read("apple/Shared/Node.swift") + &read("apple/Shared/Views/WidgetTree.swift");
    let structs = swift_structs(&swift);
    for name in ["NodeState", "Install", "Server", "AppInfo", "WidgetInfo", "WidgetNode", "VaultItem", "VaultRequest", "Arrangement.Item", "Backups", "Backups.Place", "Backups.Snapshot"] {
        assert!(structs.get(name).is_some_and(|f| !f.is_empty()), "the test reads Swift's {name}: {:?}", structs.keys());
    }
    assert!(structs["VaultItem"].iter().any(|f| f.key == "scope_name" && !f.required) && structs["VaultItem"].iter().any(|f| f.key == "title" && f.required));
    assert!(structs["NodeState"].iter().any(|f| f.key == "me" && f.required), "CodingKeys rename fields");

    let (node, _router) = start("contracts").await;
    node.create("Contracts", "Andre").await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let router = one::api::router(node.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    // An app with everything a shell shows: operations, a screen, a widget, a Vault need; on a page, arranged.
    let source = r##"export default {
        name: "lamps", title: "Lamps", icon: "lightbulb", color: "#ffaa00", description: "The lights at home",
        ops: { on: { description: "Turns one on", run: () => true } },
        screens: { main: { title: "Lamps", html: "<p>Lamps</p>" } },
        vault: { hub: { title: "Hub", fields: { token: { secret: true } }, hosts: ["hub.example.com"] } },
        widgets: { glance: { title: "Lamps", description: "Which are on", sizes: ["small", "medium"],
            options: { room: { title: "Room", default: "all", choices: ["all", "kitchen"] } },
            render: (ctx) => ({ t: "col", c: [{ t: "stat", value: 2, label: "on" }, { t: "list", c: [{ t: "item", title: "Kitchen", link: { screen: "main" } }] }] }) } },
    };"##;
    let bundle = one::bundle::pack(&[("app.js".into(), source.as_bytes().to_vec()), ("src/app.js".into(), source.as_bytes().to_vec())]).unwrap();
    let blob = node.put_blob(&bundle).unwrap();
    let me = one::node::Caller { key: node.id(), user: node.user().unwrap(), agent: None };
    node.member("app", json!({ "name": "lamps", "owner": format!("user:{}", me.user) })).await.unwrap();
    node.deploy(&me, "lamps", &blob).await.unwrap();
    node.member("app", json!({ "name": "lamps", "owner": format!("user:{}", me.user), "source": "https://example.com/lamps" })).await.unwrap();
    api(&addr, "POST", "/v1/install", json!({ "app": "lamps", "scope": "user", "installed": true })).await;
    api(&addr, "POST", "/v1/arrange", json!({ "page": "user", "items": [{ "app": "lamps", "widget": "glance", "id": "w1", "size": "small", "options": { "room": "kitchen" } }, { "app": "lamps" }], "hidden": [] })).await;
    api(&addr, "POST", "/v1/group", json!({ "name": "Home" })).await;
    api(&addr, "POST", "/v1/vault", json!({ "scope": "user", "title": "Hub", "fields": { "user": { "value": "andre" }, "token": { "value": "s3cret", "secret": true } }, "hosts": ["hub.example.com"], "apps": ["lamps"] })).await;
    api(&addr, "POST", "/v1/vault/request", json!({ "scope": "user", "app": "lamps", "need": "hub", "why": "To reach the hub" })).await;
    // Backups to a disk, once they've run.
    let disk = std::env::temp_dir().join(format!("contracts-backups-{}", std::process::id()));
    api(&addr, "POST", "/v1/backups/place", json!({ "path": disk.join("One Backups"), "title": "Disk" })).await;
    for _ in 0..100 {
        if !api(&addr, "GET", "/v1/backups", Value::Null).await["snapshots"].as_array().is_none_or(|s| s.is_empty()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let mut problems = vec![];
    for (path, ty) in [("/v1/state", "NodeState"), ("/v1/apps", "[AppInfo]"), ("/v1/widgets?scope=user", "[WidgetInfo]"), ("/v1/vault", "[VaultItem]"), ("/v1/vault/requests", "[VaultRequest]"), ("/v1/backups", "Backups")] {
        let sent = api(&addr, "GET", path, Value::Null).await;
        assert!(sent.as_array().is_none_or(|l| !l.is_empty()), "{path} sent nothing to check");
        check(&structs, "", ty, &sent, path, &mut problems);
    }
    let state = api(&addr, "GET", "/v1/state", Value::Null).await;
    assert!(!state["install"]["groups"].as_array().unwrap().is_empty() && state["arrangements"]["user"].is_object(), "{state}");
    assert!(problems.is_empty(), "the Apple apps can't decode what the node sends:\n{}", problems.join("\n"));
    let _ = std::fs::remove_dir_all(disk);
}

/// Every widget node an app draws, both shells draw: the Apple apps natively, and chats as HTML.
#[test]
fn every_widget_node_is_drawn_by_both_shells() {
    let swift = read("apple/Shared/Views/WidgetTree.swift");
    let html = read("src/mcp_view.html");
    let body = swift.split("switch node.t").nth(1).expect("WidgetTree.swift switches on node.t");
    let apple = quoted(body.split("\n    }\n").next().unwrap(), "case");
    let chats = quoted(&html, "case");
    assert!(apple.len() > 10, "{apple:?}");
    assert_eq!(apple, chats.intersection(&apple).cloned().collect(), "chats draw every node the Apple apps do");
    let mut drawn: BTreeSet<String> = BTreeSet::new();
    for dir in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("apps")).unwrap() {
        let app = dir.unwrap().path().join("app.js");
        if let Ok(js) = std::fs::read_to_string(&app) {
            drawn.extend(quoted(&js, "t:"));
        }
    }
    assert!(drawn.contains("col") && drawn.contains("text"), "{drawn:?}");
    let missing: Vec<_> = drawn.difference(&apple).collect();
    assert!(missing.is_empty(), "apps draw nodes the Apple apps don't: {missing:?}");
}

/// Everything a screen asks of its shell through the bridge, the Apple apps answer.
#[test]
fn the_apple_apps_answer_everything_a_screen_asks() {
    let bridge = read("src/screen/bridge.js");
    let swift = read("apple/Shared/Views/AppScreen.swift");
    let asked = quoted(&bridge, "{ type:");
    assert!(asked.contains("call") && asked.contains("open") && asked.contains("haptic"), "{asked:?}");
    let answered = quoted(&swift, "case");
    let missing: Vec<_> = asked.difference(&answered).collect();
    assert!(missing.is_empty(), "screens ask for what the Apple apps don't answer: {missing:?}");
}

/// A sidecar written in Go reads each call the node hands it by the names the node gives its fields.
#[test]
fn go_sidecars_read_the_calls_the_node_hands_them() {
    let call = one::sidecars::Call {
        id: 1,
        op: "send".into(),
        input: json!({}),
        caller: one::apps::Me { key: "k".into(), user: "u".into(), name: "Mac".into(), role: None, user_name: "Andre".into(), agent: None },
        from: Some("board".into()),
    };
    let sent = serde_json::to_value(&call).unwrap();
    let mut read = 0;
    for dir in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sidecars")).unwrap() {
        let main = dir.unwrap().path().join("main.go");
        let Ok(go) = std::fs::read_to_string(&main) else { continue };
        let Some(body) = go.split("type call struct {").nth(1) else { continue };
        // The struct's lines, to its closing brace at the start of a line. A field that's a struct of its own has
        // its tag on its closing line, after the fields in it.
        let (mut inner, mut paths) = (None::<Vec<String>>, vec![]);
        for line in body.split("\n}").next().unwrap().lines() {
            let tag = quoted(line, "json:").into_iter().next();
            match (line.trim_end().ends_with("struct {"), line.trim_start().starts_with('}'), tag, &mut inner) {
                (true, ..) => inner = Some(vec![]),
                (_, true, Some(tag), Some(fields)) => {
                    paths.extend(fields.iter().map(|f| vec![tag.clone(), f.clone()]));
                    inner = None;
                }
                (_, _, Some(tag), Some(fields)) => fields.push(tag),
                (_, _, Some(tag), None) => paths.push(vec![tag]),
                _ => {}
            }
        }
        for path in paths {
            let at = path.iter().fold(&sent, |v, k| &v[k.as_str()]);
            assert!(!at.is_null(), "{}: reads {}, which the node doesn't send: {sent}", main.display(), path.join("."));
            read += 1;
        }
    }
    assert!(read >= 10, "the test read the Go sidecars' calls ({read} fields)");
}

/// The kernel's servers (JavaScript, `kernel/shared/crypto.js`) and compute servers (`src/kernel.rs`) derive the same
/// keys from a scope's key, and each opens what the other seals.
#[test]
fn the_kernel_and_compute_servers_agree_on_keys_and_seals() {
    use data_encoding::BASE64URL_NOPAD as B64U;
    use one::kernel;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("kernel");
    let files = ["shared/crypto.js", "test/formats.js"].map(|p| (p.to_string(), std::fs::read(root.join(p)).unwrap()));
    let dir = std::env::temp_dir().join(format!("one-formats-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let addr = format!("127.0.0.1:{}", std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port());
    let served = kernel::Served { name: "formats".into(), module: kernel::module_from(files.to_vec(), "test/formats.js").unwrap(), class: None, listen: addr.clone(), bindings: vec![] };
    let mut child = kernel::start(&kernel::write_config(&dir, &[served]).unwrap()).unwrap();
    let started = std::time::Instant::now();
    while std::net::TcpStream::connect(&addr).is_err() {
        assert!(started.elapsed().as_secs() < 30, "workerd didn't start");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let js = |body: Value| -> Value {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        agent.post(format!("http://{addr}")).send_json(body).unwrap().body_mut().read_json().unwrap()
    };
    let b = |v: &Value| B64U.decode(v.as_str().unwrap().as_bytes()).unwrap();

    let scope: [u8; 32] = rand::random();
    let derived = js(json!({ "op": "derive", "scopeKey": B64U.encode(&scope), "app": "board", "database": "main" }));
    let app = kernel::app_key(&scope, "board");
    assert_eq!(b(&derived["app"]), app, "an app's key");
    assert_eq!(b(&derived["database"]), kernel::database_key(&app, "main"), "a database's key");

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

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
