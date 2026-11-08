//! App schedules (GLOSSARY.md): an app's schedules run in a scope on a machine its user chose, with that app's keys
//! there sealed to the machine (`keys::app_schedules_key_name`) while the log says so. Their user's devices set them
//! (`settings`), and renew them as they use the app, so app schedules nobody uses, or whose machine is off, end by
//! themselves; a machine that holds them opens that app's databases there with their keys alone, as a call opens what
//! it was handed, and never its other apps'. They're an `app-schedules` step.

use super::*;

#[derive(Default)]
pub(super) struct State {
    /// The app keys app schedules sealed to this device, by (scope, app), each by epoch: kept while they're in force.
    keys: Mutex<HashMap<(Scope, String), crate::database::stored::Keys>>,
    /// The Vault secret keys sealed to this machine, by scope and number, each the one its `vault` entry names.
    pub(super) vault: Mutex<HashMap<(Scope, u32), [u8; 32]>>,
    /// Held while this machine renews its user's app schedules.
    renewing: tokio::sync::Mutex<()>,
}

impl Node {
    /// Renews, in the background, each of this machine's user's app schedules of `app` that's due, and stops each of
    /// theirs that ran out (`settings::renew_app_schedules`): as they use it here, and now and then. One renewal at a
    /// time; one asked meanwhile is passed over.
    pub(super) fn renew_app_schedules(&self, app: &str) {
        let mine = self.with(|i| Ok(i.r.db().client(&self.0.id)?.is_some_and(|c| c.user == i.user)));
        if !mine.unwrap_or(false) {
            return;
        }
        let (node, app) = (self.clone(), app.to_string());
        tokio::spawn(async move {
            let Ok(_renewing) = node.0.app_schedules.renewing.try_lock() else { return };
            crate::settings::renew_app_schedules(&node, &app).await;
        });
    }

    /// The app schedules in force for this device to run `app`'s schedules in `scope`, with their keys loaded from the
    /// bucket: each sealed to this device whose hash is the one they name, the rest passed over. → until when it lasts.
    pub(super) async fn app_schedules_keys(&self, scope: &Scope, app: &str) -> Result<u64> {
        let now = now_ms();
        let found = self.with(|i| Ok(i.r.db().app_schedules_at(now)?.into_iter().find(|((k, s, a), _)| *k == self.0.id && s == scope && a == app).map(|(_, g)| (g.until, g.hashes))))?;
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
        self.with_or(0, |i| Ok(i.r.db().app_schedules_at(now)?.into_iter().find(|((k, s, a), _)| *k == self.0.id && s == scope && a == app).map_or(0, |(_, g)| g.until)))
    }

    /// Forgets the keys of app schedules no longer in force here, and the databases they had open.
    pub(super) fn forget_app_schedules(&self) {
        let now = now_ms();
        let held: std::collections::HashSet<(Scope, String)> = self.with_or(Default::default(), |i| Ok(i.r.db().app_schedules_at(now)?.into_iter().filter(|((k, ..), _)| *k == self.0.id).map(|((_, s, a), _)| (s, a)).collect()));
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
