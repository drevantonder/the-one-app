//! Where code in two languages must agree (the node sends, the Mac, iPhone, Android and Windows apps, the Go sidecars, the screens'
//! bridge and the chat view read), a test reads both and checks they do, so neither can drift from the other.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

fn read(path: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
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

/// The `@Serializable data class`es in Kotlin source, by name, each with the fields it reads: its JSON key (its
/// `@SerialName`, or its name), its type written as Swift's are, and whether decoding fails without it (no default,
/// and not nullable).
fn kotlin_classes(src: &str) -> BTreeMap<String, Vec<Field>> {
    // Comments go first: a doc comment may sit between two fields.
    let mut text = String::new();
    let mut rest = src;
    while let Some(at) = rest.find("/*") {
        text.push_str(&rest[..at]);
        rest = rest[at..].split_once("*/").map(|(_, r)| r).unwrap_or("");
    }
    text.push_str(rest);
    let text: String = text.lines().map(|l| l.split(" //").next().unwrap()).collect::<Vec<_>>().join("\n");
    fn swifty(ty: &str) -> String {
        let ty = ty.trim().trim_end_matches('?');
        let inner = |t: &str, prefix: &str| t.strip_prefix(prefix).and_then(|t| t.strip_suffix('>')).map(str::to_string);
        if let Some(of) = inner(ty, "List<") {
            return format!("[{}]", swifty(&of));
        }
        if let Some(kv) = inner(ty, "Map<") {
            let (k, v) = kv.split_once(',').unwrap();
            return format!("[{}: {}]", swifty(k), swifty(v));
        }
        match ty {
            "Int" | "Long" => "Int",
            "Double" | "Float" => "Double",
            "Boolean" => "Bool",
            other => other,
        }
        .to_string()
    }
    let mut out = BTreeMap::new();
    for chunk in text.split("@Serializable").skip(1) {
        let chunk = chunk.trim_start();
        let chunk = if chunk.starts_with('(') { chunk.split_once(')').map(|(_, r)| r.trim_start()).unwrap_or("") } else { chunk };
        let Some(rest) = chunk.strip_prefix("data class ") else { continue };
        let name = word(rest).to_string();
        let Some(open) = rest.find('(') else { continue };
        // Its parameters, split at the commas between them, not those in a type or a default.
        let (mut depth, mut params, mut current) = (0, vec![], String::new());
        for c in rest[open + 1..].chars() {
            match c {
                '(' | '<' | '{' | '[' => depth += 1,
                ')' | '>' | '}' | ']' if depth > 0 => depth -= 1,
                ')' => break,
                ',' if depth == 0 => {
                    params.push(std::mem::take(&mut current));
                    continue;
                }
                _ => {}
            }
            current.push(c);
        }
        params.push(current);
        let fields = params
            .iter()
            .filter_map(|p| {
                let key = p.split_once("@SerialName(\"").and_then(|(_, k)| k.split_once('"')).map(|(k, _)| k.to_string());
                let decl = p.split_once("val ").or_else(|| p.split_once("var "))?.1;
                let (field, ty) = decl.split_once(':')?;
                let (ty, default) = match ty.split_once('=') {
                    Some((ty, _)) => (ty, true),
                    None => (ty, false),
                };
                Some(Field { key: key.unwrap_or_else(|| field.trim().to_string()), ty: swifty(ty), required: !default && !ty.trim().ends_with('?') })
            })
            .collect();
        out.insert(name, fields);
    }
    out
}

/// The classes the Windows app decodes (windows/Models.cs), by name, each with the fields it reads: a property is one
/// line, `[JsonPropertyName("key")] public [required] Type Name { … }`, its type written as Swift's are (`int` as
/// `Int32`, which a bigger number doesn't fit), and System.Text.Json fails without a `required` one.
fn csharp_classes(src: &str) -> BTreeMap<String, Vec<Field>> {
    fn swifty(ty: &str) -> String {
        let ty = ty.trim().trim_end_matches('?');
        let inner = |t: &str, prefix: &str| t.strip_prefix(prefix).and_then(|t| t.strip_suffix('>')).map(str::to_string);
        if let Some(of) = inner(ty, "List<") {
            return format!("[{}]", swifty(&of));
        }
        if let Some(of) = inner(ty, "Dictionary<string,") {
            return format!("[String: {}]", swifty(&of));
        }
        match ty {
            "string" => "String",
            "int" => "Int32",
            "long" => "Int",
            "double" => "Double",
            "bool" => "Bool",
            other => other,
        }
        .to_string()
    }
    let mut out: BTreeMap<String, Vec<Field>> = BTreeMap::new();
    let mut class = None;
    for line in src.lines().map(str::trim) {
        if let Some((_, rest)) = line.split_once(" class ").filter(|(before, _)| before.starts_with("public")) {
            class = Some(word(rest).to_string());
            out.entry(word(rest).to_string()).or_default();
        }
        let (Some(class), Some(rest)) = (&class, line.strip_prefix("[JsonPropertyName(\"")) else { continue };
        let (key, rest) = rest.split_once('"').unwrap();
        let rest = rest.split_once("public ").unwrap().1;
        let (ty, _) = rest.trim_start_matches("required ").split_once(" {").unwrap().0.rsplit_once(' ').unwrap();
        out.get_mut(class).unwrap().push(Field { key: key.into(), ty: swifty(ty), required: rest.starts_with("required ") });
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
    let wrong = |what: &str| format!("{path}: the app reads {ty}, the node sent {what}");
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
        "Int32" => value.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
        "Double" => value.is_number(),
        "Bool" => value.is_boolean(),
        _ => match named(ty) {
            Some(name) => {
                for f in &structs[&name] {
                    match value.get(&f.key).filter(|v| !v.is_null()) {
                        Some(v) => check(structs, &name, &f.ty, v, &format!("{path}.{}", f.key), problems),
                        None if f.required => problems.push(format!("{path}.{}: the app's {name} needs it, the node left it out", f.key)),
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

async fn start(name: &str) -> one::node::Node {
    let dir = crate::temp(name);
    std::fs::write(dir.join("device.json"), json!({ "secret": one::entry::hex(&rand::random()), "name": name }).to_string()).unwrap();
    one::node::Node::start(&dir, one::sidecars::built_in()).await.unwrap()
}

/// GETs or POSTs the node's local API, as the Mac app does.
async fn api(addr: &str, token: &str, method: &str, path: &str, body: Value) -> Value {
    let url = format!("http://{addr}{path}");
    let (method, token) = (method.to_string(), token.to_string());
    tokio::task::spawn_blocking(move || {
        let mut reply = match method.as_str() {
            "GET" => ureq::get(&url).header(one::api::LOCAL_TOKEN_HEADER, &token).call(),
            _ => ureq::post(&url).header(one::api::LOCAL_TOKEN_HEADER, &token).send_json(&body),
        }
        .unwrap_or_else(|e| panic!("{url}: {e}"));
        reply.body_mut().read_json::<Value>().unwrap()
    })
    .await
    .unwrap()
}

/// Every payload the Mac, iPhone, Android and Windows apps decode from a node decodes with the structs (Kotlin's data
/// classes, in android/…/Node.kt; C#'s, in windows/Models.cs) they decode it into: each field they need is there, and
/// each field they read has the type they read it as. A shell built before still decodes what a newer device sends,
/// so a field leaves the payload only once no shell needs it.
#[tokio::test(flavor = "multi_thread")]
async fn the_apps_decode_what_the_node_sends() {
    let swift = read("apple/Shared/Node.swift") + &read("apple/Shared/Views/WidgetTree.swift");
    let structs = swift_structs(&swift);
    for name in ["NodeState", "Install", "Device", "AppInfo", "WidgetInfo", "WidgetNode", "VaultItem", "VaultRequest", "Arrangement.Item", "SettingsItem"] {
        assert!(structs.get(name).is_some_and(|f| !f.is_empty()), "the test reads Swift's {name}: {:?}", structs.keys());
    }
    assert!(structs["VaultItem"].iter().any(|f| f.key == "scope_name" && !f.required) && structs["VaultItem"].iter().any(|f| f.key == "title" && f.required));
    assert!(structs["NodeState"].iter().any(|f| f.key == "me" && f.required), "CodingKeys rename fields");
    let classes = kotlin_classes(&read("android/app/src/main/kotlin/app/client/Node.kt"));
    for name in ["NodeState", "Install", "Device", "AppInfo", "WidgetInfo", "WidgetNode"] {
        assert!(classes.get(name).is_some_and(|f| !f.is_empty()), "the test reads Kotlin's {name}: {:?}", classes.keys());
    }
    assert!(classes["NodeState"].iter().any(|f| f.key == "me" && f.required), "@SerialName renames fields");
    assert!(classes["AppInfo"].iter().any(|f| f.key == "screens" && f.ty == "[AppScreenInfo]" && !f.required), "{:?}", classes["AppInfo"]);
    let windows_classes = csharp_classes(&read("windows/Models.cs"));
    for name in ["NodeState", "Install", "Device", "AppInfo", "WidgetInfo", "SettingsItem"] {
        assert!(windows_classes.get(name).is_some_and(|f| !f.is_empty()), "the test reads C#'s {name}: {:?}", windows_classes.keys());
    }
    assert!(windows_classes["NodeState"].iter().any(|f| f.key == "settings_items" && f.ty == "[SettingsItem]" && !f.required), "{:?}", windows_classes["NodeState"]);

    let node = start("contracts").await;
    node.create("Contracts", "Andre").await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let router = one::api::router(node.clone());
    tokio::spawn(one::api::serve(listener, router));
    let token = node.local_token().to_string();

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
    let me = one::node::Caller { key: node.id(), user: node.user().unwrap(), agent: None, token: None };
    node.member("app", json!({ "name": "lamps", "owner": format!("user:{}", me.user) })).await.unwrap();
    node.deploy(&me, "lamps", &blob).await.unwrap();
    node.member("app", json!({ "name": "lamps", "owner": format!("user:{}", me.user), "source": "https://example.com/lamps" })).await.unwrap();
    api(&addr, &token, "POST", "/v1/install", json!({ "app": "lamps", "scope": "user", "installed": true })).await;
    api(&addr, &token, "POST", "/v1/arrange", json!({ "page": "user", "items": [{ "app": "lamps", "widget": "glance", "id": "w1", "size": "small", "options": { "room": "kitchen" } }, { "app": "lamps" }], "hidden": [] })).await;
    api(&addr, &token, "POST", "/v1/group", json!({ "name": "Home" })).await;
    api(&addr, &token, "POST", "/v1/vault", json!({ "scope": "user", "title": "Hub", "fields": { "user": { "value": "andre" }, "token": { "value": "s3cret", "secret": true } }, "hosts": ["hub.example.com"], "apps": ["lamps"] })).await;
    api(&addr, &token, "POST", "/v1/vault/request", json!({ "scope": "user", "app": "lamps", "need": "hub", "why": "To reach the hub" })).await;

    let mut problems = vec![];
    for (path, ty) in [("/v1/state", "NodeState"), ("/v1/apps", "[AppInfo]"), ("/v1/widgets?scope=user", "[WidgetInfo]"), ("/v1/vault", "[VaultItem]"), ("/v1/vault/requests", "[VaultRequest]")] {
        let sent = api(&addr, &token, "GET", path, Value::Null).await;
        assert!(sent.as_array().is_none_or(|l| !l.is_empty()), "{path} sent nothing to check");
        check(&structs, "", ty, &sent, path, &mut problems);
    }
    let mut android = vec![];
    for (path, ty) in [("/v1/state", "NodeState"), ("/v1/apps", "[AppInfo]"), ("/v1/widgets?scope=user", "[WidgetInfo]")] {
        check(&classes, "", ty, &api(&addr, &token, "GET", path, Value::Null).await, path, &mut android);
    }
    let mut windows = vec![];
    for (path, ty) in [("/v1/state", "NodeState"), ("/v1/apps", "[AppInfo]"), ("/v1/widgets?scope=user", "[WidgetInfo]"), ("/v1/badges?scope=user", "[String: Int32]")] {
        check(&windows_classes, "", ty, &api(&addr, &token, "GET", path, Value::Null).await, path, &mut windows);
    }
    let state = api(&addr, &token, "GET", "/v1/state", Value::Null).await;
    assert!(!state["install"]["groups"].as_array().unwrap().is_empty() && state["arrangements"]["user"].is_object(), "{state}");
    assert!(problems.is_empty(), "the Apple apps can't decode what the node sends:\n{}", problems.join("\n"));
    assert!(android.is_empty(), "the Android app can't decode what the node sends:\n{}", android.join("\n"));
    assert!(windows.is_empty(), "the Windows app can't decode what the node sends:\n{}", windows.join("\n"));
}

/// The kernel's services (JavaScript, `kernel/shared/crypto.js`) and nodes (`src/kernel.rs`) each open what the
/// other seals: app keys under scope keys, keys to key pairs and under secrets; and derive the same database keys.
#[test]
fn the_kernel_and_nodes_agree_on_keys_and_seals() {
    use data_encoding::BASE64URL_NOPAD as B64U;
    use one::kernel;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("kernel");
    let files = ["shared/crypto.js", "shared/token.js", "schedules/worker.js", "test/formats.js"].map(|p| (p.to_string(), std::fs::read(root.join(p)).unwrap()));
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
        let device: [u8; 32] = rand::random();
        let prf: [u8; 32] = rand::random();
        let (my_secret, my_public) = kernel::twin(&rand::random());
        let out = js(json!({
            "op": "keys", "seed": B64U.encode(&seed), "domain": "one/urls", "fields": fields, "at": 1_000_000,
            "request": request, "now": 1_000_000 + keys::FRESH, "epoch": 4, "key": B64U.encode(&epoch_key),
            "name": "global/board/main/1", "plain": B64U.encode(b"from js"), "sealed": B64U.encode(&sealed),
            "to": B64U.encode(&kernel::twin_public(&node.verifying_key().to_bytes()).unwrap()), "handoff": handoff,
            "pair": { "secret": B64U.encode(&my_secret), "public": B64U.encode(&my_public), "sealed": keys::seal_handoff(&my_public, &handoff).unwrap(), "key": data_encoding::HEXLOWER.encode(&device) },
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
        assert_eq!(out["joinCode"], keys::join_code(&device));
        assert_eq!(out["proof"], keys::invite_proof(b"an invite's secret", &device));
        let (passkey, seal_secret) = keys::passkey_keys(&prf);
        assert_eq!(out["passkey"]["sign"], data_encoding::HEXLOWER.encode(passkey.verifying_key().as_bytes()), "a passkey's signing key");
        assert_eq!(b(&out["passkey"]["seal"]), seal_secret, "and the secret sealed to");
        assert_eq!(to, kernel::twin_public(&node.verifying_key().to_bytes()).unwrap(), "a Rust endpoint seals to its key's twin");
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
    }

    // The schedules service and the node agree when a schedule runs next, across daylight saving's changes too.
    let specs = [
        json!({ "op": "x", "at": "09:00", "tz": "Australia/Melbourne" }),
        json!({ "op": "x", "at": "02:30", "tz": "America/New_York", "days": ["sun"] }),
        json!({ "op": "x", "at": "23:59", "tz": "Europe/London", "days": ["mon", "fri"] }),
        json!({ "op": "x", "every": "15m" }),
        json!({ "op": "x", "every": "1d" }),
    ];
    // Around Melbourne's change in April and October 2026, New York's in March and November, and an ordinary day.
    let instants: [u64; 5] = [1_775_318_400_000, 1_791_043_200_000, 1_772_928_000_000, 1_793_491_200_000, 1_793_600_000_000];
    for spec in &specs {
        let rust = one::schedules::Schedule::parse("s", spec).unwrap();
        for at in instants.iter().flat_map(|t| (0..48).map(move |h| t + h * 1_800_000)) {
            let js = js(json!({ "op": "next", "spec": spec, "now": at }));
            assert_eq!(js["next"].as_u64(), rust.next(at), "{spec} after {at}");
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The gateway (`kernel/gateway/worker.js`) sends a node every path the node takes kernel sign-ins at
/// (`api::SIGNED_IN`), and marks each with a header the node knows means it came from elsewhere (`api::FORWARDED`),
/// so a node on the gateway's own machine never takes it as one of the machine's own programs; through a Cloudflare
/// Tunnel, which adds no such header, it also addresses the node by its own host, never one a node takes as local. MCP
/// clients' session ids pass both ways, and a node's `www-authenticate` reaches them, saying where to sign in.
#[test]
fn the_gateway_sends_nodes_what_they_take_sign_ins_at_marked_as_from_elsewhere() {
    use std::io::{BufRead, BufReader, Write};
    use one::kernel;

    // A stand-in node: it answers each request with the request's line and headers.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let node = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // Each connection on its own, and one that closes or resets before it's answered is let go: workerd may
            // drop one it opened.
            std::thread::spawn(move || -> std::io::Result<()> {
                let mut reader = BufReader::new(stream.try_clone()?);
                let mut line = String::new();
                reader.read_line(&mut line)?;
                let mut headers = Vec::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                        break;
                    }
                    headers.push(h.split(':').next().unwrap().trim().to_lowercase());
                }
                let body = json!({ "line": line.trim(), "headers": headers }).to_string();
                write!(&stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nwww-authenticate: Bearer\r\nmcp-session-id: s\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
            });
        }
    });

    // A node behind a Cloudflare Tunnel, as the gateway's `NODE` binding reaches it: it answers each request with the
    // request's line, its Host, and its headers' names.
    let tunnelled = br#"export default { fetch(r) { const u = new URL(r.url); return Response.json({ line: `${r.method} ${u.pathname}${u.search} HTTP/1.1`, host: u.host, headers: [...r.headers.keys()] }, { headers: { "www-authenticate": "Bearer", "mcp-session-id": "s" } }); } };"#;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("kernel");
    let files = vec![("gateway/worker.js".to_string(), std::fs::read(root.join("gateway/worker.js")).unwrap())];
    for tunnel in [false, true] {
        let dir = crate::temp("gateway");
        let mut listening = vec![("gateway".to_string(), std::net::TcpListener::bind("127.0.0.1:0").unwrap())];
        let addr = listening[0].1.local_addr().unwrap().to_string();
        let mut served = vec![kernel::Served {
            name: "gateway".into(),
            module: kernel::module_from(files.clone(), "gateway/worker.js").unwrap(),
            modules: vec![],
            class: None,
            listen: Some(addr.clone()),
            bindings: vec![if tunnel { ("NODE".into(), "service:tunnelled".into()) } else { ("NODE_URL".into(), node.clone()) }],
            assets: None,
            outbound: Some(kernel::NEARBY.into()),
        }];
        if tunnel {
            listening.push(("tunnelled".into(), std::net::TcpListener::bind("127.0.0.1:0").unwrap()));
            let listen = listening[1].1.local_addr().unwrap().to_string();
            served.push(kernel::Served { name: "tunnelled".into(), module: tunnelled.to_vec(), modules: vec![], class: None, listen: Some(listen), bindings: vec![], assets: None, outbound: None });
        }
        let mut child = kernel::start(&kernel::write_config(&dir, &served).unwrap(), listening).unwrap();
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        for route in one::api::SIGNED_IN {
            let path = route.replace("{app}", "board").replace("{screen}", "main").replace("{hash}", "00").replace("{name}", "vault").replace("{op}", "now");
            let mut resp = agent.get(format!("http://{addr}{path}")).header("host", "one.example").header("mcp-session-id", "s").call().unwrap();
            if path == "/mcp" {
                for h in ["www-authenticate", "mcp-session-id"] {
                    assert!(resp.headers().contains_key(h), "the gateway kept the node's {h} from an MCP client");
                }
            }
            let seen: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
            assert_eq!(seen["line"], format!("GET {path} HTTP/1.1"), "the gateway didn't send {path} to the node");
            let marked = seen["headers"].as_array().unwrap().iter().any(|h| one::api::FORWARDED.contains(&h.as_str().unwrap()));
            assert!(marked, "the gateway sent {path} without saying it came from elsewhere: {seen}");
            if path == "/mcp" {
                assert!(seen["headers"].as_array().unwrap().iter().any(|h| h == "mcp-session-id"), "the gateway kept an MCP client's session id from the node");
            }
            if tunnel {
                assert_eq!(seen["host"], "one.example", "through a tunnel, the gateway addressed the node by a host not its own");
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The address lists the Android app by the package the app's build gives it, so links to the address open it.
#[test]
fn the_address_lists_the_android_app_by_its_package() {
    let gradle = read("android/app/build.gradle.kts");
    assert!(gradle.contains(r#"applicationId = "app.$cli.android""#), "android/app/build.gradle.kts names the app's package otherwise: change one::kernel::android_package too");
    assert_eq!(one::kernel::android_package(), format!("app.{}.android", one::config::CLI));
}

/// Every message a screen sends its host (`messages`, in src/screen/bridge.js) is one each host names, answering it
/// or refusing it: the web client's page and a chat's view, which answer frames, and the shells' web views.
#[test]
fn every_host_answers_each_message_of_the_bridge() {
    let bridge = read("src/screen/bridge.js");
    let list = &bridge[bridge.find("const messages = [").expect("bridge.js lists its messages") + 18..];
    let messages: Vec<&str> = list[..list.find(']').unwrap()].split(',').map(|m| m.trim().trim_matches('"')).filter(|m| !m.is_empty()).collect();
    assert!(messages.len() > 10 && messages.contains(&"menu"), "{messages:?}");
    // What a host names: `case "x"` (JavaScript, C#, Swift) or a `when` arm, `"x", "y" ->` (Kotlin).
    let named = |src: &str| -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in src.lines().map(str::trim) {
            let arm = line.strip_prefix("case ").or_else(|| line.starts_with('"').then(|| &line[..line.find("->").unwrap_or(0)])).unwrap_or("");
            out.extend(arm.split('"').skip(1).step_by(2).map(String::from));
        }
        out
    };
    for path in [
        "kernel/web/web.js",
        "src/mcp_view.html",
        "android/app/src/main/kotlin/app/client/Screens.kt",
        "windows/AppScreen.cs",
        "apple/Shared/Views/AppScreen.swift",
    ] {
        let names = named(&read(path));
        let missing: Vec<_> = messages.iter().filter(|m| !names.contains(**m)).collect();
        assert!(missing.is_empty(), "{path} doesn't answer the bridge's {missing:?}");
    }
}

/// Windows opens links to the address in the app only if the address's `windows-app-web-link` names its package as
/// Windows does: from the name and publisher windows/build.sh gives it, as the hash Windows made in a VM says.
#[test]
fn the_gateway_names_the_windows_app_as_windows_does() {
    let build = read("windows/build.sh");
    assert!(build.contains("file=${name//[^A-Za-z0-9]/}") && build.contains("CN=${name//[^A-Za-z0-9 ]/}"), "build.sh names the package and its publisher otherwise");
    if one::config::NAME == "The One App" {
        assert_eq!(one::kernel::windows_package(), "TheOneApp_ah4xyz62b3km4");
    }
}

/// A widget's tree is its app's, drawn on the clients' own screens, which act as the person looking: markup in every
/// field of every node it can have draws as text, and makes no element, attribute or style (src/screen/widgets.js);
/// and those screens run only their own scripts, refusing an inline handler or a script added to them (src/screen.rs),
/// in a browser.
#[test]
fn an_apps_widget_runs_nothing_on_the_clients_own_screens() {
    let chrome = one::deploy::chrome().expect("this test draws screens in Chrome or Chromium: none was found (CHROME=<path> names one)");
    let dir = crate::temp("screens");
    // What each page says once it's drawn, as its title.
    let title = |name: &str, body: &str, own: bool| -> String {
        let page = one::screen::page(&json!({ "screen": "test", "params": {}, "me": null, "config": { "name": "Test" } }), body, own);
        let path = dir.join(format!("{name}.html"));
        std::fs::write(&path, page).unwrap();
        let out = std::process::Command::new(&chrome)
            .args(["--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--virtual-time-budget=5000", "--dump-dom"])
            .arg(format!("--user-data-dir={}", dir.join("profile").display()))
            .arg(format!("file://{}", path.display()))
            .output()
            .unwrap();
        let dom = String::from_utf8_lossy(&out.stdout);
        let title = dom.split("<title>").nth(1).and_then(|t| t.split("</title>").next());
        title.unwrap_or_else(|| panic!("{name} wasn't drawn: {}", String::from_utf8_lossy(&out.stderr))).to_string()
    };

    // Drawn with no policy, so only widgets.js keeps the markup out.
    let widgets = concat!("<script>", include_str!("../src/screen/widgets.js"), "</script>", r#"<script>
      const markup = `1"/><img src=x onerror="document.documentElement.dataset.ran = 1"><svg onload="document.documentElement.dataset.ran = 1">`;
      const fields = ["text", "value", "unit", "label", "max", "w", "h", "gap", "lines", "tone", "size", "weight", "style", "at", "title", "subtitle", "trailing",
        "empty", "url", "name", "path", "badge", "kind", "align", "dim", "grow", "round", "on", "icon", "op", "header"];
      const node = (t, more = {}) => ({ ...Object.fromEntries(fields.map((f) => [f, markup])), t, values: [markup, 1, 2], labels: [markup, markup], c: [{ t: "text", text: markup }], link: "javascript:document.documentElement.dataset.ran = 1", ...more });
      const types = ["col", "row", "text", "time", "stat", "gauge", "progress", "spark", "chart", "list", "item", "image", "dot", "icon", "link", "divider", "button", "toggle"];
      const trees = [...types.map((t) => node(t)), node("chart", { kind: "line" }), node("spark", { values: [] }), node("icon", { name: undefined }), node("col", { c: types.map((t) => node(t)) })];
      const host = { run: async () => {}, follow: () => {} };
      let failed = 0;
      for (const w of [...trees.map((root) => ({ root })), { error: markup }]) {
        try {
          document.body.append(oneWidgets.card({ app: markup, widget: markup, title: markup, ...w }, host));
        } catch {
          failed++;
        }
      }
      document.querySelectorAll(".w-card, .w-card *").forEach((el) => el.dispatchEvent(new MouseEvent("click", { bubbles: true })));
      addEventListener("load", () => setTimeout(() => {
        const made = document.querySelectorAll("[onerror], [onload], img[src='x'], svg image, svg svg").length;
        const styled = [...document.querySelectorAll("[style], [class]")].filter((el) => /onerror|<|"/.test(el.getAttribute("style") + el.getAttribute("class"))).length;
        const drawn = document.querySelectorAll(".w-card").length, marks = document.querySelectorAll(".w-card polyline, .w-card rect, .w-card circle").length;
        document.title = `${document.documentElement.dataset.ran ? "ran" : "inert"} failed ${failed} made ${made} styled ${styled} drawn ${drawn} marks ${marks > 4 ? "yes" : "no"}`;
      }, 500));
    </script>"#);
    assert_eq!(title("widgets", widgets, false), "inert failed 0 made 0 styled 0 drawn 23 marks yes", "a widget's markup is only ever text");

    // A client's own screen: its scripts run, markup put in it can't run any.
    let own = r#"<script>
      document.body.insertAdjacentHTML("beforeend", `<img src=x onerror="document.documentElement.dataset.ran = 'handler'"><a id="go" href="javascript:void (document.documentElement.dataset.ran = 'link')">x</a>`);
      document.body.append(Object.assign(document.createElement("script"), { textContent: "document.documentElement.dataset.ran = 'script'" }));
      document.getElementById("go").click();
      addEventListener("load", () => setTimeout(() => (document.title = `ours ran, then ${document.documentElement.dataset.ran || "nothing"}`), 500));
    </script>"#;
    assert_eq!(title("own", own, true), "ours ran, then nothing", "a client's own screen runs only its own scripts");
    // As an app's screen, the same runs (so the policy is what stops it).
    assert_ne!(title("app", own, false), "ours ran, then nothing", "without the policy, this page does run what's put in it");
    let _ = std::fs::remove_dir_all(&dir);
}

/// This machine's own programs in every language send its node the token from the file it keeps it in, in the header
/// it reads it from (`api::LOCAL_TOKEN`, `api::LOCAL_TOKEN_HEADER`): each sidecar, with the same code, and the Mac app.
#[test]
fn every_program_on_a_machine_sends_its_node_the_token_where_it_looks() {
    let go = read("sidecars/agent-devices/node.go");
    for dir in std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sidecars")).unwrap().flatten().filter(|d| d.path().join("main.go").exists()) {
        let name = dir.file_name().to_string_lossy().to_string();
        assert_eq!(read(&format!("sidecars/{name}/node.go")), go, "sidecars/{name}/node.go differs from agent-devices'");
        assert!(read(&format!("sidecars/{name}/main.go")).contains("nodeAddr()"), "sidecars/{name} asks the node without its token");
    }
    let swift = read("apple/Shared/Node.swift");
    for (language, source) in [("Go", &go), ("Swift", &swift)] {
        for name in [one::api::LOCAL_TOKEN, one::api::LOCAL_TOKEN_HEADER] {
            assert!(source.contains(&format!("\"{name}\"")), "{language} doesn't name {name} as the node does");
        }
    }
}
