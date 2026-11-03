//! The Vault: its items in each scope, requests for one, and filling them into apps' and sidecars' requests.

use super::*;

impl Node {
    /// What a placeholder in a request of `app`'s for `user` names, as (hosts, value; no hosts for a plain field):
    /// `<item id>:field` of an item in a scope the user may use, `field` of the oldest item that has it in the user's
    /// own scope, else in `global`, or `<group id>/field` in that group's; always of an item `app` is switched on for.
    pub fn vault_for(&self, app: &str, user: &str, name: &str) -> Option<(Vec<String>, String)> {
        self.with(|i| {
            let m = &i.r.members;
            let items = |scope: &Scope| i.views.get(&scope.to_string()).map(|(_, s)| &s.items);
            let oldest = |scope: &Scope, field: &str| items(scope).and_then(|its| crate::vault::find(its, app, field)).and_then(|(_, it)| crate::vault::filled(it, field));
            Ok(match (name.split_once(':'), name.split_once('/')) {
                (Some((id, field)), _) => i
                    .views
                    .iter()
                    .filter(|(log, _)| Scope::parse(log).is_ok_and(|s| m.may_use(user, &s)))
                    .find_map(|(_, (_, s))| s.items.get(id))
                    .filter(|it| it.apps.contains(app))
                    .and_then(|it| crate::vault::filled(it, field)),
                (None, Some((group, field))) => {
                    let scope = Scope::Group(group.to_string());
                    if m.may_use(user, &scope) { oldest(&scope, field) } else { None }
                }
                (None, None) => oldest(&Scope::User(user.to_string()), name).or_else(|| oldest(&Scope::Global, name)),
            })
        })
        .ok()
        .flatten()
    }

    /// Whether `host` is the name of an app or sidecar, which an item's hosts name to let its fields into calls to
    /// that sidecar: never a site's, so a web request to it gets none.
    pub fn names_app(&self, host: &str) -> bool {
        self.0.built_in_sidecars.iter().any(|n| n.name == host) || self.0.sidecars.has(host) || self.with(|i| Ok(i.r.members.apps.contains_key(host) || i.r.members.sidecars.keys().any(|(_, name)| name == host))).unwrap_or(false)
    }

    /// Fills `text` for a sidecar's call while it waits for the sidecar's answer, as its app's requests are filled: from
    /// the items switched on for the app of the sidecar's name that the call's user may use, a secret field only when
    /// `host` is one of its item's hosts. The sidecar then sends it to `host` itself.
    pub fn sidecar_fill(&self, sidecar: &str, call: u64, text: &str, host: &str) -> Result<String> {
        ensure!(!self.names_app(host), "{host} is an app's name, not a site's");
        let me = self.0.sidecars.caller_of(sidecar, call).with_context(|| format!("no call {call} of {sidecar}'s is waiting"))?;
        crate::vault::fill(text, host, |name| self.vault_for(sidecar, &me.user, name))
    }

    /// Makes an HTTP request for an agent of `caller`'s (`{ url, method?, headers?, body? }`), as the built-in agents
    /// sidecar: the fields of the items a person lets agents use are filled in, a secret one only for its item's
    /// hosts, as an app's requests are. → `{ status, headers, body }`, a secret field's value blotted out wherever the
    /// site echoes it back, so the agent never sees one.
    pub async fn vault_fetch(&self, caller: &Caller, req: &Value) -> Result<Value> {
        let url = req["url"].as_str().context("give url")?;
        let uri: ureq::http::Uri = url.parse().with_context(|| format!("{url} isn't a web address"))?;
        ensure!(matches!(uri.scheme_str(), Some("https" | "http")), "only http and https addresses");
        let host = uri.host().context("the address has no host")?.trim_start_matches('[').trim_end_matches(']').to_lowercase();
        let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("https") { 443 } else { 80 });
        ensure!(!crate::outbound::own_port(self, &host, port), "that's this device's own port");
        ensure!(!self.names_app(&host), "{host} is an app's name, not a site's");
        let used = std::sync::Mutex::new(Vec::<String>::new());
        let fill = |text: &str| {
            crate::vault::fill(text, &host, |name| {
                let found = self.vault_for(crate::agents::NAME, &caller.user, name);
                if let Some((hosts, value)) = &found {
                    if !hosts.is_empty() {
                        used.lock().unwrap().push(value.clone());
                    }
                }
                found
            })
        };
        let filled = fill(url)?;
        let mut headers = Vec::new();
        for (name, value) in req["headers"].as_object().into_iter().flatten() {
            headers.push((name.clone(), fill(value.as_str().context("a header's value is text")?)?.into_bytes()));
        }
        let body = match &req["body"] {
            Value::Null => Vec::new(),
            Value::String(text) => fill(text)?.into_bytes(),
            other => fill(&other.to_string())?.into_bytes(),
        };
        let method = req["method"].as_str().unwrap_or(if body.is_empty() { "GET" } else { "POST" }).to_uppercase();
        let (status, headers, bytes) = tokio::task::spawn_blocking(move || crate::outbound::send(&method, &filled, headers, body)).await??;
        let secrets = used.into_inner().unwrap();
        let blot = |text: &str| secrets.iter().filter(|s| !s.is_empty()).fold(text.to_string(), |t, s| t.replace(s.as_str(), "••••"));
        let headers: serde_json::Map<String, Value> = headers.iter().map(|(n, v)| (n.clone(), json!(blot(&String::from_utf8_lossy(v))))).collect();
        const SHOWN: usize = 1 << 20;
        let cut = bytes.len() > SHOWN;
        let shown = &bytes[..bytes.len().min(SHOWN)];
        let mut out = json!({ "status": status, "headers": headers });
        match std::str::from_utf8(shown) {
            Ok(text) => out["body"] = json!(blot(text)),
            Err(_) => out["body_base64"] = json!(data_encoding::BASE64.encode(shown)),
        }
        if cut {
            out["cut"] = json!(format!("only the first {SHOWN} of {} bytes", bytes.len()));
        }
        Ok(out)
    }

    /// The items `app` is switched on for in the scopes `user` may use, as its calls see them (`ctx.vault.items()`):
    /// the same ones its placeholders are filled from, the user's own first, then their groups', then `global`'s.
    pub(super) fn vault_of(&self, app: &str, user: &str) -> Result<Vec<Value>> {
        self.with(|i| {
            let m = &i.r.members;
            let mut scopes: Vec<(Scope, &ScopeState)> = i.views.iter().filter_map(|(log, (_, s))| Scope::parse(log).ok().filter(|sc| m.may_use(user, sc)).map(|sc| (sc, s))).collect();
            scopes.sort_by_key(|(sc, _)| match sc {
                Scope::User(_) => 0,
                Scope::Group(_) => 1,
                Scope::Global => 2,
            });
            let mut out = Vec::new();
            for (scope, state) in scopes {
                let mut mine: Vec<(&String, &crate::vault::Item)> = state.items.iter().filter(|(_, it)| it.apps.contains(app)).collect();
                mine.sort_by_key(|(_, it)| it.made);
                out.extend(mine.into_iter().map(|(id, it)| crate::vault::for_app(id, &scope.to_string(), it)));
            }
            Ok(out)
        })
    }

    /// The items in the scopes `viewer` manages (or `scope`), as a person sees them: never a secret field's value.
    /// `wanted_by` is the apps installed for them that ask the Vault for fields it has.
    pub async fn vault(&self, viewer: &Caller, scope: Option<&Scope>) -> Result<Vec<Value>> {
        let needs = self.vault_needs(viewer).await?;
        let listed = self.with(|i| {
            let m = &i.r.members;
            let name = |u: &str| m.users.get(u).map(|u| u.name.clone()).unwrap_or_default();
            let mut out = Vec::new();
            for (log, (_, state)) in &i.views {
                let Ok(s) = Scope::parse(log) else { continue };
                if scope.is_some_and(|want| *want != s) || !members::manages_scope(m, &viewer.user, &s) {
                    continue;
                }
                let scope_name = handles(m, std::slice::from_ref(&s), &viewer.user).remove(0).name;
                for (id, it) in &state.items {
                    let has = |fields: &[String]| fields.iter().all(|f| it.fields.contains_key(f));
                    let wanted: Vec<&String> = needs.iter().filter(|(_, _, fields, _)| has(fields)).map(|(app, ..)| app).collect();
                    // An account: what a sign-in made, by an app that asks for accounts with those fields.
                    let account = needs.iter().any(|(app, _, fields, accounts)| *accounts && it.app.as_ref() == Some(app) && has(fields));
                    let mut wanted_by: Vec<&String> = wanted.into_iter().chain(&it.apps).collect();
                    wanted_by.sort();
                    wanted_by.dedup();
                    out.push(json!({
                        "id": id, "scope": log, "scope_name": scope_name, "title": it.title, "type": it.kind, "domain": it.domain,
                        "hosts": it.hosts, "fields": crate::vault::fields_shown(it), "apps": it.apps, "wanted_by": wanted_by, "app": it.app, "account": account,
                        "user": name(&it.user), "time": it.time, "made": it.made,
                    }));
                }
            }
            out.sort_by_key(|v| (v["scope"].as_str().map(|s| (!s.starts_with("user:"), s == "global", s.to_string())), v["made"].as_u64()));
            Ok(out)
        })?;
        let mut out = listed;
        for item in &mut out {
            if let (Some(domain), Some(app)) = (item["domain"].as_str().map(str::to_string), item["app"].as_str().or(item["apps"][0].as_str()).map(str::to_string)) {
                if let Some((image, kind)) = self.logo(&domain, &app) {
                    (item["image"], item["image_kind"]) = (json!(image), json!(kind));
                }
            }
        }
        Ok(out)
    }

    /// What the apps installed for `viewer` ask the Vault for: (app, need, its fields, whether it's accounts), from
    /// their `vault`.
    pub(super) async fn vault_needs(&self, viewer: &Caller) -> Result<Vec<(String, String, Vec<String>, bool)>> {
        let mut out = Vec::new();
        for app in self.apps(viewer).await? {
            let (Some(name), Some(needs)) = (app["name"].as_str(), app["vault"].as_object()) else { continue };
            if app["installed"].as_array().is_none_or(|i| i.is_empty()) {
                continue;
            }
            for (need, d) in needs {
                let fields: Vec<String> = d["fields"].as_object().map(|f| f.keys().cloned().collect()).unwrap_or_default();
                if !fields.is_empty() {
                    out.push((name.to_string(), need.clone(), fields, d["accounts"] == true));
                }
            }
        }
        Ok(out)
    }

    /// Makes (with no `id`) or changes an item in `scope`, for a person or agent (`caller`) → its id.
    pub async fn set_item(&self, caller: &Caller, scope: &Scope, mut data: Value) -> Result<String> {
        let id = data["id"].as_str().map(str::to_string).unwrap_or_else(new_id);
        data["id"] = json!(id);
        crate::vault::generate(&mut data);
        // Only the node says which app's call made an item.
        data.as_object_mut().context("an item is an object")?.remove("app");
        let content = self.with(|i| {
            // Agents use an item (vault_fetch) only once its person lets agents use it, never by themselves.
            let had = i.views.get(&scope.to_string()).and_then(|(_, s)| s.items.get(&id)).is_some_and(|it| it.apps.contains(crate::agents::NAME));
            let wants = data["apps"].as_array().is_some_and(|a| a.iter().any(|a| a == crate::agents::NAME));
            ensure!(caller.agent.is_none() || had || !wants, "only the person lets agents use an item: ask them with vault_request, leaving out app, and they pick or make it");
            item_entry(i, &caller.user, scope, data)
        })?;
        self.append_as(scope, &caller.user, content).await?;
        Ok(id)
    }

    /// Asks a person for an item `app` may use, in `scope`, for `caller` (an agent, or an app's page), who never sees
    /// its secret fields: what `app` declares as `need`, or `ask`'s `{ title, fields, hosts }`. → `{ id, link, scope }`.
    pub async fn request_item(&self, caller: &Caller, scope: &Scope, app: &str, need: Option<&str>, ask: &Value, why: &str) -> Result<Value> {
        let mut data = match need {
            Some(_) if app == crate::agents::NAME => bail!(
                "Not asked: need names what an app declares it needs, like app gmail, need client. For an item you use yourself, leave out need and give title, fields and hosts, and why for the person"
            ),
            Some(need) if app != members::APP => {
                let declared = self.0.apps.describe(app).await.map_err(|_| anyhow!("Not asked: {app} isn't added, so it declares nothing it needs"))?["vault"].clone();
                let d = declared[need].clone();
                if !d.is_object() {
                    let names: Vec<&str> = declared.as_object().into_iter().flatten().map(|(n, _)| n.as_str()).collect();
                    bail!(
                        "Not asked: {app} declares no {need}; {}",
                        match names.is_empty() {
                            true => "it declares nothing it needs, so give title, fields and hosts instead".to_string(),
                            false => format!("it declares {}", names.join(", ")),
                        }
                    );
                }
                let fields: serde_json::Map<String, Value> = d["fields"].as_object().into_iter().flatten().map(|(n, f)| (n.clone(), json!({ "secret": f["secret"] == true }))).collect();
                json!({ "title": d["title"], "fields": fields, "hosts": d["hosts"], "need": need })
            }
            _ => json!({ "title": ask["title"], "fields": ask["fields"], "hosts": ask["hosts"], "need": need }),
        };
        let id = new_id();
        data["id"] = json!(id);
        data["app"] = json!(app);
        data["why"] = json!(why);
        data["agent"] = json!(caller.agent);
        self.with(|i| {
            ensure!(members::manages_scope(&i.r.members, &caller.user, scope), "you can't change the Vault in {scope}");
            ensure!(i.r.members.apps.get(app).is_some_and(|a| !a.erased) || app == members::APP || app == crate::agents::NAME, "no app named {app}");
            Ok(())
        })?;
        self.append_as(scope, &caller.user, members::node("vault-request", data)).await?;
        // At the install's address (where its kernel signs people in) the web client opens it in any browser; an
        // install without one has only its apps to.
        let web = self.with(|i| Ok(crate::state::settings(&i.r.members).remove("address")))?;
        let link = match web.as_ref().and_then(Value::as_str).filter(|w| w.starts_with("https://") || w.starts_with("http://")) {
            Some(web) => format!("{}/vault?id={id}&scope={scope}", web.trim_end_matches('/')),
            None => format!("{}://vault?id={id}&scope={scope}", crate::config::CLI),
        };
        Ok(json!({ "id": id, "scope": scope.to_string(), "link": link }))
    }

    /// Answers a request in `scope`: switches its app on for `item`, or makes an item with `fields` (name → value)
    /// and `title`; with `declined`, declines it.
    pub async fn answer_request(&self, caller: &Caller, scope: &Scope, id: &str, answer: &Value) -> Result<()> {
        let content = self.with(|i| {
            let state = &i.views.get(&scope.to_string()).with_context(|| format!("no request {id} in {scope}"))?.1;
            let r = state.requests.get(id).with_context(|| format!("no request {id} in {scope}"))?;
            ensure!(r.answer.is_none(), "that request was already answered");
            if answer["declined"] == true {
                ensure!(members::manages_scope(&i.r.members, &caller.user, scope), "you can't change the Vault in {scope}");
                return Ok(members::node("vault-request", json!({ "id": id, "declined": true })));
            }
            let data = match answer["item"].as_str() {
                Some(item) => {
                    let it = state.items.get(item).context("no such item there")?;
                    let mut apps = it.apps.clone();
                    apps.insert(r.app.clone());
                    json!({ "id": item, "apps": apps, "request": id })
                }
                None => {
                    let given = answer["fields"].as_object().context("give the item's fields")?;
                    let mut fields = serde_json::Map::new();
                    for (name, secret) in &r.fields {
                        let value = given.get(name).and_then(Value::as_str).filter(|v| !v.is_empty()).with_context(|| format!("give {name}"))?;
                        fields.insert(name.clone(), json!({ "value": value, "secret": secret }));
                    }
                    let title = answer["title"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or(&r.title);
                    let mut data = json!({ "id": new_id(), "title": title, "fields": fields, "hosts": r.hosts, "apps": [r.app], "request": id });
                    // What the node asked for itself (the install's storage) is an item of that type.
                    if let Some(need) = r.need.as_ref().filter(|_| r.app == members::APP) {
                        data["type"] = json!(need);
                    }
                    data
                }
            };
            item_entry(i, &caller.user, scope, data)
        })?;
        self.append_as(scope, &caller.user, content).await?;
        Ok(())
    }

    /// The requests `viewer` may answer (here, or in `scope`, or just `id`), newest first, with their state:
    /// `waiting`, `set`, `declined` or `expired` (a day after it was made), and the items there that have the
    /// fields it asks for. Never a secret field's value.
    pub fn item_requests(&self, viewer: &Caller, scope: Option<&Scope>, id: Option<&str>) -> Result<Vec<Value>> {
        let now = now_ms();
        self.with(|i| {
            let m = &i.r.members;
            let name = |u: &str| m.users.get(u).map(|u| u.name.clone()).unwrap_or_default();
            let mut out = Vec::new();
            for (log, (_, state)) in &i.views {
                let Ok(s) = Scope::parse(log) else { continue };
                if scope.is_some_and(|want| *want != s) || !members::manages_scope(m, &viewer.user, &s) {
                    continue;
                }
                let scope_name = handles(m, std::slice::from_ref(&s), &viewer.user).remove(0).name;
                for (rid, r) in state.requests.iter().filter(|(rid, _)| id.is_none_or(|want| want == rid.as_str())) {
                    let state_now = match &r.answer {
                        Some((how, ..)) => how.as_str(),
                        None if now.saturating_sub(r.time) > REQUEST_LIFE => "expired",
                        None => "waiting",
                    };
                    let answered = r.answer.as_ref().map(|(_, by, t)| json!({ "by": name(by), "time": t }));
                    let matches: Vec<Value> = state
                        .items
                        .iter()
                        .filter(|(_, it)| r.fields.iter().all(|(f, _)| it.fields.contains_key(f)))
                        .map(|(iid, it)| json!({ "id": iid, "title": it.title, "type": it.kind, "domain": it.domain, "apps": it.apps }))
                        .collect();
                    let fields: Vec<Value> = r.fields.iter().map(|(n, secret)| json!({ "name": n, "secret": secret })).collect();
                    out.push(json!({
                        "id": rid, "scope": log, "scope_name": scope_name, "app": r.app, "title": r.title, "fields": fields, "need": r.need,
                        "hosts": r.hosts, "why": r.why, "by": name(&r.user), "agent": r.agent, "time": r.time, "state": state_now,
                        "answered": answered, "matches": matches,
                    }));
                }
            }
            out.sort_by_key(|v| std::cmp::Reverse(v["time"].as_u64().unwrap_or(0)));
            Ok(out)
        })
    }

    /// The Vault's routes, `/v1/vault…`, for the local API; `body` is a GET's query or a POST's body.
    /// None for another path.
    pub async fn vault_route(&self, me: &Caller, method: &str, path: &str, body: &Value) -> Result<Option<Value>> {
        let scope = |default: Option<&str>| body["scope"].as_str().or(default).map(|s| self.scope_for(me, s)).transpose();
        let me = &Caller { agent: body["agent"].as_str().map(str::to_string).or(me.agent.clone()), ..me.clone() };
        Ok(Some(match (method, path) {
            ("GET", "/v1/vault") => json!(self.vault(me, scope(None)?.as_ref()).await?),
            ("POST", "/v1/vault") => {
                let scope = scope(Some("user"))?.unwrap();
                let mut data = body.clone();
                data.as_object_mut().context("an item is an object")?.remove("scope");
                json!({ "id": self.set_item(me, &scope, data).await? })
            }
            ("GET", "/v1/vault/requests") => json!(self.item_requests(me, scope(None)?.as_ref(), body["id"].as_str())?),
            ("POST", "/v1/vault/request") => {
                let scope = scope(Some("user"))?.unwrap();
                self.request_item(me, &scope, body["app"].as_str().unwrap_or(""), body["need"].as_str(), body, body["why"].as_str().unwrap_or("")).await?
            }
            ("POST", "/v1/vault/answer") => {
                self.answer_request(me, &scope(Some("user"))?.unwrap(), body["id"].as_str().unwrap_or(""), body).await?;
                json!({})
            }
            ("POST", "/v1/vault/fetch") => self.vault_fetch(me, body).await?,
            _ => return Ok(None),
        }))
    }
}
