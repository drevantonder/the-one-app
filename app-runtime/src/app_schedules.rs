//! App schedules (GLOSSARY.md): an app's schedules run in a scope on a machine its user chose, with that app's keys
//! there sealed to the machine (`keys::app_schedules_key_name`) while the log says so. Their user's devices set them
//! (`settings`), and renew them as they use the app, so app schedules nobody uses, or whose machine is off, end by
//! themselves; a machine that holds them opens that app's databases there with their keys alone, and the Vault's,
//! whose key in each epoch is kept under the app's (`keys::app_schedules_vault_name`): what a call of the app is handed
//! (`keys::may_hand`), never its other apps'. They're an `app-schedules` step.

use super::*;

#[derive(Default)]
pub(super) struct State {
    /// The keys app schedules gave this device, by (scope, app): kept while they're in force.
    keys: Mutex<HashMap<(Scope, String), Held>>,
    /// Held while this machine renews its user's app schedules.
    renewing: tokio::sync::Mutex<()>,
}

/// What app schedules of an app in a scope hold, each by epoch.
#[derive(Clone, Default)]
struct Held {
    /// The app's key, sealed to the device.
    app: database::stored::Keys,
    /// The Vault's database's key, kept under the app's.
    vault: database::stored::Keys,
    /// The epochs the bucket had no Vault key under the app's for, as of the app schedules lasting `until`: one is
    /// written as they're set or renewed, which moves that on.
    no_vault: std::collections::BTreeSet<u32>,
    /// The epochs the bucket had no key of the app's sealed to the device for, as of the same: one is sealed as they're
    /// set or renewed.
    no_app: std::collections::BTreeSet<u32>,
    until: u64,
}

impl Host {
    /// Renews, in the background, each of this machine's user's app schedules of `app` that's due, and stops each of
    /// theirs that ran out (`settings::renew_app_schedules`): as they use it here, and now and then. One renewal at a
    /// time; one asked meanwhile is passed over.
    pub(super) fn renew_app_schedules(&self, app: &str) {
        let mine = self.with(|i| Ok(i.r.db().client(&self.0.holds.device())?.is_some_and(|c| c.user == i.user)));
        if !mine.unwrap_or(false) {
            return;
        }
        let (host, app) = (self.clone(), app.to_string());
        tokio::spawn(async move {
            let Ok(_renewing) = host.0.app_schedules.renewing.try_lock() else { return };
            scope::settings::renew_app_schedules(&host, &app).await;
        });
    }

    /// The app schedules in force for this device to run `app`'s schedules in `scope`, with their keys: the app's its
    /// holder opens (`Holds::app_schedules_keys`), and the Vault's database's each of those opens. → until when they last.
    pub(super) async fn app_schedules_keys(&self, scope: &Scope, app: &str) -> Result<u64> {
        let (now, me) = (now_ms(), self.0.holds.device());
        let found = self.with(|i| Ok(i.r.db().app_schedules_at(now)?.into_iter().find(|((k, s, a), _)| *k == me && s == scope && a == app).map(|(_, g)| (g.until, g.hashes))))?;
        let (until, hashes) = found.with_context(|| format!("{} doesn't run {app}'s schedules in {scope}", self.device_name()))?;
        let id = (scope.clone(), app.to_string());
        let had = self.0.app_schedules.keys.lock().unwrap().get(&id).cloned().unwrap_or_default();
        let mut held = Held { until, ..Held::default() };
        let mut missing = BTreeMap::new();
        for (n, hash) in hashes {
            match had.app.get(&n).filter(|k| key::epoch_hash(k) == hash) {
                Some(key) => drop(held.app.insert(n, *key)),
                None if had.until == until && had.no_app.contains(&n) => drop(held.no_app.insert(n)),
                None => drop(missing.insert(n, hash)),
            }
        }
        let bucket = self.bucket()?;
        held.app.extend(self.0.holds.app_schedules_keys(&bucket, scope, app, &missing).await?);
        held.no_app.extend(missing.keys().filter(|n| !held.app.contains_key(n)));
        for (n, key) in &held.app {
            if let Some(vault) = had.vault.get(n).filter(|_| !missing.contains_key(n)) {
                held.vault.insert(*n, *vault);
                continue;
            }
            if had.until == until && had.no_vault.contains(n) && !missing.contains_key(n) {
                held.no_vault.insert(*n);
                continue;
            }
            let name = database::keys::app_schedules_vault_name(&scope.to_string(), app, *n);
            let Some(o) = bucket::Bucket::get(&bucket, &name).await? else {
                held.no_vault.insert(*n);
                continue;
            };
            match key::open(key, &name, &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()) {
                Some(vault) => drop(held.vault.insert(*n, vault)),
                None => tracing::warn!("{name} doesn't open with {app}'s key: passed over"),
            }
        }
        self.0.app_schedules.keys.lock().unwrap().insert(id, held);
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
            Ok(Reply::Error(e, code)) => Err(custom_build::protocol::Coded::or_plain(code, e)),
            Ok(Reply::Elsewhere(e)) => Err(anyhow!(e)),
            Ok(Reply::Needs(need)) | Err(need) => Err(need.into()),
        }
    }

    /// The keys app schedules of `theirs` in `in_` open of a database: their app's and the Vault's, only in their
    /// scope, while they're in force.
    pub(super) fn app_schedules_handed(&self, in_: &Scope, theirs: &str, scope: &Scope, app: &str, database: &str) -> database::stored::Keys {
        if in_ != scope || self.app_schedules_until(scope, theirs) <= now_ms() {
            return Default::default();
        }
        let kept = self.0.app_schedules.keys.lock().unwrap();
        let Some(held) = kept.get(&(scope.clone(), theirs.to_string())) else { return Default::default() };
        match key::CoreDb::of(app, database) {
            Some(key::CoreDb::Vault) => held.vault.clone(),
            _ if app == theirs => held.app.iter().map(|(n, k)| (*n, key::database_key(k, app, database))).collect(),
            _ => Default::default(),
        }
    }

    /// Until when `app`'s schedules in `scope` here last: 0 once they're over.
    pub(super) fn app_schedules_until(&self, scope: &Scope, app: &str) -> u64 {
        let now = now_ms();
        self.with_or(0, |i| {
            let me = self.0.holds.device();
            Ok(i.r.db().app_schedules_at(now)?.into_iter().find(|((k, s, a), _)| *k == me && s == scope && a == app).map_or(0, |(_, g)| g.until))
        })
    }

    /// Forgets the keys of app schedules no longer in force here, and the databases they had open.
    pub(super) fn forget_app_schedules(&self) {
        let now = now_ms();
        let held: std::collections::HashSet<(Scope, String)> = self.with_or(Default::default(), |i| {
            let me = self.0.holds.device();
            Ok(i.r.db().app_schedules_at(now)?.into_iter().filter(|((k, ..), _)| *k == me).map(|((_, s, a), _)| (s, a)).collect())
        });
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
}
