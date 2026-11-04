//! Device services, as the node knows them: which run on which device, registering them, switching a device's parts on
//! and off, and the built-in ones' records.

use super::*;

impl Node {
    pub fn services(&self) -> &crate::services::Services {
        &self.0.services
    }

    /// Registers a service; the node says so in its hello to the other devices, and to shells when that changes it:
    /// an app of its name shows its picture (a Mac app's icon).
    pub async fn register_service(&self, app: &str, describe: Value) -> Result<()> {
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

    /// The install this device's parts are in, and those switched on here.
    pub fn parts_on(&self) -> Result<(crate::service::Install, std::collections::BTreeSet<String>)> {
        let address = self.address()?;
        self.with(|i| {
            let root = i.r.root.context("this device isn't in your One App yet")?.to_hex();
            let on = i.r.members.services.iter().filter(|((k, _), s)| *k == self.0.id && s.on).map(|((_, name), _)| name.clone()).collect();
            Ok((crate::service::Install { address: address.clone(), root, user: i.user.clone() }, on))
        })
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
