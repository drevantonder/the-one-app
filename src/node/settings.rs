//! A machine with a node as its user's own device (`crate::settings::Me`), as it's listed (`MACHINE`): what its CLI,
//! MCP and Mac app change of who can do what is signed by it, with the one copy every client runs; and as a client of
//! theirs, it lets in their new devices whose sign-in shows they're theirs, with no one tapping, as Settings does while
//! it lists requests.

use super::*;
use crate::bucket::Bucket;
use crate::settings::Me;

/// How often a client of its user's looks for their new devices whose sign-in lets them in.
const SIGN_INS_EVERY: Duration = Duration::from_secs(4);

/// How long an issuer's keys are kept before they're fetched again: a key it no longer signs with goes within the hour.
const ISSUER_KEYS_FOR: Duration = Duration::from_secs(3600);

/// Each issuer's keys, by where they're published, and when they were fetched.
pub(super) type IssuerKeys = HashMap<String, (Value, Instant)>;

impl crate::keys::Signs for Node {
    fn public(&self) -> [u8; 32] {
        self.0.id
    }

    async fn sign_bytes(&self, bytes: &[u8]) -> Result<[u8; 64]> {
        self.signing_key().sign_bytes(bytes).await
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
        crate::keys::public_of(&self.seal_secret())
    }

    fn welcome(&self) -> Option<crate::database::steps::Welcome> {
        self.with(|i| i.r.welcome()).ok().flatten()
    }

    async fn read(&self) -> Result<()> {
        self.read_scope_now(&Scope::Global).await
    }

    async fn write(&self, content: Content) -> Result<()> {
        self.member_content(&Caller::local(self)?, content).await
    }

    async fn held(&self, scope: &Scope) -> Result<Vec<(u32, [u8; 32])>> {
        Ok(self.epochs_held(scope).await?.0.into_iter().collect())
    }

    async fn keep_key(&self, sealed: Value) -> Result<()> {
        let text = |k: &str| sealed[k].as_str();
        let scope = Scope::parse(text("scope").unwrap_or_default())?;
        let n = sealed["n"].as_u64().context("n is an epoch's number")? as u32;
        self.seal_key(&Caller::local(self)?, &scope, n, text("to"), text("app"), sealed["vault"] == true, text("sealed").unwrap_or_default()).await.map(|_| ())
    }

    async fn app_key(&self, scope: &Scope, app: &str, n: u32) -> Result<[u8; 32]> {
        let epochs = self.epoch_keys(scope, true).await?;
        let newest = epochs.keys().next_back().copied().unwrap_or(0);
        let epoch = epochs.get(&n).with_context(|| format!("{} doesn't hold {scope}'s epoch {n}", self.device_name()))?;
        crate::keys::app_key(&self.bucket()?, epoch, &scope.to_string(), app, n, n == newest).await?.with_context(|| format!("{app} has no key in {scope}'s epoch {n}"))
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
            let mut response = match body {
                Some(body) => agent.run(request.header("content-type", "application/json").body(crate::keys::canonical(&body))?)?,
                None => agent.run(request.body(())?)?,
            };
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
}

/// The routes a machine with a node answers as its own device (`settings::on_a_node`), for its own programs.
pub(crate) fn routes() -> crate::api::Routes {
    let mut routes = crate::api::Routes::default();
    for (method, path) in crate::settings::on_a_node() {
        let filter = axum::routing::MethodFilter::try_from(axum::http::Method::from_bytes(method.as_bytes()).expect("a method")).expect("a method");
        let answer = move |axum::extract::State(node): axum::extract::State<Node>, body: axum::body::Bytes| async move { node.answer_as_mine(method, path, &body).await };
        routes = routes.route(path, crate::api::Who::ThisMachine, axum::routing::on(filter, answer));
    }
    routes
}

impl Node {
    /// What this machine's own programs ask of who can do what, answered as its own device (`settings::answer`).
    pub(crate) async fn answer_as_mine(&self, method: &str, path: &str, body: &[u8]) -> crate::api::Reply {
        let answered = crate::settings::answer(self, method, path, body).await.with_context(|| format!("{path} isn't one this device answers"))?;
        Ok(axum::Json(answered?))
    }

    /// What each start of a device does once it's in (`settings::settle_own`), in the background, on a client of its
    /// user's: their own scope's first epoch made as this node makes any it needs, one at a time.
    pub(super) fn settle_own(&self) {
        let Ok(me) = self.with(|i| Ok(i.r.db().client(&self.0.id)?.filter(|c| c.user == i.user).map(|c| c.user))) else { return };
        let Some(me) = me else { return };
        let node = self.clone();
        tokio::spawn(async move {
            let settled = async {
                let own = Scope::User(me);
                if node.with(|i| i.r.db().epoch(&own))? == 0 {
                    node.epoch_keys(&own, true).await?;
                }
                crate::settings::settle_own(&node, None, None).await
            };
            if let Err(err) = settled.await {
                tracing::warn!("settling this user's own things: {err:#}");
            }
        });
    }

    /// Every few seconds, on a client of its user's, lets in each of their new devices whose sign-in shows it's
    /// theirs (`settings::requests`), as their approving it would, with no one tapping; any other waits for them to
    /// approve it by its code. Nodes that only run calls let in no one.
    pub(super) async fn sign_in_loop(self) {
        loop {
            tokio::time::sleep(SIGN_INS_EVERY).await;
            let mine = self.with(|i| Ok(i.r.db().client(&self.0.id)?.is_some_and(|c| c.user == i.user)));
            if !mine.unwrap_or(false) || Node::address(&self).is_err() {
                continue;
            }
            if let Err(err) = crate::settings::requests(&self).await {
                tracing::debug!("looking for devices let in by their sign-in: {err:#}");
            }
        }
    }

    /// Lets back in someone who lost every device, with the network's recovery key, on this machine (never one asked
    /// over the network): the device whose join code is `code`, theirs, is listed by an entry the recovery key signs
    /// (which counts as an admin's approval, and is listed as its use), sealed each epoch of each scope they may open
    /// (opened through the recovery key's copy), and let onto the network by the recovery key. Without `confirm`, says
    /// whose it is. → what it is, or what was done.
    pub async fn approve_with_recovery(&self, code: &str, recovery: &ed25519_dalek::SigningKey, confirm: bool) -> Result<Value> {
        let at = recovery.verifying_key().to_bytes();
        ensure!(self.with(|i| Ok(i.r.db().recovery()? == Some(at)))?, "that isn't {}'s recovery key", crate::config::YOURS);
        let r = crate::settings::asked_with(self, code).await?;
        let (endpoint, user, name) = (*r.endpoint(), r.user().to_string(), r.name().to_string());
        let theirs = format!("the recovery key lets in a device of someone already in {}", crate::config::YOURS);
        ensure!(!r.new(), "{theirs}");
        let user_name = self.with(|i| Ok(i.r.db().user(&user)?.map(|u| u.name)))?.context(theirs)?;
        if !confirm {
            return Ok(json!({ "device": name, "user": user_name, "recovery": true }));
        }
        self.read_scope_now(&Scope::Global).await?;
        // Listed already by an approval that didn't finish: the rest is done again, each step once.
        if !self.with(|i| Ok(i.r.db().client(&endpoint.key)?.is_some_and(|c| c.user == user && c.seal == endpoint.seal)))? {
            self.write_step_as(recovery, &user, content::device(&endpoint, &user, &name, &[scopes::Kind::Client], None)).await?;
        }
        let relay = crate::settings::tell(self, recovery, json!({ "add": hex(&endpoint.key), "role": "client" })).await.err().map(|err| format!("{err:#}"));
        let scopes: Vec<Scope> = self.with(|i| Ok(i.r.db().may_scopes(&user, "use")?))?;
        let (mut sealed, mut not) = (Vec::new(), Vec::new());
        for scope in scopes {
            match self.seal_from_recovery(&scope, recovery, &endpoint).await {
                Ok(0) => {}
                Ok(_) => sealed.push(scope.to_string()),
                Err(err) => not.push(json!({ "scope": scope.to_string(), "why": format!("{err:#}") })),
            }
        }
        if let Err(err) = crate::settings::mark_approved(self, &endpoint.key).await {
            tracing::warn!("marking a request approved at auth: {err:#}");
        }
        Ok(json!({ "device": name, "user": user_name, "recovery": true, "sealed": sealed, "not_sealed": not, "relay": relay }))
    }

    /// Seals each epoch of `scope` its entries made, opened through the recovery key's copy, to `to` where it isn't yet.
    /// → how many it opened.
    async fn seal_from_recovery(&self, scope: &Scope, recovery: &ed25519_dalek::SigningKey, to: &crate::keys::Endpoint) -> Result<usize> {
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let hashes: BTreeMap<u32, String> = self.with(|i| Ok(i.r.db().epoch_hashes(scope)?))?;
        let secret = crate::keys::seal_secret(recovery);
        let mut opened = 0;
        for (n, hash) in hashes {
            let Some(o) = Bucket::get(&bucket, &crate::keys::epoch_name(&name, n, "recovery")).await? else {
                tracing::warn!("{scope}'s epoch {n} isn't sealed to the recovery key");
                continue;
            };
            let key = crate::keys::open_sealed(&secret, &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| crate::keys::epoch_hash(k) == hash);
            let Some(key) = key else {
                tracing::warn!("{scope}'s epoch {n} sealed to the recovery key isn't the key its entry made: passed over");
                continue;
            };
            let at = crate::keys::epoch_name(&name, n, &hex(&to.key));
            if Bucket::get(&bucket, &at).await?.is_none() {
                crate::database::stored::created(&bucket, &at, &crate::keys::seal_to(&to.seal, &key)?).await?;
            }
            opened += 1;
        }
        Ok(opened)
    }
}
