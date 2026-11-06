//! App schedules (GLOSSARY.md): an app's schedules run in a scope on a machine its person chose, with that app's keys
//! there sealed to the machine (`keys::app_schedules_key_name`) while the log says so. This device's person sets them
//! here, and renews them as they use the app, so app schedules nobody uses, or whose machine is off, end by themselves;
//! a machine that holds them opens that app's databases there with their keys alone, as a call opens what it was
//! handed, and never its other apps'. They're an `app-schedules` step.

use crate::config::YOURS;
use super::*;

/// How long app schedules last, unless its person's device renews it as they use the app.
pub const APP_SCHEDULES_DAYS: u64 = 30;
const DAY: u64 = 86_400_000;

#[derive(Default)]
pub(super) struct State {
    /// The app keys app schedules sealed to this device, by (scope, app), each by epoch: kept while they're in force.
    keys: Mutex<HashMap<(Scope, String), crate::database::stored::Keys>>,
    /// The Vault secret keys sealed to this machine, by scope and number, each the one its `vault` entry names.
    pub(super) vault: Mutex<HashMap<(Scope, u32), [u8; 32]>>,
    /// The app schedules this device is renewing now.
    renewing: Mutex<std::collections::HashSet<(Key, Scope, String)>>,
}

impl Node {
    /// Runs `app`'s schedules in `scope` on `device` for `days` (0 stops them): as this device's person, who must be the
    /// scope's or, for a group, its admin. Seals the app's key in each epoch this device opens to that device, then
    /// writes the app schedules naming each one's hash. → what it did, in words.
    pub async fn set_app_schedules(&self, me: &Caller, device: &str, app: &str, scope: &Scope, days: u64) -> Result<Value> {
        ensure!(me.user == self.user()?, "{} sets app schedules only for its own person", self.device_name());
        let endpoint = self.find_device(device)?;
        let (title, name) = (self.title_of(app).await, self.name_of(&endpoint));
        ensure!(self.with(|i| Ok(i.r.db().app(app).is_some()))?, "no app called {app}");
        let until = match days {
            0 => 0,
            days => now_ms() + days.min(APP_SCHEDULES_DAYS) * DAY,
        };
        self.write_app_schedules(endpoint, app, scope, until).await?;
        if until == 0 {
            self.end_app_schedules(me, scope).await?;
        }
        Ok(match until {
            0 => json!({ "said": format!("{name} no longer runs {title}'s schedules, and what's written from now on is under new keys it doesn't have.") }),
            until => json!({ "said": format!("{name} runs {title}'s schedules for you, for {} days, and longer as you use it.", days.min(APP_SCHEDULES_DAYS)), "until": until }),
        })
    }

    /// Writes app schedules of `app` in `scope` on `endpoint` until `until` (0: no longer), with its keys sealed first.
    async fn write_app_schedules(&self, endpoint: Key, app: &str, scope: &Scope, until: u64) -> Result<()> {
        let (seal, mine, may) = self.with(|i| {
            let m = i.r.db();
            Ok((m.seal_key(&endpoint), m.client(&self.0.id).is_some(), m.may(&i.user, "choose-devices", &scopes::object(&scope))))
        })?;
        ensure!(mine, "{} isn't one of its person's own devices, which are what set app schedules", self.device_name());
        ensure!(may, "only {scope}'s person, or its group's admins, choose where its apps' schedules run");
        let seal = seal.with_context(|| format!("that device isn't one of {YOURS}'s"))?;
        let mut hashes = BTreeMap::new();
        if until > 0 {
            let epochs = self.epoch_keys(scope, true).await?;
            let newest = epochs.keys().next_back().copied().unwrap_or(0);
            let (bucket, name, to) = (self.bucket()?, scope.to_string(), hex(&endpoint));
            for (n, epoch) in &epochs {
                let Some(key) = crate::keys::app_key(&bucket, epoch, &name, app, *n, *n == newest).await? else { continue };
                // One there already is the same key, or junk the hash written beside it passes over.
                crate::database::stored::created(&bucket, &crate::keys::app_schedules_key_name(&name, app, &to, *n), &crate::keys::seal_to(&seal, &key)?).await?;
                hashes.insert(*n, crate::keys::epoch_hash(&key));
            }
        }
        self.append(&Scope::Global, content::app_schedules(&endpoint, scope, app, until, &hashes)).await?;
        Ok(())
    }

    /// App schedules stopped or run out: a new epoch of its scope, sealed as removing a device's is, so the keys the device
    /// held open nothing written after.
    async fn end_app_schedules(&self, me: &Caller, scope: &Scope) -> Result<()> {
        self.new_epoch(me, scope).await.with_context(|| format!("starting {scope}'s new keys"))?;
        Ok(())
    }

    /// Stops, in the background, each app schedules this device's person set that ran out, as Stop does: saying so
    /// and a new epoch of its scope. Now and then, from any of their devices with a node.
    pub(super) fn end_expired_app_schedules(&self) {
        let now = now_ms();
        let Ok(me) = self.user() else { return };
        let ended: Vec<(Key, Scope, String)> = self
            .with(|i| {
                let m = i.r.db();
                if m.client(&self.0.id).is_none_or(|c| c.user != me) {
                    return Ok(Vec::new());
                }
                Ok(m.app_schedules().into_iter().filter(|((k, s, _), g)| g.by == me && g.until != 0 && g.until <= now && m.machine(k).is_some() && m.may(&me, "choose-devices", &scopes::object(s))).map(|(id, _)| id).collect())
            })
            .unwrap_or_default();
        for id in ended {
            if !self.0.app_schedules.renewing.lock().unwrap().insert(id.clone()) {
                continue;
            }
            let (node, me) = (self.clone(), Caller::own(self.0.id, me.clone()));
            tokio::spawn(async move {
                let (endpoint, scope, app) = id.clone();
                let ended = async {
                    node.write_app_schedules(endpoint, &app, &scope, 0).await?;
                    node.end_app_schedules(&me, &scope).await
                };
                if let Err(err) = ended.await {
                    tracing::warn!("ending {}'s app schedules of {app} in {scope}, which ran out: {err:#}", node.name_of(&endpoint));
                }
                node.0.app_schedules.renewing.lock().unwrap().remove(&id);
            });
        }
    }

    /// Renews, in the background, each app schedules this device's person set of `app` that's due: as they use it,
    /// once a day at most, or at once when the scope has an epoch their keys don't reach.
    pub(super) fn renew_app_schedules(&self, me: &Caller, app: &str) {
        let now = now_ms();
        let due: Vec<(Key, Scope)> = self
            .with(|i| {
                let m = i.r.db();
                if me.user != i.user || m.client(&self.0.id).is_none() {
                    return Ok(Vec::new());
                }
                let due = |s: &Scope, g: &scopes::AppSchedules| g.until < now + (APP_SCHEDULES_DAYS - 1) * DAY || g.hashes.keys().next_back().copied().unwrap_or(0) < m.epoch(s);
                Ok(m.app_schedules_at(now).into_iter().filter(|((_, s, a), g)| a == app && g.by == i.user && due(s, g)).map(|((k, s, _), _)| (k, s)).collect())
            })
            .unwrap_or_default();
        for (endpoint, scope) in due {
            let id = (endpoint, scope.clone(), app.to_string());
            if !self.0.app_schedules.renewing.lock().unwrap().insert(id.clone()) {
                continue;
            }
            let (node, app) = (self.clone(), app.to_string());
            tokio::spawn(async move {
                if let Err(err) = node.write_app_schedules(endpoint, &app, &scope, now_ms() + APP_SCHEDULES_DAYS * DAY).await {
                    tracing::warn!("renewing {}'s app schedules of {app} in {scope}: {err:#}", node.name_of(&endpoint));
                }
                node.0.app_schedules.renewing.lock().unwrap().remove(&id);
            });
        }
    }

    /// The app schedules in force for this device to run `app`'s schedules in `scope`, with their keys loaded from the
    /// bucket: each sealed to this device whose hash is the one they name, the rest passed over. → until when it lasts.
    pub(super) async fn app_schedules_keys(&self, scope: &Scope, app: &str) -> Result<u64> {
        let now = now_ms();
        let found = self.with(|i| Ok(i.r.db().app_schedules_at(now).into_iter().find(|((k, s, a), _)| *k == self.0.id && s == scope && a == app).map(|(_, g)| (g.until, g.hashes))))?;
        let (until, hashes) = found.with_context(|| format!("{} doesn't run {app}'s schedules in {scope}", self.device_name()))?;
        let id = (scope.clone(), app.to_string());
        let had = self.0.app_schedules.keys.lock().unwrap().get(&id).cloned().unwrap_or_default();
        let (bucket, secret, me) = (self.bucket()?, self.seal_secret(), hex(&self.0.id));
        let mut keys = crate::database::stored::Keys::new();
        for (n, hash) in &hashes {
            if let Some(key) = had.get(n).filter(|k| crate::keys::epoch_hash(k) == *hash) {
                keys.insert(*n, *key);
                continue;
            }
            let name = crate::keys::app_schedules_key_name(&scope.to_string(), app, &me, *n);
            let before = crate::keys::app_schedules_key_name_before(&scope.to_string(), app, &me, *n);
            let found = match crate::bucket::Bucket::get(&bucket, &name).await? {
                Some(o) => Some(o),
                None => crate::bucket::Bucket::get(&bucket, &before).await?,
            };
            let Some(o) = found else {
                tracing::warn!("{name} isn't in the bucket yet");
                continue;
            };
            match crate::keys::open_sealed(&secret, &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| crate::keys::epoch_hash(k) == *hash) {
                Some(key) => drop(keys.insert(*n, key)),
                None => tracing::warn!("{name} isn't the key its app schedules name: passed over"),
            }
        }
        self.0.app_schedules.keys.lock().unwrap().insert(id, keys);
        Ok(until)
    }

    /// Runs one of `app`'s tools here in `scope`, as `user`, with the keys of the app schedules this device holds
    /// there, and no others: its schedules' calls (from `client`).
    pub async fn app_schedules_call(&self, scope: &Scope, app: &str, user: &str, tool: &str, input: Value, client: &str) -> Result<Value> {
        self.app_schedules_keys(scope, app).await?;
        let me = Caller { key: self.0.id, user: user.to_string(), agent: None, credential: Credential::AppSchedules { scope: scope.clone(), app: app.to_string() } };
        let req = CallRequest { app: app.to_string(), tool: tool.to_string(), input, scopes: Some(vec![scope.to_string()]), client: Some(client.into()), id: Some(new_id()), ..Default::default() };
        match self.run_or_need(&req, &me).await {
            Ok(Reply::Ok(v)) => Ok(v),
            Ok(Reply::Error(e, code)) => Err(crate::protocol::Coded::or_plain(code, e)),
            Ok(Reply::Elsewhere(e)) => Err(anyhow!(e)),
            Ok(Reply::Needs(need)) | Err(need) => Err(need.into()),
        }
    }

    /// The keys app schedules of `theirs` in `in_` open of a database: only their app's in their scope, while they're in
    /// force.
    pub(super) fn app_schedules_handed(&self, in_: &Scope, theirs: &str, scope: &Scope, app: &str, database: &str) -> crate::database::stored::Keys {
        if in_ != scope || theirs != app || self.app_schedules_until(scope, app) <= now_ms() {
            return Default::default();
        }
        let kept = self.0.app_schedules.keys.lock().unwrap();
        kept.get(&(scope.clone(), app.to_string())).map(|by| by.iter().map(|(n, k)| (*n, crate::keys::database_key(k, app, database))).collect()).unwrap_or_default()
    }

    /// Until when `app`'s schedules in `scope` here last: 0 once they're over.
    pub(super) fn app_schedules_until(&self, scope: &Scope, app: &str) -> u64 {
        let now = now_ms();
        self.with(|i| Ok(i.r.db().app_schedules_at(now).into_iter().find(|((k, s, a), _)| *k == self.0.id && s == scope && a == app).map_or(0, |(_, g)| g.until))).unwrap_or(0)
    }

    /// Forgets the keys of app schedules no longer in force here, and the databases they had open.
    pub(super) fn forget_app_schedules(&self) {
        let now = now_ms();
        let held: std::collections::HashSet<(Scope, String)> = self.with(|i| Ok(i.r.db().app_schedules_at(now).into_iter().filter(|((k, ..), _)| *k == self.0.id).map(|((_, s, a), _)| (s, a)).collect())).unwrap_or_default();
        let gone: Vec<(Scope, String)> = {
            let mut kept = self.0.app_schedules.keys.lock().unwrap();
            let gone = kept.keys().filter(|id| !held.contains(*id)).cloned().collect();
            kept.retain(|id, _| held.contains(id));
            gone
        };
        if gone.is_empty() {
            return;
        }
        for (scope, app) in gone {
            tracing::info!("{} no longer runs {app}'s schedules in {scope}: its keys are forgotten", self.device_name());
            self.close_dbs(|s, db| *s == scope && db.split('/').next() == Some(app.as_str()));
        }
    }

    pub(super) async fn title_of(&self, app: &str) -> String {
        self.0.apps.describe(app).await.ok().and_then(|d| d["title"].as_str().map(str::to_string)).unwrap_or_else(|| app.to_string())
    }
}
