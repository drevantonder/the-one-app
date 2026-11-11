//! What users see of apps: screens, the clients' own screens, and how their pages are arranged; what a page's apps draw is Widgets' (widgets/).

use super::*;

pub(crate) mod routes;

impl Node {
    pub(super) fn me_for(&self, caller: &Caller) -> Result<Me> {
        self.with(|i| node_me(self, i.r.db(), caller))
    }

    /// The context a widget, hook or screen of `app` gets for `viewer`: every scope it may reach that's here.
    pub(super) fn read_context(&self, app: &str, viewer: &Caller) -> Result<Context> {
        let me = self.me_for(viewer)?;
        self.with(|i| {
            let reach = reachable(i, app, &viewer.user)?;
            let scopes: Vec<Scope> = reach.into_iter().filter(|s| i.r.holds(s)).collect();
            Ok(Context { me, now: now_ms(), scopes: handles(i.r.db(), &scopes, &viewer.user)?, read_only: true, token: String::new(), from: None, address: super::calls::address(i.r.db())?, handles: i.r.db().handles()?, labs: ::labs::on_keys(&i.r.db().settings()?) })
        })
    }

    /// The apps a page shows and the context each renders with: with `scope`, those added there, each reaching
    /// only it; without, every app, reaching all it may. Each comes with its grant, which lasts as long as it's kept.
    pub(super) fn page_contexts(&self, viewer: &Caller, scope: Option<&str>) -> Result<Vec<(String, Context, Granted)>> {
        let scope = scope.map(|s| Scope::parse_for(s, &viewer.user)).transpose()?;
        let mut out = Vec::new();
        for app in self.0.apps.names().into_iter().filter(|a| !self.deleted(a)) {
            let mut ctx = self.read_context(&app, viewer)?;
            let granted = self.grant(&app, viewer, &mut ctx, 0);
            if let Some(scope) = &scope {
                let added = self.with(|i| Ok(i.views.get(&scope.to_string()).is_some_and(|s| s.added.contains(&app))))?;
                ctx.scopes.retain(|s| added && s.id == scope.to_string());
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
        let row = self.with(|i| i.r.db().app(app))?;
        if row.as_ref().is_some_and(|a| a.deleted.is_some() && !a.erased) {
            bail!(custom_build::protocol::Coded::new(custom_build::protocol::code::APP_DELETED, format!("{app} was deleted: its owners can restore it from Recently Deleted in the App Store")));
        }
        let deployed = row.is_some_and(|a| a.bundle.is_some() && !a.erased);
        // A screen the app doesn't have (an old or mistyped link), or an app not deployed here, is a page that says so.
        let body = if deployed { self.0.apps.screen(app, name).await?.unwrap_or_default() } else { String::new() };
        self.page(&viewer, json!({ "app": app, "screen": name, "params": params, "scope": scope }), &body, linked)
    }

    /// A screen's page: its body with the design kit, the bridge, and `window.__one`, which is `context` and who's
    /// looking.
    pub(super) fn page(&self, viewer: &Caller, mut context: Value, body: &str, linked: bool) -> Result<String> {
        context["me"] = json!(self.me_for(viewer)?);
        let api = format!("http://localhost:{}", ::api::local::port());
        context["config"] = json!({ "name": custom_build::config::NAME, "cli": custom_build::config::CLI, "api": api, "made_here": custom_build::config::MADE_HERE });
        Ok(::ui::screen::page(&context, body, linked))
    }

    /// Arranges a page for `caller` alone: the order of its apps and widgets, which widgets, their sizes and options,
    /// and the apps taken off it, kept in their own scope so it follows them to every device; with their own page,
    /// `pages`, the order of their groups' pages and those they hid.
    pub async fn arrange(&self, caller: &Caller, page: &str, items: Value, hidden: Value, pages: Value) -> Result<()> {
        let scope = Scope::User(caller.user.clone());
        let content = self.with(|i| {
            ensure!(scopes::valid_page(page), "a page is user, group:<id> or global");
            ensure!(i.r.holds(&scope), "this device doesn't hold your data");
            Ok(content::arrange(page, items, hidden, pages))
        })?;
        self.append_as(&scope, &caller.user, content).await?;
        Ok(())
    }

    /// How `user` arranged their pages, by page, as their own scope's `arrange` entries say.
    pub(super) fn arrangements(&self, user: &str) -> BTreeMap<String, Value> {
        self.with(|i| Ok(::scope::read::Read { replica: &i.r, states: &i.views, scopes: &i.scope_dbs }.arrangements(user))).unwrap_or_default()
    }
}
