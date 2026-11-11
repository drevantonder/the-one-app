//! A scope's items: its database, opening their secrets with the Vault keys this device holds, and changing them.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use api::call::Event;
use api::runtime::NotGiven;
use api::runtime::prelude::*;
use api::{Caller, Credential};
use custom_build::allowed;
use scope::scopes::{self, Scope};
use serde_json::{Value, json};
use vault::{Item, Items, Lookup};

use crate::{ASK_FOR_AGENTS, Secrets, State, Watch};

/// The runtime's own database `vault`, in each scope.
pub(crate) const APP: &str = key::APP;
pub(crate) const DATABASE: &str = key::CoreDb::Vault.name();
const SCHEMA: &[&str] = &[vault::SCHEMA];

/// `scope`'s items for `me`, their secrets opened if this device holds the scope's Vault keys.
pub async fn items(rt: &Rt, me: &Caller, scope: &Scope) -> Result<Items> {
    let db = rt.open(me, scope, APP, DATABASE, SCHEMA).await?;
    let mut items = vault::load(db.lock().await.connection())?;
    let sealed_under: BTreeSet<u32> = items.values().filter_map(|it| it.sealed.as_ref().map(|s| s.n)).collect();
    if let Some(keys) = secrets(rt, scope, Some(&sealed_under)).await? {
        vault::open_all(&mut items, &keys);
    }
    Ok(items)
}

/// The Vault secret keys of `scope` this machine holds, by number, of those in `only` (all if none), if its device is
/// granted the scope's Vault now: those its device's key opens. None if it isn't granted. One the bucket hadn't is
/// asked for again only once who's who has moved on.
pub(crate) async fn secrets(rt: &Rt, scope: &Scope, only: Option<&BTreeSet<u32>>) -> Result<Option<Secrets>> {
    let device = rt.device();
    let (publics, at): (Vec<(u32, [u8; 32])>, Option<u64>) = rt.read(|r| {
        let m = r.db();
        Ok((
            match m.opens_vault(&device, scope)? {
                true => m.vault_publics(scope)?.into_iter().collect(),
                false => Vec::new(),
            },
            r.at(),
        ))
    })?;
    if publics.is_empty() {
        return Ok(None);
    }
    let state = rt.state::<State>();
    let mut out = BTreeMap::new();
    let mut missing = Vec::new();
    {
        let (held, none) = (state.secrets.lock().unwrap(), state.no_secrets.lock().unwrap());
        for (n, public) in publics.into_iter().filter(|(n, _)| only.is_none_or(|o| o.contains(n))) {
            let id = (scope.clone(), n);
            match held.get(&id) {
                Some(key) => drop(out.insert(n, *key)),
                None if none.get(&id) == Some(&at) => {}
                None => missing.push((n, public)),
            }
        }
    }
    let opened = rt.vault_keys(scope, &missing).await?;
    {
        let mut none = state.no_secrets.lock().unwrap();
        for (n, _) in &missing {
            match opened.get(n) {
                Some(_) => drop(none.remove(&(scope.clone(), *n))),
                None => drop(none.insert((scope.clone(), *n), at)),
            }
        }
    }
    state.secrets.lock().unwrap().extend(opened.iter().map(|(n, k)| ((scope.clone(), *n), *k)));
    out.extend(opened);
    Ok(Some(out))
}

/// Seals what's in `scope`'s Vault to its newest key, if this machine opens it and another before it: a write that
/// changes nothing else, done at once after this machine's own user changed which machines open it.
pub(crate) async fn reseal(rt: &Rt, scope: &Scope) -> Result<()> {
    let device = rt.device();
    if rt.read(|r| r.db().opens_vault(&device, scope))? && secrets(rt, scope, None).await?.is_some_and(|k| k.len() > 1) {
        write_item(rt, &rt.caller()?, scope, |items| Ok(items.iter().next().map(|(id, _)| json!({ "id": id })).unwrap_or(Value::Null))).await?;
    }
    Ok(())
}

/// `scopes`' items for `me`. A sign-in lacking any of their keys is told them all at once (428); this device's own
/// user passes over a scope it wasn't given the key of yet.
pub async fn items_of(rt: &Rt, me: &Caller, scopes: &[Scope]) -> Result<Vec<(Scope, Items)>> {
    let want: Vec<(Scope, &str, &str)> = scopes.iter().map(|s| (s.clone(), APP, DATABASE)).collect();
    rt.needs(me, &want).await?;
    let mut out = Vec::new();
    for scope in scopes {
        match items(rt, me, scope).await {
            Ok(items) => out.push((scope.clone(), items)),
            Err(err) if me.credential == Credential::Own && err.is::<NotGiven>() => tracing::debug!("{err:#}"),
            Err(err) => return Err(err),
        }
    }
    Ok(out)
}

/// What `app`'s placeholders are filled from in `me`'s call over `scopes`: the items there it's switched on for.
pub async fn lookup(rt: &Rt, me: &Caller, app: &str, scopes: &[Scope]) -> Result<Lookup> {
    Ok(Lookup { app: app.into(), user: me.user.clone(), scopes: items_of(rt, me, scopes).await? })
}

/// The scopes `viewer` manages that this device holds (or only `scope`).
pub(crate) fn managed_scopes(rt: &Rt, viewer: &Caller, scope: Option<&Scope>) -> Result<Vec<Scope>> {
    rt.read(|r| Ok(r.db().may_scopes(&viewer.user, "add-apps")?.into_iter().filter(|s| r.holds(s) && scope.is_none_or(|w| w == s)).collect()))
}

/// Reads `global`'s items with this device's own keys, for the secrets the runtime itself uses (the keys of the places
/// it backs up to): each used only as an admin's setting names it, since anyone may write an item. A device that doesn't
/// open `global` has none.
pub(crate) async fn refresh_own(rt: Rt) -> Result<()> {
    let me = rt.caller()?;
    ensure!(rt.read(|r| Ok(r.holds(&Scope::Global) && r.db().may(&me.user, "use", &scopes::object(&Scope::Global))))?, "this device doesn't open global");
    let items = items(&rt, &me, &Scope::Global).await?;
    *rt.state::<State>().own.lock().unwrap() = items;
    Ok(())
}

/// The items in the scopes `viewer` manages (or `scope`), as a user sees them: never a secret field's value.
/// `wanted_by` is the apps added for them that ask the Vault for fields it has.
pub(crate) async fn list(rt: &Rt, viewer: &Caller, scope: Option<&Scope>) -> Result<Vec<Value>> {
    let needs = crate::requests::needs_of(rt, viewer).await?;
    let all = items_of(rt, viewer, &managed_scopes(rt, viewer, scope)?).await?;
    let mut out = rt.read(|r| {
        let m = r.db();
        let mut out = Vec::new();
        for (s, items) in &all {
            let log = s.to_string();
            let scope_name = scope::state::whose(m, s)?;
            for (id, it) in items {
                let mut apps: Vec<&String> = Vec::new();
                for a in &it.apps {
                    if !m.app(a)?.is_some_and(|a| a.erased) {
                        apps.push(a);
                    }
                }
                let has = |fields: &[String]| fields.iter().all(|f| it.fields.contains_key(f));
                let wanted: Vec<&String> = needs.iter().filter(|(_, _, fields, _)| has(fields)).map(|(app, ..)| app).collect();
                // An account: what a sign-in made, by an app that asks for accounts with those fields.
                let account = needs.iter().any(|(app, _, fields, accounts)| *accounts && it.app.as_ref() == Some(app) && has(fields));
                let mut wanted_by: Vec<&String> = wanted.into_iter().chain(apps.iter().copied()).collect();
                wanted_by.sort();
                wanted_by.dedup();
                out.push(json!({
                    "id": id, "scope": log, "scope_name": scope_name, "title": it.title, "type": it.kind, "domain": it.domain,
                    "hosts": it.hosts, "fields": vault::fields_shown(it), "apps": apps, "wanted_by": wanted_by, "app": it.app, "account": account,
                    "user": r.user_name(&it.user)?, "time": it.time, "made": it.made,
                }));
            }
        }
        out.sort_by_key(|v| (v["scope"].as_str().map(|s| (!s.starts_with("user:"), s == "global", s.to_string())), v["made"].as_u64()));
        Ok(out)
    })?;
    for item in &mut out {
        if let (Some(domain), Some(app)) = (item["domain"].as_str().map(str::to_string), item["app"].as_str().or(item["apps"][0].as_str()).map(str::to_string)) {
            if let Some((image, kind)) = rt.logo(&domain, &app) {
                (item["image"], item["image_kind"]) = (json!(image), json!(kind));
            }
        }
    }
    Ok(out)
}

/// Changes an item in `scope` for `me`: `change` gives the change (`vault::apply`) from the scope's items as they are
/// inside the write, so two devices changing one at once lose nothing (Null: none). → the item's id. Anyone who may use
/// the scope adds an item, sealing its secrets to the scope's Vault key, and changes those they set last; only someone
/// who manages the scope's Vault changes another's, and never to switch on an erased app. What watches items of its
/// kind there follows it (`watch`).
pub(crate) async fn write_item(rt: &Rt, me: &Caller, scope: &Scope, mut change: impl FnMut(&Items) -> Result<Value> + Send) -> Result<String> {
    let (erased, manages, public, machines, scope_said) = rt.read(|r| {
        let m = r.db();
        allowed!(m.may(&me.user, "use", &scopes::object(scope)), "You don’t use {}.", r.scope_name(scope)?);
        ensure!(r.holds(scope), "This device can’t open {} yet.", r.scope_name(scope)?);
        let v = m.vault(scope)?;
        let mut machines: Vec<String> = Vec::new();
        let mut opened = false;
        for k in v.iter().flat_map(|v| v.to.iter()) {
            machines.extend(m.machine(k)?.map(|d| d.name));
            opened |= m.opens_vault(k, scope)?;
        }
        // A key no machine opens is no key to seal to: nothing sealed to it would open, whoever is granted later.
        let erased: BTreeSet<String> = m.apps()?.into_iter().filter(|a| a.erased).map(|a| a.name).collect();
        Ok((erased, m.may(&me.user, "add-apps", &scopes::object(scope)), v.filter(|_| opened).map(|v| (v.n, v.public)), machines.join(", "), r.scope_name(scope)?))
    })?;
    let keys = secrets(rt, scope, None).await?;
    let sealing = vault::Sealing { keys: keys.as_ref(), public, scope: &scope_said, machines: if machines.is_empty() { "none yet" } else { &machines } };
    let now = custom_build::now_ms();
    let (mut id, mut kind) = (String::new(), String::new());
    rt.write(me, scope, APP, DATABASE, SCHEMA, &mut |c| {
        let mut items = vault::load(c)?;
        if let Some(keys) = sealing.keys {
            vault::open_all(&mut items, keys);
        }
        let data = change(&items)?;
        let Some(at) = data["id"].as_str() else {
            id.clear();
            return Ok(());
        };
        allowed!(manages || items.get(at).is_none_or(|it| it.user == me.user), "Only its admins change an item someone else set in {scope_said}’s Vault.");
        kind = data["type"].as_str().map(str::to_string).or_else(|| items.get(at).map(|it| it.kind.clone())).unwrap_or_default();
        // Who set an item last is whoever changes it, even when nothing else does.
        id = vault::change(c, &data, &me.user, now, |a| !erased.contains(a), &sealing)?;
        Ok(())
    })
    .await?;
    if *scope == Scope::Global
        && let Err(err) = refresh_own(rt.clone()).await
    {
        tracing::debug!("global's Vault: {err:#}");
    }
    rt.emit(Event::Changed { apps: Vec::new() });
    if !id.is_empty() {
        saved(rt, me, scope, &kind, &id).await;
    }
    Ok(id)
}

/// Tells what watches items of `kind` in `scope` that `me` saved item `id` there.
async fn saved(rt: &Rt, me: &Caller, scope: &Scope, kind: &str, id: &str) {
    let watching: Vec<Watch> = rt.state::<State>().watchers.lock().unwrap().iter().filter(|w| w.scope == *scope && w.kind == kind).cloned().collect();
    for w in watching {
        if let Err(err) = (w.saved)(rt.clone(), me.clone(), id.to_string()).await {
            tracing::warn!("following an item saved in {scope}'s Vault: {err:#}");
        }
    }
}

/// Makes (with no `id`) or changes an item in `scope`, for a user or agent (`caller`) → its id.
pub async fn set_item(rt: &Rt, caller: &Caller, scope: &Scope, mut data: Value) -> Result<String> {
    let id = data["id"].as_str().map(str::to_string).unwrap_or_else(scopes::new_id);
    data["id"] = json!(id);
    vault::generate(&mut data);
    // Only the runtime says which app's call made an item.
    data.as_object_mut().context("an item is an object")?.remove("app");
    let wants = data["apps"].as_array().is_some_and(|a| a.iter().any(|a| a == scopes::AGENTS));
    write_item(rt, caller, scope, |items| {
        // Agents use an item (vault_fetch) only once its user lets agents use it, never by themselves.
        let had = items.get(&id).is_some_and(|it| it.apps.contains(scopes::AGENTS));
        ensure!(caller.agent.is_none() || had || !wants, "only the user lets agents use an item: ask them with {ASK_FOR_AGENTS} and they pick or make it");
        Ok(data.clone())
    })
    .await?;
    Ok(id)
}

/// A call's change to the Vault, by `app` for its caller, over the scopes the call named: an item it makes (switched
/// on for it, or with `same`, an item of that type whose field matches, all its secret fields given anew), one it's
/// switched on for that it changes (its hosts only with every secret field given anew), or one it lets go of
/// (`release`), deleted once no app uses it.
#[allow(clippy::too_many_arguments)]
pub async fn call_item(rt: &Rt, app: &str, me: &Caller, scopes: &[Scope], id: &str, scope: Option<String>, mut data: Value, same: Option<String>, release: bool) -> Result<()> {
    ensure!(scope::scopes::is_id(id), "an item's id is 16 hex characters");
    vault::generate(&mut data);
    let obj = data.as_object_mut().context("an item is an object")?;
    for key in ["id", "app", "apps", "deleted"] {
        obj.remove(key);
    }
    let held = items_of(rt, me, scopes).await?.into_iter().find(|(_, items)| items.contains_key(id)).map(|(s, _)| s);
    let scope = match held {
        Some(scope) => scope,
        None => {
            ensure!(!release, "no such item");
            let scope = Scope::parse_for(scope.as_deref().unwrap_or("user"), &me.user)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            scope
        }
    };
    let fields_given = |it: &Item| it.fields.iter().filter(|(_, f)| f.secret).all(|(n, _)| data["fields"][n]["value"].is_string());
    write_item(rt, me, &scope, |items| {
        let mut data = data.clone();
        Ok(match items.get(id) {
            Some(it) if release => {
                ensure!(it.apps.contains(app), "{app} isn't switched on for that item");
                let mut apps = it.apps.clone();
                apps.remove(app);
                if apps.is_empty() { json!({ "id": id, "deleted": true }) } else { json!({ "id": id, "apps": apps }) }
            }
            Some(it) => {
                let matched = same.as_deref().is_some_and(|f| it.kind == data["type"].as_str().unwrap_or(&it.kind) && it.fields.get(f).is_some_and(|x| !x.secret && data["fields"][f]["value"].as_str() == Some(x.value.as_str())));
                ensure!(it.apps.contains(app) || (matched && fields_given(it)), "{app} isn't switched on for that item");
                ensure!(data["hosts"].is_null() || fields_given(it), "an app changes an item's hosts only with every secret field given anew");
                let mut apps = it.apps.clone();
                apps.insert(app.to_string());
                data["id"] = json!(id);
                data["apps"] = json!(apps);
                data
            }
            None => {
                ensure!(!release, "no such item");
                data["id"] = json!(id);
                data["apps"] = json!([app]);
                data["app"] = json!(app);
                data
            }
        })
    })
    .await?;
    Ok(())
}
