//! The install's members and settings, as people and their clients see them and change them, and finding them by name.

use super::*;

impl Node {
    /// Where the install's kernel is, for nodes to take its sign-ins: (the install's address, the vault's).
    pub fn kernel(&self) -> Result<(String, String)> {
        self.with(|i| {
            let s = &i.r.members.settings;
            let address = s.get("address").context("this install has no address yet, so it takes no sign-ins: an admin sets one with kernel use")?;
            Ok((address.clone(), s.get("kernel_vault").context("this install doesn't know its vault: an admin says with kernel use")?.clone()))
        })
    }

    /// A link that lets someone new, called `name`, join: it works once, for a few days. Its secret is made here and
    /// signed for with this device's key, as an admin's device's, for `caller`, an admin: → (the link, until when).
    pub async fn invite_link(&self, caller: &Caller, name: &str) -> Result<(String, u64)> {
        ensure!(crate::config::SIGN_IN != "google", "people here sign in with Google, so their invite names their email: {} kernel invite <name> --email <email>", crate::config::CLI);
        let manages = self.with(|i| Ok(members::manages_scope(&i.r.members, &caller.user, &Scope::Global)))?;
        ensure!(manages, "only admins invite someone");
        let (address, _) = self.kernel()?;
        let (key, name) = (ed25519_dalek::SigningKey::from_bytes(self.0.key.as_bytes()), name.to_string());
        let invite = tokio::task::spawn_blocking(move || crate::kernel::invite(address.trim_end_matches('/'), &key, &name, None, None)).await??;
        Ok((invite.link, invite.until))
    }

    /// Whether `user` is a person of the install.
    pub fn is_user(&self, user: &str) -> bool {
        self.with(|i| Ok(i.r.members.user(user).is_some())).unwrap_or(false)
    }

    /// Lets in someone the install's kernel signed in who isn't a person of it yet, as a member, `name` as the kernel
    /// knows them: it signs in only those an admin invited, or people at the install's domain signing in with Google.
    /// This device's own person writes it, so it must be an admin.
    pub async fn let_in(&self, user: &str, name: &str) -> Result<()> {
        let me = self.user()?;
        let (admin, removed) = self.with(|i| Ok((i.r.members.is_admin(&me), i.r.members.removed_user(user))))?;
        ensure!(!removed, "you were removed from this install: an admin adds you back");
        ensure!(admin, "you aren't a person of this install yet, and {} can't let you in: its person isn't an admin", self.device_name());
        ensure!(!name.trim().is_empty(), "your sign-in doesn't say your name: sign in again");
        // A key no machine holds, the same on every device, so two letting the same person in at once add them once.
        let key: Key = <sha2::Sha256 as sha2::Digest>::digest(format!("{}/sign-in\0{user}", crate::config::CLI)).into();
        let endpoint = crate::keys::Endpoint { key, seal: key };
        let listed = members::device(&endpoint, user, &format!("{name}'s sign-in"), &[members::Kind::Client], Some((name.trim(), Role::Member)));
        self.member_content(&Caller { key: self.0.id, user: me, agent: None, token: None }, listed).await
    }

    /// Sets one of the install's settings, for an admin; `None` puts back the white label's default.
    pub async fn set_setting(&self, caller: &Caller, name: &str, value: Option<&str>) -> Result<()> {
        self.member_as(caller, "setting", json!({ "name": name, "value": value })).await
    }

    /// Puts a device, or the serverless node (`cloudflare`), first, when needed or last in the install's order of where
    /// things run, for an admin: the `use` setting, every other device's place written down as it is now.
    pub async fn set_use(&self, caller: &Caller, device: &str, place: &str) -> Result<()> {
        let place = rank::Use::parse(place).context("a place is first, needed or last")?;
        let id = if device.eq_ignore_ascii_case(rank::CLOUDFLARE) { rank::CLOUDFLARE.to_string() } else { hex(&self.find_device(device)?) };
        let here = crate::load::current().kind;
        let value = self.with(|i| {
            let m = &i.r.members;
            let kind = |k: &Key| if *k == self.0.id { here.clone() } else { i.peers.get(k).and_then(|p| p.load.as_ref()).map(|l| l.kind.clone()).unwrap_or_default() };
            let mut now: Vec<(String, String)> = m.computes().map(|s| (hex(&s.key), kind(&s.key))).collect();
            if m.settings.contains_key("serverless") {
                now.push((rank::CLOUDFLARE.into(), rank::CLOUDFLARE.into()));
            }
            Ok(rank::with(&m.settings, &now, &id, place))
        })?;
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
        let waiting = self.item_requests(viewer, None, None).map(|r| r.iter().filter(|r| r["state"] == "waiting").count()).unwrap_or(0);
        let arrangements = self.arrangements(&viewer.user);
        let install = self.0.install.lock().unwrap();
        let viewer_name = if viewer.key == self.0.id { self.device_name() } else { install.as_ref().and_then(|i| i.r.members.machine(&viewer.key)).map(|m| m.name.clone()).unwrap_or_default() };
        let backups = install.as_ref().and_then(|i| self.backup_problem(i, &viewer.user));
        let here_sidecars = install.as_ref().map(|i| self.sidecars_here(i)).unwrap_or_default();
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
            let sidecars = if s.key == self.0.id { here_sidecars.as_slice() } else { session.map(|s| s.sidecars.as_slice()).unwrap_or_default() };
            let now = json!({
                "kind": load.as_ref().map(|l| l.kind.clone()), "far": i.far.get(&s.key).is_some_and(|d| *d > FAR),
                "me": Some(s.key) == device, "here": s.key == self.0.id, "online": s.key == self.0.id || session.is_some(),
                "build": i.builds.get(&s.key).map(|r| &r.commit), "target": i.builds.get(&s.key).map(|r| &r.target),
                "sidecars": sidecars.iter().map(|d| d.0.clone()).collect::<Vec<_>>(),
                "installed_sidecars": installed_sidecars(m, s, sidecars, i.builds.get(&s.key).map(|r| r.commit.as_str())),
                "load": load,
            });
            match now {
                Value::Object(now) => now,
                _ => unreachable!(),
            }
        };
        let client_now = |c: &members::Machine| {
            match json!({ "me": Some(c.key) == device }) {
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
    pub(super) async fn member_content(&self, caller: &Caller, content: Content) -> Result<()> {
        self.append_as(&Scope::Global, &caller.user, content).await?;
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
}

/// What a device says of each sidecar serving there, as `GET /v1/state` lists it: what it is, its version (a
/// sidecar's own, or for one built into the node, the node's build), whom it serves, and who installed it.
/// Whether this node's install is a try install (`one try`), whose kernel beside it signs its person in with no passkey.
fn tried() -> bool {
    let settings = std::fs::read_to_string(crate::kernel::Settings::path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    settings.is_some_and(|s| !s["try_person"].is_null())
}

fn installed_sidecars(m: &Members, device: &members::Machine, sidecars: &[Sidecar], build: Option<&str>) -> Vec<Value> {
    let name_of = |u: &String| m.users.get(u).map(|u| u.name.clone());
    sidecars
        .iter()
        .map(|(name, users, d)| {
            let built_in = d["built_in"] == true;
            let installed = m.sidecars.get(&(device.key, name.clone())).filter(|i| i.local);
            // Registered over the local API, a sidecar counts as installed by the device's local user.
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
