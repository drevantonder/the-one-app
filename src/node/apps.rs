//! Apps: the install's, every one people may add, and adding, installing, deploying, forking and retiring them.

use super::*;

impl Node {
    pub(super) fn deployed(&self, app: &str) -> bool {
        self.with(|i| Ok(i.r.members.apps.get(app).is_some_and(|a| a.bundle.is_some()))).unwrap_or(false)
    }

    /// Whether `app` is in Recently Deleted (or erased): it doesn't run, show or run its schedules until restored.
    pub(super) fn deleted(&self, app: &str) -> bool {
        self.with(|i| Ok(i.r.members.apps.get(app).is_some_and(|a| a.deleted.is_some() || a.erased))).unwrap_or(false)
    }

    /// Deletes an app (`deleted`), restores it (`restored`) or erases it for good (`erased`), for someone acting for
    /// its owner. A deleted app stops running and leaves every page, but keeps its data until it's erased.
    pub async fn retire(&self, caller: &Caller, app: &str, how: &str) -> Result<()> {
        let (_, content) = self.with(|i| check_append(i, app, Append::Retire { app: app.into(), how: how.into() }, &[], caller))?;
        self.member_content(caller, content).await?;
        if how == "erased" {
            self.destroy_keys(app).await?;
        }
        Ok(())
    }

    /// Every app in the install, with what each does for `viewer` where it's installed for them; and the sidecars
    /// that serve them.
    pub async fn apps(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let mut out = self.catalog(viewer, false).await?;
        let mut sidecars = Vec::new();
        if viewer.user == self.user()? {
            sidecars.extend(self.with(|i| Ok(self.sidecars_here(i)))?.into_iter().map(|d| d.2));
        }
        let remote = self.with(|i| {
            let mut d: Vec<Value> = i.peers.iter().filter(|(k, _)| i.online(k)).flat_map(|(_, s)| s.sidecars.iter()).filter(|d| d.1.contains(&viewer.user)).map(|d| d.2.clone()).collect();
            d.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_string());
            Ok(d)
        })?;
        for mut describe in sidecars.into_iter().chain(remote) {
            if out.iter().any(|a| a["name"] == describe["name"]) {
                continue;
            }
            // A sidecar serves its user, so its screens are on their own page.
            if describe["screens"].as_array().is_some_and(|s| !s.is_empty()) && describe["installed"].is_null() {
                describe["installed"] = json!([Scope::User(viewer.user.clone()).to_string()]);
            }
            if let (Value::Null, Some(domain), Some(name)) = (&describe["image"], describe["domain"].as_str(), describe["name"].as_str()) {
                if let Some((image, kind)) = self.logo(domain, name) {
                    (describe["image"], describe["image_kind"]) = (json!(image), json!(kind));
                }
            }
            out.push(describe);
        }
        Ok(out)
    }

    /// Every app in the install: owner, `from`, forks, deploys, the scopes `viewer` may use it's installed in, its
    /// title and icon, and for one built in, the bundle this release has of it; for those installed for them, what it
    /// does (operations, widgets, screens, instructions). With `not_added`, also the built-in apps the install hasn't
    /// added yet, with no deploys.
    pub async fn catalog(&self, viewer: &Caller, not_added: bool) -> Result<Vec<Value>> {
        let listed = self.with(|i| {
            let m = &i.r.members;
            Ok(m.apps
                .values()
                .filter(|a| !a.erased)
                .map(|a| {
                    let mut installed: Vec<String> = i
                        .views
                        .iter()
                        .filter(|(scope, (_, s))| s.installed.contains(&a.name) && Scope::parse(scope).is_ok_and(|s| m.may_use(&viewer.user, &s)))
                        .map(|(scope, _)| scope.clone())
                        .collect();
                    // Yours, then groups', then everyone's.
                    installed.sort_by_key(|s| (!s.starts_with("user:"), s == "global", s.clone()));
                    let reach = reachable(i, &a.name, &viewer.user);
                    let history: Vec<Value> = a.history.iter().map(|(bundle, time, user)| json!({ "bundle": bundle, "time": time, "user": user })).collect();
                    let forks: Vec<&String> = m.apps.values().filter(|f| f.from.as_ref() == Some(&a.name)).map(|f| &f.name).collect();
                    let mut info = json!({ "name": a.name, "owner": a.owner.to_string(), "from": a.from, "forks": forks, "deploys": a.deploys, "bundle": a.bundle, "history": history, "installed": installed });
                    if let Some(bundle) = self.built_in(a) {
                        info["built_in"] = json!(bundle);
                    }
                    match &a.source {
                        Some(link) if !link.is_empty() => info["source"] = json!(link),
                        Some(_) => {}
                        None => info["source"] = json!(self.built_in(a).and(built_in_source(&a.name))),
                    }
                    // A deleted app leaves every page; its owners see it in Recently Deleted.
                    if let Some((time, by)) = &a.deleted {
                        info["installed"] = json!([]);
                        info["deleted"] = json!({ "time": time, "by": m.users.get(by).map(|u| &u.name), "yours": m.acts_for(&viewer.user, &a.owner) });
                        return (a.name.clone(), info, false);
                    }
                    (a.name.clone(), info, !reach.is_empty())
                })
                .collect::<Vec<_>>())
        })?;
        let missing: Vec<Value> = if not_added {
            self.with(|i| Ok(self.0.built_in.iter().filter(|(name, _)| !i.r.members.apps.contains_key(*name)).map(|(name, bundle)| json!({ "name": name, "owner": "admins", "from": null, "forks": [], "deploys": 0, "bundle": null, "history": [], "installed": [], "built_in": bundle, "source": built_in_source(name) })).collect()))?
        } else {
            Vec::new()
        };
        let mut out = Vec::new();
        for (name, mut info, mine) in listed {
            if self.0.apps.has(&name) {
                if let Ok(Value::Object(d)) = self.0.apps.describe(&name).await {
                    // A fork's module may still give its first name; the install's name for it is the one that counts.
                    let declared = Schedule::all(&Value::Object(d.clone()));
                    for (k, v) in d.into_iter().filter(|(k, _)| k != "name" && k != "schedules" && (mine || matches!(k.as_str(), "title" | "icon" | "color" | "domain" | "tagline" | "description" | "uses" | "connector" | "needs"))) {
                        info[k] = v;
                    }
                    if mine {
                        let (list, activity) = self.schedules_of(viewer, &name, &declared)?;
                        info["schedules"] = json!(list);
                        info["activity"] = json!(activity);
                    }
                    if let Some(insights) = self.insights_of(viewer, &name)? {
                        info["insights"] = insights;
                    }
                }
            }
            out.push(info);
        }
        out.extend(missing);
        for info in &mut out {
            if let Some(image) = info["bundle"].as_str().or(info["built_in"].as_str()).and_then(|b| self.image(b)) {
                info["image"] = json!(image);
            } else if let (Some(domain), Some(name)) = (info["domain"].as_str(), info["name"].as_str()) {
                if let Some((image, kind)) = self.logo(domain, name) {
                    (info["image"], info["image_kind"]) = (json!(image), json!(kind));
                }
            } else if let Some((image, kind)) = info["name"].as_str().and_then(|n| self.sidecar_image(n)) {
                // An app over a sidecar of its name shows the picture the sidecar gives (a Mac app's icon).
                (info["image"], info["image_kind"]) = (image, kind);
            }
        }
        Ok(out)
    }

    /// The logo of `domain`, the site `app` connects to, as a data URL, once this server has fetched it. Fetches it
    /// in the background when it hasn't, or did a week ago, then tells shells `app` changed.
    pub(super) fn logo(&self, domain: &str, app: &str) -> Option<(String, String)> {
        let (url, due) = self.0.logos.get(&self.0.dir, domain, now_ms());
        if due && self.0.logos.begin(domain) {
            let (node, domain, app) = (self.clone(), domain.to_string(), app.to_string());
            tokio::spawn(async move {
                let d = domain.clone();
                let logo = match tokio::task::spawn_blocking(move || crate::logos::fetch(&d)).await {
                    Ok(Ok(logo)) => Some(logo),
                    Ok(Err(err)) => {
                        tracing::warn!("{app}'s logo: {err:#}");
                        None
                    }
                    Err(_) => None,
                };
                if node.0.logos.finish(&node.0.dir, &domain, logo, now_ms()) {
                    node.emit(Event::Changed { apps: vec![app] });
                }
            });
        }
        url
    }

    /// The picture a bundle's app shows on its tile instead of its symbol: `icon.png` in its folder, as a data URL.
    /// Only from bundles this server has.
    pub(super) fn image(&self, bundle: &str) -> Option<String> {
        if let Some(image) = self.0.images.lock().unwrap().get(bundle) {
            return image.clone();
        }
        let bytes = std::fs::read(release::blob_path(&self.0.dir, bundle).ok()?).ok()?;
        let files = crate::bundle::unpack(&bytes).ok()?;
        let png = files.into_iter().find(|(path, _)| path == "src/icon.png").map(|(_, png)| format!("data:image/png;base64,{}", data_encoding::BASE64.encode(&png)));
        self.0.images.lock().unwrap().insert(bundle.to_string(), png.clone());
        png
    }

    /// The bundle this release has of `app`, if it's the built-in app of that name: owned by admins, not a fork.
    pub(super) fn built_in(&self, app: &members::App) -> Option<&String> {
        self.0.built_in.get(&app.name).filter(|_| app.owner == Principal::Admins && app.from.is_none())
    }

    /// Adds a built-in app the install doesn't have yet, owned by admins, as this release has it. Admins only.
    pub(super) async fn bring_in(&self, caller: &Caller, app: &str) -> Result<()> {
        let Some(bundle) = self.0.built_in.get(app) else { return Ok(()) };
        let (present, admin) = self.with(|i| Ok((i.r.members.apps.contains_key(app), i.r.members.is_admin(&caller.user))))?;
        if present {
            return Ok(());
        }
        ensure!(admin, "{app} is built in, but this install doesn't have it yet: an admin adds it first");
        self.member_as(caller, "app", json!({ "name": app, "owner": "admins" })).await?;
        self.member_content(caller, deploy(app, bundle)).await
    }

    /// What an app's latest deploy says it uses: (apps, sidecars). Waits a while for this server to run that deploy.
    pub(super) async fn uses(&self, app: &str) -> (Vec<String>, Vec<String>) {
        let started = Instant::now();
        while started.elapsed() < CALL_LIMIT {
            let latest = self.with(|i| Ok(i.r.members.apps.get(app).and_then(|a| a.bundle.clone()))).ok().flatten();
            if latest.is_none() || self.0.bundles.lock().unwrap().get(app) == latest.as_ref() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.uses_here(app).await
    }

    /// What the deploy of `app` this server runs says it uses: (apps, sidecars).
    pub(super) async fn uses_here(&self, app: &str) -> (Vec<String>, Vec<String>) {
        let d = if self.0.apps.has(app) { self.0.apps.describe(app).await.unwrap_or_default() } else { Value::Null };
        let list = |k: &str| d["uses"][k].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        (list("apps"), list("sidecars"))
    }

    /// The entries that install (or uninstall) `app` in `scope` for `caller`, with their own rights there. Installing
    /// also installs each app it uses, and each they use, that isn't installed there yet.
    pub(super) async fn install_entries(&self, caller: &Caller, app: &str, scope: &Scope, installed: bool) -> Result<Vec<(Scope, Content)>> {
        let mut wanted = vec![app.to_string()];
        let mut i = 0;
        while installed && i < wanted.len() {
            self.bring_in(caller, &wanted[i]).await?;
            for used in self.uses(&wanted[i]).await.0 {
                if !wanted.contains(&used) {
                    wanted.push(used);
                }
            }
            i += 1;
        }
        self.with(|i| {
            let m = &i.r.members;
            ensure!(members::manages_scope(m, &caller.user, scope), "you can't install apps in {scope}");
            ensure!(i.r.holds(scope), "this device doesn't hold {scope}");
            let here = i.views.get(&scope.to_string()).map(|(_, s)| s.installed.clone()).unwrap_or_default();
            let mut out = Vec::new();
            for (n, name) in wanted.iter().enumerate().rev() {
                ensure!(m.apps.contains_key(name), "no app named {name}{}", if n > 0 { format!(", which {app} uses") } else { String::new() });
                if n == 0 || !here.contains(name) {
                    out.push((scope.clone(), members::node(if installed { "install" } else { "uninstall" }, json!({ "app": name }))));
                }
            }
            Ok(out)
        })
    }

    /// Installs (or uninstalls) `app` in `scope` for `caller`, and what it uses.
    pub async fn install(&self, caller: &Caller, app: &str, scope: &Scope, installed: bool) -> Result<()> {
        let by = (caller.key != self.0.id).then_some(caller.key);
        for (scope, content) in self.install_entries(caller, app, scope, installed).await? {
            self.append_as(&scope, &caller.user, by, content).await?;
        }
        if self.schedules_service().is_some() && caller.token.is_some() {
            self.sync_schedules_later(caller);
        }
        Ok(())
    }

    /// Forks `app` as a new app called `name`, owned by `owner` (by default the caller), with data of its own; it
    /// runs `app`'s latest deploy until it's deployed itself.
    pub async fn fork(&self, caller: &Caller, app: &str, name: &str, owner: Option<&str>) -> Result<()> {
        let bundle = self.with(|i| {
            let m = &i.r.members;
            ensure!(!m.apps.contains_key(name), "there's already an app called {name}");
            m.apps.get(app).with_context(|| format!("no app named {app}"))?.bundle.clone().with_context(|| format!("{app} has no deploy to fork"))
        })?;
        let owner = owner.map(str::to_string).unwrap_or_else(|| format!("user:{}", caller.user));
        self.member_as(caller, "app", json!({ "name": name, "owner": owner, "from": app })).await?;
        self.member_content(caller, deploy(name, &bundle)).await
    }

    /// Deploys `bundle` as `app`'s code, for someone acting for its owner. Deploying an older bundle again rolls back.
    /// `bundle` may be the start of an earlier deploy's.
    pub async fn deploy(&self, caller: &Caller, app: &str, bundle: &str) -> Result<()> {
        let bundle = self.with(|i| {
            let m = &i.r.members;
            let a = m.apps.get(app).with_context(|| format!("no app named {app}"))?;
            ensure!(m.acts_for(&caller.user, &a.owner), "only someone acting for {app}'s owner ({}) deploys it", a.owner);
            Ok(match a.history.iter().find(|(h, ..)| bundle.len() >= 8 && h.starts_with(bundle)) {
                Some((h, ..)) => h.clone(),
                None => bundle.to_string(),
            })
        })?;
        crate::bundle::module(&self.blob(&bundle).await?)?;
        self.member_content(caller, deploy(app, &bundle)).await
    }

    /// An app's code as deployed, its latest or an earlier deploy's: its files, or one of them. Everyone may read
    /// every app's code.
    pub async fn source(&self, app: &str, bundle: Option<&str>, path: Option<&str>) -> Result<Value> {
        let history = self.with(|i| Ok(i.r.members.apps.get(app).with_context(|| format!("no app named {app}"))?.history.clone()))?;
        let bundle = match bundle {
            Some(b) => history.iter().find(|(h, ..)| h.starts_with(b)).with_context(|| format!("{app} has no deploy {b}"))?.0.clone(),
            None => history.last().with_context(|| format!("{app} has no deploy yet"))?.0.clone(),
        };
        let mut out = crate::deploy::show(&self.blob(&bundle).await?, path)?;
        out["app"] = json!(app);
        out["bundle"] = json!(bundle);
        Ok(out)
    }
}

/// Where a built-in app's code is: its folder of `apps/` in the repo releases come from, at this build's commit when
/// it's one (else main); none for a white label with no repo.
fn built_in_source(app: &str) -> Option<String> {
    let repo = crate::config::REPO;
    let commit = crate::config::BUILD.split(['-', '+']).next().filter(|c| !crate::config::BUILD.contains('+') && c.len() >= 7 && c.bytes().all(|b| b.is_ascii_hexdigit()));
    (!repo.is_empty()).then(|| format!("https://github.com/{repo}/tree/{}/apps/{app}", commit.unwrap_or("main")))
}
