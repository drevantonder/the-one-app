//! What a caller with a `Credential` other than this device's own was handed: the keys of the databases it reaches,
//! sealed to this device (`x-one-keys`, `keys::Handoff`), kept in memory for that caller alone until they expire, then
//! forgotten with the databases they opened. This device never opens such a caller's data with its own keys: a call that lacks one fails before it writes anything, saying what it needs
//! (`Needs`, which the API answers with 428), and its caller sends it again with them.

use std::collections::HashSet;

use ::database::stored::Keys;
use ::api::handed::{Need, Needs};

use super::*;

/// What this device keeps for one caller's keys, until they expire.
#[derive(Default)]
pub(super) struct Session {
    user: String,
    /// Until when its keys may be used, as its handoff says.
    pub until: u64,
    /// The database keys it was handed, by (scope, app, database), each by epoch.
    keys: HashMap<(String, String, String), Keys>,
}

#[derive(Default)]
pub(super) struct Sessions {
    pub kept: Mutex<HashMap<Held, Session>>,
    /// The databases opened with keys a caller handed (or app schedules'), each kept open for calls handed
    /// exactly those keys, and forgotten with them: once their last session or app schedules end, or no live session holds
    /// them.
    pub opened: Mutex<HashMap<(Scope, String), Vec<Opened>>>,
    /// The step keys handed here that opened what the bucket holds of their scope at their epoch, each by a hash of
    /// (scope, epoch, key): not the key, which is kept only while a session that handed it lasts.
    opened_steps: Mutex<HashSet<[u8; 32]>>,
    /// The oldest epoch each database's reader needs, by its newest hint's name, as that hint said when last read here.
    oldest: Mutex<HashMap<String, u32>>,
}

/// A database opened with keys a caller handed: shared only with calls handed exactly them.
pub(super) struct Opened {
    pub keys: Keys,
    /// Until when its keys may be used: the latest of the sessions (or app schedules) that handed them.
    pub until: u64,
    /// Opened for app schedules, whose keys no session holds: forgotten with them, or at `until`.
    pub app_schedules: bool,
    pub db: Arc<tokio::sync::Mutex<Db>>,
}

impl Session {
    fn live(&self, now: u64) -> bool {
        self.until > now
    }
}

/// What a caller's session is kept by: a client device's by its key (forgotten once
/// that device is removed), a call passed on by the node that passed it and its user.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum Held {
    Client(Key),
    Passed(Key, String),
}

impl Held {
    /// The `for` of a handoff with its calls: a client device's key, or the key of the node that passed it on.
    fn handoff_for(&self) -> String {
        match self {
            Held::Client(key) | Held::Passed(key, _) => hex(key),
        }
    }
}

impl Credential {
    /// What its session is kept by: none for this device's own keys, or app schedules', which no session holds.
    fn held(&self) -> Option<Held> {
        match self {
            Credential::Client(key) => Some(Held::Client(*key)),
            Credential::Passed { node, user } => Some(Held::Passed(*node, user.clone())),
            Credential::Own | Credential::AppSchedules { .. } => None,
        }
    }
}

fn hint_name(scope: &Scope, app: &str, database: &str) -> String {
    ::bucket::names::newest_name(&::bucket::names::database_prefix(&scope.to_string(), app, database))
}

/// How many hints of a request's databases are read from the bucket at once.
const HINTS_AT_ONCE: usize = 8;
/// How long a hint may take before the request goes on without it.
const HINT_WITHIN: Duration = Duration::from_secs(8);
/// How many of a scope database's last steps a key handed for it is checked against, newest first.
const STEPS_CHECKED: usize = 64;

/// The longest keys handed (a client device's, or with a call passed on) are kept here.
const CLIENT_KEYS: u64 = 3_600_000;

/// How often, at most, `global` is read for a client device this device doesn't list.
const CATCH_UP_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

impl Node {
    /// Whose client device `key` is, and its name: none if it isn't a device of the network, or was removed.
    pub fn client(&self, key: &Key) -> Option<(String, String)> {
        self.with_or(None, |i| Ok(i.r.db().client(key)?.map(|m| (m.user.clone(), m.name.clone()))))
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
        if let Err(err) = self.read_scope(&Scope::Global).await {
            tracing::debug!("reading global for a device it doesn't list: {err:#}");
        }
    }

    /// Keeps the keys a caller handed this device with a call (`x-one-keys`) for its calls with `credential`, as `user`'s:
    /// only if they were sealed to it (any others are left), for that credential (`for`: a client device's or the
    /// Cloudflare node's key), and not yet expired; and no longer than `most`, until when its caller's own
    /// word lasts (a session's certificate's).
    pub fn hand(&self, credential: &Credential, user: &str, sealed: &str, most: u64) -> Result<()> {
        let held = credential.held().context("nothing is handed for this device's own calls, or app schedules'")?;
        // Keys this device can't open (sealed to another: the node a client asked before, say) are left, never refused:
        // the call goes on without them and asks for what it needs (428), sealed to this device, which the client
        // checks against its own entries before handing anything. Leaving them gives nothing away.
        let handoff = match ::api::handed::open_handoff(&self.seal_secret(), sealed) {
            Ok(handoff) => handoff,
            Err(err) => {
                tracing::debug!("keys handed for {user} that this device can't open, left: {err:#}");
                return Ok(());
            }
        };
        ensure!(handoff.for_ == held.handoff_for(), "the keys handed are for another device");
        let now = now_ms();
        // An hour at most, as they're handed again with the calls.
        let until = handoff.until.min(now + CLIENT_KEYS).min(most);
        // Keys that have run out are left, not refused: the call goes on and asks for what it needs (428), and its
        // caller hands them again.
        if until <= now {
            tracing::debug!("keys handed for {user} had run out: left");
            return Ok(());
        }
        // Only keys of scopes its user may use: one of another's would have this device read and write their data
        // with it. And only epochs an entry made: a key for one beyond is junk, or for one this device hasn't read
        // yet, which a call that needs it asks for again.
        let newest: HashMap<String, u32> = self.with(|i| {
            let m = i.r.db();
            let mut newest = HashMap::new();
            for k in &handoff.keys {
                if let Ok(s) = Scope::parse(&k.scope)
                    && m.may(user, "use", &scopes::object(&s))
                {
                    newest.insert(k.scope.clone(), m.epoch(&s)?);
                }
            }
            Ok(newest)
        })?;
        let keys: Vec<::api::handed::Handed> = handoff.keys.into_iter().filter(|k| newest.get(&k.scope).is_some_and(|n| k.epoch >= 1 && k.epoch <= *n)).collect();
        let mut kept = self.0.sessions.kept.lock().unwrap();
        kept.retain(|_, s| s.live(now));
        let session = kept.entry(held).or_default();
        session.user = user.to_string();
        session.until = session.until.max(until);
        for k in keys {
            session.keys.entry((k.scope, k.app, k.database)).or_default().insert(k.epoch, k.key);
        }
        Ok(())
    }

    /// Keeps the keys another node, `from`, handed with a call it passed on for `user`: sealed here for
    /// `from`'s key, and only of `scopes`, those the call names; a service's call (`service`) only with some, since
    /// a node's word alone runs no service. → the call's credential.
    pub(super) fn hand_passed(&self, from: &Key, user: &str, sealed: &str, scopes: &[Scope], service: bool) -> Result<Credential> {
        let handoff = ::api::handed::open_handoff(&self.seal_secret(), sealed)?;
        ensure!(handoff.for_ == hex(from), "the keys handed are for another node");
        ensure!(!service || !handoff.keys.is_empty(), "a service's call passed on comes with the keys its user handed");
        let named: Vec<String> = scopes.iter().map(Scope::to_string).collect();
        ensure!(handoff.keys.iter().all(|k| named.contains(&k.scope)), "the keys handed are of a scope the call doesn't name");
        let credential = Credential::Passed { node: *from, user: user.to_string() };
        self.hand(&credential, user, sealed, u64::MAX)?;
        Ok(credential)
    }

    /// The keys a call of `app` was handed here, of `scopes` alone, for passing it on, until when they're kept: what a
    /// client would hand a node for that call (`keys::may_hand`), never another app's.
    pub(super) fn passing_on(&self, credential: &Credential, scopes: &[Scope], app: &str) -> (Vec<::api::handed::Handed>, u64) {
        let Some(held) = credential.held() else { return (Vec::new(), 0) };
        let named: Vec<String> = scopes.iter().map(Scope::to_string).collect();
        let route = ::api::handed::Route::Call(app.to_string());
        let kept = self.0.sessions.kept.lock().unwrap();
        let Some(session) = kept.get(&held).filter(|s| s.live(now_ms())) else { return (Vec::new(), 0) };
        let keys = session
            .keys
            .iter()
            .filter(|((scope, ..), _)| named.contains(scope))
            .flat_map(|((scope, app, database), by)| by.iter().map(move |(epoch, key)| (::api::handed::Need { scope: scope.clone(), epoch: *epoch, app: app.clone(), database: database.clone() }, *key)))
            .filter(|(need, _)| ::api::handed::may_hand(&route, need))
            .map(|(n, key)| ::api::handed::Handed { scope: n.scope, epoch: n.epoch, app: n.app, database: n.database, key })
            .collect();
        (keys, session.until)
    }

    /// The keys a caller was handed for a database, by epoch: for app schedules, theirs.
    fn handed(&self, me: &Caller, scope: &Scope, app: &str, database: &str) -> Keys {
        if let Credential::AppSchedules { scope: s, app: a } = &me.credential {
            return self.app_schedules_handed(s, a, scope, app, database);
        }
        let Some(held) = me.credential.held() else { return Keys::new() };
        let kept = self.0.sessions.kept.lock().unwrap();
        kept.get(&held).filter(|s| s.until > now_ms()).and_then(|s| s.keys.get(&(scope.to_string(), app.to_string(), database.to_string())).cloned()).unwrap_or_default()
    }

    /// The keys of a scope's database's steps that live sessions of users who may use it were handed, if they hold its
    /// newest epoch's: what this device reads and writes the steps of a scope it runs but doesn't open with, while
    /// someone who opens it uses it here. Of each epoch, only a key that opens what the bucket holds under it, so
    /// nobody's key has this device pass over the real steps or write any it can't be read by (`opens_steps`).
    pub(super) async fn handed_step_keys(&self, scope: &Scope) -> Result<Keys> {
        let (now, newest) = (now_ms(), self.with(|i| Ok(i.r.db().epoch(scope)?))?);
        let id = (scope.to_string(), key::APP.to_string(), key::CoreDb::Log.name().to_string());
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
            if let Some(key) = self.opens_steps(scope, n, n < newest, &offered).await? {
                keys.insert(n, key);
            }
        }
        ::database::stored::handed(keys, newest).map_err(|_| NotGiven(scope.clone(), self.device_name()).into())
    }

    /// Which of the keys `offered` for epoch `n` of a scope's database's steps opens what the bucket holds under that
    /// epoch: its newest copy, or the newest of its last `STEPS_CHECKED` steps, or its start. With nothing under it there, the one key offered, if
    /// everyone offered the same; else none. An epoch before the newest (`old`) has nothing more written under it, so
    /// what it came to holds.
    async fn opens_steps(&self, scope: &Scope, n: u32, old: bool, offered: &[[u8; 32]]) -> Result<Option<[u8; 32]>> {
        use ::bucket::Bucket;
        let checked = |k: &[u8; 32]| -> [u8; 32] {
            use sha2::Digest;
            sha2::Sha256::new().chain_update(scope.to_string()).chain_update(n.to_be_bytes()).chain_update(k).finalize().into()
        };
        if let Some(k) = offered.iter().find(|k| self.0.sessions.opened_steps.lock().unwrap().contains(&checked(k))) {
            return Ok(Some(*k));
        }
        let bucket = self.bucket()?;
        let prefix = scope::steps::prefix(&scope.to_string());
        let (at, copy, epoch) = Bucket::get(&bucket, &::bucket::names::newest_name(&prefix)).await?.map_or((0, 0, 0), |o| ::database::stored::newest(&o.bytes));
        let mut sealed = None;
        if copy > 0 && epoch == n {
            sealed = Bucket::get(&bucket, &::bucket::names::copy_name(&prefix, copy)).await?.map(|o| (::bucket::names::copy_name(&prefix, copy), o.bytes));
        }
        // From the last step anyone said is there, or this device read, on to the last there is.
        let read = self.with(|i| Ok(i.scope_dbs.get(scope).map_or(0, |r| r.at)))?;
        let mut top = at.max(read);
        for _ in 0..STEPS_CHECKED {
            match Bucket::get(&bucket, &::bucket::names::step_name(&prefix, top + 1)).await? {
                Some(_) => top += 1,
                None => break,
            }
        }
        for step in (0..=top).rev().take(STEPS_CHECKED) {
            if sealed.is_some() {
                break;
            }
            let name = ::bucket::names::step_name(&prefix, step);
            let Some(o) = Bucket::get(&bucket, &name).await? else { continue };
            match key::epoch_of(&o.bytes) {
                Ok(e) if e == n => sealed = Some((name, o.bytes)),
                Ok(e) if e < n => break,
                _ => {}
            }
        }
        let Some((name, sealed)) = sealed else {
            let [one] = offered else { return Ok(None) };
            if old {
                self.0.sessions.opened_steps.lock().unwrap().insert(checked(one));
            }
            return Ok(Some(*one));
        };
        let opens = offered.iter().find(|k| key::open(k, &name, &sealed).is_ok()).copied();
        match opens {
            Some(k) => drop(self.0.sessions.opened_steps.lock().unwrap().insert(checked(&k))),
            None => tracing::warn!("{scope}'s steps: no key handed for epoch {n} opens {name}: left"),
        }
        Ok(opens)
    }

    /// Reads the steps of each scope a caller with handed keys uses that this device runs but doesn't open (someone else's own, or
    /// a group its user isn't in), with the keys of its steps the caller hands, which it's asked for (428) if it
    /// didn't: what their calls are checked by (which apps are there, who may do what), and written after.
    pub(super) async fn read_callers_scopes(&self, me: &Caller) -> Result<()> {
        if matches!(me.credential, Credential::Own | Credential::AppSchedules { .. }) {
            return Ok(());
        }
        let want: Vec<(Scope, &str, &str)> = self.with(|i| {
            let m = i.r.db();
            let mut want = Vec::new();
            for s in m.scopes()? {
                if s != Scope::Global && i.r.holds(&s) && m.may(&me.user, "use", &scopes::object(&s)) && m.epoch(&s)? > 0 && !self.opens(m, &s)? {
                    want.push((s, key::APP, key::CoreDb::Log.name()));
                }
            }
            Ok(want)
        })?;
        self.needs(me, &want).await?;
        for (scope, ..) in &want {
            self.read_scope(scope).await?;
        }
        Ok(())
    }

    /// Whether this device opens a scope with its own keys: it's a client device of someone who uses it.
    pub(super) fn opens(&self, m: &ScopeDb, scope: &Scope) -> Result<bool> {
        let me = m.device_of(&self.0.id);
        Ok(*scope == Scope::Global || m.openers(scope)?.iter().any(|c| c.key == me))
    }

    /// A scope's newest epoch: the newest an `epoch` entry that counts made, whatever the bucket holds. 0 while it has
    /// none.
    pub(super) async fn newest_epoch(&self, scope: &Scope) -> Result<u32> {
        self.with(|i| Ok(i.r.db().epoch(scope)?))
    }

    /// The keys a caller with handed keys needs for `want`'s databases, (scope, app, database) each, that it wasn't handed: every
    /// epoch from the one its newest copy is under to the scope's newest, which it writes under.
    pub(super) async fn needs(&self, me: &Caller, want: &[(Scope, &str, &str)]) -> Result<()> {
        if me.credential == Credential::Own {
            return Ok(());
        }
        // A caller handed every epoch from the oldest it needs needs none. The oldest is as each database's hint said when
        // this device last read it, which holds until the database is written (`hint_changed`) or a call finds one older
        // needed: a caller that lacks keys it can't hand (an app with no key in the newest epoch) asks again and again,
        // and the bucket would say the same each time.
        let covered = |(scope, app, database): &(Scope, &str, &str)| -> Result<bool> {
            let newest = self.with(|i| Ok(i.r.db().epoch(scope)?))?.max(1);
            let oldest = self.0.sessions.oldest.lock().unwrap().get(&hint_name(scope, app, database)).copied().unwrap_or(1).min(newest);
            let handed = self.handed(me, scope, app, database);
            Ok((oldest..=newest).all(|n| handed.contains_key(&n)))
        };
        let mut need = Vec::new();
        let mut unknown = Vec::new();
        for w in want {
            if covered(w)? {
                continue;
            }
            let (scope, app, database) = w;
            match self.0.sessions.oldest.lock().unwrap().get(&hint_name(scope, app, database)).copied() {
                Some(oldest) => {
                    let (newest, handed) = (self.with(|i| Ok(i.r.db().epoch(scope)?))?.max(1), self.handed(me, scope, app, database));
                    need.extend((oldest.min(newest)..=newest).filter(|n| !handed.contains_key(n)).map(|epoch| Need { scope: scope.to_string(), epoch, app: app.to_string(), database: database.to_string() }));
                }
                None => unknown.push(w.clone()),
            }
        }
        let want = &unknown[..];
        let bucket = self.bucket()?;
        // Each database's newest hint, a few at once as a page asks for dozens. R2 now and then holds a read of one for half a minute
        // and resets it: a hint only says how old a key it needs, so without one it asks for the newest, and the page goes on.
        let hints: Vec<Option<Option<::bucket::Object>>> = {
            use futures_util::StreamExt;
            let bucket = &bucket;
            let names: Vec<String> = want.iter().map(|(scope, app, database)| hint_name(scope, app, database)).collect();
            futures_util::stream::iter(names)
                .map(|name| async move {
                    match tokio::time::timeout(HINT_WITHIN, ::bucket::Bucket::get(bucket, &name)).await {
                        Ok(Ok(hint)) => Some(hint),
                        Ok(Err(err)) => {
                            tracing::warn!("{name}: {err:#}");
                            None
                        }
                        Err(_) => {
                            tracing::warn!("{name}: no answer in {} s", HINT_WITHIN.as_secs());
                            None
                        }
                    }
                })
                .buffered(HINTS_AT_ONCE)
                .collect()
                .await
        };
        for ((scope, app, database), hint) in want.iter().zip(hints) {
            let newest = self.newest_epoch(scope).await?.max(1);
            let oldest = ::api::handed::oldest_needed(database, hint.as_ref().and_then(|h| h.as_ref()).map(|o| &o.bytes[..]), newest);
            // Only what the bucket said, never a read that failed.
            if hint.is_some() {
                self.0.sessions.oldest.lock().unwrap().insert(hint_name(scope, app, database), oldest);
            }
            let handed = self.handed(me, scope, app, database);
            need.extend((oldest..=newest).filter(|n| !handed.contains_key(n)).map(|epoch| Need { scope: scope.to_string(), epoch, app: app.to_string(), database: database.to_string() }));
        }
        if need.is_empty() { Ok(()) } else { Err(self.need(need).into()) }
    }

    /// Forgets what a database's newest hint said, once it's written here or another device says it wrote it: a
    /// caller's next request reads it again.
    pub(super) fn hint_changed(&self, prefix: &str) {
        self.0.sessions.oldest.lock().unwrap().remove(&::bucket::names::newest_name(prefix));
    }

    pub(super) fn need(&self, need: Vec<Need>) -> Needs {
        let me = self.endpoint();
        Needs { need, to: hex(&me.key), seal: hex(&me.seal) }
    }

    /// A call's database: opened with the keys its caller was handed (app schedules: theirs), and kept open here no
    /// longer than they (or app schedules) last; or with this device's own, for its own user alone.
    pub(super) async fn call_database(&self, me: &Caller, scope: &Scope, app: &str, database: &str, schema: Option<Vec<String>>) -> Result<(Arc<tokio::sync::Mutex<Db>>, Copied)> {
        let bucket = self.bucket()?;
        let (app_schedules, until) = match &me.credential {
            Credential::Own => {
                ensure!(me.user == self.user()?, "{} opens only its own user's data with its own keys", self.device_name());
                return Ok((self.database(scope, app, database, schema, None).await?, bucket));
            }
            Credential::AppSchedules { scope, app } => (true, self.app_schedules_until(scope, app)),
            other => (false, other.held().and_then(|h| self.0.sessions.kept.lock().unwrap().get(&h).map(|s| s.until)).unwrap_or(0)),
        };
        self.needs(me, &[(scope.clone(), app, database)]).await?;
        let keys = self.handed(me, scope, app, database);
        let db = match self.database(scope, app, database, schema, Some(super::databases::Given { keys, bucket: &bucket, until, app_schedules })).await {
            Err(err) => {
                let epoch = err.downcast_ref::<::database::stored::Missing>().map(|m| m.epoch).or(err.downcast_ref::<::database::stored::Ahead>().map(|a| a.epoch));
                // What its hint said when last read here no longer holds: it's read again next time.
                self.0.sessions.oldest.lock().unwrap().remove(&hint_name(scope, app, database));
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
    pub(super) fn keep_opened(&self, id: (Scope, String), keys: Keys, until: u64, app_schedules: bool, db: Arc<tokio::sync::Mutex<Db>>) {
        let mut opened = self.0.sessions.opened.lock().unwrap();
        let all = opened.entry(id).or_default();
        all.retain(|o| o.keys != keys);
        all.push(Opened { keys, until, app_schedules, db });
    }

    /// Forgets the databases opened with handed keys that have run out, or that no live session holds any more (one
    /// forgotten as its user lost a scope, or its device was removed): nothing opened with keys outlives them.
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
            all.retain(|o| o.until > now && (o.app_schedules || holds(&o.keys)));
        }
        opened.retain(|_, all| !all.is_empty());
    }

    /// Forgets the sessions of users who may no longer use a scope they could, as soon as this device reads that they
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

    pub(super) fn forget_removed_clients(&self) {
        let clients: Vec<Key> = self.0.sessions.kept.lock().unwrap().keys().filter_map(|h| match h { Held::Client(k) => Some(*k), _ => None }).collect();
        let gone: Vec<Key> = clients.into_iter().filter(|k| self.client(k).is_none()).collect();
        if !gone.is_empty() {
            self.0.sessions.kept.lock().unwrap().retain(|h, _| !matches!(h, Held::Client(k) if gone.contains(k)));
            self.forget_opened();
        }
    }

    /// Forgets the sessions that ran out, and the databases only they had open, with their keys.
    pub(super) fn forget_sessions(&self) {
        let now = now_ms();
        self.0.sessions.kept.lock().unwrap().retain(|_, s| s.live(now));
        self.forget_opened();
    }
}
