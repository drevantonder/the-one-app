//! Device sidecars, as the node knows them: which run on which device, registering them, and the built-in ones' records.

use super::*;

impl Node {
    pub fn sidecars(&self) -> &crate::sidecars::Sidecars {
        &self.0.sidecars
    }

    /// Registers a sidecar; the node says so in its hello to the other devices, and to shells when that changes it:
    /// an app of its name shows its picture (a Mac app's icon).
    pub async fn register_sidecar(&self, app: &str, describe: Value) -> Result<()> {
        if let Some(why) = self.removed_here(app) {
            bail!("{why}");
        }
        if let Some(domain) = describe["domain"].as_str() {
            self.logo(domain, app);
        }
        if self.0.sidecars.register(app, describe) {
            self.emit(Event::Changed { apps: vec![app.to_string()] });
        }
        self.say_hello();
        Ok(())
    }

    /// Removes a sidecar from a device (this one if none is named), or with `installed` lets it register there again,
    /// for someone acting for one of the device's owners.
    pub async fn set_sidecar(&self, caller: &Caller, device: Option<&str>, name: &str, installed: bool) -> Result<()> {
        let device = device.map(str::to_string).unwrap_or_else(|| self.device_name());
        let content = self.with(|i| sidecar_entry(&i.r.members, &caller.user, &device, name, installed))?;
        self.member_content(caller, content).await?;
        self.say_hello();
        Ok(())
    }

    /// The sidecars serving on this device, whom each serves, and what each says it is: those built into the node,
    /// then those registered. A sidecar one of its owners removed from this device isn't served here.
    pub(super) fn sidecars_here(&self, i: &Install) -> Vec<Sidecar> {
        let m = &i.r.members;
        let everyone: Vec<String> = m.users.keys().filter(|u| m.serves(&self.0.id, u)).cloned().collect();
        let removed = |name: &str| m.sidecars.get(&(self.0.id, name.to_string())).is_some_and(|d| d.removed());
        let built_in = |n: &crate::sidecars::BuiltIn| {
            let mut d = (n.describe)();
            d["name"] = json!(n.name);
            d["built_in"] = json!(true);
            d
        };
        let mut sidecars: Vec<Sidecar> = self.0.built_in_sidecars.iter().filter(|n| !removed(n.name)).map(|n| (n.name.to_string(), vec![i.user.clone()], built_in(n))).collect();
        sidecars.extend(self.0.sidecars.describe().into_iter().filter(|d| !removed(d["name"].as_str().unwrap_or(""))).map(|d| {
            let users = if d["serves"] == "everyone" { everyone.clone() } else { vec![i.user.clone()] };
            (d["name"].as_str().unwrap_or("").to_string(), users, d)
        }));
        sidecars
    }

    /// Why a sidecar isn't served here, if one of this device's owners removed it.
    pub(super) fn removed_here(&self, name: &str) -> Option<String> {
        self.with(|i| {
            let m = &i.r.members;
            Ok(m.sidecars.get(&(self.0.id, name.to_string())).filter(|d| d.removed()).map(|d| {
                let by = m.users.get(&d.by).map(|u| u.name.as_str()).unwrap_or("an owner");
                format!("{name} was removed from {} by {by}; an owner of it installs it again", self.device_name())
            }))
        })
        .ok()
        .flatten()
    }

    pub(super) fn runs_sidecar_here(&self, app: &str) -> bool {
        (self.0.built_in_sidecars.iter().any(|n| n.name == app) || self.0.sidecars.has(app)) && self.removed_here(app).is_none()
    }

    /// The picture a sidecar called `name` gives, here or on a device this one is connected to, and its kind.
    pub(super) fn sidecar_image(&self, name: &str) -> Option<(Value, Value)> {
        self.with(|i| {
            let here = self.sidecars_here(i).into_iter().map(|d| d.2);
            let there = i.peers.iter().filter(|(k, _)| i.online(k)).flat_map(|(_, s)| s.sidecars.iter().map(|d| d.2.clone()));
            Ok(here.chain(there).find(|d| d["name"] == name && d["image"].is_string()).map(|d| (d["image"].clone(), d["image_kind"].clone())))
        })
        .ok()
        .flatten()
    }

    /// Every entry a built-in sidecar has in the local user's scope, oldest first.
    /// A built-in sidecar's records, in its database of this device's local user, oldest first.
    pub(super) async fn records(&self, app: &str) -> Result<Vec<crate::sidecars::Record>> {
        let user = self.user()?;
        let scope = Scope::User(user);
        let db = self.database(&scope, app, SIDECAR_DATABASE, Some(vec![crate::sidecars::RECORDS.into()]), None).await?;
        let mut db = db.lock().await;
        self.read_ready(&(scope, format!("{app}/{SIDECAR_DATABASE}")), &mut db, &self.bucket()?).await?;
        let rows = db.query("select * from records order by n", &[])?;
        Ok(rows
            .into_iter()
            .map(|r| crate::sidecars::Record {
                id: r["n"].to_string(),
                kind: r["kind"].as_str().unwrap_or("").into(),
                data: serde_json::from_str(r["data"].as_str().unwrap_or("null")).unwrap_or_default(),
                time: r["time"].as_u64().unwrap_or(0),
                by: r["by"].as_str().unwrap_or("").into(),
                device: r["device"].as_str().unwrap_or("").into(),
            })
            .collect())
    }

    /// Brings this machine in line with a built-in sidecar's records, in the background.
    pub(super) fn sync_built_in(&self, app: &'static str) {
        let Some(sync) = self.0.built_in_sidecars.iter().find(|n| n.name == app).and_then(|n| n.sync) else { return };
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
