//! This app runtime as its features see it (`api::AppRuntime`), and the features it runs, the one list (`features`).

use std::any::{Any, TypeId};

use ::api::runtime::{Apps, Databases, Draws, Features, Fut, Here, Holds, PageApp, Peers, Rt, Runs, ScopeDatabase};
use ::scope::read::Read;

use super::*;

/// Every feature this runtime runs, a line each.
pub fn features() -> Vec<Runs> {
    vec![
        crate::app::tools::add_runs(),
        crate::app::tools::remove_runs(),
        vault_runtime::runs(),
        crate::backups::tools::runs(),
        groups_runtime::runs(),
        crate::devices::tools::runs(),
        widgets_runtime::runs(),
        crate::push::tools::runs(),
        device_logs_runtime::runs(),
        crate::insights::tools::runs(),
        crate::ai_gateway::tools::runs(),
    ]
}

/// Every feature's routes that aren't tools.
pub fn routes() -> crate::api::Routes {
    features().into_iter().fold(crate::api::Routes::default(), |routes, runs| routes.merge(runs.routes))
}

/// A `Serves` of a feature whose runtime half is still this program's: an `async fn(Node, Caller, Map<String, Value>)`.
macro_rules! node_serves {
    ($f:path) => {
        |rt, me, input| Box::pin($f(crate::node::Node::of(&rt), me, input))
    };
}
pub(crate) use node_serves;

/// What the features keep while this runtime runs, and what wakes each one's loop.
#[derive(Default)]
pub(super) struct Kept {
    states: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    wakes: HashMap<&'static str, Arc<tokio::sync::Notify>>,
    problems: Vec<(&'static str, Problem)>,
}

/// What a feature says needs a viewer's attention (`Runs.problem`).
type Problem = fn(&Rt, &Caller) -> Result<Option<Value>>;

/// Each feature's loop: what it is, how often, and what it runs.
pub(super) type Loop = (&'static str, Duration, fn(Rt) -> Fut<'static, ()>);

/// What each feature does once, as the runtime starts, and its loop.
pub(super) type Starts = (Vec<fn(&Rt)>, Vec<Loop>);

impl Kept {
    /// What `features()` keep, and what `start` runs.
    pub(super) fn of(runs: Vec<Runs>) -> (Kept, Starts) {
        let (mut kept, mut starts, mut loops) = (Kept::default(), Vec::new(), Vec::new());
        for r in runs {
            starts.extend(r.start);
            kept.problems.extend(r.problem.map(|p| (r.feature, p)));
            if !r.state.is::<()>() {
                kept.states.insert((*r.state).type_id(), r.state);
            }
            if let Some((every, f)) = r.every {
                kept.wakes.insert(r.feature, Arc::default());
                loops.push((r.feature, every, f));
            }
        }
        (kept, (starts, loops))
    }
}

impl Node {
    /// This runtime, as its features reach it.
    pub fn rt(&self) -> Rt {
        Arc::new(self.clone())
    }

    /// The runtime a feature was given, which in this program is always a `Node`.
    pub fn of(rt: &Rt) -> Node {
        (&**rt as &dyn Any).downcast_ref::<Node>().expect("this program's features run in a Node").clone()
    }

    /// Starts each feature: what it does once, then its loop, at once, then every so often, or when it's woken.
    pub(super) fn start_features(&self, (starts, loops): Starts) {
        let rt = self.rt();
        for start in starts {
            start(&rt);
        }
        for (feature, every, f) in loops {
            let (node, wake) = (self.clone(), self.0.kept.wakes[feature].clone());
            tokio::spawn(async move {
                loop {
                    if let Err(err) = f(node.rt()).await {
                        tracing::debug!("{feature}: {err:#}");
                    }
                    let _ = tokio::time::timeout(every, wake.notified()).await;
                }
            });
        }
    }

    /// What each feature says needs `viewer`'s attention (`state.features`).
    pub(super) fn features_for(&self, viewer: &Caller) -> BTreeMap<String, Value> {
        let rt = self.rt();
        let mut out = BTreeMap::new();
        for (feature, problem) in &self.0.kept.problems {
            match problem(&rt, viewer) {
                Ok(Some(said)) => _ = out.insert(feature.to_string(), said),
                Ok(None) => {}
                Err(err) => tracing::debug!("{feature}'s problem: {err:#}"),
            }
        }
        out
    }

    /// Wakes every feature's loop: what it follows may have changed all at once, as when a network is taken.
    pub(super) fn wake_all(&self) {
        for wake in self.0.kept.wakes.values() {
            wake.notify_one();
        }
    }
}

impl axum::extract::FromRef<Rt> for Node {
    fn from_ref(rt: &Rt) -> Node {
        Node::of(rt)
    }
}

impl Here for Node {
    fn id(&self) -> Key {
        self.0.id
    }

    fn device_name(&self) -> String {
        Node::device_name(self)
    }

    fn home(&self) -> &Path {
        &self.0.dir
    }

    fn ready(&self) -> bool {
        Node::ready(self)
    }

    fn caller(&self) -> Result<Caller> {
        Node::caller(self)
    }

    fn is_this_machine(&self, caller: &Caller) -> bool {
        self.this_machine(caller)
    }

    fn own_ports(&self) -> Vec<u16> {
        Node::own_ports(self)
    }

    fn address(&self) -> Result<String> {
        Node::address(self)
    }

    fn emit(&self, event: Event) {
        Node::emit(self, event)
    }

    fn signing_key(&self) -> ::key::SigningKey {
        Node::signing_key(self)
    }
}

impl ScopeDatabase for Node {
    fn read_dyn(&self, f: &mut dyn FnMut(&Read<'_>) -> Result<()>) -> Result<()> {
        self.with(|i| f(&Read { replica: &i.r, states: &i.views }))
    }

    fn append<'a>(&'a self, scope: &'a Scope, user: &'a str, content: Content) -> Fut<'a, ()> {
        Box::pin(self.append_as(scope, user, content))
    }

    fn as_member<'a>(&'a self, caller: &'a Caller, content: Content) -> Fut<'a, ()> {
        Box::pin(self.member_content(caller, content))
    }

    fn read_callers_scopes<'a>(&'a self, caller: &'a Caller) -> Fut<'a, ()> {
        Box::pin(Node::read_callers_scopes(self, caller))
    }
}

impl Databases for Node {
    fn open<'a>(&'a self, caller: &'a Caller, scope: &'a Scope, app: &'a str, database: &'a str, schema: &'a [&'a str]) -> Fut<'a, Arc<tokio::sync::Mutex<Db>>> {
        Box::pin(async move {
            let (db, bucket) = self.call_database(caller, scope, app, database, Some(schema.iter().map(|s| s.to_string()).collect())).await?;
            self.read_ready(&(scope.clone(), format!("{app}/{database}")), &mut *db.lock().await, &bucket).await?;
            Ok(db)
        })
    }

    fn write<'a>(&'a self, caller: &'a Caller, scope: &'a Scope, app: &'a str, database: &'a str, schema: &'a [&'a str], change: &'a mut (dyn FnMut(&rusqlite::Connection) -> Result<()> + Send)) -> Fut<'a, ()> {
        Box::pin(async move {
            let (db, bucket) = self.call_database(caller, scope, app, database, Some(schema.iter().map(|s| s.to_string()).collect())).await?;
            let mut db = db.lock().await;
            self.read_ready(&(scope.clone(), format!("{app}/{database}")), &mut db, &bucket).await?;
            let at = db.at();
            self.write_in(scope, &mut db, &bucket, |c| change(c)).await?;
            if db.at() > at {
                self.ping(scope, Some((app, database, db.at())));
            }
            Ok(())
        })
    }

    fn needs<'a>(&'a self, caller: &'a Caller, wants: &'a [(Scope, &'a str, &'a str)]) -> Fut<'a, ()> {
        Box::pin(Node::needs(self, caller, wants))
    }

    fn object<'a>(&'a self, name: &'a str) -> Fut<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(::bucket::Bucket::get(&self.bucket()?, name).await?.map(|o| o.bytes)) })
    }

    fn blob<'a>(&'a self, hash: &'a str) -> Fut<'a, Vec<u8>> {
        Box::pin(Node::blob(self, hash))
    }

    fn put_blob_for(&self, caller: &Caller, bytes: &[u8]) -> Result<String> {
        Node::put_blob_for(self, caller, bytes)
    }

    fn may_read_blob(&self, caller: &Caller, hash: &str) -> Result<bool> {
        Ok(self.this_machine(caller) || Node::may_read_blob(self, &caller.user, hash)?)
    }
}

impl Holds for Node {
    fn device(&self) -> Key {
        self.0.holds.device()
    }

    fn vault_keys<'a>(&'a self, scope: &'a Scope, wanted: &'a [(u32, [u8; 32])]) -> Fut<'a, BTreeMap<u32, [u8; 32]>> {
        Box::pin(async move { self.0.holds.vault_keys(&self.bucket()?, scope, wanted).await })
    }

    fn answer_as_device<'a>(&'a self, method: &'a str, path: &'a str, body: &'a [u8]) -> Fut<'a, Value> {
        Box::pin(async move { Ok(self.answer_as_mine(method, path, body).await.map_err(|e| e.0)?.0) })
    }
}

impl Apps for Node {
    fn describe<'a>(&'a self, app: &'a str) -> Fut<'a, Value> {
        Box::pin(self.0.apps.describe(app))
    }

    fn apps_for<'a>(&'a self, viewer: &'a Caller) -> Fut<'a, Vec<Value>> {
        Box::pin(self.apps(viewer))
    }

    fn logo(&self, domain: &str, app: &str) -> Option<(String, String)> {
        Node::logo(self, domain, app)
    }

    fn call<'a>(&'a self, req: CallRequest, caller: Caller) -> Fut<'a, Value> {
        Box::pin(self.call_as(req, caller))
    }

    fn page_apps(&self, viewer: &Caller, scope: Option<&str>) -> Result<Vec<PageApp>> {
        Ok(self.page_contexts(viewer, scope)?.into_iter().map(|(app, ctx, granted)| PageApp {
            scopes: ctx.scopes.iter().filter_map(|s| Scope::parse(&s.id).ok()).collect(),
            draws: Arc::new(Drawn { node: self.clone(), app: app.clone(), ctx, _granted: granted }),
            app,
        }).collect())
    }

    fn describe_any<'a>(&'a self, app: &'a str) -> Fut<'a, Value> {
        Box::pin(self.0.apps.describe_any(app))
    }

    fn title<'a>(&'a self, app: &'a str) -> Fut<'a, String> {
        Box::pin(async move { Ok(Node::title(self, app).await) })
    }

    fn built_in(&self, app: &str) -> Option<String> {
        self.0.built_in.get(app).cloned()
    }

    fn bring_in<'a>(&'a self, caller: &'a Caller, app: &'a str) -> Fut<'a, ()> {
        Box::pin(Node::bring_in(self, caller, app))
    }

    fn uses<'a>(&'a self, app: &'a str) -> Fut<'a, (Vec<String>, Vec<String>)> {
        Box::pin(async move { Ok(Node::uses(self, app).await) })
    }

    fn runs_latest<'a>(&'a self, app: &'a str, within: Duration) -> Fut<'a, bool> {
        Box::pin(async move { Ok(Node::runs_latest(self, app, within).await) })
    }

    fn nodes_for(&self, viewer: &Caller, described: &Value) -> Option<Vec<String>> {
        Node::nodes_for(self, viewer, described)
    }
}

/// An app a page shows, drawing in its context.
struct Drawn {
    node: Node,
    app: String,
    ctx: Context,
    _granted: Granted,
}

impl Draws for Drawn {
    fn widget<'a>(&'a self, name: &'a str, size: &'a str, options: &'a Value) -> Fut<'a, Value> {
        Box::pin(self.node.0.apps.widget(&self.app, name, size, options, &self.ctx))
    }

    fn widget_options<'a>(&'a self, name: &'a str) -> Fut<'a, Value> {
        Box::pin(self.node.0.apps.widget_options(&self.app, name, &self.ctx))
    }

    fn badge(&self) -> Fut<'_, Value> {
        Box::pin(self.node.0.apps.badge(&self.app, &self.ctx))
    }
}

impl Peers for Node {
    fn reaches<'a>(&'a self, device: &'a Key) -> futures_util::future::BoxFuture<'a, bool> {
        Box::pin(Node::reaches(self, device))
    }

    fn away(&self, device: &Key) -> String {
        Node::away(self, device)
    }

    fn versions(&self) -> Fut<'_, Value> {
        Box::pin(Node::versions(self))
    }
}

impl Features for Node {
    fn state_dyn(&self, of: TypeId) -> Option<&(dyn Any + Send + Sync)> {
        self.0.kept.states.get(&of).map(|s| &**s)
    }

    fn wake(&self, feature: &str) {
        if let Some(wake) = self.0.kept.wakes.get(feature) {
            wake.notify_one();
        }
    }
}
