//! What the app runtime does for AI Gateway (`ctx.ai.run`, its tools): resolve the model, pick whose key pays, hold a
//! `retryKey`'s answer, send the call through the Vault's fill and record what it used, as a GenAI span.
//!
//! `Host` is what it needs of the app runtime. It's the seam until the reorganisation's `api::AppRuntime` and the
//! Vault's runtime crate land (R4): then `Host`'s methods become those traits' (`Databases::open`/`write` for `rows`,
//! the Vault's lookup and fill for `key`/`send`, Insights' recorder for `record`) and this crate takes `&Rt`.

mod retry;

use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use ai_gateway::{Asked, Company, Defaults, Failed, Kind, Outgoing, Paid, Usage};
use anyhow::Result;
use serde_json::{Map, Value, json};

pub use retry::SCHEMA;

pub type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Whose rows: the caller's own scope's `node/ai-gateway`, or Everyone's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whose {
    Own,
    Everyone,
}

/// A key that pays: the Vault item (by id, which the request's placeholders name) and whose it is.
#[derive(Clone, Debug)]
pub struct Key {
    pub item: String,
    pub paid: Paid,
}

/// What the company said.
pub struct Sent {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

/// A finished call, for Insights: the span's name, its attributes (only `ai_gateway::SPAN_ATTRS`), how long it took,
/// and how it failed (its code, or what broke, with the message), if it did.
pub struct Span {
    pub name: String,
    pub attrs: Vec<(&'static str, Value)>,
    pub ms: u64,
    pub failed: Option<(Option<&'static str>, String)>,
    pub used: Option<Usage>,
}

/// What AI Gateway needs of the app runtime that runs a call.
pub trait Host: Send + Sync {
    /// The key that pays for `company`'s models for this caller, the most specific there is. Only the network's
    /// (Everyone's Vault) for now; an app's own and the caller's come next, looked up first, in that order.
    fn key(&self, company: Company) -> Fut<'_, Option<Key>>;
    /// Sends `out` with `key`'s fields filled in by the Vault, which sends a secret only to its item's hosts.
    fn send<'a>(&'a self, key: &'a Key, out: Outgoing) -> Fut<'a, Sent>;
    /// Reads `node/ai-gateway` of `whose` (`SCHEMA`), as rows of objects.
    fn query<'a>(&'a self, whose: Whose, sql: &'a str, params: Vec<Value>) -> Fut<'a, Vec<Value>>;
    /// Writes it, in one transaction, which runs again if another device wrote first: what `change` read still holds.
    fn write<'a>(&'a self, whose: Whose, change: Box<dyn FnMut(&rusqlite::Connection) -> Result<Value> + Send + 'a>) -> Fut<'a, Value>;
    /// Records a call's span, a child of the call that asked.
    fn record(&self, span: Span);
}

/// Each kind's default: the caller's own, over Everyone's.
pub async fn defaults(host: &dyn Host) -> Result<(Defaults, Defaults)> {
    let read = |whose| async move {
        let rows = host.query(whose, "select kind, model from defaults", Vec::new()).await?;
        let get = |kind: &str| rows.iter().find(|r| r["kind"] == kind).and_then(|r| r["model"].as_str()).map(str::to_string);
        Ok::<_, anyhow::Error>(Defaults { chat: get("chat"), decide: get("decide") })
    };
    Ok((read(Whose::Own).await?, read(Whose::Everyone).await?))
}

/// `ctx.ai.run(input, { retryKey })` for `app` (or the network's own, for an agent's `ai-gateway_run`).
pub async fn run(host: &dyn Host, app: &str, input: Value, retry_key: Option<&str>) -> Result<Value> {
    let (own, everyone) = defaults(host).await?;
    let asked = ai_gateway::ask(input, &Defaults { chat: own.chat.or(everyone.chat), decide: own.decide.or(everyone.decide) })?;
    let started = Instant::now();
    let span = |paid, used: Option<&Usage>, replayed, failed: Option<&anyhow::Error>| Span {
        name: format!("{} {}", asked.kind.as_str(), asked.route.id()),
        attrs: ai_gateway::span_attrs(&asked, paid, used, replayed),
        ms: started.elapsed().as_millis() as u64,
        failed: failed.map(|e| (e.downcast_ref::<Failed>().and_then(Failed::code), format!("{e:#}"))),
        used: used.cloned(),
    };
    let hash = blake3::hash(asked.canonical().as_bytes()).to_hex().to_string();
    if let Some(key) = retry_key {
        match retry::kept(host, app, key, &hash).await {
            Ok(Some(answer)) => {
                host.record(span(None, None, true, None));
                return Ok(with_one(answer, &asked, None, true));
            }
            Ok(None) => {}
            Err(err) => {
                host.record(span(None, None, false, Some(&err)));
                return Err(err);
            }
        }
    }
    let sent = send(host, &asked, app, retry_key, &hash).await;
    match sent {
        Ok((answer, key)) => {
            let used = ai_gateway::usage(asked.kind, &answer);
            host.record(span(Some(key.paid), Some(&used), false, None));
            let answer = with_one(answer, &asked, Some(key.paid), false);
            if let Some(retry_key) = retry_key {
                retry::done(host, app, retry_key, &answer).await?;
            }
            Ok(answer)
        }
        Err((err, paid)) => {
            host.record(span(paid, None, false, Some(&err)));
            if let Some(retry_key) = retry_key.filter(|_| !err.downcast_ref::<Failed>().is_some_and(|f| *f == Failed::Uncertain)) {
                retry::forget(host, app, retry_key).await?;
            }
            Err(err)
        }
    }
}

/// Picks the key, holds the retryKey as sent, and sends: the answer and the key that paid, or why not and who'd have paid.
async fn send(host: &dyn Host, asked: &Asked, app: &str, retry_key: Option<&str>, hash: &str) -> Result<(Value, Key), (anyhow::Error, Option<Paid>)> {
    let company = asked.route.company;
    let key = match host.key(company).await {
        Ok(Some(key)) => key,
        Ok(None) => return Err((Failed::NoKey(format!("No key pays for {}: add an {} item to Everyone's Vault in Settings › AI Gateway", asked.route.id(), company.key_kind())).into(), None)),
        Err(err) => return Err((err, None)),
    };
    let paid = Some(key.paid);
    let out = asked.request(&key.item).map_err(|e| (e.into(), paid))?;
    if let Some(retry_key) = retry_key {
        retry::sending(host, app, retry_key, hash).await.map_err(|e| (e, paid))?;
    }
    let sent = host.send(&key, out).await.map_err(|e| (e, paid))?;
    let answer = asked.answer(sent.status, sent.retry_after, &sent.body).map_err(|e| (e.into(), paid))?;
    Ok((answer, key))
}

/// The answer with `one`: the model that answered, where it ran, whose key paid and whether it's a replay.
fn with_one(mut answer: Value, asked: &Asked, paid: Option<Paid>, replayed: bool) -> Value {
    let ran = if asked.route.company == Company::WorkersAi { "workers-ai" } else { "hosted" };
    // A replay says who paid for the first call, kept with it.
    let paid = paid.map(|p| json!(p.as_str())).unwrap_or_else(|| answer["one"]["paid"].clone());
    answer["one"] = json!({ "model": asked.route.id(), "ran": ran, "paid": paid, "replayed": replayed });
    answer
}

/// `ai-gateway_models`: each company, whether a key pays for it and whose, and each kind's default and whose it is.
pub async fn models(host: &dyn Host) -> Result<Value> {
    let mut companies = Vec::new();
    for company in ai_gateway::route::ALL {
        let key = host.key(company).await?;
        let kind = if company == Company::WorkersAi { "chat, decide" } else { "chat" };
        companies.push(json!({ "id": company.id(), "title": company.title(), "kind": kind, "paid": key.map(|k| k.paid.as_str()), "key_kind": company.key_kind(), "host": company.host() }));
    }
    let (own, everyone) = defaults(host).await?;
    let default = |kind: Kind, own: &Option<String>, everyone: &Option<String>| match (own, everyone) {
        (Some(m), _) => json!({ "model": m, "whose": "yours" }),
        (None, Some(m)) => json!({ "model": m, "whose": "everyone" }),
        (None, None) => json!({ "model": Defaults::default().model(kind), "whose": "built_in" }),
    };
    Ok(json!({
        "companies": companies,
        "defaults": { "chat": default(Kind::Chat, &own.chat, &everyone.chat), "decide": default(Kind::Decide, &own.decide, &everyone.decide) },
        "everyone": { "chat": everyone.chat, "decide": everyone.decide },
    }))
}

/// `ai-gateway_set`: the default chat and decide models, the caller's own or (`admin` only) Everyone's.
pub async fn set(host: &dyn Host, input: &Map<String, Value>, admin: bool) -> Result<Value> {
    let everyone = input.get("everyone").and_then(Value::as_bool).unwrap_or(false);
    anyhow::ensure!(!everyone || admin, Failed::Refused("only an admin sets Everyone's default models".into()));
    let mut changes = Vec::new();
    for kind in [Kind::Chat, Kind::Decide] {
        let Some(model) = input.get(kind.as_str()).and_then(Value::as_str) else { continue };
        if !model.is_empty() {
            let route = ai_gateway::Route::parse(model)?;
            anyhow::ensure!(route.kind() == kind, Failed::Refused(format!("{model} is a {} model, not a {} one", route.kind().as_str(), kind.as_str())));
        }
        changes.push((kind.as_str(), model.to_string()));
    }
    anyhow::ensure!(!changes.is_empty(), Failed::Refused("give chat or decide: a model id, or empty to clear it".into()));
    let whose = if everyone { Whose::Everyone } else { Whose::Own };
    let said: Vec<String> = changes.iter().map(|(k, m)| if m.is_empty() { format!("{k} cleared") } else { format!("{k} is {m}") }).collect();
    host.write(
        whose,
        Box::new(move |c| {
            for (kind, model) in &changes {
                match model.is_empty() {
                    true => c.execute("delete from defaults where kind = ?1", [kind])?,
                    false => c.execute("insert or replace into defaults (kind, model) values (?1, ?2)", [*kind, model.as_str()])?,
                };
            }
            Ok(Value::Null)
        }),
    )
    .await?;
    Ok(json!({ "said": format!("{}'s default: {}", if everyone { "Everyone" } else { "Your" }, said.join(", ")) }))
}
