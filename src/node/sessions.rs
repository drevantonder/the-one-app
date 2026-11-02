//! What a call that came with a kernel access token gets from the vault: the keys of the databases it reaches, and URLs
//! to read and create its objects in the bucket, so this device opens a person's data only while a session of theirs
//! runs here. Keys are asked for once per token for each app, kept in memory until the token expires (or its person
//! may no longer use a scope they could), then forgotten with the databases they opened (ARCHITECTURE.md, "Keys").

use std::sync::atomic::{AtomicU64, Ordering};

use crate::bucket::{Ask, Auth, S3, Sign};

use super::*;

/// What the vault gave this device for one token, until it expires.
#[derive(Default)]
pub(super) struct Session {
    /// Whose it is.
    user: String,
    /// Until when its keys may be used: when its token expires, as the vault says.
    pub until: u64,
    /// Database keys, by `<scope>/<app>/<database>`.
    keys: HashMap<String, [u8; 32]>,
    /// Presigned URLs, by `read <name>` or `create <name>`, each with until when it works.
    urls: Arc<Mutex<HashMap<String, (String, u64)>>>,
}

/// The sessions' state on a node.
#[derive(Default)]
pub(super) struct Sessions {
    /// By a hash of the token.
    pub kept: Mutex<HashMap<[u8; 32], Session>>,
    /// Held while this device asks the vault for keys, so calls at once with one token ask once.
    asking: tokio::sync::Mutex<()>,
    /// How many times this device asked the vault for keys since it started.
    pub asked: AtomicU64,
    /// The auth service at the install's address, whose keys check the tokens calls come with.
    issuer: Mutex<Option<Arc<crate::kernel::Issuer>>>,
    /// Until when each database a session opened stays open here: then it's forgotten, its key with it.
    pub opened: Mutex<HashMap<(Scope, String), u64>>,
}

impl Session {
    /// Whether anything it holds still works at `now`.
    fn live(&self, now: u64) -> bool {
        self.until > now || self.urls.lock().unwrap().values().any(|(_, until)| *until > now)
    }
}

fn hash(token: &str) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(token.as_bytes()).into()
}

impl Node {
    /// The auth service at the install's address, as this device checks the tokens it's given.
    pub fn issuer(&self) -> Result<Arc<crate::kernel::Issuer>> {
        let (address, _) = self.kernel()?;
        let iss = crate::kernel::Issuer::new(&address).iss;
        let mut kept = self.0.sessions.issuer.lock().unwrap();
        Ok(match kept.as_ref().filter(|i| i.iss == iss) {
            Some(issuer) => issuer.clone(),
            None => kept.insert(Arc::new(crate::kernel::Issuer::new(&address))).clone(),
        })
    }

    /// A token another device passed on with a call as `user`'s, if it is theirs: an access token the install's auth
    /// service signed for them, or a scheduled run's, which only the vault can say whose it is (and it opens nothing
    /// but that person's).
    pub(super) async fn token_of(&self, token: &str, user: &str) -> Option<String> {
        if token.starts_with("run.") {
            return Some(token.to_string());
        }
        let issuer = self.issuer().ok()?;
        let asked = token.to_string();
        let claims = tokio::task::spawn_blocking(move || issuer.verify(&asked, now_ms())).await.ok()?.ok()?;
        (claims.sub == user).then(|| token.to_string())
    }

    /// The vault, for a call that came with a token, once the install's kernel is set up.
    fn kernel_vault(&self, me: &Caller) -> Option<(String, crate::kernel::Vault)> {
        let token = me.token.clone()?;
        let (_, url) = self.kernel().ok()?;
        Some((token, crate::kernel::Vault { url }))
    }

    /// The keys of `want`'s databases, (scope, app, database) each, for a call that came with a token: from what the
    /// vault gave this device for it, asking for those it lacks at once. None for a call with no token, or with no
    /// kernel to ask yet: this device's own programs open data with this device's keys.
    pub(super) async fn session_keys(&self, me: &Caller, want: &[(Scope, &str, &str)]) -> Result<Option<Vec<[u8; 32]>>> {
        let Some((token, vault)) = self.kernel_vault(me) else { return Ok(None) };
        let id = hash(&token);
        let name = |(s, a, d): &(Scope, &str, &str)| format!("{s}/{a}/{d}");
        let kept = |sessions: &HashMap<[u8; 32], Session>| -> Option<Vec<[u8; 32]>> {
            let s = sessions.get(&id).filter(|s| s.until > now_ms())?;
            want.iter().map(|w| s.keys.get(&name(w)).copied()).collect()
        };
        if let Some(keys) = kept(&self.0.sessions.kept.lock().unwrap()) {
            return Ok(Some(keys));
        }
        let _asking = self.0.sessions.asking.lock().await;
        if let Some(keys) = kept(&self.0.sessions.kept.lock().unwrap()) {
            return Ok(Some(keys));
        }
        let missing: Vec<(String, String, String)> = {
            let sessions = self.0.sessions.kept.lock().unwrap();
            let has = sessions.get(&id).filter(|s| s.until > now_ms());
            want.iter().filter(|w| !has.is_some_and(|s| s.keys.contains_key(&name(w)))).map(|(s, a, d)| (s.to_string(), a.to_string(), d.to_string())).collect()
        };
        let asked = tokio::task::spawn_blocking(move || {
            let list: Vec<(&str, &str, Option<&str>)> = missing.iter().map(|(s, a, d)| (s.as_str(), a.as_str(), Some(d.as_str()))).collect();
            vault.keys_until(&token, &list)
        })
        .await??;
        let n = self.0.sessions.asked.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!("asked the vault for {} key(s) for {}'s session ({n} asks since this device started)", asked.0.len(), me.user);
        let mut sessions = self.0.sessions.kept.lock().unwrap();
        let now = now_ms();
        sessions.retain(|_, s| s.live(now));
        let session = sessions.entry(id).or_default();
        session.user = me.user.clone();
        session.until = asked.1;
        for k in asked.0 {
            session.keys.insert(format!("{}/{}/{}", k.scope, k.app, k.database.unwrap_or_default()), k.key);
        }
        kept(&sessions).map(Some).context("the vault didn't give every key asked for")
    }

    /// The bucket as a call reaches it: for one that came with a token, through URLs the vault signs for that
    /// session, which can read and create but never delete or overwrite; otherwise as this device does.
    pub(super) fn call_bucket(&self, me: &Caller) -> Result<Copied> {
        let bucket = self.bucket()?;
        let Some((token, vault)) = self.kernel_vault(me) else { return Ok(bucket) };
        let Place::S3(s3) = &bucket.place else { return Ok(bucket) };
        let urls = self.0.sessions.kept.lock().unwrap().entry(hash(&token)).or_default().urls.clone();
        let host = s3.endpoint.split("://").nth(1).unwrap_or(&s3.endpoint);
        let signed = S3 { auth: Auth::Signed(signer(vault, token, urls, format!("https://{host}/{}/", s3.bucket))), ..s3.clone() };
        Ok(Copied::new(Place::S3(signed), self.install_dir().join("copy")))
    }

    /// A call's database: opened with the key the vault gives its session when it came with a token, and kept open
    /// here no longer than that token lasts.
    pub(super) async fn call_database(&self, me: &Caller, scope: &Scope, app: &str, database: &str, schema: Option<Vec<String>>) -> Result<(Arc<tokio::sync::Mutex<Db>>, Copied)> {
        let bucket = self.call_bucket(me)?;
        let key = match self.session_keys(me, &[(scope.clone(), app, database)]).await {
            Ok(keys) => keys.map(|k| k[0]),
            // A scope made since an admin last gave the kernel the install's keys (`kernel keys`): its devices open it
            // with their own key until then.
            Err(err) if format!("{err:#}").contains("isn't open to this person") && self.with(|i| Ok(i.r.members.may_use(&me.user, scope)))? => {
                tracing::warn!("the vault has no key for {scope} yet, so this device opens it with its own: an admin gives the kernel its keys with {} kernel keys", crate::config::CLI);
                None
            }
            Err(err) => return Err(err),
        };
        let Some(key) = key else { return Ok((self.database(scope, app, database, schema, None).await?, bucket)) };
        let id = (scope.clone(), format!("{app}/{database}"));
        let until = self.0.sessions.kept.lock().unwrap().get(&hash(me.token.as_deref().unwrap_or_default())).map(|s| s.until).unwrap_or(0);
        let db = self.database(scope, app, database, schema, Some((key, &bucket))).await?;
        let mut opened = self.0.sessions.opened.lock().unwrap();
        let at = opened.entry(id).or_default();
        *at = (*at).max(until);
        Ok((db, bucket))
    }

    /// Forgets the sessions of people who may no longer use a scope they could, as soon as this device reads that they
    /// can't: what they may still use, their next call asks the vault for again.
    pub(super) fn forget_sessions_of(&self, users: &[String]) {
        let mut kept = self.0.sessions.kept.lock().unwrap();
        let before = kept.len();
        kept.retain(|_, s| !users.contains(&s.user));
        if kept.len() < before {
            tracing::info!("forgot {} session(s) of {}, who may no longer use what they could", before - kept.len(), users.join(", "));
        }
    }

    /// Forgets the sessions whose tokens expired, and the databases only they had open, with their keys.
    pub(super) fn forget_sessions(&self) {
        let now = now_ms();
        self.0.sessions.kept.lock().unwrap().retain(|_, s| s.live(now));
        let gone: Vec<(Scope, String)> = {
            let mut opened = self.0.sessions.opened.lock().unwrap();
            let gone = opened.iter().filter(|(_, until)| **until <= now).map(|(id, _)| id.clone()).collect();
            opened.retain(|_, until| *until > now);
            gone
        };
        let mut dbs = self.0.dbs.lock().unwrap();
        for id in gone {
            dbs.remove(&id);
        }
    }
}

/// Signs a call's bucket requests by asking the vault, as `token`'s session: a read, a create (`If-None-Match: *`) or
/// a replace of the version an ETag names, and nothing else. Reading or creating a database's step asks for the next
/// few too, which a call reads or writes next, and URLs are kept until a minute before they stop working.
fn signer(vault: crate::kernel::Vault, token: String, urls: Arc<Mutex<HashMap<String, (String, u64)>>>, base: String) -> Sign {
    Arc::new(move |ask: &Ask| {
        let header = |h: &str| ask.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(h)).map(|(_, v)| *v);
        ensure!(ask.query.is_empty() && !ask.name.is_empty(), "a call reads and writes objects only by name");
        let (kind, tag) = match (ask.method, header("if-none-match"), header("if-match")) {
            ("GET", None, None) => ("read", None),
            ("PUT", Some("*"), None) => ("create", None),
            ("PUT", None, Some(tag)) => ("replace", Some(tag)),
            _ => bail!("a call only reads, creates, or replaces by ETag, in the bucket"),
        };
        let id = format!("{kind} {}", ask.name);
        if let Some((url, until)) = urls.lock().unwrap().get(&id) {
            if *until > now_ms() + 60_000 {
                return Ok(url.clone());
            }
        }
        let following = |n: u64| -> Vec<String> {
            let Some((prefix, at)) = ask.name.rsplit_once('/') else { return vec![] };
            match at.parse::<u64>().ok().filter(|at| crate::db::step_name(prefix, *at) == ask.name) {
                Some(at) => (1..=n).map(|i| crate::db::step_name(prefix, at + i)).collect(),
                None => vec![],
            }
        };
        let names = |n| [vec![ask.name.to_string()], following(n)].concat();
        let (read, create) = match kind {
            "read" => (names(15), vec![]),
            "create" => (vec![], names(3)),
            _ => (vec![], vec![]),
        };
        let replace: Vec<(&str, &str)> = tag.map(|t| (ask.name, t)).into_iter().collect();
        let given = vault.urls(&token, &read.iter().map(String::as_str).collect::<Vec<_>>(), &create.iter().map(String::as_str).collect::<Vec<_>>(), &replace)?;
        tracing::debug!("the vault signed {kind} {} for a session", ask.name);
        let mut kept = urls.lock().unwrap();
        for (kind, list) in [("read", &given.read), ("create", &given.create)] {
            for (name, url) in list {
                kept.insert(format!("{kind} {name}"), (url.clone(), given.until));
            }
        }
        let url = match kind {
            "replace" => given.replace.get(ask.name).cloned(),
            _ => kept.get(&id).map(|(url, _)| url.clone()),
        }
        .context("the vault didn't sign that")?;
        ensure!(url.starts_with(&base), "the vault signs for another bucket than the install's, which this device uses");
        Ok(url)
    })
}
