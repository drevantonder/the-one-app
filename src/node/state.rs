//! The install's members and settings, as people and their clients see them and change them, and finding them by name.

use super::*;

impl Node {
    /// Where the install's kernel is, for nodes to take its sign-ins: the install's address.
    pub fn kernel(&self) -> Result<String> {
        self.with(|i| Ok(i.r.members.settings.get("address").context("this install has no address yet, so it takes no sign-ins: an admin sets one with kernel use")?.clone()))
    }

    /// Whether `user` is a person of the install.
    pub fn is_user(&self, user: &str) -> bool {
        self.with(|i| Ok(i.r.members.user(user).is_some())).unwrap_or(false)
    }

    /// Sets one of the install's settings, for an admin; `None` puts back the white label's default.
    pub async fn set_setting(&self, caller: &Caller, name: &str, value: Option<&str>) -> Result<()> {
        self.member_as(caller, "setting", json!({ "name": name, "value": value })).await
    }

    /// Gives a device (by name or key: all of its endpoints of that name and person), or the serverless node
    /// (`cloudflare`), its run priority, for an admin: the `priority` setting, every other kept as set.
    pub async fn set_priority(&self, caller: &Caller, device: &str, priority: rank::Priority) -> Result<()> {
        let ids = if device.eq_ignore_ascii_case(rank::CLOUDFLARE) {
            vec![rank::CLOUDFLARE.to_string()]
        } else {
            let key = self.find_device(device)?;
            self.with(|i| {
                let m = &i.r.members;
                let d = m.machine(&key).context("no such device")?;
                Ok(m.machines.values().filter(|s| m.machine(&s.key).is_some() && s.user == d.user && s.name.eq_ignore_ascii_case(&d.name)).map(|s| hex(&s.key)).collect())
            })?
        };
        let value = self.with(|i| Ok(rank::with(&i.r.members.settings, &ids, priority)))?;
        self.set_setting(caller, rank::SETTING, Some(&value)).await
    }

    /// Everything a shell needs to draw this device and its install, for this device's local user.
    pub fn state(&self) -> Value {
        let user = self.user().unwrap_or_default();
        self.state_for(&Caller { key: self.0.id, user, agent: None, token: None })
    }

    /// The state as `viewer`, this device or a client it serves, sees it.
    pub fn state_for(&self, viewer: &Caller) -> Value {
        self.state_as(viewer, None)
    }

    /// The same for someone signed in with the kernel through `client` (its name), which is no device: not the
    /// device it reached, as the serverless node says too.
    pub fn state_for_sign_in(&self, viewer: &Caller, client: &str) -> Value {
        self.state_as(viewer, Some(client))
    }

    pub(super) fn state_as(&self, viewer: &Caller, sign_in: Option<&str>) -> Value {
        // The device looking, if it's one.
        let device = sign_in.is_none().then_some(viewer.key);
        let ready = self.ready();
        let waiting = self.waiting_requests(viewer).unwrap_or(0);
        let arrangements = self.arrangements(&viewer.user);
        let Ok(install) = self.0.install.lock() else { return json!({ "error": "this device couldn't answer that: a bug in it, which its log shows" }) };
        let viewer_name = if viewer.key == self.0.id { self.device_name() } else { install.as_ref().and_then(|i| i.r.members.machine(&viewer.key)).map(|m| m.name.clone()).unwrap_or_default() };
        let backups = install.as_ref().and_then(|i| self.backup_problem(i, &viewer.user));
        let here_services = install.as_ref().map(|i| self.services_here(i)).unwrap_or_default();
        // Which device is looking, to say "This device": a client device of the viewer's asks as a sign-in.
        let looking = device.or_else(|| {
            let m = &install.as_ref()?.r.members;
            m.machine(&viewer.key).filter(|d| viewer.key != self.0.id && d.user == viewer.user && d.is(members::Kind::Client)).map(|_| viewer.key)
        });
        let me = &viewer.user;
        let device_now = |s: &members::Machine| {
            let Some(i) = install.as_ref() else { return serde_json::Map::new() };
            let m = &i.r.members;
            let session = i.peers.get(&s.key).filter(|_| i.online(&s.key));
            // The containers on a device are its person's business.
            let load = if s.key == self.0.id { Some(crate::load::current()) } else { session.and_then(|s| s.load.clone()) }.map(|mut l| {
                if m.machine(&s.key).is_none_or(|d| d.user != *me) {
                    l.containers.clear();
                }
                l
            });
            let services = if s.key == self.0.id { here_services.as_slice() } else { session.map(|s| s.services.as_slice()).unwrap_or_default() };
            let now = json!({
                "kind": load.as_ref().map(|l| l.kind.clone()), "far": i.far.get(&s.key).is_some_and(|d| *d > FAR),
                "me": Some(s.key) == looking, "here": s.key == self.0.id, "online": s.key == self.0.id || session.is_some(),
                "build": i.builds.get(&s.key).map(|r| &r.commit), "target": i.builds.get(&s.key).map(|r| &r.target),
                "services": services.iter().map(|d| d.0.clone()).collect::<Vec<_>>(),
                "installed_services": installed_services(m, s, services, i.builds.get(&s.key).map(|r| r.commit.as_str())),
                "load": load,
            });
            match now {
                Value::Object(now) => now,
                _ => unreachable!(),
            }
        };
        let client_now = |c: &members::Machine| {
            match json!({ "me": Some(c.key) == looking }) {
                Value::Object(now) => now,
                _ => unreachable!(),
            }
        };
        let here = crate::state::Here {
            me: match sign_in {
                None => json!({ "key": hex(&viewer.key), "name": viewer_name, "user": viewer.user }),
                Some(client) => json!({ "key": "", "name": client, "user": viewer.user }),
            },
            client: (sign_in.is_none() && viewer.key != self.0.id).then(|| json!({ "key": hex(&viewer.key), "name": viewer_name })),
            device: json!({ "key": hex(&self.0.id), "name": self.device_name(), "ready": ready, "guide": crate::skills::address() }),
            member: install.as_ref().is_some_and(|i| i.r.members.compute(&self.0.id).is_some()),
            entries: install.as_ref().map(|i| i.r.store.count().unwrap_or(0) as u64).unwrap_or(0),
            waiting,
            arrangements: json!(arrangements),
            backups,
            guide: crate::skills::address(),
            passkeys: !tried(),
            device_now: &device_now,
            client_now: &client_now,
        };
        crate::state::state(install.as_ref().map(|i| &i.r.members), me, here)
    }

    /// Writes one of `global`'s node entries as the local user, after checking it would count.
    pub async fn member(&self, kind: &str, data: Value) -> Result<()> {
        self.member_as(&Caller { key: self.0.id, user: self.user()?, agent: None, token: None }, kind, data).await
    }

    /// The same, as `caller`, whose rights it's checked against.
    pub async fn member_as(&self, caller: &Caller, kind: &str, data: Value) -> Result<()> {
        self.member_content(caller, members::node(kind, data)).await
    }

    /// Signs one of `global`'s node entries with this device, for `caller`: it counts only if this device is theirs
    /// (on this machine, the CLI, MCP and the Mac app), and one that wouldn't count is refused, not written.
    pub async fn member_content(&self, caller: &Caller, content: Content) -> Result<()> {
        self.append_as(&Scope::Global, &caller.user, content).await?;
        // A deploy, or an app added, runs here before it's done: its screens and calls answer at once.
        self.settle().await?;
        Ok(())
    }

    /// Makes a group called `name`, with `caller` its first admin: → its id. Its first epoch is made by the first of
    /// its admin's devices to need it.
    pub async fn make_group(&self, caller: &Caller, name: &str) -> Result<String> {
        ensure!(!name.trim().is_empty(), "give the group a name");
        let group = new_id();
        self.member_as(caller, "group", json!({ "group": group, "name": name })).await?;
        Ok(group)
    }

    pub async fn rename_device(&self, name: &str) -> Result<()> {
        ensure!(!name.trim().is_empty(), "give a name");
        *self.0.name.lock().unwrap() = name.to_string();
        let path = self.0.dir.join("device.json");
        let mut file: DeviceFile = serde_json::from_slice(&std::fs::read(&path)?)?;
        file.name = name.to_string();
        write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
        if self.has_install() {
            self.member("rename", json!({ "key": hex(&self.0.id), "name": name })).await?;
        }
        Ok(())
    }

    /// Renames one of `caller`'s devices, by its key (hex), or with none, this one, for this machine's own programs.
    pub async fn rename(&self, caller: &Caller, key: Option<&str>, name: &str) -> Result<()> {
        let name = name.trim();
        let key = match key {
            Some(k) => crate::entry::unhex(k).context("a device's key is 64 hex characters")?,
            None => {
                ensure!(caller.key == self.0.id, "say which device");
                self.0.id
            }
        };
        ensure!(!name.is_empty(), "give a name");
        ensure!(self.with(|i| Ok(i.r.members.looks_after(&caller.user, &key)))?, "only its person or an admin renames a device");
        if key == self.0.id {
            return self.rename_device(name).await;
        }
        self.member_as(caller, "rename", json!({ "key": hex(&key), "name": name })).await
    }

    /// Finds a device of the install by name or key prefix.
    pub fn find_device(&self, name: &str) -> Result<Key> {
        let found: Vec<Key> = self.with(|i| Ok(i.r.members.computes().filter(|m| m.name.eq_ignore_ascii_case(name) || hex(&m.key).starts_with(name)).map(|m| m.key).collect()))?;
        let names = || self.with(|i| Ok(i.r.members.computes().map(|m| m.name.clone()).collect::<Vec<_>>().join(", "))).unwrap_or_default();
        match found.as_slice() {
            [one] => Ok(*one),
            [] => bail!("no device called {name}: your devices are {}", names()),
            _ => bail!("several devices match {name}"),
        }
    }

    /// The service endpoint offering `service` on the device called `name` (or by key prefix) that serves `user`, if
    /// there's one: only `user`'s own (`own`), or one its person shares with everyone.
    pub(super) fn find_service(&self, service: &str, name: &str, user: &str, own: bool) -> Option<Key> {
        let found: Vec<Key> = self
            .with(|i| {
                let m = &i.r.members;
                let serves = |s: &members::Machine| if own { s.user == user } else { s.user != user && m.service_serves(&s.key, user) };
                Ok(m.offering(service).filter(|s| serves(s) && !name.is_empty() && (s.name.eq_ignore_ascii_case(name) || hex(&s.key).starts_with(name))).map(|s| s.key).collect())
            })
            .ok()?;
        match found.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }

    /// Finds a device or client of the install by name or key prefix.
    pub fn find_machine(&self, name: &str) -> Result<Key> {
        let found: Vec<Key> = self.with(|i| Ok(i.r.members.machines.values().filter(|m| !m.removed && !name.is_empty() && (m.name.eq_ignore_ascii_case(name) || hex(&m.key).starts_with(name))).map(|m| m.key).collect()))?;
        let names = || self.with(|i| Ok(i.r.members.machines.values().filter(|m| !m.removed).map(|m| m.name.clone()).collect::<Vec<_>>().join(", "))).unwrap_or_default();
        match found.as_slice() {
            [one] => Ok(*one),
            [] => bail!("no device called {name}: your devices are {}", names()),
            _ => bail!("several devices match {name}"),
        }
    }

    /// Finds a user by name or id.
    pub fn find_user(&self, name: &str) -> Result<String> {
        let found: Vec<String> = self.with(|i| Ok(i.r.members.users.values().filter(|u| !u.removed && (u.id == name || u.name.eq_ignore_ascii_case(name))).map(|u| u.id.clone()).collect()))?;
        match found.as_slice() {
            [one] => Ok(one.clone()),
            [] => bail!("no user called {name}"),
            _ => bail!("several users are called {name}; give their id"),
        }
    }

    /// A scope as someone gives it: `user` (their own), `global`, `group:<id>`, or a group's name or id.
    pub fn scope_for(&self, caller: &Caller, scope: &str) -> Result<Scope> {
        Ok(match scope {
            "user" => Scope::User(caller.user.clone()),
            s => Scope::parse(s).or_else(|_| self.find_group(s).map(Scope::Group))?,
        })
    }

    /// Finds a group by name or id.
    pub fn find_group(&self, name: &str) -> Result<String> {
        let found: Vec<String> = self.with(|i| Ok(i.r.members.groups.values().filter(|g| g.id == name || g.name.eq_ignore_ascii_case(name)).map(|g| g.id.clone()).collect()))?;
        match found.as_slice() {
            [one] => Ok(one.clone()),
            [] => bail!("no group called {name}"),
            _ => bail!("several groups are called {name}; give its id"),
        }
    }

    pub(super) fn name_of(&self, key: &Key) -> String {
        self.with(|i| Ok(i.r.members.machines.get(key).map(|m| m.name.clone()))).ok().flatten().unwrap_or_else(|| short(key))
    }

    /// The install's compute devices' names by key (hex), and the serverless node's: what a trace's reader calls the
    /// nodes a client tried.
    pub fn compute_names(&self) -> Value {
        self.with(|i| {
            let m = &i.r.members;
            let name = |k: &Key| m.machines.get(k).map(|d| d.name.clone()).unwrap_or_else(|| short(k));
            let mut names: serde_json::Map<String, Value> = m.computes().map(|c| (hex(&c.key), json!(name(&c.key)))).collect();
            if let Some(key) = m.serverless() {
                names.insert(hex(&key), json!("the serverless node"));
            }
            Ok(Value::Object(names))
        })
        .unwrap_or_default()
    }
}

/// What a device says of each service serving there, as `GET /v1/state` lists it: what it is, its version (a
/// service's own, or for one built into the node, the node's build), whom it serves, and who installed it.
/// Whether this node's install is a try install (`one try`), whose kernel beside it signs its person in with no passkey.
fn tried() -> bool {
    let settings = std::fs::read_to_string(crate::kernel::Settings::path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    settings.is_some_and(|s| !s["try_person"].is_null())
}

fn installed_services(m: &Members, device: &members::Machine, services: &[Service], build: Option<&str>) -> Vec<Value> {
    let name_of = |u: &String| m.users.get(u).map(|u| u.name.clone());
    services
        .iter()
        .map(|(name, users, d)| {
            let built_in = d["built_in"] == true;
            let installed = m.services.get(&(device.key, name.clone())).filter(|i| i.local);
            // Registered over the local API, a service counts as installed by the device's local user.
            let by = match installed {
                Some(i) => name_of(&i.by),
                None if built_in => None,
                None => name_of(&device.user),
            };
            json!({
                "name": name, "title": d["title"].as_str().unwrap_or(name), "icon": d["icon"], "color": d["color"], "domain": d["domain"], "image": d["image"], "image_kind": d["image_kind"], "local_app": d["local_app"],
                "description": d["description"], "version": d["version"].as_str().or(built_in.then_some(build).flatten()),
                "built_in": built_in, "installed_by": by, "installed": installed.map(|i| i.time), "users": users.iter().filter_map(name_of).collect::<Vec<_>>(),
            })
        })
        .collect()
}
