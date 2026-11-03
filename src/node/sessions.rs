//! What a call that came with a kernel access token was handed: the keys of the databases it reaches, sealed to this
//! device (`x-one-keys`, `keys::Handoff`), kept in memory for that token alone until it expires, then forgotten with
//! the databases they opened. This device never opens a
//! token caller's data with its own keys: a call that lacks one fails before it writes anything, saying what it needs
//! (`Needs`, which the API answers with 428), and its caller sends it again with them.

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

/// The longest keys handed with no sign-in (a client device's, or with a call passed on) are kept here.
const CLIENT_KEYS: u64 = 3_600_000;

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
        self.with(|i| Ok(i.r.members.client(key).map(|m| (m.user.clone(), m.name.clone())))).ok().flatten()
    }

    /// Keeps the keys a caller handed this device with a call (`x-one-keys`) for `token`'s calls, as `user`'s: only
    /// if they were sealed to it, for that token (`for`: its sha256, or a client device's or the serverless node's
    /// key), and not yet expired; and no longer than `most`, until when its caller's own word lasts (a session's
    /// certificate's).
    pub fn hand(&self, token: &str, user: &str, sealed: &str, most: u64) -> Result<()> {
        let handoff = crate::keys::open_handoff(&self.seal_secret(), sealed)?;
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
        ensure!(until > now, "the keys handed have expired");
        // Only epochs an entry made are kept: a key for one beyond is junk, or for one this device hasn't read yet,
        // which a call that needs it asks for again.
        let newest: HashMap<String, u32> = self.with(|i| Ok(handoff.keys.iter().filter_map(|k| Some((k.scope.clone(), i.r.members.epoch(&Scope::parse(&k.scope).ok()?)))).collect()))?;
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
    /// `from`'s key, and only of `scopes`, those the call names. → what the call's keys are kept by.
    pub(super) fn hand_passed(&self, from: &Key, user: &str, sealed: &str, scopes: &[Scope]) -> Result<String> {
        let handoff = crate::keys::open_handoff(&self.seal_secret(), sealed)?;
        ensure!(handoff.for_ == hex(from), "the keys handed are for another node");
        let named: Vec<String> = scopes.iter().map(Scope::to_string).collect();
        ensure!(handoff.keys.iter().all(|k| named.contains(&k.scope)), "the keys handed are of a scope the call doesn't name");
        let token = passed_token(from, user);
        self.hand(&token, user, sealed, u64::MAX)?;
        Ok(token)
    }

    /// The keys a call with no sign-in was handed here, of `scopes` alone, for passing it on, until when they're kept.
    pub(super) fn passing_on(&self, token: &str, scopes: &[Scope]) -> (Vec<crate::keys::Handed>, u64) {
        let named: Vec<String> = scopes.iter().map(Scope::to_string).collect();
        let kept = self.0.sessions.kept.lock().unwrap();
        let Some(session) = kept.get(&hash(token)).filter(|s| s.live(now_ms())) else { return (Vec::new(), 0) };
        let keys = session.keys.iter().filter(|((scope, _, _), _)| named.contains(scope)).flat_map(|((scope, app, database), by)| by.iter().map(move |(epoch, key)| crate::keys::Handed { scope: scope.clone(), epoch: *epoch, app: app.clone(), database: database.clone(), key: *key })).collect();
        (keys, session.until)
    }

    /// The keys of `scopes` a sign-in's calls were handed here, sealed to another device (`to`, its seal key) for a
    /// call this one passes on to it, until they're kept here.
    pub(super) fn hand_on(&self, token: &str, to: &[u8; 32], scopes: &[Scope]) -> Option<String> {
        let (keys, until) = self.passing_on(token, scopes);
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

    /// A scope's newest epoch: the newest an `epoch` entry that counts made, whatever the bucket holds. 0 while it has
    /// none.
    pub(super) async fn newest_epoch(&self, scope: &Scope) -> Result<u32> {
        self.with(|i| Ok(i.r.members.epoch(scope)))
    }

    /// The keys a token caller needs for `want`'s databases, (scope, app, database) each, that it wasn't handed: every
    /// epoch from the one its newest copy is under to the scope's newest, which it writes under.
    pub(super) async fn needs(&self, me: &Caller, want: &[(Scope, &str, &str)]) -> Result<()> {
        if me.token.is_none() {
            return Ok(());
        }
        let bucket = self.bucket()?;
        let mut need = Vec::new();
        for (scope, app, database) in want {
            let newest = self.newest_epoch(scope).await?.max(1);
            let prefix = format!("{}/{app}/{database}", crate::bucket::folder(&scope.to_string()));
            let oldest = match crate::bucket::Bucket::get(&bucket, &crate::db::newest_name(&prefix)).await? {
                Some(o) => match crate::db::newest(&o.bytes) {
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
        let db = match self.database(scope, app, database, schema, Some((keys, &bucket))).await {
            Err(err) => {
                let epoch = err.downcast_ref::<crate::db::Missing>().map(|m| m.epoch).or(err.downcast_ref::<crate::db::Ahead>().map(|a| a.epoch));
                return match epoch {
                    Some(epoch) => Err(self.need(vec![Need { scope: scope.to_string(), epoch, app: app.into(), database: database.into() }]).into()),
                    None => Err(err),
                };
            }
            Ok(db) => db,
        };
        let until = match token.starts_with(standing::GRANT) {
            true => self.standing_until(token),
            false => self.0.sessions.kept.lock().unwrap().get(&hash(token)).map(|s| s.until).unwrap_or(0),
        };
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

    /// Forgets the keys handed by client devices the install no longer lists.
    pub(super) fn forget_removed_clients(&self) {
        let clients: Vec<Key> = self.0.sessions.kept.lock().unwrap().values().filter_map(|s| s.client).collect();
        let gone: Vec<Key> = clients.into_iter().filter(|k| self.client(k).is_none()).collect();
        if !gone.is_empty() {
            self.0.sessions.kept.lock().unwrap().retain(|_, s| !s.client.is_some_and(|k| gone.contains(&k)));
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
