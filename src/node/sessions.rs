//! What a call that came with a kernel access token was handed: the keys of the databases it reaches, sealed to this
//! device (`x-one-keys`, `keys::Handoff`), kept in memory for that token alone until it expires, then forgotten with
//! the databases they opened. This device never opens a
//! token caller's data with its own keys: a call that lacks one fails before it writes anything, saying what it needs
//! (`Needs`, which the API answers with 428), and its caller sends it again with them.

use std::collections::HashSet;

use crate::db::Keys;
use crate::keys::Need;

use super::*;

/// What this device keeps for one token, until it expires.
#[derive(Default)]
pub(super) struct Session {
    /// Whose it is.
    user: String,
    /// The client device whose calls over iroh it's for, if it's one's: forgotten once that device is removed.
    client: Option<Key>,
    /// Until when its keys may be used: when its token expires, as its handoff says.
    pub until: u64,
    /// The database keys it was handed, by (scope, app, database), each by epoch.
    keys: HashMap<(String, String, String), Keys>,
}

/// The sessions' state on a node.
#[derive(Default)]
pub(super) struct Sessions {
    /// By a hash of the token.
    pub kept: Mutex<HashMap<[u8; 32], Session>>,
    /// The auth service at the install's address, whose keys check the tokens calls come with.
    issuer: Mutex<Option<Arc<crate::kernel::Issuer>>>,
    /// The databases opened with keys a caller handed (or standing work's grant), each kept open for calls handed
    /// exactly those keys, and forgotten with them: once their last session or grant ends, or no live session holds
    /// them.
    pub opened: Mutex<HashMap<(Scope, String), Vec<Opened>>>,
    /// The log keys handed here that opened what the bucket holds of their scope at their epoch, each by a hash of
    /// (scope, epoch, key): not the key, which is kept only while a session that handed it lasts.
    opened_logs: Mutex<HashSet<[u8; 32]>>,
}

/// A database opened with keys a caller handed: shared only with calls handed exactly them.
pub(super) struct Opened {
    pub keys: Keys,
    /// Until when its keys may be used: the latest of the sessions (or grant) that handed them.
    pub until: u64,
    /// Opened for standing work, whose grant's keys no session holds: forgotten with the grant, or at `until`.
    pub standing: bool,
    pub db: Arc<tokio::sync::Mutex<Db>>,
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
        self.until > now
    }
}

/// What a client device's calls over iroh are kept by here, as a sign-in's are by its token: its key, proven by QUIC.
/// It's never a token anyone sends: no auth service signs it, and the API takes a bearer token only as a sign-in it
/// checks (or its own MCP's ticket, `api::AskingAs`), so no request reaches what a device was handed by naming it.
pub fn client_token(key: &Key) -> String {
    format!("{CLIENT}{}", hex(key))
}

pub(super) const CLIENT: &str = "client.";

/// How many of a log's last steps a key handed for it is checked against, newest first.
const LOG_CHECKED: usize = 64;

/// The longest keys handed with no sign-in (a client device's, or with a call passed on) are kept here.
const CLIENT_KEYS: u64 = 3_600_000;

/// How often, at most, `global` is read for a client device this device doesn't list.
const CATCH_UP_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

/// The client device a token `client_token` made is for.
fn client_of(token: &str) -> Option<Key> {
    data_encoding::HEXLOWER.decode(token.strip_prefix(CLIENT)?.as_bytes()).ok()?.try_into().ok()
}

/// What calls another node passed on for `user` with no sign-in are kept by here (the serverless node's, or a
/// machine's): that node's endpoint key, proven by QUIC, and the person it says the call is for. Like a client
/// device's, no auth service signs it, so it's never a token anyone sends, and what it was handed (sealed here for
/// that node's key) opens only that person's calls from it.
pub fn passed_token(key: &Key, user: &str) -> String {
    format!("{PASSED}{}/{user}", hex(key))
}

const PASSED: &str = "passed.";

/// The `for` of a handoff with calls that bring `token`: a client device's key, the key of the node that passed it
/// on, or the sha256 of an access token.
fn handoff_for(token: &str) -> String {
    if let Some(key) = token.strip_prefix(CLIENT) {
        return key.to_string();
    }
    if let Some((key, _)) = token.strip_prefix(PASSED).and_then(|t| t.split_once('/')) {
        return key.to_string();
    }
    crate::keys::token_id(token)
}

/// Whether `token` was made here for keys handed with no sign-in: a client device's, or a call another node passed on.
pub(super) fn unsigned(token: &str) -> bool {
    token.starts_with(CLIENT) || token.starts_with(PASSED)
}

/// Whether `token` was made here for a call another node passed on: it runs only on what it was handed, and no service
/// counts it as its person's own.
pub(super) fn passed(token: &str) -> bool {
    token.starts_with(PASSED)
}

/// Whether `token` is an access token the install's sign-in service signed, which another device can check: not a
/// client device's, one another node passed on, or a scheduled run's.
pub(super) fn is_sign_in(token: &str) -> bool {
    !token.starts_with(CLIENT) && !token.starts_with(PASSED) && !token.starts_with("run.")
}

fn hash(token: &str) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(token.as_bytes()).into()
}

impl Node {
    /// The auth service at the install's address, as this device checks the tokens it's given.
    pub fn issuer(&self) -> Result<Arc<crate::kernel::Issuer>> {
        let address = self.kernel()?;
        let iss = crate::kernel::Issuer::new(&address).iss;
        let mut kept = self.0.sessions.issuer.lock().unwrap();
        Ok(match kept.as_ref().filter(|i| i.iss == iss) {
            Some(issuer) => issuer.clone(),
            None => kept.insert(Arc::new(crate::kernel::Issuer::new(&address))).clone(),
        })
    }

    /// A token another device passed on with a call as `user`'s, if it is theirs: an access token the install's auth
    /// service signed for them, not yet expired.
    pub(super) async fn token_of(&self, token: &str, user: &str) -> Option<String> {
        let issuer = self.issuer().ok()?;
        let asked = token.to_string();
        let claims = tokio::task::spawn_blocking(move || issuer.verify(&asked, now_ms())).await.ok()?.ok()?;
        (claims.sub == user).then(|| token.to_string())
    }

    /// Whose client device `key` is, and its name: none if it isn't a device of the install, or was removed.
    pub fn client(&self, key: &Key) -> Option<(String, String)> {
        self.with(|i| Ok(i.r.db().client(key).map(|m| (m.user.clone(), m.name.clone())))).ok().flatten()
    }

    /// `client`, after reading `global`'s newest entries if this device doesn't list `key`: a device just approved on
    /// another is listed there before this one has read it, and is let in, not refused.
    pub async fn client_now(&self, key: &Key) -> Option<(String, String)> {
        if let Some(client) = self.client(key) {
            return Some(client);
        }
        self.catch_up().await;
        self.client(key)
    }

    /// Reads `global`'s newest entries, for a client device this device doesn't list: at most once every few seconds,
    /// whoever asks, since anyone who reaches this device can ask as a key it doesn't know, and one read is everyone's.
    pub async fn catch_up(&self) {
        let mut last = self.0.caught_up.lock().await;
        if last.is_some_and(|at| at.elapsed() < CATCH_UP_EVERY) {
            return;
        }
        *last = Some(std::time::Instant::now());
        if let Err(err) = self.read_log(&Scope::Global).await {
            tracing::debug!("reading global for a device it doesn't list: {err:#}");
        }
    }

    /// Keeps the keys a caller handed this device with a call (`x-one-keys`) for `token`'s calls, as `user`'s: only
    /// if they were sealed to it (any others are left), for that token (`for`: its sha256, or a client device's or the serverless node's
    /// key), and not yet expired; and no longer than `most`, until when its caller's own word lasts (a session's
    /// certificate's).
    pub fn hand(&self, token: &str, user: &str, sealed: &str, most: u64) -> Result<()> {
        // Keys this device can't open (sealed to another: the node a client asked before, say) are left, never refused:
        // the call goes on without them and asks for what it needs (428), sealed to this device, which the client
        // checks against its own entries before handing anything, an older one too. Leaving them gives nothing away.
        let handoff = match crate::keys::open_handoff(&self.seal_secret(), sealed) {
            Ok(handoff) => handoff,
            Err(err) => {
                tracing::debug!("keys handed for {user} that this device can't open, left: {err:#}");
                return Ok(());
            }
        };
        ensure!(handoff.for_ == handoff_for(token), "the keys handed are for another sign-in");
        let now = now_ms();
        // Kept no longer than a sign-in's token lasts, whatever the handoff says (one that doesn't check brings no call
        // here to use them); with none (a client device's, or a call passed on), an hour at most, as they're handed
        // again with the calls.
        let until = match token.starts_with(CLIENT) || token.starts_with(PASSED) {
            true => handoff.until.min(now + CLIENT_KEYS),
            false => handoff.until.min(self.token_expiry(token).unwrap_or(u64::MAX)),
        }
        .min(most);
        // Keys that have run out are left, not refused: the call goes on and asks for what it needs (428), and its
        // caller hands them again.
        if until <= now {
            tracing::debug!("keys handed for {user} had run out: left");
            return Ok(());
        }
        // Only keys of scopes its person may use: one of another's would have this device read and write their data
        // with it. And only epochs an entry made: a key for one beyond is junk, or for one this device hasn't read
        // yet, which a call that needs it asks for again.
        let newest: HashMap<String, u32> = self.with(|i| {
            let m = i.r.db();
            Ok(handoff.keys.iter().filter_map(|k| Scope::parse(&k.scope).ok().filter(|s| m.may(user, "use", &scopes::object(s))).map(|s| (k.scope.clone(), m.epoch(&s)))).collect())
        })?;
        let keys: Vec<crate::keys::Handed> = handoff.keys.into_iter().filter(|k| newest.get(&k.scope).is_some_and(|n| k.epoch >= 1 && k.epoch <= *n)).collect();
        let mut kept = self.0.sessions.kept.lock().unwrap();
        kept.retain(|_, s| s.live(now));
        let session = kept.entry(hash(token)).or_default();
        session.user = user.to_string();
        session.client = client_of(token);
        session.until = session.until.max(until);
        for k in keys {
            session.keys.entry((k.scope, k.app, k.database)).or_default().insert(k.epoch, k.key);
        }
        Ok(())
    }

    /// When an access token expires, if the install's auth service signed it: milliseconds.
    fn token_expiry(&self, token: &str) -> Option<u64> {
        let claims = self.issuer().ok()?.verify(token, now_ms()).ok()?;
        Some(claims.exp.saturating_mul(1000))
    }

    /// Keeps the keys another node, `from`, handed with a call it passed on for `user` with no sign-in: sealed here for
    /// `from`'s key, and only of `scopes`, those the call names; a service's call (`service`) only with some, since
    /// a node's word alone runs no service. → what the call's keys are kept by.
    pub(super) fn hand_passed(&self, from: &Key, user: &str, sealed: &str, scopes: &[Scope], service: bool) -> Result<String> {
        let handoff = crate::keys::open_handoff(&self.seal_secret(), sealed)?;
        ensure!(handoff.for_ == hex(from), "the keys handed are for another node");
        ensure!(!service || !handoff.keys.is_empty(), "a service's call passed on comes with the keys its person handed");
        let named: Vec<String> = scopes.iter().map(Scope::to_string).collect();
        ensure!(handoff.keys.iter().all(|k| named.contains(&k.scope)), "the keys handed are of a scope the call doesn't name");
        let token = passed_token(from, user);
        self.hand(&token, user, sealed, u64::MAX)?;
        Ok(token)
    }

    /// The keys a call of `app` was handed here, of `scopes` alone, for passing it on, until when they're kept: its
    /// app's databases', and the scopes' log and Vault, never another app's.
    pub(super) fn passing_on(&self, token: &str, scopes: &[Scope], app: &str) -> (Vec<crate::keys::Handed>, u64) {
        let named: Vec<String> = scopes.iter().map(Scope::to_string).collect();
        let its = |a: &str, database: &str| a == app || (a == members::APP && (database == crate::log::NAME || database == crate::vault::DATABASE));
        let kept = self.0.sessions.kept.lock().unwrap();
        let Some(session) = kept.get(&hash(token)).filter(|s| s.live(now_ms())) else { return (Vec::new(), 0) };
        let keys = session.keys.iter().filter(|((scope, a, database), _)| named.contains(scope) && its(a, database)).flat_map(|((scope, app, database), by)| by.iter().map(move |(epoch, key)| crate::keys::Handed { scope: scope.clone(), epoch: *epoch, app: app.clone(), database: database.clone(), key: *key })).collect();
        (keys, session.until)
    }

    /// The keys of `scopes` a sign-in's calls were handed here, sealed to another device (`to`, its seal key) for a
    /// call of `app` this one passes on to it, until they're kept here.
    pub(super) fn hand_on(&self, token: &str, to: &[u8; 32], scopes: &[Scope], app: &str) -> Option<String> {
        let (keys, until) = self.passing_on(token, scopes, app);
        (until > 0).then(|| crate::keys::seal_handoff(to, &crate::keys::Handoff { for_: crate::keys::token_id(token), until, keys }).ok()).flatten()
    }

    /// The keys a token caller was handed for a database, by epoch: for standing work, its grant's.
    fn handed(&self, me: &Caller, scope: &Scope, app: &str, database: &str) -> Keys {
        let Some(token) = &me.token else { return Keys::new() };
        if token.starts_with(standing::GRANT) {
            return self.standing_handed(token, scope, app, database);
        }
        let kept = self.0.sessions.kept.lock().unwrap();
        kept.get(&hash(token)).filter(|s| s.until > now_ms()).and_then(|s| s.keys.get(&(scope.to_string(), app.to_string(), database.to_string())).cloned()).unwrap_or_default()
    }

    /// The keys of a scope's log that live sessions of people who may use it were handed, if they hold its newest
    /// epoch's: what this device reads and writes the entries of a scope it runs but doesn't open with, while someone
    /// who opens it uses it here. Of each epoch, only a key that opens what the bucket holds under it, so nobody's key
    /// has this device pass over the real entries or write any it can't be read by (`log_opens`).
    pub(super) async fn handed_log(&self, scope: &Scope) -> Result<Keys> {
        let (now, newest) = (now_ms(), self.with(|i| Ok(i.r.db().epoch(scope)))?);
        let id = (scope.to_string(), members::APP.to_string(), crate::log::NAME.to_string());
        let handed: Vec<(String, Keys)> = self.0.sessions.kept.lock().unwrap().values().filter(|s| s.live(now)).filter_map(|s| s.keys.get(&id).map(|k| (s.user.clone(), k.clone()))).collect();
        let mut offered: BTreeMap<u32, Vec<[u8; 32]>> = BTreeMap::new();
        self.with(|i| {
            for (_, by) in handed.iter().filter(|(user, _)| i.r.db().may(user, "use", &scopes::object(scope))) {
                for (n, k) in by.iter().filter(|(n, _)| **n <= newest) {
                    let keys = offered.entry(*n).or_default();
                    if !keys.contains(k) {
                        keys.push(*k);
                    }
                }
            }
            Ok(())
        })?;
        let mut keys = Keys::new();
        for (n, offered) in offered {
            if let Some(key) = self.log_opens(scope, n, n < newest, &offered).await? {
                keys.insert(n, key);
            }
        }
        crate::db::handed(keys, newest).map_err(|_| NotGiven(scope.clone(), self.device_name()).into())
    }

    /// Which of the keys `offered` for epoch `n` of a scope's log opens what the bucket holds under that epoch: its
    /// newest copy, or the newest of its last `LOG_CHECKED` steps. With nothing under it there, the one key offered, if
    /// everyone offered the same; else none. An epoch before the newest (`old`) has nothing more written under it, so
    /// what it came to holds.
    async fn log_opens(&self, scope: &Scope, n: u32, old: bool, offered: &[[u8; 32]]) -> Result<Option<[u8; 32]>> {
        use crate::bucket::Bucket;
        let checked = |k: &[u8; 32]| -> [u8; 32] {
            use sha2::Digest;
            sha2::Sha256::new().chain_update(scope.to_string()).chain_update(n.to_be_bytes()).chain_update(k).finalize().into()
        };
        if let Some(k) = offered.iter().find(|k| self.0.sessions.opened_logs.lock().unwrap().contains(&checked(k))) {
            return Ok(Some(*k));
        }
        let bucket = self.bucket()?;
        let prefix = crate::log::prefix(&scope.to_string());
        let (at, copy, epoch) = Bucket::get(&bucket, &crate::db::newest_name(&prefix)).await?.map_or((0, 0, 0), |o| crate::db::newest(&o.bytes));
        let mut sealed = None;
        if copy > 0 && epoch == n {
            sealed = Bucket::get(&bucket, &crate::db::copy_name(&prefix, copy)).await?.map(|o| (crate::db::copy_name(&prefix, copy), o.bytes));
        }
        // From the last step anyone said is there, or this device read or wrote, on to the last there is.
        let read = self.with(|i| i.r.store.position(&scope.to_string()))?.0;
        let mut top = at.max(read).max(self.0.written.lock().unwrap().get(scope).copied().unwrap_or(0));
        for _ in 0..LOG_CHECKED {
            match Bucket::get(&bucket, &crate::db::step_name(&prefix, top + 1)).await? {
                Some(_) => top += 1,
                None => break,
            }
        }
        for step in (1..=top).rev().take(LOG_CHECKED) {
            if sealed.is_some() {
                break;
            }
            let name = crate::db::step_name(&prefix, step);
            let Some(o) = Bucket::get(&bucket, &name).await? else { continue };
            match crate::keys::epoch_of(&o.bytes) {
                Ok(e) if e == n => sealed = Some((name, o.bytes)),
                Ok(e) if e < n => break,
                _ => {}
            }
        }
        let Some((name, sealed)) = sealed else {
            let [one] = offered else { return Ok(None) };
            if old {
                self.0.sessions.opened_logs.lock().unwrap().insert(checked(one));
            }
            return Ok(Some(*one));
        };
        let opens = offered.iter().find(|k| crate::keys::open(k, &name, &sealed).is_ok()).copied();
        match opens {
            Some(k) => drop(self.0.sessions.opened_logs.lock().unwrap().insert(checked(&k))),
            None => tracing::warn!("{scope}'s log: no key handed for epoch {n} opens {name}: left"),
        }
        Ok(opens)
    }

    /// Reads the entries of each scope a token caller uses that this device runs but doesn't open (someone else's own,
    /// or a group its person isn't in), with the keys of its log the caller hands, which it's asked for (428) if it
    /// didn't: what their calls are checked by (which apps are there, who may do what), and written after.
    pub(super) async fn read_callers_logs(&self, me: &Caller) -> Result<()> {
        if me.token.as_deref().is_none_or(|t| t.starts_with(standing::GRANT)) {
            return Ok(());
        }
        let want: Vec<(Scope, &str, &str)> = self.with(|i| {
            let m = i.r.db();
            Ok(m.scopes().into_iter().filter(|s| *s != Scope::Global && i.r.holds(s) && m.may(&me.user, "use", &scopes::object(s)) && m.epoch(s) > 0 && !self.opens(m, s)).map(|s| (s, members::APP, crate::log::NAME)).collect())
        })?;
        self.needs(me, &want).await?;
        for (scope, ..) in &want {
            self.read_log(scope).await?;
        }
        Ok(())
    }

    /// Whether this device opens a scope with its own keys: it's a client device of someone who uses it.
    pub(super) fn opens(&self, m: &ScopeDb, scope: &Scope) -> bool {
        *scope == Scope::Global || super::databases::clients_of(m, scope).iter().any(|(k, _)| *k == self.0.id)
    }

    /// A scope's newest epoch: the newest an `epoch` entry that counts made, whatever the bucket holds. 0 while it has
    /// none.
    pub(super) async fn newest_epoch(&self, scope: &Scope) -> Result<u32> {
        self.with(|i| Ok(i.r.db().epoch(scope)))
    }

    /// The keys a token caller needs for `want`'s databases, (scope, app, database) each, that it wasn't handed: every
    /// epoch from the one its newest copy is under to the scope's newest, which it writes under.
    pub(super) async fn needs(&self, me: &Caller, want: &[(Scope, &str, &str)]) -> Result<()> {
        if me.token.is_none() {
            return Ok(());
        }
        // A caller handed every epoch from the first needs none: the hints, read from the bucket, only say how old the
        // oldest it needs is, and none is older than the first.
        let handed_all = |(scope, app, database): &(Scope, &str, &str)| -> Result<bool> {
            let handed = self.handed(me, scope, app, database);
            Ok((1..=self.with(|i| Ok(i.r.db().epoch(scope)))?.max(1)).all(|n| handed.contains_key(&n)))
        };
        let mut lacking = Vec::new();
        for w in want {
            if !handed_all(w)? {
                lacking.push(w.clone());
            }
        }
        if lacking.is_empty() {
            return Ok(());
        }
        let want = &lacking[..];
        let bucket = self.bucket()?;
        // Each database's newest hint, all at once.
        let hints = futures_util::future::try_join_all(want.iter().map(|(scope, app, database)| {
            let prefix = format!("{}/{app}/{database}", crate::bucket::folder(&scope.to_string()));
            let bucket = &bucket;
            async move { crate::bucket::Bucket::get(bucket, &crate::db::newest_name(&prefix)).await }
        }))
        .await?;
        let mut need = Vec::new();
        for ((scope, app, database), hint) in want.iter().zip(hints) {
            let newest = self.newest_epoch(scope).await?.max(1);
            let oldest = crate::keys::oldest_needed(hint.as_ref().map(|o| &o.bytes[..]), newest);
            let handed = self.handed(me, scope, app, database);
            need.extend((oldest..=newest).filter(|n| !handed.contains_key(n)).map(|epoch| Need { scope: scope.to_string(), epoch, app: app.to_string(), database: database.to_string() }));
        }
        if need.is_empty() { Ok(()) } else { Err(self.need(need).into()) }
    }

    pub(super) fn need(&self, need: Vec<Need>) -> Needs {
        let me = self.endpoint();
        Needs { need, to: hex(&me.key), seal: hex(&me.seal) }
    }

    /// A call's database: for a token caller, opened with the keys it was handed (standing work: its grant's), and
    /// kept open here no longer than its token (or grant) lasts; else with this device's own, for its own person alone.
    pub(super) async fn call_database(&self, me: &Caller, scope: &Scope, app: &str, database: &str, schema: Option<Vec<String>>) -> Result<(Arc<tokio::sync::Mutex<Db>>, Copied)> {
        let bucket = self.bucket()?;
        let Some(token) = &me.token else {
            ensure!(me.user == self.user()?, "{} opens only its own person's data with its own keys", self.device_name());
            return Ok((self.database(scope, app, database, schema, None).await?, bucket));
        };
        self.needs(me, &[(scope.clone(), app, database)]).await?;
        let keys = self.handed(me, scope, app, database);
        let (standing, until) = match token.starts_with(standing::GRANT) {
            true => (true, self.standing_until(token)),
            false => (false, self.0.sessions.kept.lock().unwrap().get(&hash(token)).map(|s| s.until).unwrap_or(0)),
        };
        let db = match self.database(scope, app, database, schema, Some(super::databases::Given { keys, bucket: &bucket, until, standing })).await {
            Err(err) => {
                let epoch = err.downcast_ref::<crate::db::Missing>().map(|m| m.epoch).or(err.downcast_ref::<crate::db::Ahead>().map(|a| a.epoch));
                return match epoch {
                    Some(epoch) => Err(self.need(vec![Need { scope: scope.to_string(), epoch, app: app.into(), database: database.into() }]).into()),
                    None => Err(err),
                };
            }
            Ok(db) => db,
        };
        Ok((db, bucket))
    }

    /// The database open here for calls handed exactly `keys`, if one is, kept until `until` at least from now on.
    pub(super) fn opened_with(&self, id: &(Scope, String), keys: &Keys, until: u64) -> Option<Arc<tokio::sync::Mutex<Db>>> {
        let mut opened = self.0.sessions.opened.lock().unwrap();
        let o = opened.get_mut(id)?.iter_mut().find(|o| o.keys == *keys)?;
        o.until = o.until.max(until);
        Some(o.db.clone())
    }

    /// Keeps a database opened with handed keys open for the calls handed exactly them, until `until`.
    pub(super) fn keep_opened(&self, id: (Scope, String), keys: Keys, until: u64, standing: bool, db: Arc<tokio::sync::Mutex<Db>>) {
        let mut opened = self.0.sessions.opened.lock().unwrap();
        let all = opened.entry(id).or_default();
        all.retain(|o| o.keys != keys);
        all.push(Opened { keys, until, standing, db });
    }

    /// Forgets the databases opened with handed keys that have run out, or that no live session holds any more (one
    /// forgotten as its person lost a scope, or its device was removed): nothing opened with keys outlives them.
    fn forget_opened(&self) {
        let now = now_ms();
        let held: HashMap<(String, String, String), Vec<Keys>> = {
            let mut held: HashMap<_, Vec<Keys>> = HashMap::new();
            for s in self.0.sessions.kept.lock().unwrap().values().filter(|s| s.live(now)) {
                for (id, keys) in &s.keys {
                    held.entry(id.clone()).or_default().push(keys.clone());
                }
            }
            held
        };
        let mut opened = self.0.sessions.opened.lock().unwrap();
        for ((scope, name), all) in opened.iter_mut() {
            let (app, database) = name.split_once('/').unwrap_or((name, ""));
            let id = (scope.to_string(), app.to_string(), database.to_string());
            let holds = |keys: &Keys| held.get(&id).is_some_and(|by| by.iter().any(|h| keys.iter().all(|(n, k)| h.get(n) == Some(k))));
            all.retain(|o| o.until > now && (o.standing || holds(&o.keys)));
        }
        opened.retain(|_, all| !all.is_empty());
    }

    /// Forgets the sessions of people who may no longer use a scope they could, as soon as this device reads that they
    /// can't: what they may still use, their next call hands it again.
    pub(super) fn forget_sessions_of(&self, users: &[String]) {
        let mut kept = self.0.sessions.kept.lock().unwrap();
        let before = kept.len();
        kept.retain(|_, s| !users.contains(&s.user));
        let forgot = before - kept.len();
        drop(kept);
        if forgot > 0 {
            tracing::info!("forgot {forgot} session(s) of {}, who may no longer use what they could", users.join(", "));
            self.forget_opened();
        }
    }

    /// Forgets the keys handed by client devices the install no longer lists.
    pub(super) fn forget_removed_clients(&self) {
        let clients: Vec<Key> = self.0.sessions.kept.lock().unwrap().values().filter_map(|s| s.client).collect();
        let gone: Vec<Key> = clients.into_iter().filter(|k| self.client(k).is_none()).collect();
        if !gone.is_empty() {
            self.0.sessions.kept.lock().unwrap().retain(|_, s| !s.client.is_some_and(|k| gone.contains(&k)));
            self.forget_opened();
        }
    }

    /// Forgets the sessions whose tokens expired, and the databases only they had open, with their keys.
    pub(super) fn forget_sessions(&self) {
        let now = now_ms();
        self.0.sessions.kept.lock().unwrap().retain(|_, s| s.live(now));
        self.forget_opened();
    }
}
