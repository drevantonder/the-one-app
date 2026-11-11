//! What this node gives AI Gateway's runtime (`ai_gateway_runtime::Host`) for a call: the key that pays
//! (`crate::ai_gateway::pays`), the Vault's fill, the caller's own `node/ai-gateway` and Everyone's, and Insights. The
//! seam until R4's `api::AppRuntime`.

use std::time::Duration;

use ::ai_gateway::{Company, Outgoing};
use ai_gateway_runtime::{Fut, Host, Key, Sent, Span, Whose};
use serde_json::Map;

use super::*;

const AI_GATEWAY: (&str, &str) = (key::APP, key::CoreDb::AiGateway.name());

/// A call to AI Gateway, by `caller`, for `app` (none for the network's own, its tools), reaching `scopes`, in `trace`.
pub(super) struct Gateway<'a> {
    pub node: &'a Node,
    pub caller: &'a Caller,
    pub app: Option<&'a str>,
    pub scopes: Vec<Scope>,
    pub trace: Option<&'a trace::Trace>,
    /// The Vault's items in `scopes`, read once for the call.
    items: tokio::sync::OnceCell<Vec<(Scope, ::vault::Items)>>,
}

impl<'a> Gateway<'a> {
    /// A call's: the scopes it reached, and always its user's own and Everyone's, whose keys pay for any app's call.
    fn new(node: &'a Node, caller: &'a Caller, app: Option<&'a str>, mut scopes: Vec<Scope>, trace: Option<&'a trace::Trace>) -> Gateway<'a> {
        for scope in [Scope::User(caller.user.clone()), Scope::Global] {
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        Gateway { node, caller, app, scopes, trace, items: tokio::sync::OnceCell::new() }
    }

    fn service(&self) -> &str {
        self.app.unwrap_or(::insights::own())
    }

    fn scope(&self, whose: Whose) -> Scope {
        match whose {
            Whose::Own => Scope::User(self.caller.user.clone()),
            Whose::Everyone => Scope::Global,
        }
    }

    async fn items(&self) -> Result<&Vec<(Scope, ::vault::Items)>> {
        self.items.get_or_try_init(|| async { vault_runtime::items_of(&self.node.rt(), self.caller, &self.scopes).await }).await
    }

    async fn db(&self, scope: &Scope) -> Result<(Arc<tokio::sync::Mutex<Db>>, Copied)> {
        let (db, bucket) = self.node.call_database(self.caller, scope, AI_GATEWAY.0, AI_GATEWAY.1, Some(ai_gateway_runtime::SCHEMA.iter().map(|s| s.to_string()).collect())).await?;
        self.node.read_ready(&(scope.clone(), format!("{}/{}", AI_GATEWAY.0, AI_GATEWAY.1)), &mut *db.lock().await, &bucket).await?;
        Ok((db, bucket))
    }

}

impl Host for Gateway<'_> {
    fn key(&self, company: Company) -> Fut<'_, Option<Key>> {
        Box::pin(async move { Ok(crate::ai_gateway::pays(self.items().await?, self.app, &self.caller.user, company).map(|(item, paid)| Key { item, paid })) })
    }

    fn send<'a>(&'a self, key: &'a Key, out: Outgoing) -> Fut<'a, Sent> {
        Box::pin(async move {
            let item = self.items().await?.iter().find_map(|(_, items)| items.get(&key.item)).context("that AI key left the Vault as the call began: ask again")?;
            let uri: ureq::http::Uri = out.url.parse().context("an AI request's address isn't a web address")?;
            let host = uri.host().unwrap_or_default().to_string();
            ensure!(!item.fields.values().any(|f| f.secret) || item.opened, "the {} that pays is opened only by the devices its Vault's admins grant, and this one isn't: grant it with one vault grant <this device>", item.kind);
            let fill = |text: &str| ::vault::fill(text, &host, |name| name.strip_prefix(&format!("{}:", key.item)).and_then(|field| ::vault::filled(item, field)));
            let url = fill(&out.url)?;
            let headers = out.headers.iter().map(|(n, v)| Ok((n.clone(), fill(v)?))).collect::<Result<Vec<_>>>()?;
            let ports = self.node.own_ports();
            tokio::task::spawn_blocking(move || post(&url, headers, out.body, ports)).await?
        })
    }

    fn query<'a>(&'a self, whose: Whose, sql: &'a str, params: Vec<Value>) -> Fut<'a, Vec<Value>> {
        Box::pin(async move {
            let (db, _) = self.db(&self.scope(whose)).await?;
            let rows = db.lock().await.query(sql, &params)?;
            Ok(rows.into_iter().map(Value::Object).collect())
        })
    }

    fn write<'a>(&'a self, whose: Whose, mut change: Box<dyn FnMut(&rusqlite::Connection) -> Result<Value> + Send + 'a>) -> Fut<'a, Value> {
        Box::pin(async move {
            let scope = self.scope(whose);
            let (db, bucket) = self.db(&scope).await?;
            let mut db = db.lock().await;
            let out = self.node.write_in(&scope, &mut db, &bucket, |c| change(c)).await?;
            self.node.ping(&scope, Some((AI_GATEWAY.0, AI_GATEWAY.1, db.at())));
            Ok(out)
        })
    }

    fn device<'a>(&'a self, kind: ::ai_gateway::Kind, input: Value) -> Fut<'a, Value> {
        Box::pin(async move {
            let model = input["model"].as_str().unwrap_or_default().to_string();
            let services = self.node.ai_services().await;
            if services.is_empty() {
                bail!(::ai_gateway::Failed::Unavailable(format!("local/{model} is a model on your devices, and none runs a service for them: switch Models on for one in Settings › Devices")));
            }
            let mut why = Vec::new();
            for service in &services {
                let call = self.node.resolve(CallRequest { app: service.clone(), service: Some(service.clone()), function: kind.as_str().into(), input: input.clone(), depth: 1, client: Some("app".into()), ..Default::default() });
                match self.node.route(call, self.caller.clone()).await {
                    Ok(answer) => return Ok(answer),
                    // No device that's on has it: the next service might.
                    Err(err) if err.is::<::api::runtime::NoneCould>() => why.push(format!("{err:#}")),
                    Err(err) => return Err(err),
                }
            }
            bail!(::ai_gateway::Failed::Unavailable(format!("none of your devices that are on has local/{model}: {}", why.join("; "))))
        })
    }

    fn here(&self) -> Fut<'_, Value> {
        Box::pin(async move {
            let mut out = Map::new();
            for service in self.node.ai_services().await {
                let call = self.node.resolve(CallRequest { app: service.clone(), service: Some(service.clone()), function: "here".into(), device: Some("all".into()), depth: 1, client: Some("app".into()), ..Default::default() });
                out.insert(service.clone(), self.node.route(call, self.caller.clone()).await.unwrap_or_else(|err| json!({ "error": format!("{err:#}") })));
            }
            Ok(Value::Object(out))
        })
    }

    fn record(&self, span: Span) {
        insights_runtime::record_gen_ai(&self.node.rt(), self.trace, self.service(), &self.caller.user, span);
    }

    fn now(&self) -> u64 {
        now_ms()
    }
}

/// Posts a filled AI request, to anywhere but this machine's own `ports`, waiting as long as a call may.
fn post(url: &str, headers: Vec<(String, String)>, body: Vec<u8>, ports: Vec<u16>) -> Result<Sent> {
    let config = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).timeout_global(Some(Duration::from_secs(::ai_gateway::MOST_SECONDS)));
    let agent = platform::reach::agent(config, platform::reach::not_ports(ports));
    // Its key is in the headers or URL, so no error says either: only where it went.
    let uri: ureq::http::Uri = url.parse().context("an AI request's address isn't a web address")?;
    let host = uri.host().unwrap_or_default().to_string();
    let mut builder = ureq::http::Request::builder().method("POST").uri(uri);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let mut resp = agent.run(builder.body(body)?).map_err(|err| match err {
        ureq::Error::Timeout(_) => anyhow::Error::from(::ai_gateway::Failed::Broke(format!("{host} took longer than {} minutes", ::ai_gateway::MOST_SECONDS / 60))),
        _ => anyhow!("couldn't reach {host}"),
    })?;
    let retry_after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
    let body = resp.body_mut().with_config().limit(::ai_gateway::MOST_BYTES as u64 * 2).read_to_vec()?;
    Ok(Sent { status: resp.status().as_u16(), retry_after, body })
}

/// What the shim's `ai.run` throws: AI Gateway's failures with their code.
fn coded(err: anyhow::Error) -> anyhow::Error {
    match err.downcast_ref::<::ai_gateway::Failed>() {
        Some(f) => match f.code() {
            Some(code) => custom_build::protocol::Coded::error(code, f),
            None => anyhow!("{f}"),
        },
        None => err,
    }
}

impl Node {
    /// The apps whose service declares `ai` (`service: { ai: true }`): each answers `here`, `chat` and `decide` with the
    /// models on its devices, in AI Gateway's shapes.
    async fn ai_services(&self) -> Vec<String> {
        let apps: Vec<String> = self.with_or(Vec::new(), |i| Ok(i.r.db().apps()?.into_iter().filter(|a| a.deleted.is_none() && !a.erased).map(|a| a.name).collect()));
        let mut out = Vec::new();
        for app in apps {
            if self.described(&app).await.is_ok_and(|d| d["service"]["ai"] == true) {
                out.push(app);
            }
        }
        out
    }

    /// `ctx.ai.run(input, { retryKey })` for a call of `app`'s.
    pub(super) async fn ai_run(&self, caller: &Caller, app: &str, scopes: &[Scope], trace: Option<&trace::Trace>, input: Value, retry_key: Option<&str>) -> Result<Value> {
        let gateway = Gateway::new(self, caller, Some(app), scopes.to_vec(), trace);
        ai_gateway_runtime::run(&gateway, app, input, retry_key).await.map_err(coded)
    }

    /// AI Gateway's tools, as `caller`: the network's own calls, from no app.
    pub(crate) async fn ai_tool(&self, caller: &Caller, tool: &str, mut input: Map<String, Value>) -> Result<Value> {
        let current = trace::current();
        let gateway = Gateway::new(self, caller, None, Vec::new(), current.as_ref());
        let answer = match tool {
            "run" => {
                let retry_key = input.remove("retry_key").and_then(|k| k.as_str().map(str::to_string));
                ai_gateway_runtime::run(&gateway, ::insights::own(), Value::Object(input), retry_key.as_deref()).await
            }
            "models" => ai_gateway_runtime::models(&gateway).await,
            "set" => {
                let admin = self.with(|i| Ok(i.r.db().may(&caller.user, "change-settings", scopes::NETWORK)))?;
                ai_gateway_runtime::set(&gateway, &input, admin).await
            }
            other => Err(anyhow!("AI Gateway has no tool {other}")),
        };
        answer.map_err(coded)
    }
}
