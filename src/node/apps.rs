//! Apps: the network's, every one users may add, and adding, deploying, forking and retiring them.

use crate::protocol::allowed;
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
            let mut d: Vec<Value> = i.peers.iter().filter(|(k, _)| i.heard(k)).flat_map(|(_, s)| s.services.iter()).filter(|d| d.1.contains(&viewer.user)).map(|d| d.2.clone()).collect();
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
                if let Some((image, kind)) = self.logo(domain, name) {
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
            Ok(crate::labs::on_keys(&i.r.db().settings()?))
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
            Ok(i.peers.iter().filter(|(k, _)| i.heard(k)).flat_map(|(_, s)| s.services.iter().map(|d| d.2.clone())).collect())
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
    /// run on, and `cloudflare` for an admin when it's built for that.
    pub async fn catalog(&self, viewer: &Caller, not_added: bool) -> Result<Vec<Value>> {
        let listed = self.with(|i| {
            let m = i.r.db();
            let apps = m.apps()?;
            let switches = m.switches()?;
            let parts_here = self.parts_here(i)?;
            apps.iter()
                .filter(|a| !a.erased)
                .map(|a| -> Result<_> {
                    let states = i.views.iter().filter_map(|(scope, st)| Some((Scope::parse(scope).ok()?, st))).collect::<Vec<_>>();
                    let added = crate::state::added(m, &a.name, &viewer.user, states.iter().map(|(s, st)| (s, *st)));
                    let runs_on = runs_on(self, i, &switches, &parts_here, &a.name, &added, &viewer.user)?;
                    let mut info = crate::state::app_info(m, a, &apps, &viewer.user, added)?;
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
            let here = self.0.apps.has(&name);
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
                    let on = self.with(|i| Ok(crate::labs::on_keys(&i.r.db().settings()?)))?;
                    tools.retain(|o| o["lab"].as_str().is_none_or(|lab| on.contains(&crate::labs::app_key(&name, lab))));
                }
                if mine {
                    let (list, activity) = self.schedules_of(viewer, &name, &declared)?;
                    info["schedules"] = json!(list);
                    info["activity"] = json!(activity);
                }
                if let Some(insights) = self.insights_of(viewer, &name)?.filter(|_| here) {
                    info["insights"] = insights;
                }
            }
            out.push(info);
        }
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

    /// Where an app that `described` itself runs its service for `viewer`, if it has one (its own, or one it uses): the
    /// names of their own nodes on a platform the service is built for, and `cloudflare` for an admin when it's built
    /// for that.
    fn nodes_for(&self, viewer: &Caller, described: &Value) -> Option<Vec<String>> {
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
            let mut nodes: Vec<String> = m
                .machines()?
                .into_iter()
                .filter(|d| !d.removed && d.user == viewer.user && d.is(scopes::Kind::Compute))
                .filter(|d| platforms.as_ref().is_none_or(|p| i.builds.get(&d.key).is_some_and(|b| p.contains(&platform_of(&b.target)))))
                .map(|d| d.name)
                .collect();
            if platforms.as_ref().is_some_and(|p| p.contains(&"cloudflare")) && m.cloudflare_node()?.is_some() && m.may(&viewer.user, "change-settings", scopes::NETWORK) {
                nodes.push(crate::rank::CLOUDFLARE.to_string());
            }
            Ok(nodes)
        })
        .ok()
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

    pub(super) fn built_in(&self, app: &scopes::App) -> Option<&String> {
        self.0.built_in.get(&app.name).filter(|_| app.owner == Principal::Admins && app.from.is_none())
    }

    /// Adds a built-in app the network doesn't have yet, owned by admins, as this release has it. Admins only.
    pub(super) async fn bring_in(&self, caller: &Caller, app: &str) -> Result<()> {
        let Some(bundle) = self.0.built_in.get(app) else { return Ok(()) };
        let (present, admin) = self.with(|i| Ok((i.r.db().app(app)?.is_some(), i.r.db().may(&caller.user, "add-apps", scopes::NETWORK))))?;
        if present {
            return Ok(());
        }
        ensure!(admin, "{app} is built in, but {} doesn't have it yet: an admin adds it first", crate::config::YOURS);
        self.member_content(caller, content::app(app, "admins")).await?;
        self.member_content(caller, content::deploy(app, bundle, [])).await
    }

    /// What an app's latest deploy says it uses: (apps, services). Waits a while for this device to run that deploy.
    pub(super) async fn uses(&self, app: &str) -> (Vec<String>, Vec<String>) {
        self.runs_latest(app, CALL_LIMIT).await;
        self.uses_here(app).await
    }

    async fn runs_latest(&self, app: &str, within: Duration) -> bool {
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

    /// The entries that add (or remove) `app` in `scope` for `caller`, with their own rights there. Adding it
    /// also adds each app it uses, and each they use, that isn't added there yet. Each of their services runs for
    /// the scope on one node, a node of the caller's it can run on (or `cloudflare` for an admin): `node` (a device's
    /// name) for `app`'s, which is asked for when there's a choice, and added again with another node, they move
    /// there; and for an app it uses, `node` if it can run there, else its one node, if it has one, and it stays where
    /// it runs already. Removing `app` stops its services there.
    pub(super) async fn add_entries(&self, caller: &Caller, app: &str, scope: &Scope, added: bool, node: Option<&str>) -> Result<Vec<(Scope, Content)>> {
        let mut wanted = vec![app.to_string()];
        let mut i = 0;
        while added && i < wanted.len() {
            self.bring_in(caller, &wanted[i]).await?;
            for used in self.uses(&wanted[i]).await.0 {
                if !wanted.contains(&used) {
                    wanted.push(used);
                }
            }
            i += 1;
        }
        // Each one's title, services, and the nodes of the caller's they can run on.
        let mut services = Vec::new();
        let mut widget = None;
        for (n, name) in wanted.iter().enumerate() {
            let nodes = match self.0.apps.describe_any(name).await {
                Ok(described) => {
                    if n == 0 {
                        widget = described["widgets"][0]["name"].as_str().map(str::to_string);
                    }
                    self.nodes_for(caller, &described)
                }
                Err(_) => None,
            };
            services.push((self.title(name).await, self.uses(name).await.1, nodes));
        }
        let arranged = self.arrangements(&caller.user);
        self.with(|i| {
            let m = i.r.db();
            let named = scope_name(i, &scope.to_string())?;
            allowed!(m.may(&caller.user, "add-apps", &scopes::object(scope)), "You can't add apps to {named}.");
            ensure!(i.r.holds(scope), "This device can't open {named} yet.");
            let here = i.views.get(&scope.to_string()).map(|s| s.added.clone()).unwrap_or_default();
            let mut out = Vec::new();
            for (n, name) in wanted.iter().enumerate().rev() {
                ensure!(m.app(name)?.is_some(), "no app named {name}{}: apps_list lists them", if n > 0 { format!(", which {app} uses") } else { String::new() });
                if n == 0 || !here.contains(name) {
                    out.push((scope.clone(), content::added(name, added)));
                }
            }
            // Where each service runs for this scope now: stopped where it no longer should, started where it should.
            let ours = |s: &scopes::Switch| s.on && s.scope.as_deref() == Some(scope.to_string().as_str());
            let switches = m.switches()?;
            for (n, (title, names, nodes)) in services.iter().enumerate() {
                let listed = |node: &str| nodes.iter().flatten().find(|l| l.eq_ignore_ascii_case(node)).cloned();
                let chosen = match (n, nodes) {
                    (_, _) if !added => None,
                    (0, Some(nodes)) => match node {
                        Some(node) => Some(listed(node).with_context(|| format!("{title} can't run on {node}: it can run on {}", if nodes.is_empty() { "none of your nodes".to_string() } else { nodes.join(", ") }))?),
                        None if nodes.len() > 1 => bail!("Where should {title} run? Give node: {}.", nodes.join(" or ")),
                        None => nodes.first().cloned(),
                    },
                    (_, Some(nodes)) => node.and_then(listed).or_else(|| (nodes.len() == 1).then(|| nodes[0].clone())),
                    _ => None,
                };
                for service in names {
                    let running: Vec<Key> = switches.iter().filter(|((_, name), s)| name == service && ours(s)).map(|((k, _), _)| *k).collect();
                    let to = match chosen.as_deref().filter(|_| n == 0 || running.is_empty()) {
                        Some(node) => Some(crate::settings::service(m, &caller.user, node, service, true, Some(scope))?),
                        None => None,
                    };
                    let to_key = to.as_ref().and_then(|c| c.data["device"].as_str()).and_then(|k| unhex(k).ok());
                    for k in running.iter().filter(|k| !added || (to_key.is_some() && Some(**k) != to_key)) {
                        out.push((Scope::Global, crate::settings::service(m, &caller.user, &hex(k), service, false, None)?));
                    }
                    if let Some(to) = to.filter(|_| !running.contains(&to_key.unwrap_or_default())) {
                        out.push((Scope::Global, to));
                    }
                }
            }
            // Newly added, its first widget goes on that page for whoever added it, unless they have one of it already.
            let own = Scope::User(caller.user.clone());
            if let (true, false, Some(widget), true) = (added, here.iter().any(|n| n == app), &widget, i.r.holds(&own)) {
                let page = if *scope == own { "user".to_string() } else { scope.to_string() };
                let was = arranged.get(&page).cloned().unwrap_or(Value::Null);
                let mut items = was["items"].as_array().cloned().unwrap_or_default();
                if !items.iter().any(|item| item["app"] == app && item["widget"].is_string()) {
                    items.push(json!({ "app": app, "widget": widget, "id": new_id() }));
                    out.push((own, content::arrange(&page, json!(items), was["hidden"].clone(), was["pages"].clone())));
                }
            }
            Ok(out)
        })
    }

    pub async fn add(&self, caller: &Caller, app: &str, scope: &Scope, added: bool, node: Option<&str>) -> Result<()> {
        self.read_callers_scopes(caller).await?;
        let entries = self.add_entries(caller, app, scope, added, node).await?;
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
            ensure!(m.app(name)?.is_none(), "there's already an app called {name}");
            m.app(app)?.with_context(|| format!("no app named {app}"))?.bundle.with_context(|| format!("{app} has no deploy to fork"))
        })?;
        let owner = owner.map(str::to_string).unwrap_or_else(|| format!("user:{}", caller.user));
        let builds = crate::app::bundle::services(&self.blob(&bundle).await?).into_values();
        self.member_content(caller, content::fork(name, &owner, app)).await?;
        self.member_content(caller, content::deploy(name, &bundle, builds)).await
    }

    /// Deploys `bundle` as `app`'s code, for someone acting for its owner. Deploying an older bundle again rolls back.
    /// `bundle` may be the start of an earlier deploy's. With `wait`, → whether this device runs it within a few
    /// seconds, as its next call here would.
    pub async fn deploy(&self, caller: &Caller, app: &str, bundle: &str, wait: bool) -> Result<bool> {
        let bundle = self.with(|i| {
            let m = i.r.db();
            let a = m.app(app)?.with_context(|| format!("no app named {app}"))?;
            allowed!(m.may(&caller.user, "deploy", &scopes::app(app)), "only someone acting for {app}'s owner ({}) deploys it", a.owner);
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
        self.member_content(caller, content::deploy(app, &bundle, builds)).await?;
        Ok(wait && self.runs_latest(app, Duration::from_secs(10)).await)
    }

    /// Deploys `bundle` as `app`, as `deploy` does, making the app first if there's none by that name, owned by
    /// `owner` (by default the caller). → `{ bundle, made, before, here, tools }`: whether it was made, the deploy it ran
    /// before, and with `wait`, whether this device runs it now and then its tools (or why it doesn't start).
    pub async fn deploy_making(&self, caller: &Caller, app: &str, bundle: &str, owner: Option<&str>, wait: bool) -> Result<Value> {
        let before = self.with(|i| {
            let m = i.r.db();
            Ok(match m.app(app)? {
                Some(a) => {
                    allowed!(m.may(&caller.user, "deploy", &scopes::app(app)), "there's already an app called {app}, and it isn't yours: give yours another name, or fork it");
                    Some(a.bundle)
                }
                None => None,
            })
        })?;
        if before.is_none() {
            ensure!(scopes::valid_app_name(app), "{app}: an app's name is lowercase words joined by -, like water-log");
            let owner = owner.map(str::to_string).unwrap_or_else(|| format!("user:{}", caller.user));
            self.member_content(caller, content::app(app, &owner)).await?;
        }
        let here = self.deploy(caller, app, bundle, wait).await?;
        let bundle = self.with(|i| Ok(i.r.db().app(app)?.and_then(|a| a.bundle.clone())))?;
        let mut out = json!({ "bundle": bundle, "made": before.is_none(), "before": before.flatten(), "here": here });
        if here {
            match self.0.apps.describe(app).await {
                Ok(d) => {
                    out["tools"] = json!(d["tools"].as_array().into_iter().flatten().filter_map(|o| o["name"].as_str()).filter(|o| !o.starts_with('_')).collect::<Vec<_>>());
                    out["lacking"] = json!(crate::app::deploy::lacking(&d));
                }
                Err(e) => out["failed"] = json!(format!("{e:#}")),
            }
        }
        Ok(out)
    }

    /// An app's code as deployed, its latest or an earlier deploy's: its files, or one of them. A built-in app no one
    /// has added gives the release's. Everyone may read every app's code.
    pub async fn source(&self, app: &str, bundle: Option<&str>, path: Option<&str>) -> Result<Value> {
        let history = self.with(|i| Ok(i.r.db().app(app)?.map(|a| a.history)))?;
        let built_in = self.0.built_in.get(app);
        ensure!(history.is_some() || built_in.is_some(), "no app named {app}");
        let history = history.unwrap_or_default();
        let bundle = match bundle {
            Some(b) => history.iter().find(|(h, ..)| h.starts_with(b)).with_context(|| format!("{app} has no deploy {b}"))?.0.clone(),
            None => history.last().map(|h| &h.0).or(built_in).with_context(|| format!("{app} has no deploy yet"))?.clone(),
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

/// Where `app`'s service runs for the scopes `added` names, and for `user` on their own nodes: `{ scope, node,
/// name, online, problem }` each, `name` none once that node was removed, `problem` what its node said went wrong.
fn runs_on(node: &Node, i: &Network, switches: &BTreeMap<(Key, String), scopes::Switch>, parts_here: &[crate::state::Part], app: &str, added: &[String], user: &str) -> Result<Vec<Value>> {
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
            let here = *k == node.0.id;
            let name = if Some(*k) == cloudflare { Some(crate::rank::CLOUDFLARE.to_string()) } else { m.machine(k)?.filter(|d| !d.removed).map(|d| d.name) };
            let parts = if here { Some(json!(parts_here)) } else { i.peers.get(k).and_then(|p| p.parts.clone()) };
            let part = parts.and_then(|p| match p {
                Value::Array(list) => list.into_iter().find(|v| v["name"] == app),
                Value::Object(map) => map.get(app).cloned(),
                _ => None,
            });
            let online = here || Some(*k) == cloudflare || i.online(k);
            out.push(json!({ "scope": s.scope, "node": hex(k), "name": name, "online": online, "problem": part.and_then(|p| p["problem"].as_str().map(str::to_string)) }));
        }
    }
    Ok(out)
}

/// A built-in app's bundle: its `app.js` as the module, and every file of its folder as its source. One with npm
/// packages (a `package.json`) or React screens (a `dist/`, built when the app was committed) is built as `<cli> deploy`
/// builds any app, its screens as they are.
pub fn built_in_bundle(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    if files.iter().any(|(p, _)| *p == "package.json" || p.starts_with("dist/")) {
        return crate::app::deploy::build_built_in(files);
    }
    let module = files.iter().find(|(p, _)| *p == "app.js").context("a built-in app has no app.js")?.1;
    let mut bundle = vec![("app.js".to_string(), module.to_vec())];
    bundle.extend(files.iter().map(|(p, b)| (format!("src/{p}"), b.to_vec())));
    crate::app::bundle::pack(&bundle)
}

