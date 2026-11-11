//! Device services, as the node knows them: which run on which device, registering them, and switching a device's
//! parts on and off.

use crate::config::YOURS;
use super::*;

pub(crate) mod routes;
pub mod cli;

impl Node {
    /// Asks the app `app` what it is, so its service is listed as that and not as itself.
    pub(super) async fn learn_app(&self, app: &str) {
        let _ = self.0.apps.describe_any(app).await;
    }

    /// A service's description, as its app says it is with what the service says of its machine over it.
    pub(super) fn listed_as_app(&self, app: &str, machine: Value) -> Value {
        let mut d = self.0.apps.identity(app).unwrap_or_else(|| json!({}));
        d["name"] = json!(app);
        for (k, v) in machine.as_object().into_iter().flatten() {
            d[k] = v.clone();
        }
        d
    }

    /// Switches a part of a device's oned (a service, or `apps`, the app service) on or off there (this device if none
    /// is named), for the device's user: that device's oned starts or stops it.
    pub async fn set_service(&self, caller: &Caller, device: Option<&str>, name: &str, on: bool) -> Result<()> {
        ensure!(device.is_some() || caller.key == self.0.id, "say which device");
        let device = device.map(str::to_string).unwrap_or_else(|| self.device_name());
        let content = self.with(|i| {
            let scope = if on { added_scope(i, &caller.user, name)? } else { None };
            crate::settings::service(i.r.db(), &caller.user, &device, name, on, scope.as_ref())
        })?;
        self.member_content(caller, content).await?;
        Ok(())
    }

    /// Whether a part of this device is switched off, by its latest `service` entry.
    pub(super) fn off(m: &ScopeDb, device: &Key, name: &str) -> Result<bool> {
        Ok(m.switch(&m.device_of(device), name)?.is_some_and(|s| !s.on))
    }

    /// What this device's oned keeps its parts by (`oned::keep_parts`): the network they're in, which are switched on
    /// here (each with when it was: switched on again while it's on, it starts afresh) and off, and whether to erase
    /// what the device keeps.
    pub fn parts_wanted(&self) -> Result<crate::oned::Wanted> {
        let address = self.address()?;
        let device = self.0.holds.device();
        self.with(|i| {
            let root = i.r.root().with_context(|| format!("this device isn't in {YOURS} yet"))?.to_hex();
            let switches = i.r.db().switches()?;
            let here = switches.into_iter().filter(|((k, _), _)| *k == device);
            let (on, off): (Vec<_>, Vec<_>) = here.partition(|(_, s)| s.on);
            Ok(crate::oned::Wanted {
                network: crate::services::endpoint::Network { address: address.clone(), root, user: i.user.clone() },
                device: self.device_name(),
                on: on.into_iter().map(|((_, name), s)| (name, s.time)).collect(),
                off: off.into_iter().map(|((_, name), _)| name).collect(),
                erase: self.0.erasing.load(std::sync::atomic::Ordering::SeqCst),
            })
        })
    }

    /// Each service switched on here before a service named the scope of the app it runs for, switched on again naming
    /// it, once: so removing the app, or moving its service, reaches it. So is each this device carries (`carried`, a
    /// Mac's helpers) with no record here yet, once its app is added: the app's added where this node runs it. Each
    /// goes on running throughout.
    pub async fn name_services_scopes(&self, carried: &[String]) {
        let Ok(caller) = Caller::local(self) else { return };
        let named = self.with(|i| {
            let m = i.r.db();
            let switches = m.switches()?;
            let unnamed = switches.iter().filter(|((k, name), s)| *k == self.0.holds.device() && s.on && s.scope.is_none() && name != scopes::APPS_PART).map(|((_, name), _)| name.clone());
            let unrecorded = carried.iter().filter(|name| !switches.contains_key(&(self.0.holds.device(), name.to_string()))).cloned();
            let mut entries = Vec::new();
            for name in unnamed.chain(unrecorded) {
                if let Some(scope) = added_scope(i, &caller.user, &name)?
                    && let Ok(entry) = crate::settings::service(m, &caller.user, &hex(&self.0.holds.device()), &name, true, Some(&scope))
                {
                    entries.push(entry);
                }
            }
            Ok(entries)
        });
        for content in named.unwrap_or_default() {
            if let Err(err) = self.member_content(&caller, content).await {
                tracing::info!("naming a service's scope: {err:#}");
            }
        }
    }

    /// Says hello to this device's part `name`, a service endpoint of its own: → whether it answered.
    pub async fn hello_part(&self, name: &str) -> bool {
        let keys: Vec<Key> = self.with_or(Default::default(), |i| Ok(i.r.db().offering(name)?.into_iter().filter(|s| s.device == Some(self.0.holds.device())).map(|s| s.key).collect()));
        let mut up = false;
        for key in keys {
            up |= tokio::time::timeout(Duration::from_secs(3), self.hello_service_now(&key)).await.unwrap_or(false);
        }
        up
    }

    /// What the app `name` this device runs says of its service's wrapper (`service: { network, user }` in its
    /// app.js), or why it can't be read: the app isn't running here, or its deploy says nothing of it.
    pub async fn service_declared(&self, name: &str) -> Result<crate::services::sandbox::Declared> {
        ensure!(self.0.apps.has(name), "its app isn't running on this device");
        let d = self.0.apps.describe(name).await?;
        ensure!(d["service"].is_object(), "its app doesn't say what its service may reach");
        Ok(serde_json::from_value(d["service"].clone())?)
    }

    pub fn app_runtime(&self) -> &crate::app::runtime::Apps {
        &self.0.apps
    }

    /// Whether this device runs apps' calls: unless its user switched its app service off.
    pub fn runs_apps(&self) -> bool {
        // What it can't read, it keeps running.
        self.with_or(true, |i| Ok(!Self::off(i.r.db(), &self.0.id, crate::database::scopes::APPS_PART)?))
    }
}

/// The scope whose added app `name` is, as `user` sees it, for a service switched on with none said: their own, if
/// it's added there, then everyone's, then one of their groups'.
pub(super) fn added_scope(i: &Network, user: &str, name: &str) -> Result<Option<Scope>> {
    let added = |s: &Scope| i.views.get(&s.to_string()).is_some_and(|v| v.added.contains(name));
    let own = Scope::User(user.to_string());
    let db = i.r.db();
    let mut groups = Vec::new();
    for g in db.groups()? {
        if db.group_role(user, &g.id)?.is_some() {
            groups.push(Scope::Group(g.id));
        }
    }
    Ok([own, Scope::Global].into_iter().chain(groups).find(|s| added(s) && db.may(user, "add-apps", &scopes::object(s))))
}
