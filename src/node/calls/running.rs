//! What a running call's app asks the node: its grant, its databases and the node's internal API, its prompt hook, a
//! service's call, and reading this device's logs.

use ::custom_build::allowed;
use super::*;

impl Node {
    /// Fills in an app's call of `app`'s service (`from` the app that made it): the fields of the items it's switched on
    /// for, a secret one only for a service named among its item's hosts. → the secrets it filled, to blot out of the
    /// answer. The node that runs the call fills it: a service endpoint fills nothing itself.
    pub async fn fill_for_service(&self, app: &str, input: &mut Value, me: &Caller, from: Option<&str>) -> Result<Vec<String>> {
        let used = std::sync::Mutex::new(Vec::<String>::new());
        if let Some(from) = from.filter(|_| input.to_string().contains("{{")) {
            let scopes = self.with(|i| Ok(reachable(i, from, &me.user)?.into_iter().filter(|s| i.r.holds(s)).collect::<Vec<_>>()))?;
            let lookup = vault_runtime::lookup(&self.rt(), me, from, &scopes).await?;
            lookup.refuse_shut(&input.to_string())?;
            ::vault::fill_value(input, app, &|name| {
                let found = lookup.get(name);
                if let Some((hosts, value)) = &found
                    && !hosts.is_empty()
                {
                    used.lock().unwrap().push(value.clone());
                }
                found
            })?;
        }
        Ok(used.into_inner().unwrap())
    }

    /// Reads the log of the device a call names (this node's own, or its device's, or one of its services'; or a client's, through the
    /// connection it holds to this node, else `Elsewhere`: another node may hold it), for a user over the network
    /// (`one logs`): the one tool of the node's own a call asks.
    pub(in crate::node) async fn read_logs(&self, req: &CallRequest, input: &Value, me: &Caller) -> Result<Reply> {
        ensure!(req.tool == "logs", "the node has no tool {}", req.tool);
        let target = match &req.device {
            Some(name) => self.with(|i| i.r.db().find_machine(name))?,
            None => self.0.id,
        };
        let name = self.name_of(&target);
        allowed!(self.may_read_logs_of(&me.user, &target)?, "only {name}'s user, or an admin of {}, reads its logs", ::custom_build::config::YOURS);
        // A device whose app runtime this is keeps its logs here: its onet's and oned's beside the runtime's own.
        if target == self.0.id || target == self.0.holds.device() {
            return Ok(Reply::Ok(::device_logs::read(&self.0.dir, &::device_logs::Ask::from_json(input)?)?));
        }
        ensure!(self.with(|i| Ok(i.r.db().client(&target)?.is_some()))?, "{name} isn't a client, and a node reads its own log");
        match self.ask_client(&target, input).await? {
            Some(answer) => Ok(match answer.get("error") {
                Some(e) => Reply::error(e.as_str().unwrap_or("it failed")),
                None => Reply::Ok(answer["ok"].clone()),
            }),
            None => Ok(Reply::Elsewhere(format!("{name} isn't connected to {}: its log is read through a node it's connected to, while its app is open", self.device_name()))),
        }
    }

    /// Whether `user` may read this device's logs: whoever may remove it from the network, its user and the network's
    /// admins (who may remove any device), since they could stop it anyway. Nothing in a log is a key or someone's data.
    pub fn may_read_logs(&self, user: &str) -> Result<bool> {
        self.may_read_logs_of(user, &self.0.id)
    }

    fn may_read_logs_of(&self, user: &str, device: &Key) -> Result<bool> {
        self.with(|i| Ok(i.r.db().may(user, "remove", &scopes::device(device))))
    }

    /// Lets the requests of a call, widget or hook of `app` through the node as `caller`'s, while it runs.
    pub(in crate::node) fn grant(&self, app: &str, caller: &Caller, ctx: &mut Context, depth: u32) -> Granted {
        let token = hex(&rand::random::<[u8; 32]>());
        let scopes = ctx.scopes.iter().filter_map(|s| Scope::parse(&s.id).ok()).collect();
        let grant = Grant { app: app.to_string(), caller: caller.clone(), scopes, read_only: ctx.read_only, puts: Vec::new(), caught_up: Vec::new(), depth, from: ctx.from.clone(), trace: trace::current() };
        self.0.grants.lock().unwrap().insert(token.clone(), grant);
        ctx.token = token.clone();
        Granted(self.clone(), token)
    }

    /// The ports this node serves on this machine: its local API, workerd, and apps' outbound requests.
    pub fn own_ports(&self) -> Vec<u16> {
        vec![::api::local::port(), self.0.apps.port(), self.0.outbound]
    }

    pub fn grant_of(&self, token: &str) -> Option<Grant> {
        self.0.grants.lock().unwrap().get(token).cloned()
    }

    /// What every app added for this machine's user says with an agent's prompt here: each app's `prompt` hook,
    /// asked at once, their answers joined. The node remembers, per app and session, when it last asked, and tells
    /// the app as `since` (null on a session's first prompt). An app that fails, says nothing or takes longer than
    /// `PROMPT_LIMIT` (the harness waits for this) is left out.
    pub async fn prompt(&self, event: Value) -> Result<String> {
        let session = event["session_id"].as_str().unwrap_or("").to_string();
        let caller = self.caller()?;
        let path = self.0.dir.join("hooks.json");
        let mut seen: serde_json::Map<String, Value> = platform::files::read_json(&path);
        let now = now_ms();
        let mut asked = tokio::task::JoinSet::new();
        for (app, ctx, granted) in self.page_contexts(&caller, None)? {
            let key = format!("{app}/{session}");
            let since = seen.get(&key).cloned().unwrap_or(Value::Null);
            let (node, event) = (self.clone(), event.clone());
            asked.spawn(async move {
                let _granted = granted;
                let has = node.0.apps.describe(&app).await.is_ok_and(|d| d["hooks"].as_array().is_some_and(|h| h.iter().any(|n| n == "prompt")));
                if !has {
                    return None;
                }
                let said = node.0.apps.hook(&app, "prompt", json!({ "event": event, "since": since }), &ctx);
                let text = match tokio::time::timeout(PROMPT_LIMIT, said).await {
                    Ok(Ok(Value::String(text))) => text.trim().to_string(),
                    Ok(Ok(_)) => String::new(),
                    Ok(Err(err)) => {
                        tracing::warn!("{app}'s prompt hook: {err:#}");
                        return None;
                    }
                    Err(_) => {
                        tracing::warn!("{app}'s prompt hook took longer than {}s; left out", PROMPT_LIMIT.as_secs());
                        return None;
                    }
                };
                Some((app, key, text))
            });
        }
        let mut said: Vec<(String, String, String)> = asked.join_all().await.into_iter().flatten().collect();
        said.sort();
        let mut out = Vec::new();
        for (_, key, text) in said {
            seen.insert(key, json!(now));
            if !text.is_empty() {
                out.push(text);
            }
        }
        let week_ago = now.saturating_sub(7 * 86_400_000);
        seen.retain(|_, t| t.as_u64().is_some_and(|t| t > week_ago));
        std::fs::write(&path, serde_json::to_vec(&seen)?)?;
        Ok(out.join("\n\n"))
    }

    /// `scope`'s database of the app a call runs, as the call reads it: caught up at its first read, so it reads one
    /// state of the scope throughout, and that read traced (where its data came from, and how long getting it took).
    pub(super) async fn call_db(&self, token: &str, grant: &Grant, scope: &Scope) -> Result<tokio::sync::OwnedMutexGuard<database::stored::Db>> {
        ensure!(grant.scopes.contains(scope), "the call didn't name {scope}");
        let started = Instant::now();
        let first = !grant.caught_up.contains(scope);
        let traced = grant.trace.clone().filter(|_| first);
        let (db, at) = trace::within(traced.clone(), async {
            let (db, bucket) = self.call_database(&grant.caller, scope, &grant.app, key::APP_DATABASE, None).await?;
            let mut db = db.lock_owned().await;
            let at = db.at();
            if first {
                self.read_ready(&(scope.clone(), format!("{}/{}", grant.app, key::APP_DATABASE)), &mut db, &bucket).await?;
                if let Some(g) = self.0.grants.lock().unwrap().get_mut(token) {
                    g.caught_up.push(scope.clone());
                }
            }
            anyhow::Ok((db, at))
        })
        .await?;
        if let Some(t) = traced {
            t.note(json!({ "step": "data", "app": grant.app, "scope": trace::kind(scope), "steps": db.at(), "new": db.at() - at }), started.elapsed());
        }
        Ok(db)
    }

    /// What the shim asks the node for a running call (`src/app/outbound.rs`), as `::api::call::Ask` names it.
    pub async fn internal(&self, token: &str, path: &str, body: Vec<u8>) -> Result<axum::response::Response> {
        use ::api::call::Ask;
        use axum::response::IntoResponse;
        let grant = &self.grant_of(token).context("only a running call asks the node")?;
        let ask = path.strip_prefix('/').and_then(Ask::parse).with_context(|| format!("the node has no ask {path}"))?;
        ensure!(!(grant.read_only && ask.writes()), "{}", ::api::call::READ_ONLY);
        let ok = |v: Value| axum::Json(v).into_response();
        Ok(match ask {
            Ask::Call => {
                let req: Value = serde_json::from_slice(&body)?;
                let service = req["service"] == true;
                let (app, runs) = (req["app"].as_str().unwrap_or(""), req[if service { "function" } else { "tool" }].as_str().unwrap_or(""));
                // An app may say which device runs it, like the one with a local model.
                let device = req["device"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                // `ctx.services` says it's a service's; `ctx.apps`, an app's, unless it runs as a service anyway (on
                // every device, or with no deploy). A service is its app's alone: other apps call that app's tools.
                let (tool, function) = if service { ("", runs) } else { (runs, "") };
                let call = self.resolve(CallRequest { app: app.into(), service: service.then(|| app.into()), tool: tool.into(), function: function.into(), input: req["input"].clone(), device, depth: grant.depth + 1, from: Some(grant.app.clone()), client: Some("app".into()), ..Default::default() });
                let (apps, services) = self.uses(&grant.app).await;
                match call.service.is_some() {
                    true => {
                        ensure!(app == grant.app, "{} can't call the {app} service: a service is only its own app's, so call the {app} app's tools with ctx.apps", grant.app);
                        ensure!(services.iter().any(|u| u == app), "{app} doesn't publish a service: add it to uses.services");
                    }
                    false => ensure!(apps.iter().any(|u| u == app), "{} doesn't use {app}", grant.app),
                }
                ensure!(grant.depth < MAX_DEPTH, "{} called {app} {} calls deep, and apps call each other at most {MAX_DEPTH} deep: is it looping?", grant.app, grant.depth + 1);
                self.count(&grant.app, "call").await?;
                // Part of the call that asked, in its trace: where it went and what that took.
                ok(json!({ "ok": trace::within(grant.trace.clone(), Box::pin(self.route(call, grant.caller.clone()))).await? }))
            }
            Ask::Query => {
                let req: Value = serde_json::from_slice(&body)?;
                let scope = Scope::parse(req["scope"].as_str().unwrap_or(""))?;
                let params = req["params"].as_array().cloned().unwrap_or_default();
                ok(json!({ "ok": self.call_db(token, grant, &scope).await?.query(req["sql"].as_str().unwrap_or(""), &params)? }))
            }
            // Whether the caller may do each permission to each object asked (`ctx.<scope>.may`), in order: the core's
            // rows, and the app's own types (its `types`) and rows (its database's `relations` in that scope).
            Ask::May => {
                let req: Value = serde_json::from_slice(&body)?;
                let scope = Scope::parse(req["scope"].as_str().unwrap_or(""))?;
                let asks: Vec<(String, String)> = serde_json::from_value(req["asks"].clone())?;
                let types = self.0.apps.describe(&grant.app).await?["types"].clone();
                let rows = match types.as_object().is_some_and(|t| !t.is_empty()) {
                    true => scope::scopes::app_relations(self.call_db(token, grant, &scope).await?.connection()),
                    false => Vec::new(),
                };
                ok(json!({ "ok": self.with(|i| Ok(i.r.db().app_may(&grant.caller.user, &asks, &types, &rows, now_ms())))? }))
            }
            Ask::Vault => ok(json!(vault_runtime::lookup(&self.rt(), &grant.caller, &grant.app, &grant.scopes).await?.for_app())),
            Ask::VaultMatch => {
                // The item a sign-in would change rather than make: one of that type whose plain field has that value.
                let req: Value = serde_json::from_slice(&body)?;
                let scope = Scope::parse_for(req["scope"].as_str().unwrap_or("user"), &grant.caller.user)?;
                ensure!(grant.scopes.contains(&scope), "the call didn't name {scope}");
                let (kind, field, value) = (req["type"].as_str().unwrap_or(""), req["field"].as_str().unwrap_or(""), req["value"].as_str());
                let items = vault_runtime::items(&self.rt(), &grant.caller, &scope).await?;
                let id = items.iter().filter(|(_, it)| it.kind == kind && it.fields.get(field).is_some_and(|f| !f.secret && Some(f.value.as_str()) == value)).min_by_key(|(_, it)| it.made).map(|(id, _)| id.clone());
                ok(json!({ "id": id }))
            }
            Ask::BlobPut => {
                let hash = self.put_blob(&body)?;
                if let Some(g) = self.0.grants.lock().unwrap().get_mut(token) {
                    g.puts.push(hash.clone());
                    // Someone not of this machine's own programs reads back what their call put, as what they put.
                    if g.caller.credential != Credential::Own {
                        self.0.blobs.put_by.lock().unwrap().entry(hash.clone()).or_default().insert(g.caller.user.clone());
                    }
                }
                ok(json!({ "blob": hash }))
            }
            Ask::BlobGet => {
                let req: Value = serde_json::from_slice(&body)?;
                let hash = req["hash"].as_str().unwrap_or("");
                let built_in = self.0.built_in.values().any(|h| h == hash);
                // A call this machine's own programs asked may read what its local API would give them: a file put there to
                // attach. A sign-in's asks here too, but reads only what its user may.
                let local = self.this_machine(&grant.caller) && release::blob_path(&self.0.dir, hash).is_ok_and(|p| p.exists());
                ensure!(built_in || local || grant.puts.iter().any(|h| h == hash) || self.may_read_blob(&grant.caller.user, hash)?, "no such blob for you");
                self.blob(hash).await?.into_response()
            }
            Ask::Appends => {
                ensure!(self.0.calls.later.lock().unwrap().contains_key(token), "only work a call left running after it returned hands in appends later");
                let req: Value = serde_json::from_slice(&body)?;
                let appends = ::api::call::appends(&req["appends"])?;
                self.write_appends(&grant.app, appends, &grant.scopes, &grant.caller).await?;
                ok(json!({}))
            }
            Ask::Done => {
                let req: Value = serde_json::from_slice(&body)?;
                if let Some(err) = req["error"].as_str() {
                    tracing::warn!("{}'s work after a call failed: {err}", grant.app);
                }
                if let Some(done) = self.0.calls.later.lock().unwrap().remove(token) {
                    done.notify_one();
                }
                ok(json!({}))
            }
            Ask::Catalog => ok(json!(self.catalog(&grant.caller, true).await?)),
            Ask::Code => {
                let req: Value = serde_json::from_slice(&body)?;
                ok(self.source(req["app"].as_str().unwrap_or_default(), None, req["path"].as_str()).await?)
            }
            Ask::Failing => ok(json!(self.failing_schedules(&grant.caller).await?)),
            // A push to the caller's devices, from this app, through the network's push Worker.
            Ask::Notify => {
                let send: crate::push::Send = serde_json::from_slice(&body)?;
                let mut note = send.note(&grant.app)?;
                // Named by the core, so a person sees which app it is and no app can pass for the network's own notes.
                let about = self.0.apps.describe(&grant.app).await?;
                note.title = format!("{} · {}", about["title"].as_str().unwrap_or(&grant.app), note.title);
                ok(json!({ "ok": self.push_note(&grant.caller.user, &note).await? }))
            }
            Ask::Devices => ok(json!(self.state_for(&grant.caller)?.network.map(|n| n.devices).unwrap_or_default())),
            // Run priority's order, as the user the call is for: only an admin changes it.
            Ask::Order => {
                let network = self.state_for(&grant.caller)?.network;
                let mut order: Vec<Value> = network.iter().flat_map(|n| &n.devices).map(|s| json!({ "name": s.name, "key": s.device.as_ref().unwrap_or(&s.key), "kind": s.kind, "priority": s.priority, "online": s.now.online, "seen": s.now.seen, "here": s.me })).collect();
                if let Some(cloudflare) = network.as_ref().and_then(|n| n.cloudflare_node.as_ref()) {
                    order.push(json!({ "name": "Cloudflare", "key": rank::CLOUDFLARE, "kind": rank::CLOUDFLARE, "priority": cloudflare.priority, "online": true, "here": false }));
                }
                let set = self.with(|i| Ok(rank::set(&i.r.db().settings()?)))?;
                order.sort_by_cached_key(|s| (rank::priority(&set, s["key"].as_str().unwrap_or("")), s["name"].as_str().unwrap_or("").to_lowercase()));
                ok(json!({ "order": order, "admin": network.is_some_and(|n| n.role == Some(scope::scopes::Role::Admin)) }))
            }
            Ask::Priority => {
                let req: Value = serde_json::from_slice(&body)?;
                let priority = rank::Priority::from_value(&req["priority"]).context("a run priority is a number from 1, or \"last\"")?;
                self.set_priority(&grant.caller, req["device"].as_str().unwrap_or(""), priority).await?;
                ok(json!({}))
            }
            Ask::Fetch => bail!("{}'s requests go out as they're made here", grant.app),
            Ask::AiRun => {
                let req: Value = serde_json::from_slice(&body)?;
                ok(json!({ "ok": self.ai_run(&grant.caller, &grant.app, &grant.scopes, grant.trace.as_ref(), req["input"].clone(), req["retry_key"].as_str()).await? }))
            }
        })
    }
}
