//! Device services, as the node knows them: which run on which device, registering them, and switching a device's
//! parts on and off.

use crate::config::YOURS;
use super::*;
use crate::state::Part;

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
        self.say_hello();
        Ok(())
    }

    /// Whether a part of this device is switched off, by its latest `service` entry.
    pub(super) fn off(m: &ScopeDb, device: &Key, name: &str) -> Result<bool> {
        Ok(m.switch(&m.device_of(device), name)?.is_some_and(|s| !s.on))
    }

    /// The network this device's parts are in, and those switched on here, each with when it was: switched on again
    /// while it's on, it starts afresh.
    pub fn parts_on(&self) -> Result<(crate::services::endpoint::Network, BTreeMap<String, u64>)> {
        let address = self.address()?;
        self.with(|i| {
            let root = i.r.root().with_context(|| format!("this device isn't in {YOURS} yet"))?.to_hex();
            let on = i.r.db().switches()?.into_iter().filter(|((k, _), s)| *k == self.0.holds.device() && s.on).map(|((_, name), s)| (name, s.time)).collect();
            Ok((crate::services::endpoint::Network { address: address.clone(), root, user: i.user.clone() }, on))
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

    /// What a part of this device is doing changed: its screens and the other devices are told.
    pub fn parts_changed(&self) {
        self.emit(Event::Devices);
        self.say_hello();
    }

    /// Says hello to this device's part `name`, a service endpoint of its own, which answers once it's up.
    pub async fn hello_part(&self, name: &str) {
        let keys: Vec<Key> = self.with_or(Default::default(), |i| Ok(i.r.db().offering(name)?.into_iter().filter(|s| s.device == Some(self.0.holds.device())).map(|s| s.key).collect()));
        for key in keys {
            let _ = tokio::time::timeout(Duration::from_secs(3), self.hello_service_now(&key)).await;
        }
    }

    /// Whether this device's part `name`, a service endpoint of its own, has answered it since `since`.
    pub fn part_up(&self, name: &str, since: Instant) -> bool {
        self.with_or(false, |i| Ok(i.r.db().offering(name)?.into_iter().any(|s| s.device == Some(self.0.holds.device()) && i.peers.get(&s.key).is_some_and(|p| p.seen >= since))))
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

    /// What this device can run, as its hello says: each part (`apps`, the app service; the services built into it;
    /// those registered with it, as the Mac app's are, which it offers while switched off too; and those oned runs as
    /// its processes, its own or fetched from a release when switched on), with whether it's switched on and, while
    /// it isn't simply on or off, what it's doing (`doing`) or what went wrong (`problem`), and its title if it gives
    /// one. Devices that don't say this run only what they list as serving.
    pub(super) fn parts_here(&self, i: &Network) -> Result<Vec<Part>> {
        let m = i.r.db();
        let on = |name: &str| -> Result<bool> { Ok(!Self::off(m, &self.0.id, name)?) };
        let mut out: BTreeMap<String, Part> = BTreeMap::new();
        out.insert(scopes::APPS_PART.into(), Part { on: on(scopes::APPS_PART)?, ..Part::off(scopes::APPS_PART) });
        // A process of oned's is on only by its entry: with none, it's off; but a Mac helper the Mac app carries is on
        // unless switched off, as oned runs it.
        let switched = |name: &str| -> Result<bool> { Ok(m.switch(&self.0.holds.device(), name)?.is_some_and(|s| s.on) || (crate::oned::helper(name).is_some() && on(name)?)) };
        let (processes, now) = self.0.parts.now();
        for name in &processes {
            out.insert(name.clone(), Part { on: switched(name)?, ..Part::off(name) });
        }
        for (name, now) in now {
            if let Some(part) = out.get_mut(&name) {
                (part.doing, part.problem) = (now.doing.map(str::to_string), now.problem);
            }
        }
        for (name, held) in self.0.parts.held() {
            if let Some(part) = out.get_mut(&name) {
                (part.sandbox, part.user) = (held.sandbox, held.user);
            }
        }
        Ok(out.into_values().collect())
    }

    /// Why a service isn't served here, if its user switched it off.
    pub fn removed_here(&self, name: &str) -> Option<String> {
        self.with_or(None, |i| {
            let m = i.r.db();
            Ok(Self::off(m, &self.0.id, name)?.then(|| format!("{} is switched off on {}; its user switches it on in Settings › Devices", self.service_title(name), self.device_name())))
        })
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
