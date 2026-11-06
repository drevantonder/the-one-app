//! The Vault: its items in each scope, the node's database `vault` there (`src/vault.rs`), requests for one, which
//! machines open each scope's (`vault grant`), and filling them into apps' and services' requests. A caller's items
//! open as its other data does: with this device's own keys for its own person, else with the keys the caller hands
//! (`x-one-keys`), that database's alone; and their secrets, hosts and apps only on a machine its admins granted the
//! scope's Vault key, which nobody hands.

use std::collections::BTreeSet;

use crate::vault::{Items, Lookup};

use super::*;

/// How long a request for an item waits for its answer.
const REQUEST_LIFE: u64 = 86_400_000;

pub mod app;
pub mod cli;
pub(crate) mod routes;

/// The node's database of a scope's items.
const VAULT: (&str, &str) = (scopes::APP, crate::vault::DATABASE);

impl Node {
    /// A scope's Vault database as `me`'s work opens it (`call_database`), brought up to date.
    async fn vault_db(&self, me: &Caller, scope: &Scope) -> Result<(Arc<tokio::sync::Mutex<Db>>, Copied)> {
        let (db, bucket) = self.call_database(me, scope, VAULT.0, VAULT.1, Some(vec![crate::vault::SCHEMA.into()])).await?;
        self.read_ready(&(scope.clone(), format!("{}/{}", VAULT.0, VAULT.1)), &mut *db.lock().await, &bucket).await?;
        Ok((db, bucket))
    }

    /// A scope's items, as `me` opens them: their secrets too, if this machine opens the scope's Vault.
    pub(super) async fn items(&self, me: &Caller, scope: &Scope) -> Result<Items> {
        let keys = self.vault_secrets(scope).await?;
        let (db, _) = self.vault_db(me, scope).await?;
        let db = db.lock().await;
        let mut items = crate::vault::load(db.connection())?;
        if let Some(keys) = &keys {
            crate::vault::open_all(&mut items, keys);
        }
        Ok(items)
    }

    /// The Vault secret keys of `scope` this machine holds, by number, if it's granted the scope's Vault now: each
    /// sealed to it in the bucket whose public half is the one its `vault` entry names. None if it isn't granted.
    pub(super) async fn vault_secrets(&self, scope: &Scope) -> Result<Option<BTreeMap<u32, [u8; 32]>>> {
        let publics: Vec<(u32, [u8; 32])> = self.with(|i| {
            let m = i.r.db();
            Ok(match m.opens_vault(&self.0.id, scope) {
                true => m.vault_publics(scope).into_iter().collect(),
                false => Vec::new(),
            })
        })?;
        if publics.is_empty() {
            return Ok(None);
        }
        let (secret, me) = (self.seal_secret(), hex(&self.0.id));
        let mut out = BTreeMap::new();
        for (n, public) in publics {
            if let Some(key) = self.0.app_schedules.vault.lock().unwrap().get(&(scope.clone(), n)) {
                out.insert(n, *key);
                continue;
            }
            let name = crate::keys::vault_key_name(&scope.to_string(), n, &me);
            let Some(o) = crate::bucket::Bucket::get(&self.bucket()?, &name).await? else { continue };
            match crate::keys::open_sealed(&secret, &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| crate::keys::public_of(k) == public) {
                Some(key) => {
                    self.0.app_schedules.vault.lock().unwrap().insert((scope.clone(), n), key);
                    out.insert(n, key);
                }
                None => tracing::warn!("{name} isn't the Vault key its entry names: passed over"),
            }
        }
        Ok(Some(out))
    }

    /// Says which machines open `scope`'s Vault: `device` added to them (`on`) or taken off, as this device's person,
    /// who must be the scope's admin. Always a new key pair, sealed to each machine and the network's recovery key,
    /// its public half in a `vault` entry; what's sealed to the old one is sealed to the new by a machine granted both,
    /// at its next write there. → what it did, in words.
    pub async fn vault_grant(&self, me: &Caller, scope: &Scope, device: &str, on: bool) -> Result<Value> {
        ensure!(me.user == self.user()?, "{} grants only for its own person", self.device_name());
        let endpoint = self.find_device(device)?;
        let (mine, may, current, recovery) = self.with(|i| {
            let m = i.r.db();
            Ok((m.client(&self.0.id).is_some(), m.may(&i.user, "choose-devices", &scopes::object(&scope)), m.vault(scope).unwrap_or_default(), m.recovery()))
        })?;
        ensure!(mine, "{} isn't one of its person's own devices, which are what grant", self.device_name());
        ensure!(may, "only {scope}'s person, or its admins, say which machines open its Vault");
        let mut to: Vec<Key> = current.to.iter().copied().filter(|k| *k != endpoint).collect();
        if on {
            to.push(endpoint);
        }
        self.new_vault_key(scope, current.n + 1, &to, &recovery.context("the network has no recovery key to seal its Vault to")?).await?;
        // Sealed to the new key here, if this machine opens it: a write that changes nothing else.
        if to.contains(&self.0.id) && self.vault_secrets(scope).await?.is_some_and(|k| k.len() > 1) {
            if let Err(err) = self.write_item(me, scope, |items| Ok(items.iter().next().map(|(id, _)| json!({ "id": id })).unwrap_or(Value::Null))).await {
                tracing::debug!("sealing {scope}'s Vault to its new key: {err:#}");
            }
        }
        let name = self.name_of(&endpoint);
        Ok(json!({ "said": match on {
            true => format!("{name} opens this Vault now, and fills its items into requests to their sites."),
            false => format!("{name} no longer opens this Vault: what's set from now on is sealed to a key it doesn't have."),
        } }))
    }

    /// Makes `scope`'s Vault key `n`, for the machines `to`: its `vault` entry first (which, first in causal order,
    /// makes it), then its secret half sealed to each of them and to the recovery key.
    pub(super) async fn new_vault_key(&self, scope: &Scope, n: u32, to: &[Key], recovery: &Key) -> Result<()> {
        let secret: [u8; 32] = rand::random();
        let public = crate::keys::public_of(&secret);
        self.append(&Scope::Global, content::vault(scope, n, &public, to)).await?;
        ensure!(self.with(|i| Ok(i.r.db().vault_publics(scope).get(&n) == Some(&public)))?, "another device made {scope}'s Vault key {n} just now");
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let seals: Vec<(String, [u8; 32])> = self.with(|i| Ok(to.iter().filter_map(|k| i.r.db().seal_key(k).map(|s| (hex(k), s))).collect()))?;
        for (at, seal) in std::iter::once(("recovery".to_string(), crate::keys::twin_public(recovery)?)).chain(seals) {
            crate::database::stored::created(&bucket, &crate::keys::vault_key_name(&name, n, &at), &crate::keys::seal_to(&seal, &secret)?).await?;
        }
        if to.contains(&self.0.id) {
            self.0.app_schedules.vault.lock().unwrap().insert((scope.clone(), n), secret);
        }
        Ok(())
    }

    /// `scopes`' items for `me`. A sign-in lacking any of their keys is told them all at once (428); this device's own
    /// person passes over a scope it wasn't given the key of yet.
    async fn items_of(&self, me: &Caller, scopes: &[Scope]) -> Result<Vec<(Scope, Items)>> {
        let want: Vec<(Scope, &str, &str)> = scopes.iter().map(|s| (s.clone(), VAULT.0, VAULT.1)).collect();
        self.needs(me, &want).await?;
        let mut out = Vec::new();
        for scope in scopes {
            match self.items(me, scope).await {
                Ok(items) => out.push((scope.clone(), items)),
                Err(err) if me.credential == Credential::Own && err.is::<NotGiven>() => tracing::debug!("{err:#}"),
                Err(err) => return Err(err),
            }
        }
        Ok(out)
    }

    /// What `app`'s placeholders are filled from in `me`'s call over `scopes`: the items there it's switched on for.
    pub async fn lookup(&self, me: &Caller, app: &str, scopes: &[Scope]) -> Result<Lookup> {
        Ok(Lookup { app: app.into(), user: me.user.clone(), scopes: self.items_of(me, scopes).await? })
    }

    /// The scopes `user` may use that this device holds: where an agent's request and a service's are filled from.
    fn usable_scopes(&self, user: &str) -> Result<Vec<Scope>> {
        self.with(|i| {
            let m = i.r.db();
            Ok(m.may_scopes(user, "use").into_iter().filter(|s| i.r.holds(s)).collect())
        })
    }

    /// The scopes this device holds whose Vault `viewer` manages (or just `scope`).
    fn managed_scopes(&self, viewer: &Caller, scope: Option<&Scope>) -> Result<Vec<Scope>> {
        self.with(|i| {
            let m = i.r.db();
            Ok(m.may_scopes(&viewer.user, "add-apps").into_iter().filter(|s| i.r.holds(s) && scope.is_none_or(|w| w == s)).collect())
        })
    }

    /// Reads `global`'s items with this device's own keys, in the background (`refresh_own_items`).
    pub(super) fn read_own_items(&self) {
        let node = self.clone();
        tokio::spawn(async move { node.refresh_own_items().await });
    }

    /// Reads `global`'s items with this device's own keys, for the secrets the node itself uses (the keys of the
    /// places it backs up to), and follows them if they changed: each only as an admin's setting names it,
    /// since anyone may write an item. A device that doesn't open `global` has none.
    async fn refresh_own_items(&self) {
        let read = async {
            let me = Caller::local(self)?;
            ensure!(self.with(|i| Ok(i.r.holds(&Scope::Global) && i.r.db().may(&me.user, "use", &scopes::object(&Scope::Global))))?, "this device doesn't open global");
            self.items(&me, &Scope::Global).await
        };
        let items = match read.await {
            Ok(items) => items,
            Err(err) => return tracing::debug!("global's Vault: {err:#}"),
        };
        let changed = self
            .with(|i| {
                let changed = serde_json::to_value(&i.vault).ok() != serde_json::to_value(&items).ok();
                i.vault = items;
                Ok(changed)
            })
            .unwrap_or(false);
        if changed {
            self.follow_storage();
        }
    }

    /// Whether `host` is the name of an app or service, which an item's hosts name to let its fields into calls to
    /// that service: never a site's, so a web request to it gets none.
    pub fn names_app(&self, host: &str) -> bool {
        self.0.services.has(host) || self.with(|i| Ok(i.r.db().app(host).is_some() || i.r.db().switches().keys().any(|(_, name)| name == host))).unwrap_or(false)
    }

    /// Fills `text` for a service's call while it waits for the service's answer, as its app's requests are filled: from
    /// the items switched on for the app of the service's name that the call's user may use, a secret field only when
    /// `host` is one of its item's hosts, and only for this device's own person, whose keys it has. The service then
    /// sends it to `host` itself.
    pub async fn service_fill(&self, service: &str, call: u64, text: &str, host: &str) -> Result<String> {
        ensure!(!self.names_app(host), "{host} is an app's name, not a site's");
        let asked = self.0.services.caller_of(service, call).with_context(|| format!("no call {call} of {service}'s is waiting"))?;
        let me = Caller::local(self)?;
        ensure!(asked.user == me.user, "{} fills in only its own person's items", self.device_name());
        let lookup = self.lookup(&me, service, &self.usable_scopes(&me.user)?).await?;
        lookup.refuse_shut(text)?;
        crate::vault::fill(text, host, |name| lookup.get(name))
    }

    /// Makes an HTTP request for an agent of `caller`'s (`{ url, method?, headers?, body? }`), as the built-in agents
    /// service: the fields of the items a person lets agents use are filled in, a secret one only for its item's
    /// hosts, as an app's requests are. → `{ status, headers, body }`, a secret field's value blotted out wherever the
    /// site echoes it back, so the agent never sees one.
    pub async fn vault_fetch(&self, caller: &Caller, req: &Value) -> Result<Value> {
        let url = req["url"].as_str().context("give url")?;
        let uri: ureq::http::Uri = url.parse().with_context(|| format!("{url} isn't a web address"))?;
        ensure!(matches!(uri.scheme_str(), Some("https" | "http")), "only http and https addresses");
        let host = uri.host().context("the address has no host")?.trim_start_matches('[').trim_end_matches(']').to_lowercase();
        let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("https") { 443 } else { 80 });
        ensure!(!crate::app::outbound::own_port(self, &host, port), "that's this device's own port");
        ensure!(!self.names_app(&host), "{host} is an app's name, not a site's");
        let lookup = self.lookup(caller, scopes::AGENTS, &self.usable_scopes(&caller.user)?).await?;
        let used = std::sync::Mutex::new(Vec::<String>::new());
        let missing = std::sync::Mutex::new(Vec::<String>::new());
        let fill = |text: &str| {
            crate::vault::fill(text, &host, |name| {
                let found = lookup.get(name);
                match &found {
                    Some((hosts, value)) if !hosts.is_empty() => used.lock().unwrap().push(value.clone()),
                    Some(_) => {}
                    None => missing.lock().unwrap().push(name.to_string()),
                }
                found
            })
        };
        lookup.refuse_shut(&req.to_string())?;
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
        // A placeholder left as it is would go to the site as it is: say why it wasn't filled instead, sending nothing.
        if let Some(name) = missing.into_inner().unwrap().first() {
            bail!("Not sent: {}", Self::why_unfilled(&lookup, name));
        }
        let method = req["method"].as_str().unwrap_or(if body.is_empty() { "GET" } else { "POST" }).to_uppercase();
        let (status, headers, bytes) = tokio::task::spawn_blocking(move || crate::app::outbound::send(&method, &filled, headers, body)).await??;
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

    /// Why an agent's placeholder `name` found nothing in the items it was filled from (`lookup`), and what to do, in
    /// a sentence.
    fn why_unfilled(lookup: &crate::vault::Lookup, name: &str) -> String {
        let ask = "vault_request, leaving out app, asks the person to let you";
        let found = name.split_once(':').map(|(id, field)| (id, field, lookup.scopes.iter().find_map(|(_, items)| items.get(id).cloned())));
        match found {
            Some((id, _, None)) => format!("no item {id} is in a Vault you may use: vault_list shows them, and {ask} use one"),
            Some((_, _, Some(it))) if !it.apps.contains(scopes::AGENTS) => format!("agents may not use {} yet: {ask}", it.title),
            Some((_, field, Some(it))) => {
                let fields: Vec<&str> = it.fields.keys().map(String::as_str).collect();
                format!("{} has no field {field}; it has {}", it.title, fields.join(", "))
            }
            None => {
                let field = name.rsplit('/').next().unwrap_or(name);
                format!("no item agents may use has a field {field}: vault_request, leaving out app, asks the person for one")
            }
        }
    }
    /// The items in the scopes `viewer` manages (or `scope`), as a person sees them: never a secret field's value.
    /// `wanted_by` is the apps installed for them that ask the Vault for fields it has.
    pub async fn vault(&self, viewer: &Caller, scope: Option<&Scope>) -> Result<Vec<Value>> {
        let needs = self.vault_needs(viewer).await?;
        let all = self.items_of(viewer, &self.managed_scopes(viewer, scope)?).await?;
        let listed = self.with(|i| {
            let m = i.r.db();
            let name = |u: &str| m.user_ever(u).map(|u| u.name).unwrap_or_default();
            let mut out = Vec::new();
            for (s, items) in &all {
                let log = s.to_string();
                let scope_name = handles(m, std::slice::from_ref(s), &viewer.user).remove(0).name;
                for (id, it) in items {
                    let apps: Vec<&String> = it.apps.iter().filter(|a| !m.app(*a).is_some_and(|a| a.erased)).collect();
                    let has = |fields: &[String]| fields.iter().all(|f| it.fields.contains_key(f));
                    let wanted: Vec<&String> = needs.iter().filter(|(_, _, fields, _)| has(fields)).map(|(app, ..)| app).collect();
                    // An account: what a sign-in made, by an app that asks for accounts with those fields.
                    let account = needs.iter().any(|(app, _, fields, accounts)| *accounts && it.app.as_ref() == Some(app) && has(fields));
                    let mut wanted_by: Vec<&String> = wanted.into_iter().chain(apps.iter().copied()).collect();
                    wanted_by.sort();
                    wanted_by.dedup();
                    out.push(json!({
                        "id": id, "scope": log, "scope_name": scope_name, "title": it.title, "type": it.kind, "domain": it.domain,
                        "hosts": it.hosts, "fields": crate::vault::fields_shown(it), "apps": apps, "wanted_by": wanted_by, "app": it.app, "account": account,
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

    /// Changes an item in `scope` for `me`: `change` gives the change (`crate::vault::apply`) from the scope's items as
    /// they are inside the write, so two devices changing one at once lose nothing (Null: none). → the item's id.
    /// Anyone who may use the scope adds an item, sealing its secrets to the scope's Vault key, and changes those they
    /// set last; only someone who manages the scope's Vault changes another's, and never to switch on an erased app.
    async fn write_item(&self, me: &Caller, scope: &Scope, mut change: impl FnMut(&Items) -> Result<Value>) -> Result<String> {
        let (erased, manages, public, machines) = self.with(|i| {
            let m = i.r.db();
            ensure!(m.may(&me.user, "use", &scopes::object(scope)), "You don’t use {}.", scope_name(i, &scope.to_string()));
            ensure!(i.r.holds(scope), "This device can’t open {} yet.", scope_name(i, &scope.to_string()));
            let v = m.vault(scope);
            let machines: Vec<String> = v.as_ref().map(|v| v.to.iter().filter_map(|k| m.machine(k)).map(|d| d.name).collect()).unwrap_or_default();
            Ok((m.apps().into_iter().filter(|a| a.erased).map(|a| a.name).collect::<BTreeSet<String>>(), m.may(&me.user, "add-apps", &scopes::object(&scope)), v.map(|v| (v.n, v.public)), machines.join(", ")))
        })?;
        let keys = self.vault_secrets(scope).await?;
        let scope_said = self.with(|i| Ok(scope_name(i, &scope.to_string())))?;
        let sealing = crate::vault::Sealing { keys: keys.as_ref(), public, scope: &scope_said, machines: if machines.is_empty() { "none yet" } else { &machines } };
        let (db, bucket) = self.vault_db(me, scope).await?;
        let mut db = db.lock().await;
        let (at, now) = (db.at(), now_ms());
        let id = self
            .write_in(scope, &mut db, &bucket, |c| {
                let mut items = crate::vault::load(c)?;
                if let Some(keys) = sealing.keys {
                    crate::vault::open_all(&mut items, keys);
                }
                let data = change(&items)?;
                let Some(id) = data["id"].as_str() else { return Ok(String::new()) };
                ensure!(manages || items.get(id).is_none_or(|it| it.user == me.user), "Only its admins change an item someone else set in {scope_said}’s Vault.");
                // Who set an item last is whoever changes it, even when nothing else does.
                crate::vault::change(c, &data, &me.user, now, |a| !erased.contains(a), &sealing)
            })
            .await?;
        if db.at() > at {
            self.ping(scope, Some((VAULT.0, VAULT.1, db.at())));
        }
        drop(db);
        if *scope == Scope::Global {
            self.refresh_own_items().await;
        }
        self.emit(Event::Changed { apps: Vec::new() });
        Ok(id)
    }

    /// Makes (with no `id`) or changes an item in `scope`, for a person or agent (`caller`) → its id.
    pub async fn set_item(&self, caller: &Caller, scope: &Scope, mut data: Value) -> Result<String> {
        let id = data["id"].as_str().map(str::to_string).unwrap_or_else(new_id);
        data["id"] = json!(id);
        crate::vault::generate(&mut data);
        // Only the node says which app's call made an item.
        data.as_object_mut().context("an item is an object")?.remove("app");
        let wants = data["apps"].as_array().is_some_and(|a| a.iter().any(|a| a == scopes::AGENTS));
        self.write_item(caller, scope, |items| {
            // Agents use an item (vault_fetch) only once its person lets agents use it, never by themselves.
            let had = items.get(&id).is_some_and(|it| it.apps.contains(scopes::AGENTS));
            ensure!(caller.agent.is_none() || had || !wants, "only the person lets agents use an item: ask them with vault_request, leaving out app, and they pick or make it");
            Ok(data.clone())
        })
        .await?;
        if *scope == Scope::Global
            && let Err(err) = self.repin_place(caller, &id).await
        {
            tracing::warn!("naming a place to back up to again, as its details changed: {err:#}");
        }
        Ok(id)
    }

    /// A call's change to the Vault, by `app` for its caller, over the scopes the call named: an item it makes
    /// (switched on for it, or with `same`, an item of that type whose field matches, all its secret fields given
    /// anew), one it's switched on for that it changes (its hosts only with every secret field given anew), or one it
    /// lets go of (`release`), deleted once no app uses it.
    pub(super) async fn call_item(&self, app: &str, me: &Caller, scopes: &[Scope], id: &str, scope: Option<String>, mut data: Value, same: Option<String>, release: bool) -> Result<()> {
        ensure!(crate::database::scopes::is_id(id), "an item's id is 16 hex characters");
        crate::vault::generate(&mut data);
        let obj = data.as_object_mut().context("an item is an object")?;
        for key in ["id", "app", "apps", "deleted"] {
            obj.remove(key);
        }
        let held = self.items_of(me, scopes).await?.into_iter().find(|(_, items)| items.contains_key(id)).map(|(s, _)| s);
        let scope = match held {
            Some(scope) => scope,
            None => {
                ensure!(!release, "no such item");
                let scope = Scope::parse_for(scope.as_deref().unwrap_or("user"), &me.user)?;
                ensure!(scopes.contains(&scope), "the call didn't name {scope}");
                scope
            }
        };
        let fields_given = |it: &crate::vault::Item| it.fields.iter().filter(|(_, f)| f.secret).all(|(n, _)| data["fields"][n]["value"].is_string());
        self.write_item(me, &scope, |items| {
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

    /// Asks a person for an item `app` may use, in `scope`, for `caller` (an agent, or an app's page), who never sees
    /// its secret fields: what `app` declares as `need`, or `ask`'s `{ title, fields, hosts }`. → `{ id, link, scope }`.
    /// From a page whose Vault they can't change (a member on Everyone's), it's asked in their own, which the app's calls
    /// for them read first. A `shared` need is set once, by an admin in Everyone's Vault, for everyone: anyone else is
    /// told which admin to ask.
    pub async fn request_item(&self, caller: &Caller, scope: &Scope, app: &str, need: Option<&str>, ask: &Value, why: &str) -> Result<Value> {
        let own = Scope::User(caller.user.clone());
        let shared = match need {
            Some(need) if app != scopes::AGENTS && app != scopes::APP => self.0.apps.describe(app).await.ok().map(|d| d["vault"][need].clone()).filter(|d| d["shared"] == true),
            _ => None,
        };
        let manages = |scope: &Scope| self.with(|i| Ok(i.r.db().may(&caller.user, "add-apps", &scopes::object(&scope))));
        let scope = match &shared {
            Some(d) if !manages(&Scope::Global)? => bail!("Ask {} to set up {}: only an admin can, once for everyone", self.admin_names()?, d["title"].as_str().unwrap_or("this")),
            Some(_) => &Scope::Global,
            None if manages(scope)? => scope,
            None => &own,
        };
        let mut data = match need {
            Some(_) if app == scopes::AGENTS => bail!(
                "Not asked: need names what an app declares it needs, like app gmail, need client. For an item you use yourself, leave out need and give title, fields and hosts, and why for the person"
            ),
            Some(need) if app != scopes::APP => {
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
                // Where to find it, as the app says, when the asker gave no reason of their own; `{address}` in it is
                // the network's.
                let why = match why.is_empty() {
                    true => d["why"].as_str().unwrap_or("").replace("{address}", &self.address().unwrap_or_default()),
                    false => why.to_string(),
                };
                json!({ "title": d["title"], "fields": fields, "hosts": d["hosts"], "need": need, "why": why })
            }
            _ => json!({ "title": ask["title"], "fields": ask["fields"], "hosts": ask["hosts"], "need": need }),
        };
        let id = new_id();
        data["id"] = json!(id);
        data["app"] = json!(app);
        if data["why"].as_str().unwrap_or("").is_empty() {
            data["why"] = json!(why);
        }
        data["agent"] = json!(caller.agent);
        self.with(|i| {
            ensure!(i.r.db().may(&caller.user, "add-apps", &scopes::object(&scope)), "Only its admins change {}’s Vault.", scope_name(i, &scope.to_string()));
            ensure!(i.r.db().app(app).is_some_and(|a| !a.erased) || app == scopes::APP || app == scopes::AGENTS, "no app named {app}");
            Ok(())
        })?;
        self.append_as(scope, &caller.user, content::node("vault-request", data)).await?;
        // At the network's address (where people sign in) the web client opens it in any browser; a
        // network without one has only its apps to.
        let web = self.with(|i| Ok(crate::state::settings(&i.r.db().settings()).remove("address")))?;
        let link = match web.as_ref().and_then(Value::as_str).filter(|w| w.starts_with("https://") || w.starts_with("http://")) {
            Some(web) => format!("{}/vault?id={id}&scope={scope}", web.trim_end_matches('/')),
            None => format!("{}://vault?id={id}&scope={scope}", crate::config::CLI),
        };
        Ok(json!({ "id": id, "scope": scope.to_string(), "link": link }))
    }

    /// The network's admins' names, as someone asks them: "Andre", "Andre or Sam", "Andre, Sam or Kim".
    fn admin_names(&self) -> Result<String> {
        let mut names = self.with(|i| Ok(i.r.db().users().into_iter().filter(|u| !u.removed && i.r.db().may(&u.id, "change-settings", scopes::NETWORK)).map(|u| u.name).collect::<Vec<_>>()))?;
        names.sort();
        Ok(match names.split_last() {
            None => "an admin".to_string(),
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        })
    }

    /// The apps that use what a request of `app`'s for `need` gets: `app`, and when the need is `shared`, every added
    /// app that declares a shared need of the same type, so one item an admin sets counts for them all.
    async fn sharing(&self, app: &str, need: Option<&str>) -> Vec<String> {
        let mut out = vec![app.to_string()];
        let Some(need) = need.filter(|_| app != scopes::AGENTS && app != scopes::APP) else { return out };
        let Ok(d) = self.0.apps.describe(app).await else { return out };
        let d = &d["vault"][need];
        if d["shared"] != true || d["type"].as_str().is_none_or(str::is_empty) {
            return out;
        }
        let added: Vec<String> = self.with(|i| Ok(i.r.db().apps().into_iter().filter(|a| !a.erased && a.name != app).map(|a| a.name).collect())).unwrap_or_default();
        for other in added {
            if let Ok(o) = self.0.apps.describe(&other).await {
                if o["vault"].as_object().is_some_and(|v| v.values().any(|n| n["shared"] == true && n["type"] == d["type"])) {
                    out.push(other);
                }
            }
        }
        out
    }

    /// Answers a request in `scope`: switches its app on for `item` (with every app that shares its need), or makes an
    /// item with `fields` (name → value) and `title`; with `declined`, declines it.
    pub async fn answer_request(&self, caller: &Caller, scope: &Scope, id: &str, answer: &Value) -> Result<()> {
        let asked = self.with(|i| Ok(i.views.get(&scope.to_string()).and_then(|s| s.requests.get(id)).map(|r| (r.app.clone(), r.need.clone()))))?;
        let (sharing, kind) = match &asked {
            Some((app, need)) => {
                let kind = match need.as_deref().filter(|_| app != scopes::AGENTS && app != scopes::APP) {
                    Some(need) => self.0.apps.describe(app).await.ok().and_then(|d| d["vault"][need]["type"].as_str().map(str::to_string)),
                    None => None,
                };
                (self.sharing(app, need.as_deref()).await, kind)
            }
            None => (Vec::new(), None),
        };
        let r = self.with(|i| {
            let state = &i.views.get(&scope.to_string()).with_context(|| format!("no request {id} in {scope}"))?;
            let r = state.requests.get(id).with_context(|| format!("no request {id} in {scope}"))?;
            ensure!(r.answer.is_none(), "that request was already answered");
            ensure!(now_ms().saturating_sub(r.time) <= REQUEST_LIFE, "that request has expired: ask for it again");
            ensure!(i.r.db().may(&caller.user, "add-apps", &scopes::object(&scope)), "Only its admins change {}’s Vault.", scope_name(i, &scope.to_string()));
            Ok(r.clone())
        })?;
        if answer["declined"] == true {
            return self.append_as(scope, &caller.user, content::node("vault-request", json!({ "id": id, "declined": true }))).await.map(|_| ());
        }
        // Named by the request it answers, so answering again (its `set` entry failed, or two devices answered at once)
        // changes that item rather than making another.
        let made = id.to_string();
        self.write_item(caller, scope, |items| {
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
                    // What the node asked for itself (the network's storage) is an item of that type; what an app
                    // declared, of the type it declared.
                    if let Some(need) = r.need.as_ref().filter(|_| r.app == scopes::APP) {
                        data["type"] = json!(need);
                    } else if let Some(kind) = &kind {
                        data["type"] = json!(kind);
                    }
                    data
                }
            })
        })
        .await?;
        self.append_as(scope, &caller.user, content::node("vault-request", json!({ "id": id, "set": true }))).await?;
        // A place to back up to counts once an admin's device names it (`pin_place`), as answering does.
        if r.app == scopes::APP && r.need.as_deref() == Some(crate::backups::KIND) && *scope == Scope::Global {
            let item = answer["item"].as_str().unwrap_or(&made).to_string();
            let fields = self.items(caller, scope).await?.get(&item).map(|it| it.fields.iter().map(|(n, f)| (n.clone(), f.value.clone())).collect());
            self.pin_place(caller, &item, Some(&fields.context("the place's item wasn't there")?)).await?;
        }
        Ok(())
    }

    /// The requests `viewer` may answer (here, or in `scope`, or just `id`), newest first, with their state:
    /// `waiting`, `set`, `declined` or `expired` (a day after it was made), and the items there that have the
    /// fields it asks for. Never a secret field's value.
    pub async fn item_requests(&self, viewer: &Caller, scope: Option<&Scope>, id: Option<&str>) -> Result<Vec<Value>> {
        let now = now_ms();
        let wanted = |rid: &String| id.is_none_or(|want| want == rid.as_str());
        // The items there that have a request's fields, from the scopes that have one.
        let managed = self.managed_scopes(viewer, scope)?;
        let asked: Vec<Scope> = self.with(|i| Ok(managed.into_iter().filter(|s| i.views.get(&s.to_string()).is_some_and(|st| st.requests.keys().any(wanted))).collect()))?;
        let all = self.items_of(viewer, &asked).await?;
        self.with(|i| {
            let m = i.r.db();
            let name = |u: &str| m.user_ever(u).map(|u| u.name).unwrap_or_default();
            let mut out = Vec::new();
            for (s, items) in &all {
                let (log, scope_name) = (s.to_string(), handles(m, std::slice::from_ref(s), &viewer.user).remove(0).name);
                let Some(state) = i.views.get(&log) else { continue };
                for (rid, r) in state.requests.iter().filter(|(rid, _)| wanted(rid)) {
                    let state_now = match &r.answer {
                        Some((how, ..)) => how.as_str(),
                        None if now.saturating_sub(r.time) > REQUEST_LIFE => "expired",
                        None => "waiting",
                    };
                    let answered = r.answer.as_ref().map(|(_, by, t)| json!({ "by": name(by), "time": t }));
                    let matches: Vec<Value> = items
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

    /// How many requests `viewer` may answer are waiting, from the log alone: no item is opened to count them.
    pub(super) fn waiting_requests(&self, viewer: &Caller) -> Result<usize> {
        let now = now_ms();
        let scopes = self.managed_scopes(viewer, None)?;
        self.with(|i| {
            let waiting = |s: &Scope| i.views.get(&s.to_string()).map_or(0, |st| st.requests.values().filter(|r| r.answer.is_none() && now.saturating_sub(r.time) <= REQUEST_LIFE).count());
            Ok(scopes.iter().map(waiting).sum())
        })
    }
}
