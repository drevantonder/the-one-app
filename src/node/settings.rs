//! A machine with a node as its user's own device (`crate::settings::Me`), as it's listed (`MACHINE`): what its CLI,
//! MCP and Mac app change of who can do what is signed by it, with the one copy every client runs; and on an admin's,
//! it lets in those an invite already said yes to, as Settings does while it lists requests.

use super::*;
use crate::bucket::Bucket;
use crate::settings::Me;

/// How often an admin's client looks for someone their invite lets in.
const INVITES_EVERY: Duration = Duration::from_secs(4);

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

    async fn stored(&self, name: &str) -> Result<Option<Vec<u8>>> {
        Ok(Bucket::get(&self.bucket()?, name).await?.map(|o| o.bytes))
    }

    async fn open(&self, sealed: &[u8]) -> Result<Vec<u8>> {
        crate::keys::open_sealed(&self.seal_secret(), sealed)
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

    /// Every few seconds, on an admin's client, lets in each newcomer an invite already said yes to
    /// (`settings::requests`). Everyone else waits for someone to tap Approve.
    pub(super) async fn invites_loop(self) {
        loop {
            tokio::time::sleep(INVITES_EVERY).await;
            let admin = self.with(|i| Ok(i.r.db().client(&self.0.id)?.is_some_and(|c| c.user == i.user) && i.r.db().may(&i.user, "invite", scopes::NETWORK)));
            if !admin.unwrap_or(false) || Node::address(&self).is_err() {
                continue;
            }
            if let Err(err) = crate::settings::requests(&self).await {
                tracing::debug!("looking for someone an invite lets in: {err:#}");
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
        let admin = self.with(|i| Ok(i.r.db().may(&user, "invite", scopes::NETWORK)))?;
        let relay = crate::settings::tell(self, recovery, json!({ "add": hex(&endpoint.key), "role": "client", "admin": admin })).await.err().map(|err| format!("{err:#}"));
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
        let twin = crate::keys::twin_public(&recovery.verifying_key().to_bytes())?;
        let mut opened = 0;
        for (n, hash) in hashes {
            let Some(key) = self.opened_for(&name, n, &hash, &secret, &twin).await? else {
                tracing::warn!("{scope}'s epoch {n} isn't sealed to the recovery key, or isn't the key its entry made: passed over");
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

    /// Epoch `n` of the scope named `name`, opened with the recovery key whose twin's secret and public are given.
    async fn opened_for(&self, name: &str, n: u32, hash: &str, secret: &[u8; 32], twin: &[u8; 32]) -> Result<Option<[u8; 32]>> {
        let bucket = self.bucket()?;
        let mut kept = Vec::new();
        for at in crate::keys::recovery_epoch_names(name, n, twin) {
            kept.extend(Bucket::get(&bucket, &at).await?.map(|o| o.bytes));
        }
        Ok(crate::keys::opened_recovery_epoch(secret, hash, &kept))
    }

    /// Replaces the recovery key on this machine, from its file, the only way to take one that was seen or stolen out of
    /// use: every epoch and Vault key it opens is sealed to the next too, so nothing already there is lost to it; the next
    /// is named; and each scope still in use gets a new epoch, and each Vault new keys, sealed to the next and not the
    /// old, so what the old one opens stops growing. Copies held under the old key stop counting, as they're kept under
    /// its name, and nothing already sealed is touched. Run again (with `old` that was, or is now, the network's key) it
    /// finishes what it didn't. Without `confirm`, says what it would do. → what was done.
    pub async fn replace_recovery(&self, old: &ed25519_dalek::SigningKey, new: &ed25519_dalek::SigningKey, confirm: bool) -> Result<Value> {
        self.read_scope_now(&Scope::Global).await?;
        let (old_public, new_public) = (old.verifying_key().to_bytes(), new.verifying_key().to_bytes());
        ensure!(old_public != new_public, "The new recovery key is the same as the one it replaces.");
        let (now, user, scopes, holders) = self.with(|i| {
            let db = i.r.db();
            let scopes: Vec<Scope> = db.scopes()?.into_iter().filter(|s| db.epoch(s).is_ok_and(|n| n > 0)).filter(|s| in_use(db, s)).collect();
            Ok((db.recovery()?, i.user.clone(), scopes, db.recovery_holders()?.len()))
        })?;
        ensure!(now == Some(old_public) || now == Some(new_public), "That file doesn't hold {}'s recovery key.", crate::config::YOURS);
        if !confirm {
            return Ok(json!({ "said": format!("Would replace {}'s recovery key: {} scopes get new keys, and whatever {holders} holder(s) were sealed stops counting until the new key is sealed to them. Run it again with --confirm.", crate::config::YOURS, scopes.len()) }));
        }
        let not_carried = self.carry_to_next(old, new).await?;
        if now == Some(old_public) {
            self.write_step_as(old, &user, content::recovery(&new_public)).await?;
            self.read_scope_now(&Scope::Global).await?;
        }
        ensure!(self.with(|i| i.r.db().recovery())? == Some(new_public), "The next recovery key wasn't taken. Nothing else changed: run this again.");
        let (renewed, not_renewed) = self.renew_under_next(new, &user).await?;
        let said = format!(
            "{} has a new recovery key: {} scopes have new keys, and old holders' copies no longer count. Seal the new key to its holders with `{} devices holders seal` when you're ready.{}{}",
            crate::config::YOURS,
            renewed.len(),
            crate::config::CLI,
            if not_carried.is_empty() { String::new() } else { format!(" Couldn't carry over: {}.", not_carried.join(", ")) },
            if not_renewed.is_empty() { String::new() } else { format!(" Couldn't renew {}: run this again.", not_renewed.iter().map(|n| n["scope"].as_str().unwrap_or("")).collect::<Vec<_>>().join(", ")) },
        );
        Ok(json!({ "said": said, "renewed": renewed, "not_renewed": not_renewed, "not_carried": not_carried }))
    }

    /// Seals every epoch and Vault key `old` opens to `new`, under `new`'s own name, beside what's there: → what it couldn't.
    async fn carry_to_next(&self, old: &ed25519_dalek::SigningKey, new: &ed25519_dalek::SigningKey) -> Result<Vec<String>> {
        let bucket = self.bucket()?;
        let (old_secret, old_twin) = (crate::keys::seal_secret(old), crate::keys::twin_public(&old.verifying_key().to_bytes())?);
        let new_twin = crate::keys::twin_public(&new.verifying_key().to_bytes())?;
        let to = crate::keys::recovery_to(&new_twin);
        let (hashes, vaults) = self.with(|i| {
            let db = i.r.db();
            let scopes = db.scopes()?;
            let hashes: Vec<(Scope, BTreeMap<u32, String>)> = scopes.iter().map(|s| Ok::<_, anyhow::Error>((s.clone(), db.epoch_hashes(s)?))).collect::<Result<_>>()?;
            let vaults: Vec<(Scope, BTreeMap<u32, Key>)> = scopes.iter().map(|s| Ok::<_, anyhow::Error>((s.clone(), db.vault_publics(s)?))).collect::<Result<_>>()?;
            Ok((hashes, vaults))
        })?;
        let mut not = Vec::new();
        for (scope, epochs) in hashes {
            let name = scope.to_string();
            for (n, hash) in epochs {
                // Under the new key already: made since a handover that stopped.
                if self.opened_for(&name, n, &hash, &crate::keys::seal_secret(new), &new_twin).await?.is_some() {
                    continue;
                }
                match self.opened_for(&name, n, &hash, &old_secret, &old_twin).await? {
                    Some(key) => drop(crate::database::stored::created(&bucket, &crate::keys::epoch_name(&name, n, &to), &crate::keys::seal_to(&new_twin, &key)?).await?),
                    None => not.push(format!("{scope}'s epoch {n}")),
                }
            }
        }
        for (scope, publics) in vaults {
            let name = scope.to_string();
            for (n, public) in publics {
                let mut secret = None;
                if let Some(o) = Bucket::get(&bucket, &crate::keys::vault_key_name(&name, n, "recovery")).await?
                    && crate::keys::open_sealed(&crate::keys::seal_secret(new), &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).is_some_and(|k| crate::keys::public_of(&k) == public)
                {
                    continue;
                }
                for at in [crate::keys::vault_key_name(&name, n, &crate::keys::recovery_to(&old_twin)), crate::keys::vault_key_name(&name, n, "recovery")] {
                    if let Some(o) = Bucket::get(&bucket, &at).await?
                        && let Some(k) = crate::keys::open_sealed(&old_secret, &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok())
                        && crate::keys::public_of(&k) == public
                    {
                        secret = Some(k);
                        break;
                    }
                }
                match secret {
                    Some(k) => drop(crate::database::stored::created(&bucket, &crate::keys::vault_key_name(&name, n, &to), &crate::keys::seal_to(&new_twin, &k)?).await?),
                    None => not.push(format!("{scope}'s Vault key {n}")),
                }
            }
        }
        Ok(not)
    }

    /// A new epoch of each scope still in use, and new Vault keys, signed by the network's recovery key `new`, sealed to it
    /// and to whoever may open them: each one already sealed to it is left as it is. → what was done, and what wasn't.
    async fn renew_under_next(&self, new: &ed25519_dalek::SigningKey, user: &str) -> Result<(Vec<String>, Vec<Value>)> {
        let bucket = self.bucket()?;
        let (secret, twin) = (crate::keys::seal_secret(new), crate::keys::twin_public(&new.verifying_key().to_bytes())?);
        let (scopes, vaults) = self.with(|i| {
            let db = i.r.db();
            Ok((db.scopes()?.into_iter().filter(|s| in_use(db, s)).collect::<Vec<_>>(), db.vaults()?))
        })?;
        let (mut renewed, mut not) = (Vec::new(), Vec::new());
        for scope in scopes {
            let name = scope.to_string();
            let (newest, hash) = self.with(|i| {
                let db = i.r.db();
                let n = db.epoch(&scope)?;
                Ok((n, db.epoch_hash(&scope, n)?))
            })?;
            if newest == 0 {
                continue;
            }
            // Sealed to the new key under `recovery`, as only an epoch made since the handover is: a run that finishes one
            // that stopped. A carried-over copy, under its own name, doesn't count.
            let held = Bucket::get(&bucket, &crate::keys::epoch_name(&name, newest, "recovery")).await?;
            if hash.is_some_and(|h| crate::keys::opened_recovery_epoch(&secret, &h, &held.into_iter().map(|o| o.bytes).collect::<Vec<_>>()).is_some()) {
                continue;
            }
            match self.epoch_by_recovery(new, user, &scope, newest + 1, &twin).await {
                Ok(()) => renewed.push(name),
                Err(err) => not.push(json!({ "scope": name, "why": format!("{err:#}") })),
            }
        }
        for (scope, vault) in vaults.into_iter().filter(|(s, _)| self.with(|i| Ok(in_use(i.r.db(), s))).unwrap_or(false)) {
            let name = scope.to_string();
            let held = Bucket::get(&bucket, &crate::keys::vault_key_name(&name, vault.n, "recovery")).await?;
            if held.and_then(|o| crate::keys::open_sealed(&secret, &o.bytes).ok()).and_then(|k| <[u8; 32]>::try_from(k).ok()).is_some_and(|k| crate::keys::public_of(&k) == vault.public) {
                continue;
            }
            if let Err(err) = self.vault_by_recovery(new, user, &scope, vault.n + 1, &vault.to, &twin).await {
                not.push(json!({ "scope": format!("{name}'s Vault"), "why": format!("{err:#}") }));
            }
        }
        Ok((renewed, not))
    }

    /// Starts epoch `n` of `scope`, signed by the network's recovery key: its step first, as a new epoch always is, then
    /// sealed to the recovery key's twin and to each client of whoever may open it.
    async fn epoch_by_recovery(&self, recovery: &ed25519_dalek::SigningKey, user: &str, scope: &Scope, n: u32, twin: &[u8; 32]) -> Result<()> {
        let to: Vec<crate::keys::Endpoint> = self.with(|i| Ok(i.r.db().openers(scope)?.into_iter().filter(|c| c.seal != c.key).map(|c| crate::keys::Endpoint { key: c.key, seal: c.seal }).collect()))?;
        let key: [u8; 32] = rand::random();
        let hash = crate::keys::epoch_hash(&key);
        self.write_step_as(recovery, user, content::epoch(scope, n, &to.iter().map(|e| e.key).collect::<Vec<_>>(), &hash)).await?;
        self.read_scope_now(&Scope::Global).await?;
        ensure!(self.with(|i| i.r.db().epoch_hash(scope, n))? == Some(hash), "Another device made new keys for {scope} just now. Run this again.");
        let (bucket, name) = (self.bucket()?, scope.to_string());
        crate::database::stored::created(&bucket, &crate::keys::epoch_name(&name, n, "recovery"), &crate::keys::seal_to(twin, &key)?).await?;
        for e in &to {
            crate::database::stored::created(&bucket, &crate::keys::epoch_name(&name, n, &hex(&e.key)), &crate::keys::seal_to(&e.seal, &key)?).await?;
        }
        self.forget_epochs(scope);
        Ok(())
    }

    /// New keys for a Vault, signed by the network's recovery key, sealed to it and to the machines that open it now.
    async fn vault_by_recovery(&self, recovery: &ed25519_dalek::SigningKey, user: &str, scope: &Scope, n: u32, to: &[Key], twin: &[u8; 32]) -> Result<()> {
        let secret: [u8; 32] = rand::random();
        let public = crate::keys::public_of(&secret);
        self.write_step_as(recovery, user, content::vault(scope, n, &public, to)).await?;
        self.read_scope_now(&Scope::Global).await?;
        let seals = self.with(|i| {
            let db = i.r.db();
            ensure!(db.vault_publics(scope)?.get(&n) == Some(&public), "Another device made new keys for this Vault just now. Run this again.");
            let mut seals = Vec::new();
            for k in to {
                seals.extend(db.seal_key(k)?.map(|s| (hex(k), s)));
            }
            Ok(seals)
        })?;
        let (bucket, name) = (self.bucket()?, scope.to_string());
        crate::database::stored::created(&bucket, &crate::keys::vault_key_name(&name, n, "recovery"), &crate::keys::seal_to(twin, &secret)?).await?;
        for (at, seal) in seals {
            crate::database::stored::created(&bucket, &crate::keys::vault_key_name(&name, n, &at), &crate::keys::seal_to(&seal, &secret)?).await?;
        }
        Ok(())
    }
}

/// Whether a scope still has someone to use it: not a removed user's own, nor a group that's gone.
fn in_use(db: &ScopeDb, scope: &Scope) -> bool {
    match scope {
        Scope::Global => true,
        Scope::User(u) => db.user(u).is_ok_and(|u| u.is_some()),
        Scope::Group(g) => db.group(g).is_ok_and(|g| g.is_some()),
    }
}
