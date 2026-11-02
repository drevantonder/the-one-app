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

    /// A link that lets someone new, called `name`, join: it works once, for a few days. Made at the install's vault as
    /// `caller`, an admin signed in, since the kernel checks who's asking, not this server: → (the link, until when).
    pub async fn invite_link(&self, caller: &Caller, name: &str) -> Result<(String, u64)> {
        ensure!(crate::config::SIGN_IN != "google", "people join by signing in with Google, so they need no invite link");
        ensure!(!name.trim().is_empty(), "give their name");
        let manages = self.with(|i| Ok(members::manages_scope(&i.r.members, &caller.user, &Scope::Global)))?;
        ensure!(manages, "only admins invite someone");
        let token = caller.token.clone().context("sign in first: an invite link is made by someone signed in")?;
        let (address, url) = self.kernel()?;
        let name = name.trim().to_string();
        let (secret, until) = tokio::task::spawn_blocking(move || crate::kernel::Vault { url }.invite(&token, &name)).await??;
        Ok((format!("{}/invite#{secret}", address.trim_end_matches('/')), until))
    }

    /// The key of each scope this server holds, with the people who may open it and its admins (Everyone's and groups'),
    /// for an admin handing them to the install's kernel (`kernel keys`), so a serverless node opens what this one does
    /// and the vault knows who may say who no longer does.
    pub async fn keys_for_kernel(&self, caller: &Caller) -> Result<Vec<Value>> {
        let scopes = self.with(|i| {
            let m = &i.r.members;
            ensure!(members::manages_scope(m, &caller.user, &Scope::Global), "only admins give the kernel the install's keys");
            let people = |s: &Scope| m.users.values().filter(|u| !u.removed && m.may_use(&u.id, s)).map(|u| (u.id.clone(), u.name.clone())).collect::<BTreeMap<_, _>>();
            let admins = |s: &Scope| m.users.keys().filter(|u| !matches!(s, Scope::User(_)) && m.role_in(u, s) == Some(Role::Admin)).cloned().collect::<Vec<_>>();
            let title = |s: &Scope| match s {
                Scope::Global => "Everyone".to_string(),
                Scope::User(u) => m.users.get(u).map(|u| u.name.clone()).unwrap_or_default(),
                Scope::Group(g) => m.groups.get(g).map(|g| g.name.clone()).unwrap_or_default(),
            };
            Ok(m.scopes().into_iter().filter(|s| i.r.holds(s)).map(|s| (s.clone(), title(&s), people(&s), admins(&s))).collect::<Vec<_>>())
        })?;
        let mut out = Vec::new();
        for (scope, title, people, admins) in scopes {
            let key = self.scope_key(&scope).await?;
            out.push(json!({ "scope": scope.to_string(), "title": title, "key": data_encoding::BASE64URL_NOPAD.encode(&key), "people": people.keys().collect::<Vec<_>>(), "names": people, "admins": admins }));
        }
        Ok(out)
    }

    /// Whether `user` is a person of the install.
    pub fn is_user(&self, user: &str) -> bool {
        self.with(|i| Ok(i.r.members.user(user).is_some())).unwrap_or(false)
    }

    /// Lets in someone the install's kernel signed in who isn't a person of it yet, as a member, `name` as the kernel
    /// knows them: it signs in only those an admin invited, or people at the install's domain signing in with Google.
    /// This server's own person writes it, so it must be an admin.
    pub async fn let_in(&self, user: &str, name: &str) -> Result<()> {
        let me = self.user()?;
        let (admin, removed) = self.with(|i| Ok((i.r.members.is_admin(&me), i.r.members.removed_user(user))))?;
        ensure!(!removed, "you were removed from this install: an admin adds you back");
        ensure!(admin, "you aren't a person of this install yet, and {} can't let you in: its person isn't an admin", self.server_name());
        ensure!(!name.trim().is_empty(), "your sign-in doesn't say your name: sign in again");
        // A key no machine holds, the same on every server, so two letting the same person in at once add them once.
        let key: Key = <sha2::Sha256 as sha2::Digest>::digest(format!("{}/sign-in\0{user}", crate::config::CLI)).into();
        let admit = members::admit(&key, user, &format!("{name}'s sign-in"), false, Some((name.trim(), Role::Member)));
        self.member_as(&Caller { key: self.0.id, user: me, agent: None, token: None }, "admit", admit.data).await
    }

    /// Sets one of the install's settings, for an admin; `None` puts back the white label's default.
    pub async fn set_setting(&self, caller: &Caller, name: &str, value: Option<&str>) -> Result<()> {
        self.member_as(caller, "setting", json!({ "name": name, "value": value })).await
    }

    /// Puts a server, or the serverless node (`cloudflare`), first, when needed or last in the install's order of where
    /// things run, for an admin: the `use` setting, every other server's place written down as it is now.
    pub async fn set_use(&self, caller: &Caller, server: &str, place: &str) -> Result<()> {
        let place = rank::Use::parse(place).context("a place is first, needed or last")?;
        let id = if server.eq_ignore_ascii_case(rank::CLOUDFLARE) { rank::CLOUDFLARE.to_string() } else { hex(&self.find_server(server)?) };
        let here = crate::load::current().kind;
        let value = self.with(|i| {
            let m = &i.r.members;
            let kind = |k: &Key| if *k == self.0.id { here.clone() } else { i.peers.get(k).and_then(|p| p.load.as_ref()).map(|l| l.kind.clone()).unwrap_or_default() };
            let mut now: Vec<(String, String)> = m.servers().map(|s| (hex(&s.key), kind(&s.key))).collect();
            if m.settings.contains_key("serverless") {
                now.push((rank::CLOUDFLARE.into(), rank::CLOUDFLARE.into()));
            }
            Ok(rank::with(&m.settings, &now, &id, place))
        })?;
        self.set_setting(caller, rank::SETTING, Some(&value)).await
    }

    /// Everything a shell needs to draw this server and its install, for this server's local user.
    pub fn state(&self) -> Value {
        let user = self.user().unwrap_or_default();
        self.state_for(&Caller { key: self.0.id, user, agent: None, token: None })
    }

    /// The state as `viewer`, this server or a client it serves, sees it.
    pub fn state_for(&self, viewer: &Caller) -> Value {
        self.state_as(viewer, None)
    }

    /// The same for someone signed in with the kernel through `client` (its name), which is no device: not the
    /// server it reached, as the serverless node says too.
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
        let viewer_name = if viewer.key == self.0.id { self.server_name() } else { install.as_ref().and_then(|i| i.r.members.machine(&viewer.key)).map(|m| m.name.clone()).unwrap_or_default() };
        let backups = install.as_ref().and_then(|i| self.backup_problem(i, &viewer.user));
        let here_sidecars = install.as_ref().map(|i| self.sidecars_here(i)).unwrap_or_default();
        let me = &viewer.user;
        let server_now = |s: &members::Machine| {
            let Some(i) = install.as_ref() else { return serde_json::Map::new() };
            let m = &i.r.members;
            let session = i.peers.get(&s.key).filter(|_| i.online(&s.key));
            // The containers on a server are its owners' business.
            let load = if s.key == self.0.id { Some(crate::load::current()) } else { session.and_then(|s| s.load.clone()) }.map(|mut l| {
                if !m.owners(&s.key).iter().any(|o| m.acts_for(me, o)) {
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
            server: json!({ "key": hex(&self.0.id), "name": self.server_name(), "ready": ready, "guide": crate::skills::address() }),
            member: install.as_ref().is_some_and(|i| i.r.members.server(&self.0.id).is_some()),
            entries: install.as_ref().map(|i| i.r.store.count().unwrap_or(0) as u64).unwrap_or(0),
            waiting,
            arrangements: json!(arrangements),
            backups,
            guide: crate::skills::address(),
            server_now: &server_now,
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

    pub(super) async fn member_content(&self, caller: &Caller, content: Content) -> Result<()> {
        let kind = content.kind.clone();
        let by = (caller.key != self.0.id).then_some(caller.key);
        // One that wouldn't count would change nothing anywhere: refuse it instead.
        let counts = self.with(|i| i.r.would_count(&caller.user, by, content.clone(), now_ms()))?;
        ensure!(counts, "that changes nothing: you may not {kind} that");
        let before = self.with(|i| Ok(i.r.members.roles()))?;
        self.append_as(&Scope::Global, &caller.user, by, content).await?;
        let after = self.with(|i| Ok(i.r.members.roles()))?;
        self.tell_vault(caller, members::roles_changed(&before, &after)).await;
        Ok(())
    }

    /// Tells the install's vault how people's roles changed, as `caller`, who changed them: so someone removed opens
    /// nothing there at once, rather than once an admin next runs `kernel keys`. A role someone gave themselves (accepting
    /// an invite as an admin) the vault hears from the next `kernel keys`.
    pub(super) async fn tell_vault(&self, caller: &Caller, changed: Vec<(String, Option<Role>, Vec<Scope>)>) {
        let Ok((_, url)) = self.kernel() else { return };
        for (user, role, scopes) in changed.into_iter().filter(|(user, role, _)| role.is_none() || *user != caller.user) {
            let told = match caller.token.clone() {
                None => Err(anyhow!("it was changed by a program on this machine that isn't signed in")),
                Some(token) => {
                    let (vault, user, scopes) = (crate::kernel::Vault { url: url.clone() }, user.clone(), scopes.clone());
                    tokio::task::spawn_blocking(move || vault.role(&token, &user, role, &scopes)).await.map_err(anyhow::Error::from).and_then(|r| r)
                }
            };
            if let Err(err) = told {
                let scopes: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
                tracing::warn!("the vault still has {user} as before in {}, until an admin runs {} kernel keys: {err:#}", scopes.join(", "), crate::config::CLI);
            }
        }
    }

    pub async fn rename_server(&self, name: &str) -> Result<()> {
        ensure!(!name.trim().is_empty(), "give a name");
        *self.0.name.lock().unwrap() = name.to_string();
        let path = self.0.dir.join("server.json");
        let mut file: ServerFile = serde_json::from_slice(&std::fs::read(&path)?)?;
        file.name = name.to_string();
        write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
        if self.has_install() {
            self.member("rename", json!({ "key": hex(&self.0.id), "name": name })).await?;
        }
        Ok(())
    }

    /// Finds a server of the install by name or key prefix.
    pub fn find_server(&self, name: &str) -> Result<Key> {
        let found: Vec<Key> = self.with(|i| Ok(i.r.members.servers().filter(|m| m.name.eq_ignore_ascii_case(name) || hex(&m.key).starts_with(name)).map(|m| m.key).collect()))?;
        match found.as_slice() {
            [one] => Ok(*one),
            [] => bail!("no device called {name}"),
            _ => bail!("several devices match {name}"),
        }
    }

    /// Finds a server or client of the install by name or key prefix.
    pub fn find_machine(&self, name: &str) -> Result<Key> {
        let found: Vec<Key> = self.with(|i| Ok(i.r.members.machines.values().filter(|m| !m.removed && !name.is_empty() && (m.name.eq_ignore_ascii_case(name) || hex(&m.key).starts_with(name))).map(|m| m.key).collect()))?;
        match found.as_slice() {
            [one] => Ok(*one),
            [] => bail!("no device called {name}"),
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

/// What a server says of each sidecar serving there, as `GET /v1/state` lists it: what it is, its version (a
/// sidecar's own, or for one built into the node, the node's build), whom it serves, and who installed it.
fn installed_sidecars(m: &Members, server: &members::Machine, sidecars: &[Sidecar], build: Option<&str>) -> Vec<Value> {
    let name_of = |u: &String| m.users.get(u).map(|u| u.name.clone());
    sidecars
        .iter()
        .map(|(name, users, d)| {
            let built_in = d["built_in"] == true;
            let installed = m.sidecars.get(&(server.key, name.clone())).filter(|i| i.local);
            // Registered over the local API, a sidecar counts as installed by the server's local user.
            let by = match installed {
                Some(i) => name_of(&i.by),
                None if built_in => None,
                None => name_of(&server.user),
            };
            json!({
                "name": name, "title": d["title"].as_str().unwrap_or(name), "icon": d["icon"], "color": d["color"], "domain": d["domain"], "image": d["image"], "image_kind": d["image_kind"], "local_app": d["local_app"],
                "description": d["description"], "version": d["version"].as_str().or(built_in.then_some(build).flatten()),
                "built_in": built_in, "installed_by": by, "installed": installed.map(|i| i.time), "users": users.iter().filter_map(name_of).collect::<Vec<_>>(),
            })
        })
        .collect()
}
