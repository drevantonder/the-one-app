//! Device services, as the node knows them: which run on which device, registering them, and switching a device's
//! parts on and off.

use crate::config::YOURS;
use super::*;

pub(crate) mod routes;

/// What registered with this device before, in its home.
const OFFERED: &str = "offered.json";

impl Node {
    pub fn services(&self) -> &crate::services::Services {
        &self.0.services
    }

    /// Registers a service, which says only what's about its machine; the node says so in its hello to the other
    /// devices, and to shells when that changes it: an app of its name shows its picture (a Mac app's icon).
    pub async fn register_service(&self, app: &str, describe: Value) -> Result<()> {
        self.0.services.offer(app);
        self.learn_app(app).await;
        // Kept, so its switch shows while what offers it (the Mac app) isn't running.
        let title = json!(self.service_title(app));
        let mut kept = self.offered_before();
        if kept.get(app) != Some(&title) {
            kept.insert(app.to_string(), title);
            let _ = std::fs::write(self.0.dir.join(OFFERED), serde_json::to_vec(&kept).unwrap_or_default());
        }
        if let Some(why) = self.removed_here(app) {
            bail!("{why}");
        }
        self.stand_down(app)?;
        if let Some(domain) = self.listed_as_app(app, describe.clone())["domain"].as_str() {
            self.logo(domain, app);
        }
        if self.0.services.register(app, describe) {
            self.emit(Event::Changed { apps: vec![app.to_string()] });
        }
        self.say_hello();
        Ok(())
    }

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
    /// is named), for the device's person: that device's oned starts or stops it.
    pub async fn set_service(&self, caller: &Caller, device: Option<&str>, name: &str, on: bool) -> Result<()> {
        ensure!(device.is_some() || caller.key == self.0.id, "say which device");
        let device = device.map(str::to_string).unwrap_or_else(|| self.device_name());
        let content = self.with(|i| service_entry(i.r.db(), &caller.user, &device, name, on, added_scope(i, &caller.user, name).filter(|_| on).as_ref()))?;
        self.member_content(caller, content).await?;
        self.say_hello();
        Ok(())
    }

    /// Whether a part of this device is switched off, by its latest `service` entry.
    pub(super) fn off(m: &ScopeDb, device: &Key, name: &str) -> bool {
        m.switch(device, name).is_some_and(|s| !s.on)
    }

    /// The network this device's parts are in, and those switched on here, each with when it was: switched on again
    /// while it's on, it starts afresh.
    pub fn parts_on(&self) -> Result<(crate::services::endpoint::Network, BTreeMap<String, u64>)> {
        let address = self.address()?;
        self.with(|i| {
            let root = i.r.root().with_context(|| format!("this device isn't in {YOURS} yet"))?.to_hex();
            let on = i.r.db().switches().into_iter().filter(|((k, _), s)| *k == self.0.id && s.on).map(|((_, name), s)| (name, s.time)).collect();
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
            let switches = m.switches();
            let unnamed = switches.iter().filter(|((k, name), s)| *k == self.0.id && s.on && s.scope.is_none() && name != scopes::APPS_PART).map(|((_, name), _)| name.clone());
            let unrecorded = carried.iter().filter(|name| !switches.contains_key(&(self.0.id, name.to_string()))).cloned();
            Ok(unnamed
                .chain(unrecorded)
                .filter_map(|name| added_scope(i, &caller.user, &name).map(|scope| (name, scope)))
                .filter_map(|(name, scope)| service_entry(m, &caller.user, &hex(&self.0.id), &name, true, Some(&scope)).ok())
                .collect::<Vec<_>>())
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

    /// Whether this device's own endpoint for the service `name` serves it, as its last hello said: a Mac's helper app,
    /// once macOS gave it its permission.
    pub fn part_serves(&self, name: &str) -> bool {
        self.with(|i| Ok(i.r.db().offering(name).into_iter().any(|s| s.device == Some(self.0.id) && i.online(&s.key) && i.peers.get(&s.key).is_some_and(|p| p.services.iter().any(|(n, ..)| n == name)))))
            .unwrap_or(false)
    }

    /// Refuses the Mac app's own copy of a service whose helper app serves it here now, and forgets it if it was
    /// registered: the Mac app, asking again every few seconds, answers again only if the helper stops.
    pub fn stand_down(&self, app: &str) -> Result<()> {
        if self.part_serves(app) {
            if self.0.services.forget(app) {
                self.emit(Event::Changed { apps: vec![app.to_string()] });
                self.say_hello();
            }
            bail!("{} runs as its own program on {} now", self.service_title(app), self.device_name());
        }
        Ok(())
    }

    /// Says hello to this device's part `name`, a service endpoint of its own, which answers once it's up.
    pub async fn hello_part(&self, name: &str) {
        let keys: Vec<Key> = self.with(|i| Ok(i.r.db().offering(name).into_iter().filter(|s| s.device == Some(self.0.id)).map(|s| s.key).collect())).unwrap_or_default();
        for key in keys {
            let _ = tokio::time::timeout(Duration::from_secs(3), self.hello_service_now(&key)).await;
        }
    }

    /// Whether this device's part `name`, a service endpoint of its own, has answered it since `since`.
    pub fn part_up(&self, name: &str, since: Instant) -> bool {
        self.with(|i| Ok(i.r.db().offering(name).into_iter().any(|s| s.device == Some(self.0.id) && i.peers.get(&s.key).is_some_and(|p| p.seen >= since)))).unwrap_or(false)
    }

    /// What the app `name` this device runs says of its service's wrapper (`service: { network, person }` in its
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

    /// Whether this device runs apps' calls: unless its person switched its app service off.
    pub fn runs_apps(&self) -> bool {
        self.with(|i| Ok(!Self::off(i.r.db(), &self.0.id, crate::database::scopes::APPS_PART))).unwrap_or(true)
    }

    /// The services serving on this device, whom each serves, and what each says it is. One its person switched off
    /// here isn't served.
    pub(super) fn services_here(&self, i: &Network) -> Vec<Service> {
        let m = i.r.db();
        let everyone: Vec<String> = m.users().into_iter().map(|u| u.id).filter(|u| m.serves(&self.0.id, u)).collect();
        let removed = |name: &str| Self::off(m, &self.0.id, name);
        let services = self.0.services.describe().into_iter().filter(|d| !removed(d["name"].as_str().unwrap_or(""))).map(|d| {
            let (name, users) = (d["name"].as_str().unwrap_or("").to_string(), if d["serves"] == "everyone" { everyone.clone() } else { vec![i.user.clone()] });
            let d = self.listed_as_app(&name, d);
            (name, users, d)
        });
        services.collect()
    }

    /// What this device can run, as its hello says: each part (`apps`, the app service; the services built into it;
    /// those registered with it, as the Mac app's are, which it offers while switched off too; and those oned runs as
    /// its processes, its own or fetched from a release when switched on), with whether it's switched on and, while
    /// it isn't simply on or off, what it's doing (`doing`) or what went wrong (`problem`), and its title if it gives
    /// one. Devices that don't say this run only what they list as serving.
    pub(super) fn parts_here(&self, i: &Network) -> Vec<Value> {
        let m = i.r.db();
        let on = |name: &str| !Self::off(m, &self.0.id, name);
        let mut out: BTreeMap<String, Value> = BTreeMap::new();
        out.insert(scopes::APPS_PART.into(), json!({ "name": scopes::APPS_PART, "on": on(scopes::APPS_PART) }));
        for (name, title) in self.offered_before() {
            out.insert(name.clone(), json!({ "name": name, "title": title, "on": on(&name) }));
        }
        for name in self.0.services.offered() {
            out.insert(name.clone(), json!({ "name": name, "title": self.service_title(&name), "on": on(&name) }));
        }
        // A process of oned's is on only by its entry: with none, it's off; but a Mac helper the Mac app carries is on
        // unless switched off, as oned runs it.
        let switched = |name: &str| m.switch(&self.0.id, name).is_some_and(|s| s.on) || (crate::oned::helper(name).is_some() && on(name));
        let (processes, now) = crate::oned::parts();
        for name in &processes {
            out.insert(name.clone(), json!({ "name": name, "on": switched(&name) }));
        }
        for (name, now) in now {
            if let (Some(Value::Object(part)), Value::Object(now)) = (out.get_mut(&name), json!(now)) {
                part.extend(now);
            }
        }
        for (name, held) in crate::oned::held() {
            if let (Some(Value::Object(part)), Value::Object(held)) = (out.get_mut(&name), json!(held)) {
                part.extend(held);
            }
        }
        // A registered one (the Mac app's), switched on but not registered now: starting while it registers again,
        // as it does within seconds of this node or it starting; stopped once it hasn't for a while.
        let starting = crate::handoff::since_start().is_some_and(|s| s < Duration::from_secs(60));
        let lately: BTreeSet<String> = self.0.services.offered().into_iter().collect();
        let before = self.offered_before();
        for (name, part) in out.iter_mut() {
            let registered = lately.contains(name) || before.contains_key(name);
            if !registered || processes.contains(name) || !on(name) || self.0.services.has(name) || part.get("problem").is_some_and(|p| !p.is_null()) {
                continue;
            }
            if starting || lately.contains(name) {
                part["doing"] = json!("Starting");
            } else {
                part["problem"] = json!("It stopped, and hasn’t started again. Its device’s log says why.");
            }
        }
        out.into_values().collect()
    }

    /// The services that registered here before, by their titles: what the device can run while what offers them
    /// isn't running.
    fn offered_before(&self) -> BTreeMap<String, Value> {
        std::fs::read(self.0.dir.join(OFFERED)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Why a service isn't served here, if its person switched it off.
    pub fn removed_here(&self, name: &str) -> Option<String> {
        self.with(|i| {
            let m = i.r.db();
            Ok(Self::off(m, &self.0.id, name).then(|| format!("{} is switched off on {}; its person switches it on in Settings › Devices", self.service_title(name), self.device_name())))
        })
        .ok()
        .flatten()
    }

    pub(super) fn runs_service_here(&self, app: &str) -> bool {
        self.0.services.has(app) && self.removed_here(app).is_none()
    }
}
