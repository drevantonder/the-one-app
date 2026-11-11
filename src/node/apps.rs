//! Apps: the network's, every one users may add, and retiring them. Adding, deploying and forking them is deploy/'s.

use ::custom_build::allowed;
use super::*;

pub(crate) mod routes;
pub mod cli;

impl Node {
    /// What users call `app`, for what they read: its title, or its service's, else its name.
    pub(super) async fn title(&self, app: &str) -> String {
        match self.0.apps.title(app).await {
            Some(title) => title,
            None => self.service_title(app),
        }
    }

    pub(super) fn service_title(&self, name: &str) -> String {
        self.0.apps.identity(name).and_then(|d| d["title"].as_str().filter(|t| !t.trim().is_empty()).map(str::to_string)).unwrap_or_else(|| name.to_string())
    }

    pub(super) fn deployed(&self, app: &str) -> bool {
        self.with_or(false, |i| Ok(i.r.db().app(app)?.is_some_and(|a| a.bundle.is_some())))
    }

    /// Whether `app` is in Recently Deleted (or erased): it doesn't run, show or run its schedules until restored.
    pub(super) fn deleted(&self, app: &str) -> bool {
        self.with_or(false, |i| Ok(i.r.db().app(app)?.is_some_and(|a| a.deleted.is_some() || a.erased)))
    }

    /// Erases each app who's who says is erased that this node hasn't since it started, whichever device wrote the
    /// step (`settings::retire`): its databases closed and forgotten here, and its keys deleted from the bucket, before
    /// the step's read is done. Each node that reads the step does it, as deleting what's gone does nothing; one that
    /// fails (the bucket didn't answer) is tried again the next time who's who changes.
    pub(super) async fn follow_erased(&self) {
        let Ok(erased) = self.with(|i| Ok(i.r.db().apps()?.into_iter().filter(|a| a.erased).map(|a| a.name).collect::<Vec<_>>())) else { return };
        for app in erased {
            if !self.0.erased.lock().unwrap().insert(app.clone()) {
                continue;
            }
            if let Err(err) = self.destroy_keys(&app).await {
                tracing::warn!("erasing {app}'s keys: {err:#}");
                self.0.erased.lock().unwrap().remove(&app);
            }
        }
    }

    /// Every app in the network, with what it does for `viewer` where it's added for them; and the services that
    /// serve them.
    pub async fn apps(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let mut out = self.catalog(viewer, false).await?;
        let remote = self.with(|i| {
            let mut d: Vec<Value> = self.services_up(i)?.into_iter().map(|(_, d)| d).filter(|d| d.1.contains(&viewer.user)).map(|d| d.2).collect();
            d.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_string());
            Ok(d)
        })?;
        for mut describe in remote {
            if out.iter().any(|a| a["name"] == describe["name"]) {
                continue;
            }
            // A service serves its user, so its screens are on their own page.
            if describe["screens"].as_array().is_some_and(|s| !s.is_empty()) && describe["added"].is_null() {
                describe["added"] = json!([Scope::User(viewer.user.clone()).to_string()]);
            }
            if let (Value::Null, Some(domain), Some(name)) = (&describe["image"], describe["domain"].as_str(), describe["name"].as_str()) {
                if let Some((image, kind)) = deploy_runtime::logo(&self.rt(), domain, name) {
                    (describe["image"], describe["image_kind"]) = (json!(image), json!(kind));
                }
            }
            out.push(describe);
        }
        Ok(out)
    }

    /// What `app` says it is and does, as its own code describes it: one added here, or a built-in no one has added,
    /// which touches no data.
    pub async fn described(&self, app: &str) -> Result<Value> {
        self.0.apps.describe_any(app).await
    }

    /// Every lab in the network, for an admin's Settings › Labs (`src/labs.rs`): each app's, then each
    /// service's, each `{ setting, title, description, on }`, an app's or service's with `app` or `service`, its
    /// `name`, `title`, `icon` and `color`.
    pub async fn labs(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let on = self.with(|i| {
            allowed!(i.r.db().may(&viewer.user, "change-settings", scopes::NETWORK), "Only admins change Labs.");
            Ok(::labs::on_keys(&i.r.db().settings()?))
        })?;
        let lab = |setting: String, name: &str, l: &Value| json!({ "title": l["title"].as_str().unwrap_or(name), "description": l["description"].as_str().unwrap_or(""), "on": on.contains(&setting), "setting": setting });
        let mut out: Vec<Value> = Vec::new();
        let from = |kind: &str, d: &Value, l: &mut Value| {
            l[kind] = json!({ "name": d["name"], "title": d["title"].as_str().filter(|t| !t.is_empty()).or(d["name"].as_str()), "icon": d["icon"], "color": d["color"] });
        };
        let mut apps = self.catalog(viewer, false).await?;
        apps.sort_by_key(|a| a["title"].as_str().unwrap_or("").to_lowercase());
        for a in apps.iter().filter(|a| a["deleted"].is_null()) {
            for (name, l) in a["labs"].as_object().into_iter().flatten() {
                let mut l = lab(::labs::app_key(a["name"].as_str().unwrap_or(""), name), name, l);
                from("app", a, &mut l);
                out.push(l);
            }
        }
        let mut services: Vec<Value> = self.with(|i| {
            Ok(self.services_up(i)?.into_iter().map(|(_, d)| d.2).collect())
        })?;
        services.sort_by_key(|d| d["title"].as_str().or(d["name"].as_str()).unwrap_or("").to_lowercase());
        services.dedup_by(|a, b| a["name"] == b["name"]);
        for d in &services {
            for (name, l) in d["labs"].as_object().into_iter().flatten() {
                let mut l = lab(format!("lab.service.{}.{name}", d["name"].as_str().unwrap_or("")), name, l);
                from("service", d, &mut l);
                out.push(l);
            }
        }
        Ok(out)
    }

    /// Every app in the network: owner, `from`, forks, deploys, the scopes `viewer` may use it's added in, its
    /// title and icon, and for one built in, the bundle this release has of it; for those added for them, what it
    /// does (tools, widgets, screens, instructions). With `not_added`, also the built-in apps the network hasn't
    /// added yet, with no deploys, each as its own code describes it. Every app that says what it is has `offers` (a
    /// main screen it `opens` to, `screens`: that or a widget, `accounts`), and, with a service, `nodes`: the names of the viewer's own nodes it can
    /// run on.
    pub async fn catalog(&self, viewer: &Caller, not_added: bool) -> Result<Vec<Value>> {
        let listed = self.with(|i| {
            let m = i.r.db();
            let apps = m.apps()?;
            let switches = m.switches()?;
            apps.iter()
                .filter(|a| !a.erased)
                .map(|a| -> Result<_> {
                    let states = i.views.iter().filter_map(|(scope, st)| Some((Scope::parse(scope).ok()?, st))).collect::<Vec<_>>();
                    let added = scope::state::added(m, &a.name, &viewer.user, states.iter().map(|(s, st)| (s, *st)));
                    let runs_on = runs_on(self, i, &switches, &a.name, &added, &viewer.user)?;
                    let mut info = scope::state::app_info(m, a, &apps, &viewer.user, added)?;
                    info["runs_on"] = json!(runs_on);
                    if let Some(bundle) = self.built_in(a) {
                        info["built_in"] = json!(bundle);
                    }
                    if a.source.is_none() {
                        info["source"] = json!(self.built_in(a).and(built_in_source(&a.name)));
                    }
                    if a.deleted.is_some() {
                        return Ok((a.name.clone(), info, false));
                    }
                    let reach = reachable(i, &a.name, &viewer.user)?;
                    Ok((a.name.clone(), info, !reach.is_empty()))
                })
                .collect::<Result<Vec<_>>>()
        })?;
        let missing: Vec<Value> = if not_added {
            self.with(|i| {
                let mut missing = Vec::new();
                for (name, bundle) in self.0.built_in.iter() {
                    if i.r.db().app(name)?.is_none() {
                        missing.push(json!({ "name": name, "owner": "admins", "from": null, "forks": [], "deploys": 0, "bundle": null, "history": [], "added": [], "built_in": bundle, "source": built_in_source(name) }));
                    }
                }
                Ok(missing)
            })?
        } else {
            Vec::new()
        };
        let mut out = Vec::new();
        for (name, mut info, mine) in listed.into_iter().chain(missing.into_iter().map(|info| (info["name"].as_str().unwrap_or_default().to_string(), info, false))) {
            let described = self.0.apps.describe_any(&name).await;
            if let Ok(Value::Object(d)) = described {
                let described = Value::Object(d.clone());
                let opens = d["screens"].as_array().is_some_and(|s| s.iter().any(|s| s["name"] == "main"));
                info["offers"] = json!({
                    "opens": opens,
                    "screens": opens || d["widgets"].as_array().is_some_and(|w| !w.is_empty()),
                    "accounts": d["tools"].as_array().is_some_and(|o| o.iter().any(|o| o["accounts"] == true)),
                });
                if let Some(nodes) = self.nodes_for(viewer, &described) {
                    info["nodes"] = json!(nodes);
                }
                // A fork's module may still give its first name; the network's name for it is the one that counts.
                let declared = Schedule::all(&described);
                for (k, v) in d.into_iter().filter(|(k, _)| k != "name" && k != "schedules" && (mine || matches!(k.as_str(), "title" | "icon" | "color" | "domain" | "tagline" | "description" | "uses" | "service" | "needs" | "advanced" | "labs"))) {
                    info[k] = v;
                }
                // A tool of a lab that's off isn't there.
                if let Some(tools) = info["tools"].as_array_mut() {
                    let on = self.with(|i| Ok(::labs::on_keys(&i.r.db().settings()?)))?;
                    tools.retain(|o| o["lab"].as_str().is_none_or(|lab| on.contains(&::labs::app_key(&name, lab))));
                }
                if mine {
                    let (list, activity) = app_schedules_runtime::schedules_of(&self.rt(), viewer, &name, &declared)?;
                    info["schedules"] = json!(list);
                    info["activity"] = json!(activity);
                }
            }
            out.push(info);
        }
        for info in &mut out {
            if let Some(image) = info["bundle"].as_str().or(info["built_in"].as_str()).and_then(|b| self.image(b)) {
                (info["image"], info["image_kind"]) = (json!(image), json!("icon"));
            } else if let (Some(domain), Some(name)) = (info["domain"].as_str(), info["name"].as_str()) {
                if let Some((image, kind)) = deploy_runtime::logo(&self.rt(), domain, name) {
                    (info["image"], info["image_kind"]) = (json!(image), json!(kind));
                }
            }
        }
        Ok(out)
    }

    /// Where an app that `described` itself runs its service for `viewer`, if it has one (its own, or one it uses): the
    /// names of their own nodes on a platform the service is built for.
    pub(super) fn nodes_for(&self, viewer: &Caller, described: &Value) -> Option<Vec<String>> {
        let platforms: Option<Vec<&str>> = described["service"]["platforms"].as_array().map(|p| p.iter().filter_map(Value::as_str).collect());
        let uses_service = described["uses"]["services"].as_array().is_some_and(|s| !s.is_empty());
        if described["service"].is_null() && !uses_service {
            return None;
        }
        self.with(|i| {
            let m = i.r.db();
            let platform_of = |target: &str| match target {
                t if t.contains("apple") => "mac",
                t if t.contains("windows") => "windows",
                t if t.contains("android") => "android",
                _ => "linux",
            };
            let nodes: Vec<String> = m
                .machines()?
                .into_iter()
                .filter(|d| !d.removed && d.user == viewer.user && d.is(scopes::Kind::Compute))
                .filter(|d| platforms.as_ref().is_none_or(|p| i.running(&d.key).is_some_and(|b| p.contains(&platform_of(&b.target)))))
                .map(|d| d.name)
                .collect();
            Ok(nodes)
        })
        .ok()
    }

    /// The picture a bundle's app shows on its tile instead of its symbol: `icon.png` in its folder, a finished app icon
    /// (an external brand's own), as a data URL.
    /// Only from bundles this device has.
    pub(super) fn image(&self, bundle: &str) -> Option<String> {
        if let Some(image) = self.0.images.lock().unwrap().get(bundle) {
            return image.clone();
        }
        let bytes = std::fs::read(::database::blob_path(&self.0.dir, bundle).ok()?).ok()?;
        let files = ::deploy::bundle::unpack(&bytes).ok()?;
        let png = files.into_iter().find(|(path, _)| path == "src/icon.png").map(|(_, png)| format!("data:image/png;base64,{}", data_encoding::BASE64.encode(&png)));
        self.0.images.lock().unwrap().insert(bundle.to_string(), png.clone());
        png
    }

    pub(super) fn built_in(&self, app: &scopes::App) -> Option<&String> {
        self.0.built_in.get(&app.name).filter(|_| app.owner == Principal::Admins && app.from.is_none())
    }

    /// Adds a built-in app the network doesn't have yet, owned by admins, as this release has it. Admins only.
    pub(super) async fn bring_in(&self, caller: &Caller, app: &str) -> Result<()> {
        self.write_built_in(caller, app).await?;
        self.settle().await
    }

    /// Writes a built-in app's deploy if the network doesn't have it, leaving it to the caller to run it here.
    pub(super) async fn write_built_in(&self, caller: &Caller, app: &str) -> Result<()> {
        let Some(bundle) = self.0.built_in.get(app) else { return Ok(()) };
        let (present, admin) = self.with(|i| Ok((i.r.db().app(app)?.is_some(), i.r.db().may(&caller.user, "add-apps", scopes::NETWORK))))?;
        if present {
            return Ok(());
        }
        ensure!(admin, "{app} is built in, but {} doesn't have it yet: an admin adds it first", ::custom_build::config::YOURS);
        self.append_as(&Scope::Global, &caller.user, content::app(app, "admins")).await?;
        self.append_as(&Scope::Global, &caller.user, content::deploy(app, bundle, [])).await?;
        Ok(())
    }

    /// What an app's latest deploy says it uses: (apps, services). Waits a while for this device to run that deploy.
    pub(super) async fn uses(&self, app: &str) -> (Vec<String>, Vec<String>) {
        self.runs_latest(app, CALL_LIMIT).await;
        self.uses_here(app).await
    }

    pub(super) async fn runs_latest(&self, app: &str, within: Duration) -> bool {
        let runs = || {
            let latest = self.with_or(None, |i| Ok(i.r.db().app(app)?.and_then(|a| a.bundle.clone())));
            latest.is_none() || self.0.bundles.lock().unwrap().get(app) == latest.as_ref()
        };
        let mut settled = self.0.settled.subscribe();
        tokio::time::timeout(within, settled.wait_for(|_| runs())).await.is_ok()
    }

    pub(super) async fn uses_here(&self, app: &str) -> (Vec<String>, Vec<String>) {
        let d = if self.0.apps.has(app) { self.0.apps.describe(app).await.unwrap_or_default() } else { Value::Null };
        let list = |k: &str| d["uses"][k].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        (list("apps"), list("services"))
    }
}

/// Where a built-in app's code is: its folder of `apps/` in the repo releases come from, at this build's commit when
/// it's one (else main); none for a repo elsewhere, as only GitHub has a page for a folder at a commit.
fn built_in_source(app: &str) -> Option<String> {
    let repo = ::custom_build::config::REPO.strip_prefix("https://github.com/")?;
    let commit = ::custom_build::config::BUILD.split(['-', '+']).next().filter(|c| !::custom_build::config::BUILD.contains('+') && c.len() >= 7 && c.bytes().all(|b| b.is_ascii_hexdigit()));
    Some(format!("https://github.com/{repo}/tree/{}/apps/{app}", commit.unwrap_or("main")))
}

/// Where `app`'s service runs for the scopes `added` names, and for `user` on their own nodes: `{ scope, node,
/// name, online, seen (ms), problem }` each, `name` none once that node was removed, `problem` what its node said went wrong.
fn runs_on(node: &Node, i: &Network, switches: &BTreeMap<(Key, String), scopes::Switch>, app: &str, added: &[String], user: &str) -> Result<Vec<Value>> {
    let m = i.r.db();
    let cloudflare = m.cloudflare_node()?;
    let mut out = Vec::new();
    for ((k, n), s) in switches {
        let on = n == app
            && s.on
            && match &s.scope {
                Some(s) => added.contains(s),
                None => m.machine(k)?.is_some_and(|d| d.user == user),
            };
        if on {
            // What it's doing is its device's, as its onet said; it's run by its app runtime, where it has one of its own.
            let parts = i.parts.get(k).cloned();
            let k = &m.runtime_for(k)?.unwrap_or(*k);
            let here = *k == node.0.id;
            let name = if Some(*k) == cloudflare { Some(scope::rank::CLOUDFLARE.to_string()) } else { m.machine(k)?.filter(|d| !d.removed).map(|d| d.name) };
            let part = parts.and_then(|p| match p {
                Value::Array(list) => list.into_iter().find(|v| v["name"] == app),
                Value::Object(map) => map.get(app).cloned(),
                _ => None,
            });
            let online = here || Some(*k) == cloudflare || i.online(k);
            out.push(json!({ "scope": s.scope, "node": hex(k), "name": name, "online": online, "seen": i.seen(k), "problem": part.and_then(|p| p["problem"].as_str().map(str::to_string)) }));
        }
    }
    Ok(out)
}
