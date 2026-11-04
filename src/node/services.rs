//! Device services, as the node knows them: which run on which device, registering them, switching a device's parts on
//! and off, and the built-in ones' records.

use super::*;

/// What registered with this device before, in its home.
const OFFERED: &str = "offered.json";

impl Node {
    pub fn services(&self) -> &crate::services::Services {
        &self.0.services
    }

    /// Registers a service; the node says so in its hello to the other devices, and to shells when that changes it:
    /// an app of its name shows its picture (a Mac app's icon).
    pub async fn register_service(&self, app: &str, describe: Value) -> Result<()> {
        self.0.services.offer(app, &describe);
        // Kept, so its switch shows while what offers it (the Mac app) isn't running.
        let mut kept = self.offered_before();
        if kept.get(app) != Some(&describe["title"]) {
            kept.insert(app.to_string(), describe["title"].clone());
            let _ = std::fs::write(self.0.dir.join(OFFERED), serde_json::to_vec(&kept).unwrap_or_default());
        }
        if let Some(why) = self.removed_here(app) {
            bail!("{why}");
        }
        if let Some(domain) = describe["domain"].as_str() {
            self.logo(domain, app);
        }
        if self.0.services.register(app, describe) {
            self.emit(Event::Changed { apps: vec![app.to_string()] });
        }
        self.say_hello();
        Ok(())
    }

    /// Switches a part of a device's oned (a service, or `apps`, the app service) on or off there (this device if none
    /// is named), for the device's person: that device's oned starts or stops it.
    pub async fn set_service(&self, caller: &Caller, device: Option<&str>, name: &str, on: bool) -> Result<()> {
        ensure!(device.is_some() || caller.key == self.0.id, "say which device");
        let device = device.map(str::to_string).unwrap_or_else(|| self.device_name());
        let content = self.with(|i| service_entry(&i.r.members, &caller.user, &device, name, on))?;
        self.member_content(caller, content).await?;
        self.say_hello();
        Ok(())
    }

    /// Whether a part of this device is switched off, by its latest `service` entry.
    pub(super) fn off(m: &crate::members::Members, device: &Key, name: &str) -> bool {
        m.services.get(&(*device, name.to_string())).is_some_and(|s| !s.on)
    }

    /// The install this device's parts are in, and those switched on here, each with when it was: switched on again
    /// while it's on, it starts afresh.
    pub fn parts_on(&self) -> Result<(crate::service::Install, BTreeMap<String, u64>)> {
        let address = self.address()?;
        self.with(|i| {
            let root = i.r.root.context("this device isn't in your One App yet")?.to_hex();
            let on = i.r.members.services.iter().filter(|((k, _), s)| *k == self.0.id && s.on).map(|((_, name), s)| (name.clone(), s.time)).collect();
            Ok((crate::service::Install { address: address.clone(), root, user: i.user.clone() }, on))
        })
    }

    /// What a part of this device is doing changed: its screens and the other devices are told.
    pub fn parts_changed(&self) {
        self.emit(Event::Devices);
        self.say_hello();
    }

    /// Says hello to this device's part `name`, a service endpoint of its own, which answers once it's up.
    pub async fn hello_part(&self, name: &str) {
        let keys: Vec<Key> = self.with(|i| Ok(i.r.members.offering(name).filter(|s| s.device == Some(self.0.id)).map(|s| s.key).collect())).unwrap_or_default();
        for key in keys {
            let _ = tokio::time::timeout(Duration::from_secs(3), self.hello_service_now(&key)).await;
        }
    }

    /// Whether this device's part `name`, a service endpoint of its own, has answered it since `since`.
    pub fn part_up(&self, name: &str, since: Instant) -> bool {
        self.with(|i| Ok(i.r.members.offering(name).any(|s| s.device == Some(self.0.id) && i.peers.get(&s.key).is_some_and(|p| p.seen >= since)))).unwrap_or(false)
    }

    /// Whether this device runs apps' calls: unless its person switched its app service off.
    pub fn runs_apps(&self) -> bool {
        self.with(|i| Ok(!Self::off(&i.r.members, &self.0.id, crate::members::APPS_PART))).unwrap_or(true)
    }

    /// The services serving on this device, whom each serves, and what each says it is: those built into the node,
    /// then those registered. One its person switched off here isn't served.
    pub(super) fn services_here(&self, i: &Install) -> Vec<Service> {
        let m = &i.r.members;
        let everyone: Vec<String> = m.users.keys().filter(|u| m.serves(&self.0.id, u)).cloned().collect();
        let removed = |name: &str| Self::off(m, &self.0.id, name);
        let built_in = |n: &crate::services::BuiltIn| {
            let mut d = (n.describe)();
            d["name"] = json!(n.name);
            d["built_in"] = json!(true);
            d
        };
        let mut services: Vec<Service> = self.0.built_in_services.iter().filter(|n| !removed(n.name)).map(|n| (n.name.to_string(), vec![i.user.clone()], built_in(n))).collect();
        services.extend(self.0.services.describe().into_iter().filter(|d| !removed(d["name"].as_str().unwrap_or(""))).map(|d| {
            let users = if d["serves"] == "everyone" { everyone.clone() } else { vec![i.user.clone()] };
            (d["name"].as_str().unwrap_or("").to_string(), users, d)
        }));
        services
    }

    /// What this device can run, as its hello says: each part (`apps`, the app service; the services built into it;
    /// those registered with it, as the Mac app's are, which it offers while switched off too; and those oned runs as
    /// its processes, its own or fetched from a release when switched on), with whether it's switched on and, while
    /// it isn't simply on or off, what it's doing (`doing`) or what went wrong (`problem`), and its title if it gives
    /// one. Devices that don't say this run only what they list as serving.
    pub(super) fn parts_here(&self, i: &Install) -> Vec<Value> {
        let m = &i.r.members;
        let on = |name: &str| !Self::off(m, &self.0.id, name);
        let mut out: BTreeMap<String, Value> = BTreeMap::new();
        out.insert(members::APPS_PART.into(), json!({ "name": members::APPS_PART, "on": on(members::APPS_PART) }));
        for n in &self.0.built_in_services {
            out.insert(n.name.into(), json!({ "name": n.name, "title": (n.describe)()["title"], "on": on(n.name) }));
        }
        for (name, title) in self.offered_before() {
            out.insert(name.clone(), json!({ "name": name, "title": title, "on": on(&name) }));
        }
        for (name, d) in self.0.services.offered() {
            out.insert(name.clone(), json!({ "name": name, "title": d["title"], "on": on(&name) }));
        }
        // A process of oned's is on only by its entry: with none, it's off.
        let switched = |name: &str| m.services.get(&(self.0.id, name.to_string())).is_some_and(|s| s.on);
        let (processes, now) = crate::oned::parts();
        for name in processes {
            let title = crate::services::hosted().iter().find(|h| h.name == name).map(|h| (h.describe)()["title"].clone()).unwrap_or(Value::Null);
            out.insert(name.clone(), json!({ "name": name, "title": title, "on": switched(&name) }));
        }
        for (name, now) in now {
            if let (Some(Value::Object(part)), Value::Object(now)) = (out.get_mut(&name), json!(now)) {
                part.extend(now);
            }
        }
        // The Mac app's, switched on, start once it registers again, within seconds.
        for (name, _) in self.0.services.offered() {
            if on(&name) && !self.0.services.has(&name) && let Some(part) = out.get_mut(&name) {
                part["doing"] = json!("Starting");
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
    pub(super) fn removed_here(&self, name: &str) -> Option<String> {
        self.with(|i| {
            let m = &i.r.members;
            Ok(Self::off(m, &self.0.id, name).then(|| format!("{} is switched off on {}; its person switches it on in Settings › Devices", self.service_title(name), self.device_name())))
        })
        .ok()
        .flatten()
    }

    pub(super) fn runs_service_here(&self, app: &str) -> bool {
        (self.0.built_in_services.iter().any(|n| n.name == app) || self.0.services.has(app)) && self.removed_here(app).is_none()
    }

    /// Every entry a built-in service has in the local user's scope, oldest first.
    /// A built-in service's records, in its database of this device's local user, oldest first.
    pub(super) async fn records(&self, app: &str) -> Result<Vec<crate::services::Record>> {
        let user = self.user()?;
        let scope = Scope::User(user);
        let db = self.database(&scope, app, RECORDS_DATABASE, Some(vec![crate::services::RECORDS.into()]), None).await?;
        let mut db = db.lock().await;
        self.read_ready(&(scope, format!("{app}/{RECORDS_DATABASE}")), &mut db, &self.bucket()?).await?;
        let rows = db.query("select * from records order by n", &[])?;
        Ok(rows
            .into_iter()
            .map(|r| crate::services::Record {
                id: r["n"].to_string(),
                kind: r["kind"].as_str().unwrap_or("").into(),
                data: serde_json::from_str(r["data"].as_str().unwrap_or("null")).unwrap_or_default(),
                time: r["time"].as_u64().unwrap_or(0),
                by: r["by"].as_str().unwrap_or("").into(),
                device: r["device"].as_str().unwrap_or("").into(),
            })
            .collect())
    }

    /// Brings this machine in line with a built-in service's records, in the background.
    pub(super) fn sync_built_in(&self, app: &'static str) {
        let Some(sync) = self.0.built_in_services.iter().find(|n| n.name == app).and_then(|n| n.sync) else { return };
        let node = self.clone();
        tokio::spawn(async move {
            match node.records(app).await {
                Ok(records) => {
                    let _ = tokio::task::spawn_blocking(move || sync(&records)).await;
                }
                Err(err) => tracing::debug!("{app}'s records: {err:#}"),
            }
        });
    }
}
