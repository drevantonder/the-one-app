//! Requests for an item: asking a user for one an app or agents may use, answering one, and which wait.

use anyhow::{Context, Result, anyhow, bail, ensure};
use api::Caller;
use api::runtime::prelude::*;
use custom_build::allowed;
use scope::content;
use scope::scopes::{self, Scope};
use serde_json::{Value, json};

use crate::items::{items_of, managed_scopes, saved, write_item};

/// Whether an app added can declare what it needs: the agents and the Vault itself are not apps.
fn declares_needs(app: &str) -> bool {
    app != scopes::AGENTS && app != key::APP
}

/// What the apps added for `viewer` ask the Vault for: (app, need, its fields, whether it's accounts), from their
/// `vault`.
pub(crate) async fn needs_of(rt: &Rt, viewer: &Caller) -> Result<Vec<(String, String, Vec<String>, bool)>> {
    let mut out = Vec::new();
    for app in rt.apps_for(viewer).await? {
        let (Some(name), Some(needs)) = (app["name"].as_str(), app["vault"].as_object()) else { continue };
        if app["added"].as_array().is_none_or(|i| i.is_empty()) {
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

/// What `app` declares it needs, by name.
async fn declared_needs(rt: &Rt, app: &str) -> Result<Value> {
    Ok(rt.describe(app).await.map_err(|_| anyhow!("Not asked: {app} isn't added, so it declares nothing it needs"))?["vault"].clone())
}

/// Asks a user for an item `app` may use, in `scope`, for `caller` (an agent, or an app's page), who never sees its
/// secret fields: what `app` declares as `need`, or `ask`'s `{ title, fields, hosts }`. → `{ id, link, scope }`. From
/// a page whose Vault they can't change (a member on Everyone's), it's asked in their own, which the app's calls for
/// them read first. A `shared` need is set once, by an admin in Everyone's Vault, for everyone: anyone else is told
/// which admin to ask.
pub async fn request_item(rt: &Rt, caller: &Caller, scope: &Scope, app: &str, need: Option<&str>, ask: &Value, why: &str) -> Result<Value> {
    let own = Scope::User(caller.user.clone());
    let shared = match need {
        Some(need) if declares_needs(app) => Some(declared_needs(rt, app).await?[need].clone()).filter(|d| d["shared"] == true),
        _ => None,
    };
    let manages = |scope: &Scope| rt.read(|r| Ok(r.db().may(&caller.user, "add-apps", &scopes::object(scope))));
    let scope = match &shared {
        Some(d) if !manages(&Scope::Global)? => bail!("Ask {} to set up {}: only an admin can, once for everyone", admin_names(rt)?, d["title"].as_str().unwrap_or("this")),
        Some(_) => &Scope::Global,
        None if manages(scope)? => scope,
        None => &own,
    };
    let (title, fields, hosts, said) = match need {
        Some(_) if app == scopes::AGENTS => bail!(
            "Not asked: need names what an app declares it needs, like app gmail, need client. For an item you use yourself, leave out need and give title, fields and hosts, and why for the user"
        ),
        Some(need) if app != key::APP => {
            let declared = declared_needs(rt, app).await?;
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
            // Where to find it, as the app says, when the asker gave no reason of their own; `{address}` in it is the
            // network's.
            let why = match why.is_empty() {
                true => d["why"].as_str().unwrap_or("").replace("{address}", &rt.address().unwrap_or_default()),
                false => why.to_string(),
            };
            (d["title"].clone(), json!(fields), d["hosts"].clone(), why)
        }
        _ => (ask["title"].clone(), ask["fields"].clone(), ask["hosts"].clone(), String::new()),
    };
    let id = scopes::new_id();
    let why = if said.is_empty() { why } else { &said };
    rt.read(|r| {
        allowed!(r.db().may(&caller.user, "add-apps", &scopes::object(scope)), "Only its admins change {}’s Vault.", r.scope_name(scope)?);
        ensure!(r.db().app(app)?.is_some_and(|a| !a.erased) || app == key::APP || app == scopes::AGENTS, "no app named {app}");
        Ok(())
    })?;
    let request = content::Request { id: &id, app, title, fields, hosts, need, why, agent: caller.agent.as_deref() };
    rt.append(scope, &caller.user, content::vault_request(request)).await?;
    // At the network's address (where users sign in) the web client opens it in any browser; a network without one
    // has only its apps to.
    let web = rt.read(|r| Ok(scope::state::settings(&r.db().settings()?).remove("address")))?;
    let link = match web.as_deref().filter(|w| w.starts_with("https://") || w.starts_with("http://")) {
        Some(web) => format!("{}/vault?id={id}&scope={scope}", web.trim_end_matches('/')),
        None => format!("{}://vault?id={id}&scope={scope}", custom_build::config::CLI),
    };
    Ok(json!({ "id": id, "scope": scope.to_string(), "link": link }))
}

/// The network's admins' names, as someone asks them: "Andre", "Andre or Sam", "Andre, Sam or Kim".
fn admin_names(rt: &Rt) -> Result<String> {
    let mut names = rt.read(|r| Ok(r.db().users()?.into_iter().filter(|u| !u.removed && r.db().may(&u.id, "change-settings", scopes::NETWORK)).map(|u| u.name).collect::<Vec<_>>()))?;
    names.sort();
    Ok(match names.split_last() {
        None => "an admin".to_string(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    })
}

/// The apps that use what a request of `app`'s for `need` gets: `app`, and when the need is `shared`, every added app
/// that declares a shared need of the same type, so one item an admin sets counts for them all.
async fn sharing(rt: &Rt, app: &str, need: Option<&str>) -> Vec<String> {
    let mut out = vec![app.to_string()];
    let Some(need) = need.filter(|_| declares_needs(app)) else { return out };
    let Ok(d) = rt.describe(app).await else { return out };
    let d = &d["vault"][need];
    if d["shared"] != true || d["type"].as_str().is_none_or(str::is_empty) {
        return out;
    }
    let added: Vec<String> = rt.read_or(Default::default(), |r| Ok(r.db().apps()?.into_iter().filter(|a| !a.erased && a.name != app).map(|a| a.name).collect()));
    for other in added {
        if let Ok(o) = rt.describe(&other).await
            && o["vault"].as_object().is_some_and(|v| v.values().any(|n| n["shared"] == true && n["type"] == d["type"]))
        {
            out.push(other);
        }
    }
    out
}

/// Answers a request in `scope`: switches its app on for `item` (with every app that shares its need), or makes an item
/// with `fields` (name → value) and `title`; with `declined`, declines it.
pub async fn answer_request(rt: &Rt, caller: &Caller, scope: &Scope, id: &str, answer: &Value) -> Result<()> {
    let asked = rt.read(|r| Ok(r.state(scope).and_then(|s| s.requests.get(id)).map(|r| (r.app.clone(), r.need.clone()))))?;
    let (sharing, kind) = match &asked {
        Some((app, need)) => {
            let kind = match need.as_deref().filter(|_| declares_needs(app)) {
                Some(need) => rt.describe(app).await?["vault"][need]["type"].as_str().map(str::to_string),
                None => None,
            };
            (sharing(rt, app, need.as_deref()).await, kind)
        }
        None => (Vec::new(), None),
    };
    let r = rt.read(|v| {
        let state = v.state(scope).with_context(|| format!("no request {id} in {scope}"))?;
        let r = state.requests.get(id).with_context(|| format!("no request {id} in {scope}"))?;
        ensure!(r.answer.is_none(), "that request was already answered");
        ensure!(custom_build::now_ms().saturating_sub(r.time) <= scope::state::REQUEST_LIFE, "that request has expired: ask for it again");
        allowed!(v.db().may(&caller.user, "add-apps", &scopes::object(scope)), "Only its admins change {}’s Vault.", v.scope_name(scope)?);
        Ok(r.clone())
    })?;
    if answer["declined"] == true {
        return rt.append(scope, &caller.user, content::vault_answer(id, true)).await;
    }
    // Named by the request it answers, so answering again (its `set` step failed, or two devices answered at once)
    // changes that item rather than making another.
    let made = id.to_string();
    let saved_as = write_item(rt, caller, scope, |items| {
        Ok(match answer["item"].as_str() {
            Some(item) => {
                let it = items.get(item).context("no such item there")?;
                let mut apps = it.apps.clone();
                apps.insert(r.app.clone());
                apps.extend(sharing.iter().cloned());
                json!({ "id": item, "apps": apps })
            }
            None => {
                let given = answer["fields"].as_object().context("give the item's fields")?;
                let mut fields = serde_json::Map::new();
                for (name, secret) in &r.fields {
                    let value = given.get(name).and_then(Value::as_str).filter(|v| !v.is_empty()).with_context(|| format!("give {name}"))?;
                    fields.insert(name.clone(), json!({ "value": value, "secret": secret }));
                }
                let title = answer["title"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or(&r.title);
                let mut data = json!({ "id": made, "title": title, "fields": fields, "hosts": r.hosts, "apps": sharing });
                // What the runtime asked for itself is an item of the type it asked for; what an app declared, of the
                // type it declared.
                if let Some(need) = r.need.as_ref().filter(|_| r.app == key::APP) {
                    data["type"] = json!(need);
                } else if let Some(kind) = &kind {
                    data["type"] = json!(kind);
                }
                data
            }
        })
    })
    .await?;
    rt.append(scope, &caller.user, content::vault_answer(id, false)).await?;
    saved(rt, caller, scope, &saved_as, r.app == key::APP).await;
    Ok(())
}

/// The requests `viewer` may answer (here, or in `scope`, or just `id`), newest first, with their state: `waiting`,
/// `set`, `declined` or `expired` (a day after it was made), and the items there that have the fields it asks for. Never
/// a secret field's value.
pub(crate) async fn item_requests(rt: &Rt, viewer: &Caller, scope: Option<&Scope>, id: Option<&str>) -> Result<Vec<Value>> {
    let now = custom_build::now_ms();
    let wanted = |rid: &String| id.is_none_or(|want| want == rid.as_str());
    // The items there that have a request's fields, from the scopes that have one.
    let managed = managed_scopes(rt, viewer, scope)?;
    let asked: Vec<Scope> = rt.read(|r| Ok(managed.into_iter().filter(|s| r.state(s).is_some_and(|st| st.requests.keys().any(wanted))).collect()))?;
    let all = items_of(rt, viewer, &asked).await?;
    rt.read(|v| {
        let m = v.db();
        let mut out = Vec::new();
        for (s, items) in &all {
            let (log, scope_name) = (s.to_string(), scope::state::whose(m, s)?);
            let Some(state) = v.state(s) else { continue };
            for (rid, r) in state.requests.iter().filter(|(rid, _)| wanted(rid)) {
                let state_now = match &r.answer {
                    Some((how, ..)) => how.as_str(),
                    None if now.saturating_sub(r.time) > scope::state::REQUEST_LIFE => "expired",
                    None => "waiting",
                };
                let answered = r.answer.as_ref().map(|(_, by, t)| v.user_name(by).map(|by| json!({ "by": by, "time": t }))).transpose()?;
                let matches: Vec<Value> = items
                    .iter()
                    .filter(|(_, it)| r.fields.iter().all(|(f, _)| it.fields.contains_key(f)))
                    .map(|(iid, it)| json!({ "id": iid, "title": it.title, "type": it.kind, "domain": it.domain, "apps": it.apps }))
                    .collect();
                let fields: Vec<Value> = r.fields.iter().map(|(n, secret)| json!({ "name": n, "secret": secret })).collect();
                out.push(json!({
                    "id": rid, "scope": log, "scope_name": scope_name, "app": r.app, "title": r.title, "fields": fields, "need": r.need,
                    "hosts": r.hosts, "why": r.why, "by": v.user_name(&r.user)?, "agent": r.agent, "time": r.time, "state": state_now,
                    "answered": answered, "matches": matches,
                }));
            }
        }
        out.sort_by_key(|v| std::cmp::Reverse(v["time"].as_u64().unwrap_or(0)));
        Ok(out)
    })
}

/// How many requests `viewer` may answer are waiting, from the log alone: no item is opened to count them.
pub fn waiting(rt: &Rt, viewer: &Caller) -> Result<usize> {
    let now = custom_build::now_ms();
    let scopes = managed_scopes(rt, viewer, None)?;
    rt.read(|r| Ok(scope::state::waiting(r.db(), &viewer.user, scopes.iter().filter_map(|s| Some((s, r.state(s)?))), now)))
}
