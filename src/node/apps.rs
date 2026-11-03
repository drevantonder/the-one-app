//! Apps: the install's, every one people may add, and adding, installing, deploying, forking and retiring them.

use super::*;

impl Node {
    /// What people call `app`, for what they read: its title, or its sidecar's, else its name.
    pub(super) async fn title(&self, app: &str) -> String {
        match self.0.apps.title(app).await {
            Some(title) => title,
            None => self.sidecar_title(app),
        }
    }

    /// What people call the sidecar `name`: its title, else its name.
    pub(super) fn sidecar_title(&self, name: &str) -> String {
        let built_in = self.0.built_in_sidecars.iter().find(|b| b.name == name).and_then(|b| (b.describe)()["title"].as_str().map(str::to_string));
        built_in.or_else(|| self.0.sidecars.title(name)).unwrap_or_else(|| name.to_string())
    }

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
        let title = self.title(app).await;
        let (_, content) = self.with(|i| check_append(i, app, Append::Retire { app: app.into(), how: how.into() }, &[], caller, &title))?;
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

    /// `apps`, as a window draws them: what shows each one and where (no operations, schemas or instructions, which
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
            // For an app whose sign-ins keep accounts in the Vault, how many the viewer has: none means a sign-in comes first.
            if mine && let Some(needs) = info["vault"].as_object() {
                let accounts: Vec<Vec<String>> = needs.values().filter(|d| d["accounts"] == true).map(|d| d["fields"].as_object().map(|f| f.keys().cloned().collect()).unwrap_or_default()).collect();
                if !accounts.is_empty() {
                    info["accounts"] = json!(self.accounts_of(viewer, &name, &accounts)?);
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

    /// How many of `viewer`'s items, in the scopes they may use, are accounts `app`'s sign-in made: with all the fields
    /// of one of `needs`.
    fn accounts_of(&self, viewer: &Caller, app: &str, needs: &[Vec<String>]) -> Result<usize> {
        self.with(|i| {
            let m = &i.r.members;
            Ok(i.views
                .iter()
                .filter(|(log, _)| Scope::parse(log).is_ok_and(|s| m.may_use(&viewer.user, &s)))
                .flat_map(|(_, (_, s))| s.items.values())
                .filter(|it| it.app.as_deref() == Some(app) && needs.iter().any(|fields| fields.iter().all(|f| it.fields.contains_key(f))))
                .count())
        })
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
        ensure!(admin, "{app} is built in, but {} doesn't have it yet: an admin adds it first", crate::config::YOURS);
        self.member_as(caller, "app", json!({ "name": app, "owner": "admins" })).await?;
        self.member_content(caller, deploy(app, bundle)).await
    }

    /// What an app's latest deploy says it uses: (apps, sidecars). Waits a while for this device to run that deploy.
    pub(super) async fn uses(&self, app: &str) -> (Vec<String>, Vec<String>) {
        self.runs_latest(app, CALL_LIMIT).await;
        self.uses_here(app).await
    }

    /// Whether this device runs `app`'s latest deploy (or it has none), waiting up to `within` for it to.
    async fn runs_latest(&self, app: &str, within: Duration) -> bool {
        let started = Instant::now();
        loop {
            let latest = self.with(|i| Ok(i.r.members.apps.get(app).and_then(|a| a.bundle.clone()))).ok().flatten();
            if latest.is_none() || self.0.bundles.lock().unwrap().get(app) == latest.as_ref() {
                return true;
            }
            if started.elapsed() >= within {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// What the deploy of `app` this device runs says it uses: (apps, sidecars).
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
            let named = scope_name(i, &scope.to_string());
            ensure!(members::manages_scope(m, &caller.user, scope), "You can't add apps to {named}.");
            ensure!(i.r.holds(scope), "This device can't open {named} yet.");
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
        let entries = self.install_entries(caller, app, scope, installed).await?;
        self.may_schedule(caller, &installing(&entries)).await?;
        for (scope, content) in entries {
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
    /// `bundle` may be the start of an earlier deploy's. With `wait`, → whether this device runs it within a few
    /// seconds, as its next call here would.
    pub async fn deploy(&self, caller: &Caller, app: &str, bundle: &str, wait: bool) -> Result<bool> {
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
        self.member_content(caller, deploy(app, &bundle)).await?;
        Ok(wait && self.runs_latest(app, Duration::from_secs(10)).await)
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

/// The apps install entries install.
pub(super) fn installing(entries: &[(Scope, Content)]) -> Vec<&str> {
    entries.iter().filter(|(_, c)| c.kind == "install").filter_map(|(_, c)| c.data["app"].as_str()).collect()
}
