//! A call to AI Gateway, wherever it runs: resolve the model, pick whose key pays, hold a `retryKey`'s answer, send it
//! and record what it used, as a GenAI span. `Host` is what runs it: the app runtime on a machine (`ai-gateway-runtime`)
//! or oned on Cloudflare, each sending through its own fill and keeping its own rows.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::{Map, Value, json};

use crate::retry;
use crate::{Asked, Company, Defaults, Failed, Kind, Outgoing, Paid, Span, Usage};

/// What `Host` gives back, in time: sent between threads on a machine, and not on oned on Cloudflare, whose futures are
/// JavaScript's promises.
#[cfg(not(target_family = "wasm"))]
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
#[cfg(target_family = "wasm")]
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + 'a>>;

/// What a host must be to be shared: across threads on a machine; on oned on Cloudflare, nothing.
#[cfg(not(target_family = "wasm"))]
pub trait Shared: Send + Sync {}
#[cfg(not(target_family = "wasm"))]
impl<T: Send + Sync> Shared for T {}
#[cfg(target_family = "wasm")]
pub trait Shared {}
#[cfg(target_family = "wasm")]
impl<T> Shared for T {}

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

/// What AI Gateway needs of the app runtime that runs a call.
pub trait Host: Shared {
    /// The key that pays for `company`'s models for this caller, the most specific there is: the app's own, then the
    /// caller's, then the network's (Everyone's).
    fn key(&self, company: Company) -> Fut<'_, Option<Key>>;
    /// Sends `out` with `key`'s fields filled in by the Vault, which sends a secret only to its item's hosts.
    fn send<'a>(&'a self, key: &'a Key, out: Outgoing) -> Fut<'a, Sent>;
    /// Reads `node/ai-gateway` of `whose` (`SCHEMA`), as rows of objects.
    fn query<'a>(&'a self, whose: Whose, sql: &'a str, params: Vec<Value>) -> Fut<'a, Vec<Value>>;
    /// Writes it, in one transaction, which runs again if another device wrote first: what `change` read still holds.
    fn write<'a>(&'a self, whose: Whose, change: Box<dyn FnMut(&rusqlite::Connection) -> Result<Value> + Send + 'a>) -> Fut<'a, Value>;
    /// Asks a model on one of the caller's devices: the function `kind` (`chat`, `decide`) of a service that declares
    /// `ai`, with `input` (the model's own name in `model`), on whichever device has it. `Failed::Unavailable` when none
    /// that's on does.
    fn device<'a>(&'a self, kind: Kind, input: Value) -> Fut<'a, Value>;
    /// Every model on the caller's devices, as those services' `here` says, by device.
    fn here(&self) -> Fut<'_, Value>;
    /// Records a call's span, a child of the call that asked.
    fn record(&self, span: Span);
    /// Now, in ms since 1970.
    fn now(&self) -> u64;
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
    let asked = crate::ask(input, &Defaults { chat: own.chat.or(everyone.chat), decide: own.decide.or(everyone.decide) })?;
    let started = host.now();
    let span = |paid, used: Option<&Usage>, replayed, failed: Option<&anyhow::Error>| Span {
        name: format!("{} {}", asked.kind.as_str(), asked.route.id()),
        attrs: crate::span_attrs(&asked, paid, used, replayed),
        ms: host.now().saturating_sub(started),
        failed: failed.map(|e| (e.downcast_ref::<Failed>().and_then(Failed::code), format!("{e:#}"))),
        used: used.cloned(),
    };
    let hash = blake3::hash(asked.canonical().as_bytes()).to_hex().to_string();
    if let Some(key) = retry_key {
        match retry::kept(host, app, key, &hash, host.now()).await {
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
        Ok((answer, paid)) => {
            let used = crate::usage(asked.kind, &answer);
            host.record(span(Some(paid), Some(&used), false, None));
            let answer = with_one(answer, &asked, Some(paid), false);
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

/// Picks who pays, holds the retryKey as sent, and sends: the answer and who paid, or why not and who'd have paid. A
/// device's model is paid by no key.
async fn send(host: &dyn Host, asked: &Asked, app: &str, retry_key: Option<&str>, hash: &str) -> Result<(Value, Paid), (anyhow::Error, Option<Paid>)> {
    let company = asked.route.company;
    if company == Company::Local {
        let paid = Some(Paid::Device);
        if let Some(retry_key) = retry_key {
            retry::sending(host, app, retry_key, hash).await.map_err(|e| (e, paid))?;
        }
        let answer = host.device(asked.kind, asked.on_device()).await.map_err(|e| (e, paid))?;
        return Ok((asked.fits(answer).map_err(|e| (e.into(), paid))?, Paid::Device));
    }
    let key = match host.key(company).await {
        Ok(Some(key)) => key,
        Ok(None) => return Err((Failed::NoKey(format!("No key pays for {}: add an {} item in Settings › AI Gateway (Everyone's), to your own Vault, or switched on for the app", asked.route.id(), company.key_kind())).into(), None)),
        Err(err) => return Err((err, None)),
    };
    let paid = Some(key.paid);
    let out = asked.request(&key.item).map_err(|e| (e.into(), paid))?;
    if let Some(retry_key) = retry_key {
        retry::sending(host, app, retry_key, hash).await.map_err(|e| (e, paid))?;
    }
    let sent = host.send(&key, out).await.map_err(|e| (e, paid))?;
    let answer = asked.answer(sent.status, sent.retry_after, &sent.body).map_err(|e| (e.into(), paid))?;
    Ok((answer, key.paid))
}

/// The answer with `one`: the model that answered, where it ran, whose key paid and whether it's a replay.
fn with_one(mut answer: Value, asked: &Asked, paid: Option<Paid>, replayed: bool) -> Value {
    let ran = match asked.route.company {
        Company::WorkersAi => "workers-ai",
        Company::Local => "device",
        _ => "hosted",
    };
    // A replay says who paid for the first call, kept with it.
    let paid = paid.map(|p| json!(p.as_str())).unwrap_or_else(|| answer["one"]["paid"].clone());
    answer["one"] = json!({ "model": asked.route.id(), "ran": ran, "paid": paid, "replayed": replayed });
    answer
}

/// `ai-gateway_models`: each company, whether a key pays for it and whose, and each kind's default and whose it is.
pub async fn models(host: &dyn Host) -> Result<Value> {
    let mut companies = Vec::new();
    for company in crate::route::ALL {
        let key = host.key(company).await?;
        let kind = if company == Company::WorkersAi { "chat, decide" } else { "chat" };
        companies.push(json!({ "id": company.id(), "title": company.title(), "kind": kind, "paid": key.map(|k| k.paid.as_str()), "key_kind": company.key_kind(), "host": company.host() }));
    }
    // Off or with no model, a device lists none: the rest still are.
    let devices = host.here().await.unwrap_or(Value::Null);
    let (own, everyone) = defaults(host).await?;
    let default = |kind: Kind, own: &Option<String>, everyone: &Option<String>| match (own, everyone) {
        (Some(m), _) => json!({ "model": m, "whose": "yours" }),
        (None, Some(m)) => json!({ "model": m, "whose": "everyone" }),
        (None, None) => json!({ "model": Defaults::default().model(kind), "whose": "built_in" }),
    };
    Ok(json!({
        "companies": companies,
        "devices": devices,
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
            let route = crate::Route::parse(model)?;
            if let Some(its) = route.kind().filter(|k| *k != kind) {
                anyhow::bail!(Failed::Refused(format!("{model} is a {} model, not a {} one", its.as_str(), kind.as_str())));
            }
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
