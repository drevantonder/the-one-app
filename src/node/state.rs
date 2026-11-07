//! The network's members and settings, as people and their clients see them and change them, and finding them by name.

use super::*;
use crate::state::{Answering, ClientHere, DeviceNow, InstalledService, State, Viewer};

impl Node {
    pub fn is_user(&self, user: &str) -> bool {
        self.with_or(false, |i| Ok(i.r.db().user(user)?.is_some()))
    }

    /// Sets one of the network's settings, for an admin; `None` puts back the custom build's default.
    pub async fn set_setting(&self, caller: &Caller, name: &str, value: Option<&str>) -> Result<()> {
        self.member_content(caller, content::setting(name, value)).await
    }

    /// Gives a device (by name or key: all of its endpoints of that name and person), or the Cloudflare node
    /// (`cloudflare`), its run priority, for an admin: the `priority` setting, every other kept as set.
    pub async fn set_priority(&self, caller: &Caller, device: &str, priority: rank::Priority) -> Result<()> {
        let ids = if device.eq_ignore_ascii_case(rank::CLOUDFLARE) {
            vec![rank::CLOUDFLARE.to_string()]
        } else {
            vec![hex(&self.find_device(device)?)]
        };
        let value = self.with(|i| Ok(rank::with(&i.r.db().settings()?, &ids, priority)))?;
        self.set_setting(caller, rank::SETTING, Some(&value)).await
    }

    /// Everything a shell needs to draw this device and its network, for this device's local user.
    pub fn state(&self) -> Result<State> {
        let user = self.user().unwrap_or_default();
        self.state_for(&Caller::own(self.0.id, user))
    }

    /// The state as `viewer`, this device or a client it serves, sees it.
    pub fn state_for(&self, viewer: &Caller) -> Result<State> {
        self.state_as(viewer, None)
    }

    /// The same for someone signed in at the network's address through `client` (its name), which is no device: not the
    /// device it reached, as the Cloudflare node says too.
    pub fn state_for_sign_in(&self, viewer: &Caller, client: &str) -> Result<State> {
        self.state_as(viewer, Some(client))
    }

    pub(super) fn state_as(&self, viewer: &Caller, sign_in: Option<&str>) -> Result<State> {
        // The device looking, if it's one.
        let device = sign_in.is_none().then_some(viewer.key);
        let ready = self.ready();
        let waiting = self.waiting_requests(viewer).unwrap_or(0);
        let arrangements = self.arrangements(&viewer.user);
        let Ok(network) = self.0.network.lock() else { bail!("this device couldn't answer that: a bug in it, which its log shows") };
        let viewer_name = if viewer.key == self.0.id { self.device_name() } else { network.as_ref().map(|i| i.r.db().machine(&viewer.key)).transpose()?.flatten().map(|m| m.name.clone()).unwrap_or_default() };
        let backups = network.as_ref().map(|i| self.backup_problem(i, &viewer.user)).transpose()?.flatten();
        let here_services = network.as_ref().map(|i| self.services_here(i)).transpose()?.unwrap_or_default();
        let here_parts = network.as_ref().map(|i| self.parts_here(i)).transpose()?;
        // Which device is looking, to say "This device": a client device of the viewer's asks as a sign-in.
        let looking = match (device, network.as_ref()) {
            (Some(key), _) => Some(key),
            (None, Some(i)) => i.r.db().machine(&viewer.key)?.filter(|d| viewer.key != self.0.id && d.user == viewer.user && d.is(scopes::Kind::Client)).map(|_| viewer.key),
            (None, None) => None,
        };
        let me = &viewer.user;
        let device_now = |s: &scopes::Machine| -> Result<DeviceNow> {
            let Some(i) = network.as_ref() else { return Ok(DeviceNow::default()) };
            let m = i.r.db();
            let session = i.peers.get(&s.key).filter(|_| i.online(&s.key));
            // The containers on a device are its person's business.
            let mut load = if s.key == self.0.id { Some(crate::load::current()) } else { session.and_then(|s| s.load.clone()) };
            if let Some(l) = &mut load
                && m.machine(&s.key)?.is_none_or(|d| d.user != *me)
            {
                l.containers.clear();
            }
            let mut services: Vec<Service> = if s.key == self.0.id { here_services.clone() } else { session.map(|s| s.services.clone()).unwrap_or_default() };
            // What its services with endpoints of their own run there, as each says: they're part of it.
            for (k, p) in i.peers.iter().filter(|(k, _)| i.online(k)) {
                if m.machine(k)?.is_some_and(|p| p.is(scopes::Kind::Service) && p.device == Some(s.key)) {
                    for d in p.services.iter() {
                        if !services.iter().any(|h| h.0 == d.0) {
                            services.push(d.clone());
                        }
                    }
                }
            }
            let build = i.builds.get(&s.key);
            Ok(DeviceNow {
                kind: load.as_ref().map(|l| l.kind.clone()),
                far: i.far.get(&s.key).is_some_and(|d| *d > FAR),
                here: s.key == self.0.id,
                online: s.key == self.0.id || session.is_some(),
                build: build.map(|r| r.commit.clone()),
                target: build.map(|r| r.target.clone()),
                services: services.iter().map(|d| d.0.clone()).collect(),
                installed_services: installed_services(m, s, &services, build.map(|r| r.commit.as_str()))?,
                // What it can run, its person's and the admins' to see.
                parts: if !m.may(me, "remove", &scopes::device(&s.key)) { None } else if s.key == self.0.id { here_parts.clone() } else { session.and_then(|s| s.parts.clone()).and_then(|p| serde_json::from_value(p).ok()) },
                load,
            })
        };
        let here = crate::state::Here {
            me: match sign_in {
                None => Viewer { key: hex(&viewer.key), name: viewer_name.clone(), user: viewer.user.clone() },
                Some(client) => Viewer { key: String::new(), name: client.to_string(), user: viewer.user.clone() },
            },
            client: (sign_in.is_none() && viewer.key != self.0.id).then(|| ClientHere { key: hex(&viewer.key), name: viewer_name }),
            device: Answering { key: hex(&self.0.id), name: self.device_name(), ready, guide: self.guide() },
            looking,
            member: network.as_ref().map(|i| i.r.db().compute(&self.0.id)).transpose()?.flatten().is_some(),
            // `global`'s steps read.
            steps: network.as_ref().map(|i| i.r.at().unwrap_or(0)).unwrap_or(0),
            waiting,
            arrangements,
            backups,
            guide: self.guide(),
            throwaway: matches!(crate::mode::here(), crate::mode::Mode::Throwaway(_)),
            device_now: &device_now,
        };
        crate::state::state(network.as_ref().map(|i| i.r.db()), me, here)
    }

    /// Signs one of `global`'s node entries with this device, for `caller`: it counts only if this device is theirs
    /// (on this machine, the CLI, MCP and the Mac app), and one that wouldn't count is refused, not written.
    pub async fn member_content(&self, caller: &Caller, content: Content) -> Result<()> {
        self.append_as(&Scope::Global, &caller.user, content).await?;
        // A deploy, or an app added, runs here before it's done: its screens and calls answer at once.
        self.settle().await?;
        Ok(())
    }

    pub async fn rename_device(&self, name: &str) -> Result<()> {
        ensure!(!name.trim().is_empty(), "give a name");
        *self.0.name.lock().unwrap() = name.to_string();
        let path = self.0.dir.join("device.json");
        let mut file: DeviceFile = serde_json::from_slice(&std::fs::read(&path)?)?;
        file.name = name.to_string();
        write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
        if self.has_network() {
            self.member_content(&Caller::local(self)?, content::rename("key", &hex(&self.0.id), name)).await?;
        }
        Ok(())
    }

    pub fn find_device(&self, name: &str) -> Result<Key> {
        self.with(|i| i.r.db().find_node(name))
    }

    /// The service endpoint offering `service` on the device called `name` (or by key prefix) that serves `user`, if
    /// there's one: only `user`'s own (`own`), or one its person shares with everyone.
    pub(super) fn find_service(&self, service: &str, name: &str, user: &str, own: bool) -> Option<Key> {
        let found: Vec<Key> = self
            .with(|i| {
                let m = i.r.db();
                let serves = |s: &scopes::Machine| if own { s.user == user } else { s.user != user && m.may(user, "use", &scopes::service(&s.key)) };
                // Named as the device it's part of.
                let on = |s: &scopes::Machine| -> Result<bool> {
                    Ok(match s.device {
                        Some(d) => m.machine(&d)?.is_some_and(|d| d.name.eq_ignore_ascii_case(name) || hex(&d.key).starts_with(name)),
                        None => false,
                    })
                };
                let mut found = Vec::new();
                for s in m.offering(service)? {
                    if serves(&s) && !name.is_empty() && on(&s)? {
                        found.push(s.key);
                    }
                }
                Ok(found)
            })
            .ok()?;
        match found.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }

    /// Finds a device or client of the network by name or key prefix.
    pub fn find_machine(&self, name: &str) -> Result<Key> {
        self.with(|i| i.r.db().find_machine(name))
    }

    pub fn find_user(&self, name: &str) -> Result<String> {
        self.with(|i| i.r.db().find_user(name))
    }

    /// A scope as someone gives it: `user` (their own), `global`, `group:<id>`, or a group's name or id.
    pub fn scope_for(&self, caller: &Caller, scope: &str) -> Result<Scope> {
        Scope::parse_for(scope, &caller.user).or_else(|_| self.find_group(scope).map(Scope::Group))
    }

    pub fn find_group(&self, name: &str) -> Result<String> {
        self.with(|i| i.r.db().find_group(name))
    }

    pub(super) fn name_of(&self, key: &Key) -> String {
        // A service with an endpoint of its own, by the device it's part of.
        let of = |m: &ScopeDb| -> Result<Option<scopes::Machine>> {
            if let Some(d) = m.service(key)?.and_then(|s| s.device)
                && let Some(machine) = m.machine_ever(&d)?
            {
                return Ok(Some(machine));
            }
            m.machine_ever(key)
        };
        self.with_or(None, |i| Ok(of(i.r.db())?.map(|m| m.name))).unwrap_or_else(|| short(key))
    }

    /// The network's compute devices' names by key (hex), and the Cloudflare node's (`cloudflare`): what a trace's reader calls the
    /// nodes a client tried.
    pub fn compute_names(&self) -> Value {
        self.with(|i| {
            let m = i.r.db();
            let mut names = serde_json::Map::new();
            for c in m.computes()? {
                names.insert(hex(&c.key), json!(m.machine_ever(&c.key)?.map(|d| d.name.clone()).unwrap_or_else(|| short(&c.key))));
            }
            if let Some(key) = m.cloudflare_node()? {
                names.insert(hex(&key), json!(rank::CLOUDFLARE));
            }
            Ok(Value::Object(names))
        })
        .unwrap_or_default()
    }
}

/// What a device says of each service serving there, as `GET /v1/state` lists it.
fn installed_services(m: &ScopeDb, device: &scopes::Machine, services: &[Service], build: Option<&str>) -> Result<Vec<InstalledService>> {
    let name_of = |u: &String| -> Result<Option<String>> { Ok(m.user_ever(u)?.map(|u| u.name)) };
    let text = |v: &Value| v.as_str().map(str::to_string);
    let mut out = Vec::new();
    for (name, users, d) in services {
        let built_in = d["built_in"] == true;
        let installed = m.switch(&device.key, name)?.filter(|i| i.on);
        // Registered over the local API, a service counts as installed by the device's local user.
        let by = match &installed {
            Some(i) => name_of(&i.by)?,
            None if built_in => None,
            None => name_of(&device.user)?,
        };
        let mut who = Vec::new();
        for u in users {
            who.extend(name_of(u)?);
        }
        out.push(InstalledService {
            name: name.clone(),
            title: d["title"].as_str().unwrap_or(name).to_string(),
            icon: text(&d["icon"]),
            color: text(&d["color"]),
            domain: text(&d["domain"]),
            image: text(&d["image"]),
            image_kind: text(&d["image_kind"]),
            description: text(&d["description"]),
            version: d["version"].as_str().or(built_in.then_some(build).flatten()).map(str::to_string),
            built_in,
            installed_by: by,
            installed: installed.map(|i| i.time),
            users: who,
        });
    }
    Ok(out)
}
