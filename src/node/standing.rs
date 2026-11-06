//! Standing work (ARCHITECTURE.md): an app kept running in a scope on a machine its person chose, with
//! that app's keys there sealed to the machine (`keys::grant_name`) while a `grant` entry says so. This device's person
//! grants it here, and renews it as they use the app, so a grant nobody uses, or whose machine is off, ends by itself;
//! a machine that holds one opens that app's databases there with the grant's keys alone, as a call opens what it was
//! handed, and never its other apps'.

use crate::config::YOURS;
use super::*;

/// How long a grant lasts, unless its person's device renews it as they use the app.
pub const GRANT_DAYS: u64 = 30;
const DAY: u64 = 86_400_000;

/// What standing work's calls are kept by here, in place of a sign-in's token: `grant.<scope> <app>`. No sign-in
/// signs one and it's never passed on, so a device that's sent one gets nothing with it.
pub(super) const GRANT: &str = "grant.";

/// The token a grant's calls run with here.
pub(super) fn grant_token(scope: &Scope, app: &str) -> String {
    format!("{GRANT}{scope} {app}")
}

/// The scope and app a grant's token is for.
fn grant_of(token: &str) -> Option<(Scope, &str)> {
    let (scope, app) = token.strip_prefix(GRANT)?.split_once(' ')?;
    Some((Scope::parse(scope).ok()?, app))
}

/// The grants' state on a node.
#[derive(Default)]
pub(super) struct Standing {
    /// The app keys grants sealed to this device, by (scope, app), each by epoch: kept while the grant is in force.
    keys: Mutex<HashMap<(Scope, String), crate::db::Keys>>,
    /// The Vault secret keys sealed to this machine, by scope and number, each the one its `vault` entry names.
    pub(super) vault: Mutex<HashMap<(Scope, u32), [u8; 32]>>,
    /// The grants this device is renewing now.
    renewing: Mutex<std::collections::HashSet<(Key, Scope, String)>>,
}

impl Node {
    /// Keeps `app` running in `scope` on `device` for `days` (0 stops it): as this device's person, who must be the
    /// scope's or, for a group, its admin. Seals the app's key in each epoch this device opens to that device, then
    /// writes the `grant` naming each one's hash. → what it did, in words.
    pub async fn keep_running(&self, me: &Caller, device: &str, app: &str, scope: &Scope, days: u64) -> Result<Value> {
        ensure!(me.user == self.user()?, "{} grants only for its own person", self.device_name());
        let endpoint = self.find_device(device)?;
        let (title, name) = (self.title_of(app).await, self.name_of(&endpoint));
        ensure!(self.with(|i| Ok(i.r.db().app(app).is_some()))?, "no app called {app}");
        let until = match days {
            0 => 0,
            days => now_ms() + days.min(GRANT_DAYS) * DAY,
        };
        self.grant_to(endpoint, app, scope, until).await?;
        if until == 0 {
            self.end_grant(me, scope).await?;
        }
        Ok(match until {
            0 => json!({ "said": format!("{name} no longer keeps {title} running, and what's written from now on is under new keys it doesn't have.") }),
            until => json!({ "said": format!("{name} keeps {title} running for you, for {} days, and longer as you use it.", days.min(GRANT_DAYS)), "until": until }),
        })
    }

    /// Writes a grant of `app` in `scope` to `endpoint` until `until` (0: no longer), with its keys sealed first.
    async fn grant_to(&self, endpoint: Key, app: &str, scope: &Scope, until: u64) -> Result<()> {
        let (seal, mine, may) = self.with(|i| {
            let m = i.r.db();
            Ok((m.seal_key(&endpoint), m.client(&self.0.id).is_some(), m.may(&i.user, "keep", &scopes::object(&scope))))
        })?;
        ensure!(mine, "{} isn't one of its person's own devices, which are what grant", self.device_name());
        ensure!(may, "only {scope}'s person, or its group's admins, keep its apps running");
        let seal = seal.with_context(|| format!("that device isn't one of {YOURS}'s"))?;
        let mut hashes = BTreeMap::new();
        if until > 0 {
            let epochs = self.epoch_keys(scope, true).await?;
            let newest = epochs.keys().next_back().copied().unwrap_or(0);
            let (bucket, name, to) = (self.bucket()?, scope.to_string(), hex(&endpoint));
            for (n, epoch) in &epochs {
                let Some(key) = crate::db::app_key(&bucket, epoch, &name, app, *n, *n == newest).await? else { continue };
                // One there already is the same key, or junk its grant's hash passes over.
                crate::db::created(&bucket, &crate::keys::grant_name(&name, app, &to, *n), &crate::cloudflare::seal_to(&seal, &key)?).await?;
                hashes.insert(*n, crate::keys::epoch_hash(&key));
            }
        }
        self.append(&Scope::Global, members::grant(&endpoint, scope, app, until, &hashes)).await?;
        Ok(())
    }

    /// A grant stopped or run out: a new epoch of its scope, sealed as removing a device's is, so the keys the device
    /// held open nothing written after.
    async fn end_grant(&self, me: &Caller, scope: &Scope) -> Result<()> {
        self.new_epoch(me, scope).await.with_context(|| format!("starting {scope}'s new keys"))?;
        Ok(())
    }

    /// Stops, in the background, each grant this device's person made that ran out, as Stop does: a `grant` saying so
    /// and a new epoch of its scope. Now and then, from any of their devices with a node.
    pub(super) fn end_expired_grants(&self) {
        let now = now_ms();
        let Ok(me) = self.user() else { return };
        let ended: Vec<(Key, Scope, String)> = self
            .with(|i| {
                let m = i.r.db();
                if m.client(&self.0.id).is_none_or(|c| c.user != me) {
                    return Ok(Vec::new());
                }
                Ok(m.keeps().into_iter().filter(|((k, s, _), g)| g.by == me && g.until != 0 && g.until <= now && m.machine(k).is_some() && m.may(&me, "keep", &scopes::object(s))).map(|(id, _)| id).collect())
            })
            .unwrap_or_default();
        for id in ended {
            if !self.0.standing.renewing.lock().unwrap().insert(id.clone()) {
                continue;
            }
            let (node, me) = (self.clone(), Caller { key: self.0.id, user: me.clone(), agent: None, token: None });
            tokio::spawn(async move {
                let (endpoint, scope, app) = id.clone();
                let ended = async {
                    node.grant_to(endpoint, &app, &scope, 0).await?;
                    node.end_grant(&me, &scope).await
                };
                if let Err(err) = ended.await {
                    tracing::warn!("ending {}'s grant of {app} in {scope}, which ran out: {err:#}", node.name_of(&endpoint));
                }
                node.0.standing.renewing.lock().unwrap().remove(&id);
            });
        }
    }

    /// Renews, in the background, each grant this device's person made of `app` that's due: as they use it, once a
    /// day at most, or at once when the scope has an epoch the grant's keys don't reach.
    pub(super) fn renew_grants(&self, me: &Caller, app: &str) {
        let now = now_ms();
        let due: Vec<(Key, Scope)> = self
            .with(|i| {
                let m = i.r.db();
                if me.user != i.user || m.client(&self.0.id).is_none() {
                    return Ok(Vec::new());
                }
                let due = |s: &Scope, g: &members::Grant| g.until < now + (GRANT_DAYS - 1) * DAY || g.hashes.keys().next_back().copied().unwrap_or(0) < m.epoch(s);
                Ok(m.keeps_at(now).into_iter().filter(|((_, s, a), g)| a == app && g.by == i.user && due(s, g)).map(|((k, s, _), _)| (k, s)).collect())
            })
            .unwrap_or_default();
        for (endpoint, scope) in due {
            let id = (endpoint, scope.clone(), app.to_string());
            if !self.0.standing.renewing.lock().unwrap().insert(id.clone()) {
                continue;
            }
            let (node, app) = (self.clone(), app.to_string());
            tokio::spawn(async move {
                if let Err(err) = node.grant_to(endpoint, &app, &scope, now_ms() + GRANT_DAYS * DAY).await {
                    tracing::warn!("renewing {}'s grant of {app} in {scope}: {err:#}", node.name_of(&endpoint));
                }
                node.0.standing.renewing.lock().unwrap().remove(&id);
            });
        }
    }

    /// The grant in force for this device to keep `app` running in `scope`, with its keys loaded from the bucket: each
    /// sealed to this device whose hash is the one the grant names, the rest passed over. → until when it lasts.
    pub(super) async fn standing_keys(&self, scope: &Scope, app: &str) -> Result<u64> {
        let now = now_ms();
        let found = self.with(|i| Ok(i.r.db().keeps_at(now).into_iter().find(|((k, s, a), _)| *k == self.0.id && s == scope && a == app).map(|(_, g)| (g.until, g.hashes))))?;
        let (until, hashes) = found.with_context(|| format!("{} doesn't keep {app} running in {scope}", self.device_name()))?;
        let id = (scope.clone(), app.to_string());
        let had = self.0.standing.keys.lock().unwrap().get(&id).cloned().unwrap_or_default();
        let (bucket, secret, me) = (self.bucket()?, self.seal_secret(), hex(&self.0.id));
        let mut keys = crate::db::Keys::new();
        for (n, hash) in &hashes {
            if let Some(key) = had.get(n).filter(|k| crate::keys::epoch_hash(k) == *hash) {
                keys.insert(*n, *key);
                continue;
            }
            let name = crate::keys::grant_name(&scope.to_string(), app, &me, *n);
            let Some(o) = crate::bucket::Bucket::get(&bucket, &name).await? else {
                tracing::warn!("{name} isn't in the bucket yet");
                continue;
            };
            match crate::cloudflare::open_sealed(&secret, &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| crate::keys::epoch_hash(k) == *hash) {
                Some(key) => drop(keys.insert(*n, key)),
                None => tracing::warn!("{name} isn't the key its grant names: passed over"),
            }
        }
        self.0.standing.keys.lock().unwrap().insert(id, keys);
        Ok(until)
    }

    /// Runs one of `app`'s tools here in `scope`, as `user`, with the keys of the grant this device holds to keep it
    /// running there, and no others: standing work's calls (its schedules', from `client`).
    pub async fn standing_call(&self, scope: &Scope, app: &str, user: &str, tool: &str, input: Value, client: &str) -> Result<Value> {
        self.standing_keys(scope, app).await?;
        let me = Caller { key: self.0.id, user: user.to_string(), agent: None, token: Some(grant_token(scope, app)) };
        let req = CallRequest { app: app.to_string(), tool: tool.to_string(), input, scopes: Some(vec![scope.to_string()]), client: Some(client.into()), id: Some(new_id()), ..Default::default() };
        match self.run_or_need(&req, &me).await {
            Ok(Reply::Ok(v)) => Ok(v),
            Ok(Reply::Error(e) | Reply::Elsewhere(e)) => Err(anyhow!(e)),
            Ok(Reply::Needs(need)) | Err(need) => Err(need.into()),
        }
    }

    /// The keys a grant's token opens of a database: only its app's in its scope, while the grant is in force.
    pub(super) fn standing_handed(&self, token: &str, scope: &Scope, app: &str, database: &str) -> crate::db::Keys {
        let Some((s, a)) = grant_of(token) else { return Default::default() };
        if s != *scope || a != app || self.standing_until(token) <= now_ms() {
            return Default::default();
        }
        let kept = self.0.standing.keys.lock().unwrap();
        kept.get(&(s, a.to_string())).map(|by| by.iter().map(|(n, k)| (*n, crate::cloudflare::database_key(k, database))).collect()).unwrap_or_default()
    }

    /// Until when a grant's token's grant lasts: 0 once it's over.
    pub(super) fn standing_until(&self, token: &str) -> u64 {
        let Some((scope, app)) = grant_of(token) else { return 0 };
        let now = now_ms();
        self.with(|i| Ok(i.r.db().keeps_at(now).into_iter().find(|((k, s, a), _)| *k == self.0.id && *s == scope && a == app).map_or(0, |(_, g)| g.until))).unwrap_or(0)
    }

    /// Forgets the keys of grants no longer in force here, and the databases they had open.
    pub(super) fn forget_standing(&self) {
        let now = now_ms();
        let held: std::collections::HashSet<(Scope, String)> = self.with(|i| Ok(i.r.db().keeps_at(now).into_iter().filter(|((k, ..), _)| *k == self.0.id).map(|((_, s, a), _)| (s, a)).collect())).unwrap_or_default();
        let gone: Vec<(Scope, String)> = {
            let mut kept = self.0.standing.keys.lock().unwrap();
            let gone = kept.keys().filter(|id| !held.contains(*id)).cloned().collect();
            kept.retain(|id, _| held.contains(id));
            gone
        };
        if gone.is_empty() {
            return;
        }
        for (scope, app) in gone {
            tracing::info!("{} no longer keeps {app} running in {scope}: its keys are forgotten", self.device_name());
            self.close_dbs(|s, db| *s == scope && db.split('/').next() == Some(app.as_str()));
        }
    }

    /// An app's title, or its name.
    pub(super) async fn title_of(&self, app: &str) -> String {
        self.0.apps.describe(app).await.ok().and_then(|d| d["title"].as_str().map(str::to_string)).unwrap_or_else(|| app.to_string())
    }
}
