//! What a call that came with a kernel access token was handed: the keys of the databases it reaches, sealed to this
//! device (`x-one-keys`, `keys::Handoff`), kept in memory for that token alone until it expires, then forgotten with
//! the databases they opened; and URLs to read and create its objects in the bucket. This device never opens a
//! token caller's data with its own keys: a call that lacks one fails before it writes anything, saying what it needs
//! (`Needs`, which the API answers with 428), and its caller sends it again with them.

use crate::bucket::{Ask, Auth, S3, Sign};
use crate::db::Keys;
use crate::keys::Need;

use super::*;

/// What this device keeps for one token, until it expires.
#[derive(Default)]
pub(super) struct Session {
    /// Whose it is.
    user: String,
    /// Until when its keys may be used: when its token expires, as its handoff says.
    pub until: u64,
    /// The database keys it was handed, by (scope, app, database), each by epoch.
    keys: HashMap<(String, String, String), Keys>,
    /// Presigned URLs, by `read <name>` or `create <name>`, each with until when it works.
    urls: Arc<Mutex<HashMap<String, (String, u64)>>>,
}

/// The sessions' state on a node.
#[derive(Default)]
pub(super) struct Sessions {
    /// By a hash of the token.
    pub kept: Mutex<HashMap<[u8; 32], Session>>,
    /// The auth service at the install's address, whose keys check the tokens calls come with.
    issuer: Mutex<Option<Arc<crate::kernel::Issuer>>>,
    /// Until when each database a session opened stays open here: then it's forgotten, its keys with it.
    pub opened: Mutex<HashMap<(Scope, String), u64>>,
}

/// Keys a call needs that it wasn't handed: the API answers 428 with them, and with this device's key and the key to
/// seal them to.
#[derive(Debug, Clone)]
pub struct Needs {
    pub need: Vec<Need>,
    pub to: String,
    pub seal: String,
}

impl std::fmt::Display for Needs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list: Vec<String> = self.need.iter().map(|n| format!("{}'s {}/{} in epoch {}", n.scope, n.app, n.database, n.epoch)).collect();
        write!(f, "it needs keys it wasn't handed: {}", list.join(", "))
    }
}

impl std::error::Error for Needs {}

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

    /// Keeps the keys a caller handed this device with a call (`x-one-keys`) for `token`'s calls, as `user`'s: only
    /// if they were sealed to it, for that token (`for`), and not yet expired.
    pub fn hand(&self, token: &str, user: &str, sealed: &str) -> Result<()> {
        let handoff = crate::keys::open_handoff(&self.seal_secret(), sealed)?;
        ensure!(handoff.for_ == crate::keys::token_id(token), "the keys handed are for another sign-in");
        let now = now_ms();
        ensure!(handoff.until > now, "the keys handed have expired");
        let mut kept = self.0.sessions.kept.lock().unwrap();
        kept.retain(|_, s| s.live(now));
        let session = kept.entry(hash(token)).or_default();
        session.user = user.to_string();
        session.until = session.until.max(handoff.until);
        for k in handoff.keys {
            session.keys.entry((k.scope, k.app, k.database)).or_default().insert(k.epoch, k.key);
        }
        Ok(())
    }

    /// The keys a token caller was handed for a database, by epoch.
    fn handed(&self, me: &Caller, scope: &Scope, app: &str, database: &str) -> Keys {
        let Some(token) = &me.token else { return Keys::new() };
        let kept = self.0.sessions.kept.lock().unwrap();
        kept.get(&hash(token)).filter(|s| s.until > now_ms()).and_then(|s| s.keys.get(&(scope.to_string(), app.to_string(), database.to_string())).cloned()).unwrap_or_default()
    }

    /// A scope's newest epoch as this device knows it: the members', or the newest the bucket has a copy of for the
    /// recovery key, which is sealed first. 0 while it has none.
    pub(super) async fn newest_epoch(&self, scope: &Scope) -> Result<u32> {
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let mut n = self.with(|i| Ok(i.r.members.epoch(scope)))?;
        while crate::bucket::Bucket::get(&bucket, &crate::keys::epoch_name(&name, n + 1, "recovery")).await?.is_some() {
            n += 1;
        }
        Ok(n)
    }

    /// The keys a token caller needs for `want`'s databases, (scope, app, database) each, that it wasn't handed: every
    /// epoch from the one its newest copy is under to the scope's newest, which it writes under.
    pub(super) async fn needs(&self, me: &Caller, want: &[(Scope, &str, &str)]) -> Result<()> {
        if me.token.is_none() {
            return Ok(());
        }
        let bucket = self.call_bucket(me)?;
        let mut need = Vec::new();
        for (scope, app, database) in want {
            let newest = self.newest_epoch(scope).await?.max(1);
            let prefix = format!("{}/{app}/{database}", crate::bucket::folder(&scope.to_string()));
            let oldest = match crate::bucket::Bucket::get(&bucket, &crate::db::newest_name(&prefix)).await? {
                Some(o) => match crate::db::newest(&o.bytes)? {
                    (_, 0, _) => 1,
                    (_, _, epoch) => epoch.clamp(1, newest),
                },
                None => newest,
            };
            let handed = self.handed(me, scope, app, database);
            need.extend((oldest..=newest).filter(|n| !handed.contains_key(n)).map(|epoch| Need { scope: scope.to_string(), epoch, app: app.to_string(), database: database.to_string() }));
        }
        if need.is_empty() { Ok(()) } else { Err(self.need(need).into()) }
    }

    fn need(&self, need: Vec<Need>) -> Needs {
        let me = self.endpoint();
        Needs { need, to: hex(&me.key), seal: hex(&me.seal) }
    }

    /// The bucket as a call reaches it: for one that came with a token, through URLs the vault signs for that
    /// session, which can read and create but never delete or overwrite; otherwise as this device does.
    pub(super) fn call_bucket(&self, me: &Caller) -> Result<Copied> {
        let bucket = self.bucket()?;
        let Some((token, vault)) = self.kernel_vault(me) else { return Ok(bucket) };
        let Place::S3(s3) = &bucket.place else { return Ok(bucket) };
        let urls = {
            let mut kept = self.0.sessions.kept.lock().unwrap();
            let session = kept.entry(hash(&token)).or_default();
            // Whose it is, so a removal forgets its URLs too, even if it never asked for keys.
            session.user = me.user.clone();
            session.urls.clone()
        };
        let host = s3.endpoint.split("://").nth(1).unwrap_or(&s3.endpoint);
        let signed = S3 { auth: Auth::Signed(signer(vault, token, urls, format!("https://{host}/{}/", s3.bucket))), ..s3.clone() };
        Ok(Copied::new(Place::S3(signed), self.install_dir().join("copy")))
    }

    /// A call's database: for a token caller, opened with the keys it was handed, and kept open here no longer than
    /// its token lasts; else with this device's own, for its own person alone.
    pub(super) async fn call_database(&self, me: &Caller, scope: &Scope, app: &str, database: &str, schema: Option<Vec<String>>) -> Result<(Arc<tokio::sync::Mutex<Db>>, Copied)> {
        let bucket = self.call_bucket(me)?;
        let Some(token) = &me.token else {
            ensure!(me.user == self.user()?, "{} opens only its own person's data with its own keys", self.device_name());
            return Ok((self.database(scope, app, database, schema, None).await?, bucket));
        };
        self.needs(me, &[(scope.clone(), app, database)]).await?;
        let keys = self.handed(me, scope, app, database);
        let db = match self.database(scope, app, database, schema, Some((keys, &bucket))).await {
            Err(err) => match err.downcast::<crate::db::Missing>() {
                Ok(m) => return Err(self.need(vec![Need { scope: scope.to_string(), epoch: m.epoch, app: app.into(), database: database.into() }]).into()),
                Err(err) => return Err(err),
            },
            Ok(db) => db,
        };
        let until = self.0.sessions.kept.lock().unwrap().get(&hash(token)).map(|s| s.until).unwrap_or(0);
        let mut opened = self.0.sessions.opened.lock().unwrap();
        let at = opened.entry((scope.clone(), format!("{app}/{database}"))).or_default();
        *at = (*at).max(until);
        Ok((db, bucket))
    }

    /// Forgets the sessions of people who may no longer use a scope they could, as soon as this device reads that they
    /// can't: what they may still use, their next call hands it again.
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
