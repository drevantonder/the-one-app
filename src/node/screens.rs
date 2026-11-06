//! What people see of apps: widgets, badges, screens and the clients' own screens, and how their pages are arranged.

use super::*;

pub(crate) mod routes;

const BADGE_LIMIT: Duration = Duration::from_secs(5);

impl Node {
    pub(super) fn me_for(&self, caller: &Caller) -> Result<Me> {
        self.with(|i| Ok(node_me(self, i.r.db(), caller)))
    }

    /// The context a widget, hook or screen of `app` gets for `viewer`: every scope it may reach that's here.
    pub(super) fn read_context(&self, app: &str, viewer: &Caller) -> Result<Context> {
        let me = self.me_for(viewer)?;
        self.with(|i| {
            let reach = reachable(i, app, &viewer.user);
            let scopes: Vec<Scope> = reach.into_iter().filter(|s| i.r.holds(s)).collect();
            Ok(Context { me, now: now_ms(), scopes: handles(i.r.db(), &scopes, &viewer.user), read_only: true, token: String::new(), from: None, address: super::calls::address(i.r.db()), handles: i.r.db().handles(), labs: crate::labs::on_keys(&i.r.db().settings()) })
        })
    }

    /// Every widget, rendered for `viewer`; with `scope` (`user`, `group:<id>` or `global`), those of the apps
    /// installed there, for a shell's page of that scope, each reaching only it. Each comes at its own options at every
    /// size it draws, with its options (choices worked out now), and as each widget the viewer put on that page with
    /// options of their own (`placed`, by its id, at its size).
    pub async fn widgets(&self, viewer: Caller, scope: Option<&str>) -> Result<Vec<Value>> {
        let page = scope.map(|s| if s == format!("user:{}", viewer.user) { "user" } else { s });
        let placed: Vec<Value> = page
            .and_then(|p| self.arrangements(&viewer.user).remove(p))
            .and_then(|a| a["items"].as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|item| item["widget"].is_string() && item["options"].as_object().is_some_and(|o| !o.is_empty()))
            .collect();
        let none = json!({});
        let mut out = Vec::new();
        let contexts = self.page_contexts(&viewer, scope)?;
        // The keys of every app's database here that the caller didn't hand, asked for at once (428), as the Vault
        // does: an app's render reaches its data in its own process, which can't say what it lacked as a need, so a
        // widget would only say it stopped working until something else had them.
        let want: Vec<(Scope, &str, &str)> = contexts.iter().flat_map(|(app, ctx, _)| ctx.scopes.iter().filter_map(|s| Some((Scope::parse(&s.id).ok()?, app.as_str(), "main")))).collect();
        self.needs(&viewer, &want).await?;
        for (app, ctx, _granted) in contexts {
            let describe = self.0.apps.describe(&app).await?;
            for w in describe["widgets"].as_array().into_iter().flatten() {
                let widget = w["name"].as_str().unwrap_or_default();
                let size = w["size"].as_str().unwrap_or("medium");
                let options = match w["options"].as_array().is_some_and(|o| !o.is_empty()) {
                    true => self.0.apps.widget_options(&app, widget, &ctx).await.unwrap_or_else(|_| w["options"].clone()),
                    false => Value::Null,
                };
                match self.0.apps.widget(&app, widget, size, &none, &ctx).await {
                    Ok(root) => {
                        // Each other size it draws, as its own tree; one that fails is left out, and shells draw `root`.
                        let mut sizes = serde_json::Map::new();
                        for other in w["sizes"].as_array().into_iter().flatten().filter_map(|s| s.as_str()).filter(|s| *s != size) {
                            if let Ok(tree) = self.0.apps.widget(&app, widget, other, &none, &ctx).await {
                                sizes.insert(other.to_string(), tree);
                            }
                        }
                        sizes.insert(size.to_string(), root.clone());
                        let mut mine = serde_json::Map::new();
                        for item in placed.iter().filter(|i| i["app"] == app.as_str() && i["widget"] == widget) {
                            let at = item["size"].as_str().unwrap_or(size);
                            let tree = match self.0.apps.widget(&app, widget, at, &item["options"], &ctx).await {
                                Ok(tree) => json!({ "sizes": { at: tree } }),
                                Err(err) => json!({ "error": format!("{err:#}") }),
                            };
                            mine.insert(item["id"].as_str().unwrap_or_default().to_string(), tree);
                        }
                        let mut one = json!({ "app": app, "widget": widget, "title": w["title"], "description": w["description"], "size": size, "sizes": sizes, "root": root });
                        if !options.is_null() {
                            one["options"] = options;
                        }
                        if !mine.is_empty() {
                            one["placed"] = Value::Object(mine);
                        }
                        out.push(one)
                    }
                    // A shell that knows `error` says the widget is broken; one that doesn't still draws `root`.
                    Err(err) => out.push(json!({
                        "app": app, "widget": widget, "title": w["title"], "description": w["description"], "size": size, "error": format!("{err:#}"),
                        "root": { "t": "text", "text": "This widget stopped working.", "tone": "muted" },
                    })),
                }
            }
        }
        Ok(out)
    }

    /// Each app's badge, the count its icon shows, for `viewer`, as `widgets` reaches scopes; apps with none, or 0,
    /// are left out, as is one that fails or takes longer than `BADGE_LIMIT`: a page waits for its badges.
    pub async fn badges(&self, viewer: Caller, scope: Option<&str>) -> Result<Value> {
        let mut asked = tokio::task::JoinSet::new();
        for (app, ctx, granted) in self.page_contexts(&viewer, scope)? {
            let node = self.clone();
            asked.spawn(async move {
                let _granted = granted;
                let badge = async { if node.0.apps.describe(&app).await?["badge"] == true { node.0.apps.badge(&app, &ctx).await } else { Ok(Value::Null) } };
                match tokio::time::timeout(BADGE_LIMIT, badge).await {
                    Ok(Ok(n)) => n.as_f64().is_some_and(|n| n > 0.0).then_some((app, n)),
                    Ok(Err(err)) => {
                        tracing::warn!("{app} badge: {err:#}");
                        None
                    }
                    Err(_) => {
                        tracing::warn!("{app} badge took longer than {}s; left out", BADGE_LIMIT.as_secs());
                        None
                    }
                }
            });
        }
        Ok(Value::Object(asked.join_all().await.into_iter().flatten().collect()))
    }

    /// The apps a page shows and the context each renders with: with `scope`, those installed there, each reaching
    /// only it; without, every app, reaching all it may. Each comes with its grant, which lasts as long as it's kept.
    pub(super) fn page_contexts(&self, viewer: &Caller, scope: Option<&str>) -> Result<Vec<(String, Context, Granted)>> {
        let scope = scope.map(|s| Scope::parse_for(s, &viewer.user)).transpose()?;
        let mut out = Vec::new();
        for app in self.0.apps.names().into_iter().filter(|a| !self.deleted(a)) {
            let mut ctx = self.read_context(&app, viewer)?;
            let granted = self.grant(&app, viewer, &mut ctx, 0);
            if let Some(scope) = &scope {
                let installed = self.with(|i| Ok(i.views.get(&scope.to_string()).is_some_and(|s| s.installed.contains(&app))))?;
                ctx.scopes.retain(|s| installed && s.id == scope.to_string());
            }
            if !ctx.scopes.is_empty() {
                out.push((app, ctx, granted));
            }
        }
        Ok(out)
    }

    /// An app screen as a whole page: the app's HTML with the design kit, the bridge, and who's looking.
    /// `scope` is the page a shell opened it from, a hint the app may follow or not.
    pub async fn screen(&self, viewer: Caller, app: &str, name: &str, params: Value, scope: Option<&str>, linked: bool) -> Result<String> {
        let deployed = self.with(|i| Ok(i.r.db().app(app).is_some_and(|a| a.bundle.is_some())))?;
        // A screen the app doesn't have (an old or mistyped link), or an app not deployed here, is a page that says so.
        let body = if deployed { self.0.apps.screen(app, name).await?.unwrap_or_default() } else { String::new() };
        self.page(&viewer, json!({ "app": app, "screen": name, "params": params, "scope": scope }), &body, false, linked)
    }

    /// A screen's page: its body with the design kit, the bridge, and `window.__one`, which is `context` and who's
    /// looking.
    pub(super) fn page(&self, viewer: &Caller, mut context: Value, body: &str, own: bool, linked: bool) -> Result<String> {
        context["me"] = json!(self.me_for(viewer)?);
        let api = format!("http://localhost:{}", crate::api::port());
        context["config"] = json!({ "name": crate::config::NAME, "cli": crate::config::CLI, "api": api, "made_here": crate::config::MADE_HERE });
        Ok(crate::screen::page(&context, body, own, linked))
    }

    /// Arranges a page for `caller` alone: the order of its apps and widgets, which widgets, their sizes and options,
    /// and the apps taken off it, kept in their own scope so it follows them to every device; with their own page,
    /// `pages`, the order of their groups' pages and those they hid.
    pub async fn arrange(&self, caller: &Caller, page: &str, items: Value, hidden: Value, pages: Value) -> Result<()> {
        let scope = Scope::User(caller.user.clone());
        let content = self.with(|i| {
            ensure!(scopes::valid_page(page), "a page is user, group:<id> or global");
            ensure!(i.r.holds(&scope), "this device doesn't hold your data");
            let mut data = json!({ "page": page, "items": items, "hidden": if hidden.is_null() { json!([]) } else { hidden } });
            if page == "user" && !pages.is_null() {
                data["pages"] = pages;
            }
            Ok(content::node("arrange", data))
        })?;
        self.append_as(&scope, &caller.user, content).await?;
        Ok(())
    }

    /// How `user` arranged their pages, by page, as their own scope's `arrange` entries say.
    pub(super) fn arrangements(&self, user: &str) -> BTreeMap<String, Value> {
        self.with(|i| Ok(i.views.get(&Scope::User(user.to_string()).to_string()).map(|s| s.arrangements.clone()).unwrap_or_default())).unwrap_or_default()
    }
}
