//! AI Gateway's secrets, who pays and what's kept: what using it wouldn't show. Each runs the gateway's own code
//! (`ai_gateway_runtime::run`) against a stand-in for the node it runs on (`Stand`), whose `send` is where the outside
//! world begins: no request leaves it. It runs with the rest of `cargo test` (tests/main.rs).

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ::ai_gateway::{Company, Kind, Outgoing, Paid};
use ai_gateway_runtime::{Connection, Fut, Host, Key, Sent, Span, Whose};
use ::scope::scopes::Scope;
use vault::{Field, Item, Items};
use serde_json::{Value, json};

/// An `AI key: <company>` item, with its secret `api_key`, sent only to `hosts`, switched on for `apps`, made at `made`.
fn key_item(company: Company, secret: &str, hosts: &[&str], apps: &[&str], made: u64) -> Item {
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("api_key".to_string(), Field { value: secret.to_string(), secret: true });
    if company == Company::WorkersAi {
        fields.insert("account_id".to_string(), Field { value: "acct".to_string(), secret: false });
    }
    Item { title: format!("{} Key", company.title()), kind: company.key_kind(), fields, hosts: hosts.iter().map(|h| h.to_string()).collect(), apps: apps.iter().map(|a| a.to_string()).collect(), made, opened: true, ..Item::default() }
}

fn items(list: &[(&str, Item)]) -> Items {
    list.iter().map(|(id, it)| (id.to_string(), it.clone())).collect()
}

/// A node as AI Gateway sees it: the caller's own and Everyone's `node/ai-gateway` in memory, the key `pays` picks from
/// `vault`, and a company that answers with `answer`, counting what's sent.
struct Stand {
    own: Mutex<Connection>,
    everyone: Mutex<Connection>,
    vault: Vec<(Scope, Items)>,
    app: Option<String>,
    user: String,
    answer: fn(&Outgoing) -> Sent,
    sent: AtomicUsize,
    keys_sent: Mutex<Vec<String>>,
    spans: Mutex<Vec<Span>>,
}

impl Stand {
    fn new(vault: Vec<(Scope, Items)>, app: Option<&str>, user: &str, answer: fn(&Outgoing) -> Sent) -> Stand {
        let db = || {
            let c = Connection::open_in_memory().unwrap();
            for m in ai_gateway_runtime::SCHEMA {
                c.execute_batch(m).unwrap();
            }
            Mutex::new(c)
        };
        Stand { own: db(), everyone: db(), vault, app: app.map(str::to_string), user: user.to_string(), answer, sent: AtomicUsize::new(0), keys_sent: Mutex::new(Vec::new()), spans: Mutex::new(Vec::new()) }
    }

    fn db(&self, whose: Whose) -> &Mutex<Connection> {
        match whose {
            Whose::Own => &self.own,
            Whose::Everyone => &self.everyone,
        }
    }
}

impl Host for Stand {
    fn key(&self, company: Company) -> Fut<'_, Option<Key>> {
        Box::pin(async move { Ok(one::ai_gateway::pays(&self.vault, self.app.as_deref(), &self.user, company).map(|(item, paid)| Key { item, paid })) })
    }

    fn send<'a>(&'a self, key: &'a Key, out: Outgoing) -> Fut<'a, Sent> {
        Box::pin(async move {
            self.sent.fetch_add(1, Ordering::SeqCst);
            self.keys_sent.lock().unwrap().push(key.item.clone());
            Ok((self.answer)(&out))
        })
    }

    fn query<'a>(&'a self, whose: Whose, sql: &'a str, params: Vec<Value>) -> Fut<'a, Vec<Value>> {
        Box::pin(async move {
            let c = self.db(whose).lock().unwrap();
            let mut stmt = c.prepare(sql)?;
            let names: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
            let params: Vec<Box<dyn rusqlite::ToSql>> = params.iter().map(|p| -> Box<dyn rusqlite::ToSql> { match p { Value::Number(n) => Box::new(n.as_i64().unwrap_or(0)), Value::String(s) => Box::new(s.clone()), _ => Box::new(p.to_string()) } }).collect();
            let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
                let mut row = serde_json::Map::new();
                for (i, n) in names.iter().enumerate() {
                    let v: rusqlite::types::Value = r.get(i)?;
                    row.insert(n.clone(), match v { rusqlite::types::Value::Integer(i) => json!(i), rusqlite::types::Value::Text(t) => json!(t), _ => Value::Null });
                }
                Ok(Value::Object(row))
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    fn write<'a>(&'a self, whose: Whose, mut change: Box<dyn FnMut(&Connection) -> anyhow::Result<Value> + Send + 'a>) -> Fut<'a, Value> {
        Box::pin(async move {
            let mut c = self.db(whose).lock().unwrap();
            let t = c.transaction()?;
            let out = change(&t)?;
            t.commit()?;
            Ok(out)
        })
    }

    fn device<'a>(&'a self, _: Kind, _: Value) -> Fut<'a, Value> {
        Box::pin(async move { anyhow::bail!(::ai_gateway::Failed::Unavailable("no device here".into())) })
    }

    fn here(&self) -> Fut<'_, Value> {
        Box::pin(async move { Ok(Value::Null) })
    }

    fn record(&self, span: Span) {
        self.spans.lock().unwrap().push(span);
    }

    fn now(&self) -> u64 {
        1_800_000_000_000
    }
}

/// A chat completion that says `text`.
fn says(text: &str) -> Sent {
    let body = json!({ "model": "m", "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }], "usage": { "prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10 } });
    Sent { status: 200, retry_after: None, body: body.to_string().into_bytes() }
}

fn chat(model: &str, text: &str) -> Value {
    json!({ "model": model, "messages": [{ "role": "user", "content": text }] })
}

/// A key goes only to its company's host: every company's request is to that host and fills the key only for it, never
/// for a request anywhere else; and an app that lists the items it's switched on for never reads a key's value.
#[test]
fn a_key_goes_only_to_its_providers_host() {
    let secret = "sk-not-a-real-key-0123456789";
    for company in ::ai_gateway::route::ALL {
        let item = key_item(company, secret, &[company.host()], &["notes"], 1);
        let model = if company == Company::WorkersAi { format!("{}/@cf/meta/llama-3.1-8b-instruct", company.id()) } else { format!("{}/some-model", company.id()) };
        let asked = ::ai_gateway::ask(chat(&model, "hi"), &Default::default()).unwrap();
        let out = asked.request("k1").unwrap();
        let host = out.url.split("://").nth(1).and_then(|r| r.split(['/', ':']).next()).unwrap();
        assert_eq!(host, company.host(), "{}'s request goes to its own host", company.title());
        let lookup = |name: &str| name.strip_prefix("k1:").and_then(|f| vault::filled(&item, f));
        let filled: Vec<String> = out.headers.iter().map(|(_, v)| vault::fill(v, host, lookup).unwrap()).collect();
        assert!(filled.iter().any(|v| v.contains(secret)), "{}'s key is filled into its request", company.title());
        for elsewhere in ["evil.example", "api.openai.com.evil.example", "127.0.0.1"] {
            let anywhere = out.headers.iter().all(|(_, v)| vault::fill(v, elsewhere, lookup).is_err() || !v.contains("{{"));
            assert!(anywhere, "{}'s key is never filled into a request to {elsewhere}", company.title());
        }
        let shown = vault::Lookup { app: "notes".into(), user: "u1".into(), scopes: vec![(Scope::Global, items(&[("k1", item.clone())]))] }.for_app();
        assert!(!json!(shown).to_string().contains(secret), "an app never reads a key's value");
    }
}

/// The most specific key pays, and only for whom it's for: an app's own pays its calls and never another app's, a
/// user's pays theirs and never another user's, Everyone's pays the rest; and a key that fails is the call's failure,
/// never falling back to the next.
#[tokio::test]
async fn whose_key_pays() {
    let g = Company::Google;
    let host = [g.host()];
    let vault = vec![
        (Scope::Global, items(&[("everyones", key_item(g, "e", &host, &["node"], 1)), ("notes-own", key_item(g, "n", &host, &["notes"], 2))])),
        (Scope::User("u1".into()), items(&[("u1-own", key_item(g, "u", &host, &["node"], 3))])),
    ];
    let pays = |app: Option<&str>, user: &str| one::ai_gateway::pays(&vault, app, user, g);
    assert_eq!(pays(Some("notes"), "u1"), Some(("notes-own".into(), Paid::App)), "an app's own key pays its calls first");
    assert_eq!(pays(Some("notes"), "u2"), Some(("notes-own".into(), Paid::App)));
    assert_eq!(pays(Some("tally"), "u1"), Some(("u1-own".into(), Paid::User)), "another app's call never takes notes' key: the user's pays");
    assert_eq!(pays(Some("tally"), "u2"), Some(("everyones".into(), Paid::Network)), "another user's call never takes u1's key");
    assert_eq!(pays(None, "u2"), Some(("everyones".into(), Paid::Network)));
    let only_an_apps = vec![(Scope::Global, items(&[("notes-own", key_item(g, "n", &host, &["notes"], 2))]))];
    assert_eq!(one::ai_gateway::pays(&only_an_apps, Some("tally"), "u1", g), None, "an app's key pays for no one else's calls");

    // The user's key fails: the call fails with it, and nothing else is sent.
    let refused = |_: &Outgoing| Sent { status: 401, retry_after: None, body: b"bad key".to_vec() };
    let stand = Stand::new(vault.clone(), Some("tally"), "u1", refused);
    let err = ai_gateway_runtime::run(&stand, "tally", chat("google/gemini-x", "hi"), None).await.unwrap_err();
    assert!(format!("{err:#}").contains("401"), "{err:#}");
    assert_eq!(*stand.keys_sent.lock().unwrap(), vec!["u1-own".to_string()], "a failing key never falls back to Everyone's");
}

/// A retry with the same `retryKey` and input replays the first answer and sends nothing; the same key with other input
/// is refused; and the key is the caller's own, so another user's call with it is sent, never given someone's answer.
#[tokio::test]
async fn a_retry_is_never_charged_twice() {
    let vault = vec![(Scope::Global, items(&[("everyones", key_item(Company::OpenAi, "e", &["api.openai.com"], &["node"], 1))]))];
    let stand = Stand::new(vault.clone(), Some("claims"), "u1", |_| says("first"));
    let input = chat("openai/gpt-x", "read this claim");
    let first = ai_gateway_runtime::run(&stand, "claims", input.clone(), Some("c1:pay")).await.unwrap();
    let again = ai_gateway_runtime::run(&stand, "claims", input.clone(), Some("c1:pay")).await.unwrap();
    assert_eq!(stand.sent.load(Ordering::SeqCst), 1, "the retry sent nothing");
    assert_eq!(again["one"]["replayed"], true);
    assert_eq!(again["choices"], first["choices"], "the retry is the first answer");
    let changed = ai_gateway_runtime::run(&stand, "claims", chat("openai/gpt-x", "another claim"), Some("c1:pay")).await.unwrap_err();
    assert_eq!(changed.downcast_ref::<::ai_gateway::Failed>(), Some(&::ai_gateway::Failed::RetryChanged));
    assert_eq!(stand.sent.load(Ordering::SeqCst), 1, "changed input on a used key sends nothing");

    let someone_else = Stand::new(vault, Some("claims"), "u2", |_| says("theirs"));
    let theirs = ai_gateway_runtime::run(&someone_else, "claims", input, Some("c1:pay")).await.unwrap();
    assert_eq!(someone_else.sent.load(Ordering::SeqCst), 1, "another user's call with the same key is sent");
    assert_eq!(theirs["one"]["replayed"], false, "never someone else's answer");
}

/// What a call records holds no prompt and no answer: its span's attributes are only those `SPAN_ATTRS` lists, and the
/// rows sent to telemetry carry none of a call's text, sent, replayed or broken (a broken one keeps only what the company
/// said, cleaned).
#[tokio::test]
async fn no_prompt_or_answer_in_insights() {
    const PROMPT: &str = "PROMPT-a-private-thing-4417";
    const ANSWER: &str = "ANSWER-a-private-reply-9921";
    let vault = vec![(Scope::Global, items(&[("everyones", key_item(Company::OpenAi, "e", &["api.openai.com"], &["node"], 1))]))];
    let ok = Stand::new(vault.clone(), Some("notes"), "u1", |_| says(ANSWER));
    let input = json!({ "model": "openai/gpt-x", "messages": [{ "role": "system", "content": PROMPT }, { "role": "user", "content": PROMPT }], "max_completion_tokens": 50, "temperature": 0.2 });
    ai_gateway_runtime::run(&ok, "notes", input.clone(), Some("r1")).await.unwrap();
    ai_gateway_runtime::run(&ok, "notes", input.clone(), Some("r1")).await.unwrap();
    let broke = Stand::new(vault, Some("notes"), "u1", |_| Sent { status: 500, retry_after: None, body: b"{\"error\":\"overloaded\"}".to_vec() });
    let _ = ai_gateway_runtime::run(&broke, "notes", input, None).await;

    let mut batch = insights::Batch::default();
    for stand in [&ok, &broke] {
        for span in stand.spans.lock().unwrap().drain(..) {
            for (key, _) in &span.attrs {
                assert!(::ai_gateway::SPAN_ATTRS.contains(key), "{key} isn't an attribute a GenAI span may have");
            }
            assert!(!span.name.contains(PROMPT) && !span.name.contains(ANSWER));
            batch.gen_ai(1_800_000_000_000, None, "notes", &"0".repeat(64), "u1", span);
        }
    }
    let rows = batch.rows().to_string();
    assert!(rows.contains("gen_ai.usage.input_tokens"), "the call was recorded: {rows}");
    assert!(rows.contains("overloaded"), "a broken call keeps what it said, cleaned: {rows}");
    assert!(!rows.contains(PROMPT), "a prompt reached Insights: {rows}");
    assert!(!rows.contains(ANSWER), "an answer reached Insights: {rows}");
}
