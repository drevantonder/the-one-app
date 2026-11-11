//! AI Gateway, as an app runtime runs it: `ctx.ai.run` for an app's call (`run`) and its tools, as their `Host`
//! (ai-gateway/src/call.rs) through `api::AppRuntime`. The key that pays is one of the Vault's items the call reached,
//! sent through the Vault's fill; its rows are the caller's own `node/ai-gateway` and Everyone's; a device's model is
//! asked of a service that declares `ai`; each call's span goes to Insights.

mod tools;

use std::time::Duration;

use ai_gateway::{Company, Fut, Host, Key, Outgoing, Sent, Span, Whose};
use anyhow::{Context, Result, anyhow, bail, ensure};
use api::call::CallRequest;
use api::runtime::Runs;
use api::runtime::prelude::*;
use api::{Caller, trace};
use scope::scopes::Scope;
use serde_json::{Map, Value, json};

const AI_GATEWAY: (&str, &str) = (key::APP, key::CoreDb::AiGateway.name());

pub fn runs() -> Runs {
    Runs { tools: tools::served(), ..Runs::named(ai_gateway::NAME) }
}

/// `ctx.ai.run(input, { retryKey })` for a call of `app`'s by `caller`, reaching `scopes`, in `trace`.
pub async fn run(rt: &Rt, caller: &Caller, app: &str, scopes: &[Scope], trace: Option<&trace::Trace>, input: Value, retry_key: Option<&str>) -> Result<Value> {
    let call = Call::new(rt, caller, Some(app), scopes.to_vec(), trace);
    ai_gateway::run(&call, app, input, retry_key).await.map_err(coded)
}

/// A call to AI Gateway, by `caller`, for `app` (none for the network's own, its tools), reaching `scopes`, in `trace`.
struct Call<'a> {
    rt: &'a Rt,
    caller: &'a Caller,
    app: Option<&'a str>,
    scopes: Vec<Scope>,
    trace: Option<&'a trace::Trace>,
    /// The Vault's items in `scopes`, read once for the call.
    items: tokio::sync::OnceCell<Vec<(Scope, vault::Items)>>,
}

impl<'a> Call<'a> {
    /// A call's: the scopes it reached, and always its user's own and Everyone's, whose keys pay for any app's call.
    fn new(rt: &'a Rt, caller: &'a Caller, app: Option<&'a str>, mut scopes: Vec<Scope>, trace: Option<&'a trace::Trace>) -> Call<'a> {
        for scope in [Scope::User(caller.user.clone()), Scope::Global] {
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        Call { rt, caller, app, scopes, trace, items: tokio::sync::OnceCell::new() }
    }

    fn scope(&self, whose: Whose) -> Scope {
        match whose {
            Whose::Own => Scope::User(self.caller.user.clone()),
            Whose::Everyone => Scope::Global,
        }
    }

    async fn items(&self) -> Result<&Vec<(Scope, vault::Items)>> {
        self.items.get_or_try_init(|| async { vault_runtime::items_of(self.rt, self.caller, &self.scopes).await }).await
    }

    /// The apps whose service declares `ai` (`service: { ai: true }`): each answers `here`, `chat` and `decide` with the
    /// models on its devices, in AI Gateway's shapes.
    async fn ai_services(&self) -> Vec<String> {
        let apps: Vec<String> = self.rt.read_or(Vec::new(), |r| Ok(r.db().apps()?.into_iter().filter(|a| a.deleted.is_none() && !a.erased).map(|a| a.name).collect()));
        let mut out = Vec::new();
        for app in apps {
            if self.rt.describe_any(&app).await.is_ok_and(|d| d["service"]["ai"] == true) {
                out.push(app);
            }
        }
        out
    }

    /// The function `function` of the AI service `service`, on `device` (none for whichever has it).
    fn asked(&self, service: &str, function: &str, input: Value, device: Option<&str>) -> CallRequest {
        CallRequest { app: service.into(), service: Some(service.into()), function: function.into(), input, device: device.map(str::to_string), depth: 1, client: Some("app".into()), ..Default::default() }
    }
}

impl Host for Call<'_> {
    fn key(&self, company: Company) -> Fut<'_, Option<Key>> {
        Box::pin(async move { Ok(ai_gateway::pays(self.items().await?, self.app, &self.caller.user, company).map(|(item, paid)| Key { item, paid })) })
    }

    fn send<'a>(&'a self, key: &'a Key, out: Outgoing) -> Fut<'a, Sent> {
        Box::pin(async move {
            let item = self.items().await?.iter().find_map(|(_, items)| items.get(&key.item)).context("that AI key left the Vault as the call began: ask again")?;
            let uri: ureq::http::Uri = out.url.parse().context("an AI request's address isn't a web address")?;
            let host = uri.host().unwrap_or_default().to_string();
            ensure!(!item.fields.values().any(|f| f.secret) || item.opened, "the {} that pays is opened only by the devices its Vault's admins grant, and this one isn't: grant it with one vault grant <this device>", item.kind);
            let fill = |text: &str| vault::fill(text, &host, |name| name.strip_prefix(&format!("{}:", key.item)).and_then(|field| vault::filled(item, field)));
            let url = fill(&out.url)?;
            let headers = out.headers.iter().map(|(n, v)| Ok((n.clone(), fill(v)?))).collect::<Result<Vec<_>>>()?;
            let ports = self.rt.own_ports();
            tokio::task::spawn_blocking(move || post(&url, headers, out.body, ports)).await?
        })
    }

    fn query<'a>(&'a self, whose: Whose, sql: &'a str, params: Vec<Value>) -> Fut<'a, Vec<Value>> {
        Box::pin(async move {
            let db = self.rt.open(self.caller, &self.scope(whose), AI_GATEWAY.0, AI_GATEWAY.1, ai_gateway::SCHEMA).await?;
            let rows = db.lock().await.query(sql, &params)?;
            Ok(rows.into_iter().map(Value::Object).collect())
        })
    }

    fn write<'a>(&'a self, whose: Whose, mut change: Box<dyn FnMut(&rusqlite::Connection) -> Result<Value> + Send + 'a>) -> Fut<'a, Value> {
        Box::pin(async move {
            let mut out = Value::Null;
            self.rt
                .write(self.caller, &self.scope(whose), AI_GATEWAY.0, AI_GATEWAY.1, ai_gateway::SCHEMA, &mut |c| {
                    out = change(c)?;
                    Ok(())
                })
                .await?;
            Ok(out)
        })
    }

    fn device<'a>(&'a self, kind: ai_gateway::Kind, input: Value) -> Fut<'a, Value> {
        Box::pin(async move {
            let model = input["model"].as_str().unwrap_or_default().to_string();
            let services = self.ai_services().await;
            if services.is_empty() {
                bail!(ai_gateway::Failed::Unavailable(format!("local/{model} is a model on your devices, and none runs a service for them: switch Models on for one in Settings › Devices")));
            }
            let mut why = Vec::new();
            for service in &services {
                match self.rt.call(self.asked(service, kind.as_str(), input.clone(), None), self.caller.clone()).await {
                    Ok(answer) => return Ok(answer),
                    // No device that's on has it: the next service might.
                    Err(err) if err.is::<api::runtime::NoneCould>() => why.push(format!("{err:#}")),
                    Err(err) => return Err(err),
                }
            }
            bail!(ai_gateway::Failed::Unavailable(format!("none of your devices that are on has local/{model}: {}", why.join("; "))))
        })
    }

    fn here(&self) -> Fut<'_, Value> {
        Box::pin(async move {
            let mut out = Map::new();
            for service in self.ai_services().await {
                let asked = self.rt.call(self.asked(&service, "here", Value::Null, Some("all")), self.caller.clone()).await;
                out.insert(service.clone(), asked.unwrap_or_else(|err| json!({ "error": format!("{err:#}") })));
            }
            Ok(Value::Object(out))
        })
    }

    fn record(&self, span: Span) {
        insights_runtime::record_gen_ai(self.rt, self.trace, self.app.unwrap_or(insights::own()), &self.caller.user, span);
    }

    fn now(&self) -> u64 {
        custom_build::now_ms()
    }
}

/// Posts a filled AI request, to anywhere but this machine's own `ports`, waiting as long as a call may.
fn post(url: &str, headers: Vec<(String, String)>, body: Vec<u8>, ports: Vec<u16>) -> Result<Sent> {
    let config = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).timeout_global(Some(Duration::from_secs(ai_gateway::MOST_SECONDS)));
    let agent = platform::reach::agent(config, platform::reach::not_ports(ports));
    // Its key is in the headers or URL, so no error says either: only where it went.
    let uri: ureq::http::Uri = url.parse().context("an AI request's address isn't a web address")?;
    let host = uri.host().unwrap_or_default().to_string();
    let mut builder = ureq::http::Request::builder().method("POST").uri(uri);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let mut resp = agent.run(builder.body(body)?).map_err(|err| match err {
        ureq::Error::Timeout(_) => anyhow::Error::from(ai_gateway::Failed::Broke(format!("{host} took longer than {} minutes", ai_gateway::MOST_SECONDS / 60))),
        _ => anyhow!("couldn't reach {host}"),
    })?;
    let retry_after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
    let body = resp.body_mut().with_config().limit(ai_gateway::MOST_BYTES as u64 * 2).read_to_vec()?;
    Ok(Sent { status: resp.status().as_u16(), retry_after, body })
}

/// What the shim's `ai.run` throws, and a tool fails with: AI Gateway's failures with their code.
fn coded(err: anyhow::Error) -> anyhow::Error {
    match err.downcast_ref::<ai_gateway::Failed>() {
        Some(f) => match f.code() {
            Some(code) => custom_build::protocol::Coded::error(code, f),
            None => anyhow!("{f}"),
        },
        None => err,
    }
}
