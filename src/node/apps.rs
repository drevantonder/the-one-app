//! Apps: the network's, every one people may add, and adding, installing, deploying, forking and retiring them.

use super::*;

impl Node {
    /// What people call `app`, for what they read: its title, or its service's, else its name.
    pub(super) async fn title(&self, app: &str) -> String {
        match self.0.apps.title(app).await {
            Some(title) => title,
            None => self.service_title(app),
        }
    }

    /// What people call the service `name`: its title, else its name.
    pub(super) fn service_title(&self, name: &str) -> String {
        self.0.services.title(name).unwrap_or_else(|| name.to_string())
    }

    pub(super) fn deployed(&self, app: &str) -> bool {
        self.with(|i| Ok(i.r.db().app(app).is_some_and(|a| a.bundle.is_some()))).unwrap_or(false)
    }

    /// Whether `app` is in Recently Deleted (or erased): it doesn't run, show or run its schedules until restored.
    pub(super) fn deleted(&self, app: &str) -> bool {
        self.with(|i| Ok(i.r.db().app(app).is_some_and(|a| a.deleted.is_some() || a.erased))).unwrap_or(false)
    }

    /// Deletes an app (`deleted`), restores it (`restored`) or erases it for good (`erased`), for someone acting for
    /// its owner. A deleted app stops running and leaves every page, but keeps its data until it's erased.
    pub async fn retire(&self, caller: &Caller, app: &str, how: &str) -> Result<()> {
        let title = self.title(app).await;
        let (_, content) = self.with(|i| check_append(i, Append::Retire { app: app.into(), how: how.into() }, &[], caller, &title))?;
        self.member_content(caller, content).await?;
        if how == "erased" {
            self.destroy_keys(app).await?;
        }
        Ok(())
    }

    /// Every app in the network, with what each does for `viewer` where it's installed for them; and the services
    /// that serve them.
    pub async fn apps(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let mut out = self.catalog(viewer, false).await?;
        let mut services = Vec::new();
        if viewer.user == self.user()? {
            services.extend(self.with(|i| Ok(self.services_here(i)))?.into_iter().map(|d| d.2));
        }
        // Push, which every node answers itself.
        let mut push = crate::push::describe();
        push["name"] = json!(crate::push::NAME);
        services.push(push);
        let remote = self.with(|i| {
            let mut d: Vec<Value> = i.peers.iter().filter(|(k, _)| i.online(k)).flat_map(|(_, s)| s.services.iter()).filter(|d| d.1.contains(&viewer.user)).map(|d| d.2.clone()).collect();
            d.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_string());
            Ok(d)
        })?;
        for mut describe in services.into_iter().chain(remote) {
            if out.iter().any(|a| a["name"] == describe["name"]) {
                continue;
            }
            // A service serves its user, so its screens are on their own page.
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

    /// Every lab in the network, for an admin's Settings › Labs (`src/labs.rs`): each app's, then each
    /// service's, each `{ setting, title, description, on }`, an app's or service's with `app` or `service`, its
    /// `name`, `title`, `icon` and `color`.
    pub async fn labs(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let on = self.with(|i| {
            ensure!(i.r.db().may(&viewer.user, "change-settings", scopes::NETWORK), "Only admins change Labs.");
            Ok(crate::labs::on_keys(&i.r.db().settings()))
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
                let mut l = lab(crate::labs::app_key(a["name"].as_str().unwrap_or(""), name), name, l);
                from("app", a, &mut l);
                out.push(l);
            }
        }
        let mut services: Vec<Value> = self.with(|i| {
            let here = self.services_here(i).into_iter().map(|d| d.2);
            Ok(here.chain(i.peers.iter().filter(|(k, _)| i.online(k)).flat_map(|(_, s)| s.services.iter().map(|d| d.2.clone()))).collect())
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

    /// `apps`, as a window draws them: what shows each one and where (no tools, schemas or instructions, which
    /// whoever needs them asks the whole list for), its picture named (`/v1/images/<name>`) rather than in it, so the
    /// list is small and each picture is kept for good where it's shown.
    pub async fn apps_drawn(&self, viewer: &Caller) -> Result<Vec<Value>> {
        const DRAWN: &[&str] = &["name", "title", "icon", "color", "domain", "image", "image_kind", "installed", "deleted", "screens", "widgets", "owner"];
        let mut out = Vec::new();
        for app in self.apps(viewer).await? {
            let Value::Object(mut app) = app else { continue };
            app.retain(|k, _| DRAWN.contains(&k.as_str()));
            if let Some(Value::String(image)) = app.get_mut("image")
                && image.starts_with("data:")
            {
                *image = self.name_image(std::mem::take(image));
            }
            out.push(Value::Object(app));
        }
        Ok(out)
    }

    /// A picture's address by its content: `/v1/images/<name>`, which `named_image` gives back.
    fn name_image(&self, data_url: String) -> String {
        use sha2::Digest;
        let name = data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(data_url.as_bytes())[..16]);
        let url = format!("/v1/images/{name}");
        self.0.named_images.lock().unwrap().insert(name, data_url);
        url
    }

    /// The picture `apps_drawn` named `name`, as its type and bytes: looked for again in this machine's person's apps
    /// if this node didn't name it (it's newer than the list that did, or another node made that list).
    pub async fn named_image(&self, name: &str) -> Option<(String, Vec<u8>)> {
        let found = || self.0.named_images.lock().unwrap().get(name).cloned();
        let url = match found() {
            Some(url) => url,
            None => {
                // Anyone can ask for a name, so the list is made again for one at most every ten seconds.
                static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let now = now_ms();
                if now.saturating_sub(LAST.load(std::sync::atomic::Ordering::Relaxed)) < 10_000 {
                    return None;
                }
                LAST.store(now, std::sync::atomic::Ordering::Relaxed);
                let me = Caller { key: self.id(), user: self.user().ok()?, agent: None, token: None };
                self.apps_drawn(&me).await.ok()?;
                found()?
            }
        };
        let (head, data) = url.strip_prefix("data:")?.split_once(',')?;
        let kind = head.strip_suffix(";base64")?;
        Some((kind.to_string(), data_encoding::BASE64.decode(data.as_bytes()).ok()?))
    }

    /// Every app in the network: owner, `from`, forks, deploys, the scopes `viewer` may use it's installed in, its
    /// title and icon, and for one built in, the bundle this release has of it; for those installed for them, what it
    /// does (tools, widgets, screens, instructions). With `not_added`, also the built-in apps the network hasn't
    /// added yet, with no deploys.
    pub async fn catalog(&self, viewer: &Caller, not_added: bool) -> Result<Vec<Value>> {
        let listed = self.with(|i| {
            let m = i.r.db();
            let apps = m.apps();
            let switches = m.switches();
            let parts_here = self.parts_here(i);
            Ok(apps
                .iter()
                .filter(|a| !a.erased)
                .map(|a| {
                    let mut installed: Vec<String> = i
                        .views
                        .iter()
                        .filter(|(scope, s)| s.installed.contains(&a.name) && Scope::parse(scope).is_ok_and(|s| m.may(&viewer.user, "use", &scopes::object(&s))))
                        .map(|(scope, _)| scope.clone())
                        .collect();
                    // Yours, then groups', then everyone's.
                    installed.sort_by_key(|s| (!s.starts_with("user:"), s == "global", s.clone()));
                    let reach = reachable(i, &a.name, &viewer.user);
                    let history: Vec<Value> = a.history.iter().map(|(bundle, time, user)| json!({ "bundle": bundle, "time": time, "user": user })).collect();
                    let forks: Vec<&String> = apps.iter().filter(|f| f.from.as_ref() == Some(&a.name)).map(|f| &f.name).collect();
                    let runs_on = runs_on(self, i, &switches, &parts_here, &a.name, &installed, &viewer.user);
                    let mut info = json!({ "name": a.name, "owner": a.owner.to_string(), "from": a.from, "forks": forks, "deploys": a.deploys, "bundle": a.bundle, "history": history, "installed": installed, "runs_on": runs_on });
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
                        info["deleted"] = json!({ "time": time, "by": m.user_ever(by).map(|u| u.name), "yours": m.may(&viewer.user, "delete", &scopes::app(&a.name)) });
                        return (a.name.clone(), info, false);
                    }
                    (a.name.clone(), info, !reach.is_empty())
                })
                .collect::<Vec<_>>())
        })?;
        let missing: Vec<Value> = if not_added {
            self.with(|i| Ok(self.0.built_in.iter().filter(|(name, _)| i.r.db().app(name).is_none()).map(|(name, bundle)| json!({ "name": name, "owner": "admins", "from": null, "forks": [], "deploys": 0, "bundle": null, "history": [], "installed": [], "built_in": bundle, "source": built_in_source(name) })).collect()))?
        } else {
            Vec::new()
        };
        let mut out = Vec::new();
        for (name, mut info, mine) in listed {
            if self.0.apps.has(&name) {
                if let Ok(Value::Object(d)) = self.0.apps.describe(&name).await {
                    // A fork's module may still give its first name; the network's name for it is the one that counts.
                    let declared = Schedule::all(&Value::Object(d.clone()));
                    for (k, v) in d.into_iter().filter(|(k, _)| k != "name" && k != "schedules" && (mine || matches!(k.as_str(), "title" | "icon" | "color" | "domain" | "tagline" | "description" | "uses" | "service" | "needs" | "advanced" | "labs"))) {
                        info[k] = v;
                    }
                    // A tool of a lab that's off isn't there.
                    if let Some(ops) = info["ops"].as_array_mut() {
                        let on = self.with(|i| Ok(crate::labs::on_keys(&i.r.db().settings())))?;
                        ops.retain(|o| o["lab"].as_str().is_none_or(|lab| on.contains(&crate::labs::app_key(&name, lab))));
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
                (info["image"], info["image_kind"]) = (json!(image), json!("icon"));
            } else if let (Some(domain), Some(name)) = (info["domain"].as_str(), info["name"].as_str()) {
                if let Some((image, kind)) = self.logo(domain, name) {
                    (info["image"], info["image_kind"]) = (json!(image), json!(kind));
                }
            }
        }
        Ok(out)
    }

    /// The logo of `domain`, the site `app` connects to, as a data URL, once this device has fetched it. Fetches it
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

    /// The picture a bundle's app shows on its tile instead of its symbol: `icon.png` in its folder, a finished app icon
    /// (an external brand's own), as a data URL.
    /// Only from bundles this device has.
    pub(super) fn image(&self, bundle: &str) -> Option<String> {
        if let Some(image) = self.0.images.lock().unwrap().get(bundle) {
            return image.clone();
        }
        let bytes = std::fs::read(release::blob_path(&self.0.dir, bundle).ok()?).ok()?;
        let files = crate::app::bundle::unpack(&bytes).ok()?;
        let png = files.into_iter().find(|(path, _)| path == "src/icon.png").map(|(_, png)| format!("data:image/png;base64,{}", data_encoding::BASE64.encode(&png)));
        self.0.images.lock().unwrap().insert(bundle.to_string(), png.clone());
        png
    }

    /// The bundle this release has of `app`, if it's the built-in app of that name: owned by admins, not a fork.
    pub(super) fn built_in(&self, app: &scopes::App) -> Option<&String> {
        self.0.built_in.get(&app.name).filter(|_| app.owner == Principal::Admins && app.from.is_none())
    }

    /// Adds a built-in app the network doesn't have yet, owned by admins, as this release has it. Admins only.
    pub(super) async fn bring_in(&self, caller: &Caller, app: &str) -> Result<()> {
        let Some(bundle) = self.0.built_in.get(app) else { return Ok(()) };
        let (present, admin) = self.with(|i| Ok((i.r.db().app(app).is_some(), i.r.db().may(&caller.user, "add-apps", scopes::NETWORK))))?;
        if present {
            return Ok(());
        }
        ensure!(admin, "{app} is built in, but {} doesn't have it yet: an admin adds it first", crate::config::YOURS);
        self.member_as(caller, "app", json!({ "name": app, "owner": "admins" })).await?;
        self.member_content(caller, deploy(app, bundle, [])).await
    }

    /// What an app's latest deploy says it uses: (apps, services). Waits a while for this device to run that deploy.
    pub(super) async fn uses(&self, app: &str) -> (Vec<String>, Vec<String>) {
        self.runs_latest(app, CALL_LIMIT).await;
        self.uses_here(app).await
    }

    /// Whether this device runs `app`'s latest deploy (or it has none), waiting up to `within` for it to.
    async fn runs_latest(&self, app: &str, within: Duration) -> bool {
        let started = Instant::now();
        loop {
            let latest = self.with(|i| Ok(i.r.db().app(app).and_then(|a| a.bundle.clone()))).ok().flatten();
            if latest.is_none() || self.0.bundles.lock().unwrap().get(app) == latest.as_ref() {
                return true;
            }
            if started.elapsed() >= within {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// What the deploy of `app` this device runs says it uses: (apps, services).
    pub(super) async fn uses_here(&self, app: &str) -> (Vec<String>, Vec<String>) {
        let d = if self.0.apps.has(app) { self.0.apps.describe(app).await.unwrap_or_default() } else { Value::Null };
        let list = |k: &str| d["uses"][k].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        (list("apps"), list("services"))
    }

    /// The entries that install (or uninstall) `app` in `scope` for `caller`, with their own rights there. Installing
    /// also installs each app it uses, and each they use, that isn't installed there yet, and runs its own services
    /// for it on `node` (a device's name, or `cloudflare`), one node each: added again with another node, they move
    /// there. The services of the apps it uses are theirs to place, each added with its own node (the App Store's
    /// add does). Uninstalling stops its services there.
    pub(super) async fn install_entries(&self, caller: &Caller, app: &str, scope: &Scope, installed: bool, node: Option<&str>) -> Result<Vec<(Scope, Content)>> {
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
        let services = self.uses(app).await.1;
        self.with(|i| {
            let m = i.r.db();
            let named = scope_name(i, &scope.to_string());
            ensure!(m.may(&caller.user, "add-apps", &scopes::object(scope)), "You can't add apps to {named}.");
            ensure!(i.r.holds(scope), "This device can't open {named} yet.");
            let here = i.views.get(&scope.to_string()).map(|s| s.installed.clone()).unwrap_or_default();
            let mut out = Vec::new();
            for (n, name) in wanted.iter().enumerate().rev() {
                ensure!(m.app(name).is_some(), "no app named {name}{}", if n > 0 { format!(", which {app} uses") } else { String::new() });
                if n == 0 || !here.contains(name) {
                    out.push((scope.clone(), content::node(if installed { "install" } else { "uninstall" }, json!({ "app": name }))));
                }
            }
            // Where each service runs for this scope now: stopped where it no longer should, started where it should.
            let ours = |s: &scopes::Switch| s.on && s.scope.as_deref() == Some(scope.to_string().as_str());
            let switches = m.switches();
            for service in &services {
                let running: Vec<Key> = switches.iter().filter(|((_, n), s)| n == service && ours(s)).map(|((k, _), _)| *k).collect();
                let to = match node.filter(|_| installed) {
                    Some(node) => Some(service_entry(m, &caller.user, node, service, true, Some(scope))?),
                    None => None,
                };
                let to_key = to.as_ref().and_then(|c| c.data["device"].as_str()).and_then(|k| unhex(k).ok());
                for k in running.iter().filter(|k| !installed || (to_key.is_some() && Some(**k) != to_key)) {
                    out.push((Scope::Global, service_entry(m, &caller.user, &hex(k), service, false, None)?));
                }
                if let Some(to) = to.filter(|_| !running.contains(&to_key.unwrap_or_default())) {
                    out.push((Scope::Global, to));
                }
            }
            Ok(out)
        })
    }

    /// Installs (or uninstalls) `app` in `scope` for `caller`, and what it uses, its services on `node`.
    pub async fn install(&self, caller: &Caller, app: &str, scope: &Scope, installed: bool, node: Option<&str>) -> Result<()> {
        self.read_callers_scopes(caller).await?;
        let entries = self.install_entries(caller, app, scope, installed, node).await?;
        for (scope, content) in entries {
            self.append_as(&scope, &caller.user, content).await?;
        }
        Ok(())
    }

    /// Forks `app` as a new app called `name`, owned by `owner` (by default the caller), with data of its own; it
    /// runs `app`'s latest deploy until it's deployed itself.
    pub async fn fork(&self, caller: &Caller, app: &str, name: &str, owner: Option<&str>) -> Result<()> {
        let bundle = self.with(|i| {
            let m = i.r.db();
            ensure!(m.app(name).is_none(), "there's already an app called {name}");
            m.app(app).with_context(|| format!("no app named {app}"))?.bundle.with_context(|| format!("{app} has no deploy to fork"))
        })?;
        let owner = owner.map(str::to_string).unwrap_or_else(|| format!("user:{}", caller.user));
        let builds = crate::app::bundle::services(&self.blob(&bundle).await?).into_values();
        self.member_as(caller, "app", json!({ "name": name, "owner": owner, "from": app })).await?;
        self.member_content(caller, deploy(name, &bundle, builds)).await
    }

    /// Deploys `bundle` as `app`'s code, for someone acting for its owner. Deploying an older bundle again rolls back.
    /// `bundle` may be the start of an earlier deploy's. With `wait`, → whether this device runs it within a few
    /// seconds, as its next call here would.
    pub async fn deploy(&self, caller: &Caller, app: &str, bundle: &str, wait: bool) -> Result<bool> {
        let bundle = self.with(|i| {
            let m = i.r.db();
            let a = m.app(app).with_context(|| format!("no app named {app}"))?;
            ensure!(m.may(&caller.user, "deploy", &scopes::app(app)), "only someone acting for {app}'s owner ({}) deploys it", a.owner);
            ensure!(a.deleted.is_none(), "{app} is in Recently Deleted: apps_restore {app} brings it back to deploy to, apps_erase {app} frees its name for good, or give this app another name");
            Ok(match a.history.iter().find(|(h, ..)| bundle.len() >= 8 && h.starts_with(bundle)) {
                Some((h, ..)) => h.clone(),
                None => bundle.to_string(),
            })
        })?;
        // From elsewhere, only a bundle they may read: their own, or one an entry names for them; so too its service's
        // builds, which the deploy names among its blobs.
        let mine = |blob: &str| -> Result<bool> { Ok(caller.key == self.0.id || self.may_read_blob(&caller.user, blob)?) };
        ensure!(mine(&bundle)?, "{app} has no deploy {bundle}: its versions are in apps_list's history");
        let bytes = self.blob(&bundle).await?;
        crate::app::bundle::module(&bytes)?;
        let builds: Vec<String> = crate::app::bundle::services(&bytes).into_values().collect();
        for build in &builds {
            ensure!(mine(build)?, "{app}'s bundle names a build of its service, {build}, that isn't here: put it first");
        }
        self.member_content(caller, deploy(app, &bundle, builds)).await?;
        Ok(wait && self.runs_latest(app, Duration::from_secs(10)).await)
    }

    /// Deploys `bundle` as `app`, as `deploy` does, making the app first if there's none by that name, owned by
    /// `owner` (by default the caller). → `{ bundle, made, before, here, ops }`: whether it was made, the deploy it ran
    /// before, and with `wait`, whether this device runs it now and then its tools (or why it doesn't start).
    pub async fn deploy_making(&self, caller: &Caller, app: &str, bundle: &str, owner: Option<&str>, wait: bool) -> Result<Value> {
        let before = self.with(|i| {
            let m = i.r.db();
            Ok(match m.app(app) {
                Some(a) => {
                    ensure!(m.may(&caller.user, "deploy", &scopes::app(app)), "there's already an app called {app}, and it isn't yours: give yours another name, or fork it");
                    Some(a.bundle)
                }
                None => None,
            })
        })?;
        if before.is_none() {
            ensure!(scopes::valid_app_name(app), "{app}: an app's name is lowercase words joined by -, like water-log");
            let owner = owner.map(str::to_string).unwrap_or_else(|| format!("user:{}", caller.user));
            self.member_as(caller, "app", json!({ "name": app, "owner": owner })).await?;
        }
        let here = self.deploy(caller, app, bundle, wait).await?;
        let bundle = self.with(|i| Ok(i.r.db().app(app).and_then(|a| a.bundle.clone())))?;
        let mut out = json!({ "bundle": bundle, "made": before.is_none(), "before": before.flatten(), "here": here });
        if here {
            match self.0.apps.describe(app).await {
                Ok(d) => out["ops"] = json!(d["ops"].as_array().into_iter().flatten().filter_map(|o| o["name"].as_str()).filter(|o| !o.starts_with('_')).collect::<Vec<_>>()),
                Err(e) => out["failed"] = json!(format!("{e:#}")),
            }
        }
        Ok(out)
    }

    /// An app's code as deployed, its latest or an earlier deploy's: its files, or one of them. Everyone may read
    /// every app's code.
    pub async fn source(&self, app: &str, bundle: Option<&str>, path: Option<&str>) -> Result<Value> {
        let history = self.with(|i| Ok(i.r.db().app(app).with_context(|| format!("no app named {app}"))?.history))?;
        let bundle = match bundle {
            Some(b) => history.iter().find(|(h, ..)| h.starts_with(b)).with_context(|| format!("{app} has no deploy {b}"))?.0.clone(),
            None => history.last().with_context(|| format!("{app} has no deploy yet"))?.0.clone(),
        };
        let mut out = crate::app::deploy::show(&self.blob(&bundle).await?, path)?;
        out["app"] = json!(app);
        out["bundle"] = json!(bundle);
        Ok(out)
    }
}

/// Where a built-in app's code is: its folder of `apps/` in the repo releases come from, at this build's commit when
/// it's one (else main); none for a custom build with no repo.
fn built_in_source(app: &str) -> Option<String> {
    let repo = crate::config::REPO;
    let commit = crate::config::BUILD.split(['-', '+']).next().filter(|c| !crate::config::BUILD.contains('+') && c.len() >= 7 && c.bytes().all(|b| b.is_ascii_hexdigit()));
    (!repo.is_empty()).then(|| format!("https://github.com/{repo}/tree/{}/apps/{app}", commit.unwrap_or("main")))
}

/// Where `app`'s service runs for the scopes `installed` names, and for `user` on their own nodes: `{ scope, node,
/// name, online, problem }` each, `name` none once that node was removed, `problem` what its node said went wrong.
fn runs_on(node: &Node, i: &Network, switches: &BTreeMap<(Key, String), scopes::Switch>, parts_here: &[Value], app: &str, installed: &[String], user: &str) -> Vec<Value> {
    let m = i.r.db();
    let cloudflare = m.cloudflare_node();
    switches
        .iter()
        .filter(|((k, n), s)| n == app && s.on && s.scope.as_ref().map_or(m.machine(k).is_some_and(|d| d.user == user), |s| installed.contains(s)))
        .map(|((k, _), s)| {
            let here = *k == node.0.id;
            let name = if Some(*k) == cloudflare { Some(crate::rank::CLOUDFLARE.to_string()) } else { m.machine(k).filter(|d| !d.removed).map(|d| d.name) };
            let parts = if here { Some(Value::from(parts_here.to_vec())) } else { i.peers.get(k).and_then(|p| p.parts.clone()) };
            let part = parts.and_then(|p| match p {
                Value::Array(list) => list.into_iter().find(|v| v["name"] == app),
                Value::Object(map) => map.get(app).cloned(),
                _ => None,
            });
            let online = here || Some(*k) == cloudflare || i.online(k);
            json!({ "scope": s.scope, "node": hex(k), "name": name, "online": online, "problem": part.and_then(|p| p["problem"].as_str().map(str::to_string)) })
        })
        .collect()
}
