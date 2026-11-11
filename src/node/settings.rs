//! A machine with a node as its user's own device (`::scope::settings::Me`), as it's listed (`MACHINE`): what its CLI,
//! MCP and Mac app change of who can do what is signed by it, with the one copy every client runs; and on an admin's,
//! it lets in those an invite already said yes to, as Settings does while it lists requests.

use super::*;
use ::bucket::Bucket;
use ::scope::settings::Me;

/// How often a client looks for someone an invite lets in.
const INVITES_EVERY: Duration = Duration::from_secs(4);

/// How long an issuer's keys are kept before they're fetched again: a key it no longer signs with goes within the hour.
const ISSUER_KEYS_FOR: Duration = Duration::from_secs(3600);

/// Each issuer's keys, by where they're published, and when they were fetched.
pub(super) type IssuerKeys = HashMap<String, (Value, Instant)>;

impl key::Signs for Node {
    fn public(&self) -> [u8; 32] {
        key::Signs::public(&self.0.holds)
    }

    async fn sign_bytes(&self, bytes: &[u8]) -> Result<[u8; 64]> {
        self.0.holds.sign_bytes(bytes).await
    }
}

impl Me for Node {
    fn who<R>(&self, f: impl FnOnce(&ScopeDb) -> Result<R>) -> Result<R> {
        self.with(|i| f(i.r.db()))
    }

    fn user(&self) -> Result<String> {
        Node::user(self)
    }

    fn seal(&self) -> [u8; 32] {
        key::Opens::public(&self.0.holds)
    }

    fn welcome(&self) -> Option<::scope::steps::Welcome> {
        self.with(|i| i.r.welcome()).ok().flatten()
    }

    async fn read(&self) -> Result<()> {
        self.read_scope_now(&Scope::Global).await
    }

    async fn write(&self, content: Content) -> Result<()> {
        self.member_content(&self.caller()?, content).await
    }

    async fn held(&self, scope: &Scope) -> Result<Vec<(u32, [u8; 32])>> {
        Ok(self.epochs_held(scope).await?.0.into_iter().collect())
    }

    async fn keep_key(&self, sealed: Value) -> Result<()> {
        let text = |k: &str| sealed[k].as_str();
        let scope = Scope::parse(text("scope").unwrap_or_default())?;
        let n = sealed["n"].as_u64().context("n is an epoch's number")? as u32;
        self.seal_key(&self.caller()?, &scope, n, text("to"), text("app"), sealed["vault"] == true, text("sealed").unwrap_or_default()).await.map(|_| ())
    }

    async fn app_key(&self, scope: &Scope, app: &str, n: u32) -> Result<[u8; 32]> {
        let epochs = self.epoch_keys(scope, true).await?;
        let epoch = epochs.get(&n).with_context(|| format!("{} doesn't hold {scope}'s epoch {n}", self.device_name()))?;
        // An app added after an epoch ended has no key in it until this asks, as a browser's own keyring makes one.
        database::keys::app_key(&self.bucket()?, epoch, &scope.to_string(), app, n, true).await?.with_context(|| format!("{app} has no key in {scope}'s epoch {n}"))
    }

    fn made_epoch(&self, scope: &Scope, _: u32, _: [u8; 32]) {
        self.forget_epochs(scope);
    }

    fn address(&self) -> Option<String> {
        Node::address(self).ok()
    }

    async fn ask(&self, method: &str, url: &str, header: Option<(&str, String)>, body: Option<Value>) -> Result<(u16, Value)> {
        let (method, url, header) = (method.to_string(), url.to_string(), header.map(|(n, v)| (n.to_string(), v)));
        tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
            let mut request = ureq::http::Request::builder().method(method.as_str()).uri(&url);
            if let Some((name, value)) = header {
                request = request.header(name, value);
            }
            let request = match body {
                Some(body) => request.header("content-type", "application/json").body(key::canonical(&body))?,
                None => request.body(Vec::new())?,
            };
            let twice = if method == "GET" { platform::hedge::Twice::Safe } else { platform::hedge::Twice::Never };
            let (mut response, _) = platform::hedge::run(&agent, "address", request, twice)?;
            let status = response.status().as_u16();
            Ok((status, response.body_mut().read_json().unwrap_or(Value::Null)))
        })
        .await?
    }

    async fn issuer_keys(&self, url: &str) -> Option<Value> {
        if let Some((keys, when)) = self.0.issuer_keys.lock().unwrap().get(url)
            && when.elapsed() < ISSUER_KEYS_FOR
        {
            return Some(keys.clone());
        }
        let (status, keys) = Me::ask(self, "GET", url, None, None).await.inspect_err(|err| tracing::warn!("fetching {url}, to check a sign-in: {err:#}")).ok()?;
        (status == 200).then_some(())?;
        self.0.issuer_keys.lock().unwrap().insert(url.to_string(), (keys.clone(), Instant::now()));
        Some(keys)
    }

    async fn kept(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.dir.join(name)).ok()
    }

    async fn keep(&self, name: &str, bytes: &[u8]) -> Result<()> {
        write_private(&self.0.dir.join(name), bytes)
    }

    fn now(&self) -> u64 {
        now_ms()
    }

    async fn wait(&self, ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    async fn leave(&self) -> Result<()> {
        bail!("{} is this device: remove it from another of your devices", self.device_name())
    }

    async fn stored(&self, name: &str) -> Result<Option<Vec<u8>>> {
        Ok(Bucket::get(&self.bucket()?, name).await?.map(|o| o.bytes))
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        database::stored::created(&self.bucket()?, name, bytes).await
    }

    async fn delete(&self, name: &str) -> Result<()> {
        Bucket::delete(&self.bucket()?, name).await
    }

    async fn write_as(&self, key: &key::SigningKey, user: &str, content: Content) -> Result<()> {
        self.write_step_as(key, user, content).await.map(drop)
    }

    async fn open(&self, sealed: &[u8]) -> Result<Vec<u8>> {
        key::open_sealed_by(&self.0.holds, sealed).await
    }
}

/// The routes a machine with a node answers as its own device (`settings::on_a_node`), for its own programs.
pub(crate) fn routes() -> crate::api::Routes {
    let mut routes = crate::api::Routes::default();
    for (method, path) in ::scope::settings::on_a_node() {
        let filter = axum::routing::MethodFilter::try_from(axum::http::Method::from_bytes(method.as_bytes()).expect("a method")).expect("a method");
        let answer = move |axum::extract::State(node): axum::extract::State<Node>, body: axum::body::Bytes| async move { node.answer_as_mine(method, path, &body).await };
        routes = routes.route(path, ::api::Who::ThisMachine, axum::routing::on(filter, answer));
    }
    use ::api::Who;
    use axum::routing::post;
    routes
        .route("/api/pair", Who::Clients, post(own::pair))
        .route("/api/name", Who::ThisMachine, post(own::name))
        .route("/api/devices/approve-with-recovery", Who::ThisMachine, post(own::approve_with_recovery))
        .route("/api/devices/recovery-key/replace", Who::ThisMachine, post(own::replace_recovery))
        .route("/api/devices/recovery-holders/seal", Who::ThisMachine, post(own::seal_recovery))
}

/// Devices' routes this machine signs as its own device (`::devices::says`): pairing a machine, its own name, and letting
/// one in with the network's recovery key, which the command on this computer read (`devices::cli`).
mod own {
    use axum::Json;
    use axum::extract::State;
    use serde::Deserialize;
    use serde_json::json;

    use ::api::{Body, Me, Reply, done, given};
    use ::devices::says::{ApproveWithRecovery, Pair, ReplaceRecovery, SealRecovery, recovery_key};

    use crate::node::Node;

    pub async fn pair(State(node): State<Node>, me: Me, Body(body): Body<Pair>) -> Reply {
        let (code, user) = node.pair_as(&me.caller, body.user.as_deref())?;
        Ok(Json(json!({ "code": code, "user": user })))
    }

    /// Renames this device, for this machine's own programs: another is renamed as any device's user does
    /// (`settings::answer`).
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Name {
        name: String,
    }

    pub async fn name(State(node): State<Node>, Body(body): Body<Name>) -> Reply {
        node.rename_device(given(&body.name, "name")?).await?;
        done()
    }

    pub async fn approve_with_recovery(State(node): State<Node>, Body(body): Body<ApproveWithRecovery>) -> Reply {
        let key = recovery_key(&body.recovery, "a recovery key")?;
        Ok(Json(::scope::settings::recovery::approve(&node, given(&body.code, "code")?, &key, body.confirm).await?))
    }

    pub async fn replace_recovery(State(node): State<Node>, Body(body): Body<ReplaceRecovery>) -> Reply {
        Ok(Json(::scope::settings::recovery::replace(&node, &recovery_key(&body.recovery, "a recovery key")?, &recovery_key(&body.next, "the next recovery key")?, body.confirm).await?))
    }

    pub async fn seal_recovery(State(node): State<Node>, Body(body): Body<SealRecovery>) -> Reply {
        ::scope::settings::recovery::seal(&node, given(&body.to, "to")?, given(&body.copy, "copy")?).await?;
        done()
    }
}

impl Node {
    /// What this machine's own programs ask of who can do what, answered as its own device (`settings::answer`).
    pub(crate) async fn answer_as_mine(&self, method: &str, path: &str, body: &[u8]) -> ::api::Reply {
        let answered = ::scope::settings::answer(self, method, path, body).await.with_context(|| format!("{path} isn't one this device answers"))?;
        Ok(axum::Json(answered?))
    }

    /// What each start of a device does once it's in (`settings::settle_own`), in the background, on a client of its
    /// user's: their own scope's first epoch made as this node makes any it needs, one at a time.
    pub(super) fn settle_own(&self) {
        let Ok(me) = self.with(|i| Ok(i.r.db().client(&self.0.holds.device())?.filter(|c| c.user == i.user).map(|c| c.user))) else { return };
        let Some(me) = me else { return };
        let node = self.clone();
        tokio::spawn(async move {
            let settled = async {
                let own = Scope::User(me);
                if node.with(|i| i.r.db().epoch(&own))? == 0 {
                    node.epoch_keys(&own, true).await?;
                }
                ::scope::settings::settle_own(&node, None, None).await
            };
            if let Err(err) = settled.await {
                tracing::warn!("settling this user's own things: {err:#}");
            }
        });
    }

    /// Every few seconds, on a client of the network's, while an invite is open, lets in each newcomer it already said
    /// yes to (`settings::requests`). Everyone else waits for someone to tap Approve.
    pub(super) async fn invites_loop(self) {
        loop {
            tokio::time::sleep(INVITES_EVERY).await;
            let client = self.with(|i| Ok(i.r.db().client(&self.0.holds.device())?.is_some_and(|c| c.user == i.user)));
            if !client.unwrap_or(false) || Node::address(&self).is_err() || !::scope::settings::invite_open(&self).await {
                continue;
            }
            if let Err(err) = ::scope::settings::requests(&self).await {
                tracing::debug!("looking for someone an invite lets in: {err:#}");
            }
        }
    }
}
