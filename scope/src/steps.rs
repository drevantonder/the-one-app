//! A scope database's steps: the database is the truth, and every change to it is a step at the next free position of
//! its own in the bucket, which the bucket keeps in one order.
//!
//! A step is one change, signed by the device that made it as its user (`Step`): its content (`content`'s kinds), for
//! one position. Its position is signed in it (its `seq`), so a step can't be written twice, or anywhere but where its
//! writer meant it: one that loses its position to another is signed again for the next. A node only carries a client's
//! step to the bucket: it can hold one back, but change none.
//!
//! ```text
//! header     canonical JSON { v: 1, author, log, seq, backlink: null, previous: [], time, user, epoch, body, size }
//! signature  Ed25519 by `author` over "one/step\0" ‖ header
//! hash       sha256(header)
//! ```
//!
//! `log` is the scope, `seq` the position, `epoch` the scope's its writer knew, `body` the sha256 of the body (hex, the
//! content as JSON) and `size` its length; `backlink` and `previous` are always empty, as every step signed has them.
//! `cloudflare/shared/crypto.js` signs and checks steps the same (`signStep`), and `tests/contracts.rs` checks they
//! agree.
//!
//! `global`'s database starts at its first step that is a freeze (`Change::Freeze`, `Truth::first`): it carries the database
//! there, its first copy, which counts only if it comes to the hash the freeze names, names the network (its `root`,
//! which every device of the network pins) and its recovery key, and was signed by that key. A network starts with one
//! (`first_freeze`); the two made before steps were frozen by their recovery key once, at step 2. A freeze whose copy a
//! newer release wrote stops the reader there, as a newer release's step does, never passed over for the next; anything
//! else shaped like a freeze is nothing. A device let in starts where the device that let it in had read to (its `Welcome`),
//! which pins who's who by its hash.
//!
//! Every other scope's database starts, before its first step, from its start (`Change::Start`, step 0): for a scope whose
//! frozen log the freeze names, what that log's entries added up to there, written once by a device holding its keys;
//! for any other, nothing.
//!
//! Every reader applies each step after to the database as the steps before it left it, and counts it only if all
//! three of the protocol's rules hold there:
//! 1. the network's first step and its recovery key: the freeze's copy, checked as above; and only the recovery key
//!    lets a device in by itself;
//! 2. epochs never go down: a step names its scope's epoch its writer knew, never one above the database's;
//! 3. its signer is one of its user's own devices still listed, or for a scope's own state, a device that runs the
//!    scope's calls for them (`rules::signed`); and its user holds the permission the change needs, at that step:
//!    `may`, `may_give`, `may_take` and `may_make` over the database's rows, each kind's once (`rules::apply`).
//!
//! A step that doesn't count is passed over, its position taken, as a database's are: nothing it says happens. What a
//! counted one changes is its kind's rule's (`rules`), applied to a fresh copy of the database (`scope::fresh`).
//!
//! Its steps are sealed with the key the frozen log had in each epoch (`keys::permanent`, `seal`), under their own names. A
//! reader opens them itself: a node passes them on sealed. The frozen logs (`<prefix>/../log/`) are history, kept in the
//! bucket and read by nothing.
//!
//! ```text
//! <prefix>/0                  a scope's start, but `global`'s, with its copy
//! <prefix>/<n>               step n, from 1: one signed step, its header, signature and body, and a freeze's copy
//! <prefix>/copy/<n>          the database after step n, every `stored::COPY_EVERY` steps
//! <prefix>/newest            the newest hint, as a database's
//! ```

use ::custom_build::allowed;
use ::custom_build::config::YOURS;
use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::SigningKey;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::content::{self, Change, Content};
use crate::scopes::Scope;
use ::key::{Hash, Key, hex, unhex};
use crate::permissions;

use crate::scopes::{self, ScopeDb};
use ::key::CoreDb;
use ::custom_build::protocol::READS;
use crate::scope::RULE;

pub fn prefix(scope: &str) -> String {
    ::bucket::names::core_prefix(CoreDb::Scope, scope)
}

/// What a step's signature is for.
pub const DOMAIN: &str = "one/step";

/// The most any number in a step's header may be: 2^53 - 1, the most JavaScript's JSON keeps exactly.
pub const NUMBER_MAX: u64 = (1 << 53) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub author: Key,
    pub log: String,
    pub seq: u32,
    pub time: u64,
    pub user: String,
    pub epoch: u32,
    pub body: [u8; 32],
    pub size: u64,
}

impl Header {
    fn json(&self) -> Value {
        json!({
            "v": 1, "author": hex(&self.author), "log": self.log, "seq": self.seq, "backlink": null, "previous": [],
            "time": self.time, "user": self.user, "epoch": self.epoch, "body": hex(&self.body), "size": self.size,
        })
    }

    fn parse(v: &Value) -> Result<Header> {
        ensure!(v["v"] == 1, "not a step this device reads");
        ensure!(v["backlink"].is_null() && v["previous"] == json!([]), "a step follows nothing but its position");
        let text = |k: &str| v[k].as_str().with_context(|| format!("no {k}"));
        let number = |k: &str| v[k].as_u64().filter(|n| *n <= NUMBER_MAX).with_context(|| format!("no {k}, or one above 2^53"));
        Ok(Header {
            author: unhex(text("author")?)?,
            log: text("log")?.to_string(),
            seq: u32::try_from(number("seq")?)?,
            time: number("time")?,
            user: text("user")?.to_string(),
            epoch: u32::try_from(number("epoch")?)?,
            body: unhex(text("body")?)?,
            size: number("size")?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Step {
    pub header: Header,
    /// The header's bytes, as signed and hashed.
    raw: Vec<u8>,
    signature: [u8; 64],
    hash: Hash,
    pub body: Vec<u8>,
    pub content: Content,
}

impl Step {
    /// Decodes and checks a step from the wire: its header canonical, its signature, body and content.
    pub fn decode(header: &[u8], signature: &[u8], body: &[u8]) -> Result<Self> {
        let v: Value = serde_json::from_slice(header).context("decode header")?;
        ensure!(::key::canonical(&v) == header, "its header isn't canonical JSON");
        let h = Header::parse(&v)?;
        ::key::verify(&h.author, DOMAIN, &v, signature).context("bad signature")?;
        Scope::parse(&h.log)?;
        ensure!(h.size == body.len() as u64, "body size mismatch");
        ensure!(h.body == ::key::sha256(body), "body hash mismatch");
        let mut content: Content = serde_json::from_slice(body).context("decode body")?;
        crate::migrate::content(&mut content);
        Ok(Step { hash: Hash::from_bytes(::key::sha256(header)), header: h, raw: header.to_vec(), signature: signature.try_into().context("a signature is 64 bytes")?, body: body.to_vec(), content })
    }

    /// (header, signature, body), as the bucket keeps it (`encode`).
    pub fn encode(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (self.raw.clone(), self.signature.to_vec(), self.body.clone())
    }

    /// As `POST /api/steps` takes it: `{ header, signature, body }`, each base64url.
    pub fn to_json(&self) -> Value {
        let b = |v: &[u8]| data_encoding::BASE64URL_NOPAD.encode(v);
        json!({ "header": b(&self.raw), "signature": b(&self.signature), "body": b(&self.body) })
    }

    /// As `to_json` says it, checked.
    pub fn from_json(v: &Value) -> Result<Self> {
        let b = |k: &str| data_encoding::BASE64URL_NOPAD.decode(v[k].as_str().unwrap_or("").as_bytes()).map_err(|_| anyhow::anyhow!("{k} isn't base64url"));
        Step::decode(&b("header")?, &b("signature")?, &b("body")?)
    }

    pub fn author(&self) -> Key {
        self.header.author
    }
    pub fn seq(&self) -> u32 {
        self.header.seq
    }
    pub fn hash(&self) -> Hash {
        self.hash
    }
    pub fn log(&self) -> &str {
        &self.header.log
    }
    pub fn scope(&self) -> Scope {
        Scope::parse(self.log()).expect("checked when it arrived")
    }
    pub fn time(&self) -> u64 {
        self.header.time
    }
    pub fn user(&self) -> &str {
        &self.header.user
    }
    pub fn epoch(&self) -> u32 {
        self.header.epoch
    }
    /// How apps name a step: the first 8 hex characters of its author, and its position.
    pub fn id(&self) -> String {
        format!("{}:{}", ::key::short(&self.author()), self.seq())
    }
}

pub fn sign(key: &SigningKey, scope: &Scope, at: u64, user: &str, epoch: u32, time: u64, content: Content) -> Result<Step> {
    let (header, body) = unsigned(&key.verifying_key().to_bytes(), scope, at, user, epoch, time, &content)?;
    let signature = ::key::sign(key, DOMAIN, &header.json());
    Ok(Step::signed(header, signature, body, content))
}

/// `sign`, by what signs (a browser's key, which can't be read).
pub async fn sign_by(signs: &impl ::key::Signs, scope: &Scope, at: u64, user: &str, epoch: u32, time: u64, content: Content) -> Result<Step> {
    let (header, body) = unsigned(&signs.public(), scope, at, user, epoch, time, &content)?;
    let signature = ::key::sign_by(signs, DOMAIN, &header.json()).await?;
    Ok(Step::signed(header, signature, body, content))
}

fn unsigned(author: &[u8; 32], scope: &Scope, at: u64, user: &str, epoch: u32, time: u64, content: &Content) -> Result<(Header, Vec<u8>)> {
    let seq = u32::try_from(at).context("that database has more steps than a step can name")?;
    let body = serde_json::to_vec(content)?;
    Ok((Header { author: *author, log: scope.to_string(), seq, time, user: user.into(), epoch, body: ::key::sha256(&body), size: body.len() as u64 }, body))
}

impl Step {
    fn signed(header: Header, signature: [u8; 64], body: Vec<u8>, content: Content) -> Step {
        let raw = ::key::canonical(&header.json());
        Step { hash: Hash::from_bytes(::key::sha256(&raw)), header, raw, signature, body, content }
    }
}

/// A step as a reader has it: the step, and the copy a freeze carries.
#[derive(Clone)]
pub struct Given {
    pub step: Step,
    pub copy: Option<Vec<u8>>,
}

/// A step as the bucket holds it, before it's sealed: its header, signature and body, and a freeze's copy, as CBOR.
pub fn encode(step: &Step, copy: Option<&[u8]>) -> Vec<u8> {
    let (h, s, b) = step.encode();
    let mut parts = vec![ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b)];
    parts.extend(copy.map(|c| ciborium::Value::Bytes(c.to_vec())));
    let mut out = Vec::new();
    ciborium::into_writer(&ciborium::Value::Array(parts), &mut out).expect("cbor in memory");
    out
}

/// A step as `encode` wrote it, its signature checked, for position `at` of `scope`'s database.
pub fn decode(scope: &Scope, at: u64, bytes: &[u8]) -> Result<Given> {
    let ciborium::Value::Array(parts) = ciborium::from_reader(bytes)? else { bail!("not a step") };
    let (h, s, b, copy) = match parts.as_slice() {
        [ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b)] => (h, s, b, None),
        [ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b), ciborium::Value::Bytes(c)] => (h, s, b, Some(c.clone())),
        _ => bail!("not a step"),
    };
    let step = Step::decode(h, s, b)?;
    ensure!(step.log() == scope.to_string() && u64::from(step.seq()) == at, "a step for another place");
    Ok(Given { step, copy })
}

/// A step a device of the network's own passed on opened (`encode`'s bytes): none if it isn't one. Only a service
/// endpoint takes them so, from its own device's node, holding no keys of its own.
pub fn given(scope: &Scope, at: u64, opened: Option<&[u8]>) -> Option<Given> {
    opened.and_then(|b| decode(scope, at, b).ok())
}

pub fn encoded(given: &Given) -> Vec<u8> {
    encode(&given.step, given.copy.as_deref())
}

/// What a sealed step is to a reader with `keys`, its log's keys by epoch.
pub enum Opened {
    Step(Given),
    /// Nothing that will ever be one: it doesn't open with its epoch's key, isn't a step for its place, or is under an
    /// epoch above every one made.
    Junk,
    /// Under an epoch that was made, whose key the reader doesn't have: it reads on from here once it does.
    Waits(u32),
}

/// Step `at` of `scope`'s database, sealed as the bucket holds it, opened with `keys`, under `made`, the newest epoch
/// of the scope made as the reader knows it: one above it is junk, since an epoch is always made before anything is
/// sealed under it, and a reader waits only for an epoch that was. Every reader judges it so.
pub fn open(keys: &database::stored::Keys, made: u32, scope: &Scope, at: u64, sealed: &[u8]) -> Opened {
    let name = ::bucket::names::step_name(&prefix(&scope.to_string()), at);
    let Ok(epoch) = ::key::epoch_of(sealed) else { return Opened::Junk };
    let Some(key) = keys.get(&epoch) else { return if epoch > made { Opened::Junk } else { Opened::Waits(epoch) } };
    match ::key::open(key, &name, sealed).and_then(|b| decode(scope, at, &b)) {
        Ok(given) => Opened::Step(given),
        Err(err) => {
            tracing::debug!("{name}: {err:#}");
            Opened::Junk
        }
    }
}

/// What `read` read: the steps by position, none for one that will never be a step; and the epoch of the one it stopped
/// at, if it's under one that was made whose key it has not.
#[derive(Default)]
pub struct Read {
    pub steps: Vec<Option<Given>>,
    pub waits: Option<u32>,
}

/// Reads `scope`'s steps after `from` from the bucket, opened with the keys of its log by epoch: each by position, none
/// for one that will never be a step (`open`, under `made`, the newest epoch of the scope `global`'s steps made). Stops
/// at the first position with nothing, or one under an epoch made it has no key for (`Read::waits`); fails at a
/// position with nothing that a later step says was written (`gap`), rather than read on as if the steps ended there.
pub async fn read(bucket: &impl ::bucket::Bucket, keys: &database::stored::Keys, scope: &Scope, from: u64, made: u32) -> Result<Read> {
    let prefix = prefix(&scope.to_string());
    let mut out = Read::default();
    loop {
        let at = from + out.steps.len() as u64 + 1;
        let Some(o) = bucket.get(&::bucket::names::step_name(&prefix, at)).await? else {
            let after = bucket.get(&::bucket::names::step_name(&prefix, at + 1)).await?;
            gap(keys, scope, at, after.as_ref().map(|o| &o.bytes[..]))?;
            break;
        };
        match open(keys, made, scope, at, &o.bytes) {
            Opened::Step(given) => out.steps.push(Some(given)),
            Opened::Junk => out.steps.push(None),
            Opened::Waits(epoch) => {
                out.waits = Some(epoch);
                break;
            }
        }
    }
    Ok(out)
}

/// Fails if step `at` of `scope`'s database, which isn't in the bucket, was written: the one after it, `after` as the
/// bucket holds it, is a step (it opens with `keys` and is signed for its place). Steps are written one after another,
/// so one gone is one deleted, and reading on as if it never was would pass over whatever it and those after it
/// changed. Anything else there is anyone's to write (the URL signer creates a position only after the one before it,
/// but an older one didn't), so it says nothing: the steps end at `at`, and the next writer writes there. Nor does the
/// newest hint.
fn gap(keys: &database::stored::Keys, scope: &Scope, at: u64, after: Option<&[u8]>) -> Result<()> {
    let later = after.is_some_and(|b| matches!(open(keys, u32::MAX, scope, at + 1, b), Opened::Step(_)));
    ensure!(!later, "step {at} of {scope}'s database isn't in the bucket, though later ones were written: this device reads no further until it's back");
    Ok(())
}

/// The furthest a freeze may say a log ends: 2⁵³, as far as a JSON number holds exactly, past any log's end.
pub const MOST_STEPS: u64 = 1 << 53;

/// Where the log was frozen, as its freeze says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Freeze {
    pub at: u64,
    /// The state hash its copy comes to, as it is (`stored::state_hash`).
    pub hash: String,
    /// The last step of every scope's log that counts, by scope: `global`'s among them.
    pub logs: BTreeMap<String, u64>,
}

impl Freeze {
    pub fn of(step: &Step, at: u64) -> Result<Freeze> {
        ensure!(step.content.is(Change::Freeze) && step.scope() == Scope::Global, "not a freeze");
        Freeze::from_data(&step.content.data, at)
    }

    fn from_data(data: &Value, at: u64) -> Result<Freeze> {
        let mut logs = BTreeMap::new();
        for (scope, n) in data["logs"].as_object().context("a freeze names every scope's log")? {
            Scope::parse(scope)?;
            let n = n.as_u64().context("a log's end is a position")?;
            ensure!(n <= MOST_STEPS, "a log's end is a position a log can reach");
            logs.insert(scope.clone(), n);
        }
        ensure!(logs.get("global").is_some_and(|n| *n > 0), "a freeze names where global's log ends");
        Ok(Freeze { at, hash: data["hash"].as_str().context("a freeze names its copy's hash")?.to_string(), logs })
    }

    /// As a freeze says it, and as a reader keeps it.
    pub fn content(&self) -> Content {
        content::node(Change::Freeze, json!({ "hash": self.hash, "logs": self.logs }))
    }

    /// The last step of `global`'s log it holds.
    pub fn global(&self) -> u64 {
        self.logs.get("global").copied().unwrap_or(0)
    }
}

/// Seals a step or copy of a scope's database as `name` under the newest epoch `keys` has.
pub fn seal(keys: &database::stored::Keys, name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let (epoch, key) = keys.last_key_value().with_context(|| format!("no key to write {name} with"))?;
    Ok(::key::seal(*epoch, key, name, bytes))
}

pub fn connection(bytes: &[u8]) -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    conn.deserialize_read_exact("main", bytes, bytes.len(), false)?;
    conn.query_row("select count(*) from sqlite_schema", [], |_| Ok(()))?;
    Ok(conn)
}

/// A scope's database another device wrote (a copy), or one this device kept from an older release, in this release's
/// form: what it says, built again here, with what only its steps said carried over (`carry`).
pub fn normalize(copy: Connection, scope: &Scope, now: u64) -> Result<ScopeDb> {
    ensure!(version(&copy)? <= READS, Newer);
    crate::migrate::database(&copy)?;
    ScopeDb::open(crate::scope::fresh(&copy, scope)?, now)
}

/// The freeze a new network starts with, at step 1 of `global`'s database, signed by its recovery key as `user`: who's
/// who as `start` says it (`content::first`), the network `root`. → the step, and its copy.
pub fn first_freeze(recovery: &SigningKey, root: Hash, start: &Value, user: &str, now: u64) -> Result<Given> {
    let mut db = first(root, start, now)?;
    ensure!(db.recovery()? == Some(recovery.verifying_key().to_bytes()), "a network's first freeze is signed by the recovery key it names");
    let freeze = Freeze { at: 1, hash: database::stored::state_hash(db.conn())?, logs: [("global".to_string(), 1)].into() };
    let step = sign(recovery, &Scope::Global, 1, user, 1, now, freeze.content())?;
    Ok(Given { step, copy: Some(db.bytes()?) })
}

/// `global`'s database as a new network starts, `root`, from `start` (`content::first`): its first user, device,
/// epoch and recovery key, with this release's types, roles and rules.
pub fn first(root: Hash, start: &Value, now: u64) -> Result<ScopeDb> {
    let db = crate::scope::empty(&Scope::Global)?;
    db.pragma_update(None, "user_version", READS)?;
    crate::rules::first(&db, root, start, now)?;
    crate::scope::derive(&db)?;
    ScopeDb::open(db, now)
}

/// Where a scope's database's start is: position 0, before its first step.
pub fn start_name(scope: &Scope) -> String {
    ::bucket::names::step_name(&prefix(&scope.to_string()), 0)
}

pub fn start(key: &SigningKey, scope: &Scope, user: &str, epoch: u32, now: u64, db: &mut ScopeDb) -> Result<Given> {
    ensure!(*scope != Scope::Global, "global's database starts from its freeze");
    let hash = database::stored::state_hash(db.conn())?;
    let step = sign(key, scope, 0, user, epoch, now, content::node(Change::Start, json!({ "hash": hash })))?;
    Ok(Given { step, copy: Some(db.bytes()?) })
}

/// A scope's database before its first step (not `global`'s), as `global`'s database `global` says, the freeze
/// `freeze` among it: one with nothing in it, unless the freeze names the scope's frozen log, when it's the copy its
/// start carries (`Change::Start`, step 0, `given`), which counts only if its copy comes to the hash it names and its signer is
/// a device the network ever listed: it opened with the scope's keys, and was written once, by this release, so no
/// later removal takes it back. Without one that counts, the scope can't be read yet.
pub fn started(freeze: &Freeze, global: &ScopeDb, scope: &Scope, given: Option<&Given>, now: u64) -> Result<ScopeDb> {
    ensure!(*scope != Scope::Global, "global's database starts from its freeze");
    if freeze.logs.get(&scope.to_string()).is_none_or(|n| *n == 0) {
        return ScopeDb::empty(scope, now);
    }
    let given = given.with_context(|| format!("{scope}'s database has no start yet: one of its devices writes it"))?;
    let step = &given.step;
    ensure!(step.content.is(Change::Start) && step.seq() == 0 && step.scope() == *scope, "{scope}'s step 0 isn't its start");
    ensure!(global.machine_ever(&step.author())?.is_some(), "{scope}'s start isn't from one of the network's devices");
    let copy = connection(given.copy.as_deref().context("a start carries its copy")?)?;
    ensure!(Some(database::stored::state_hash(&copy)?.as_str()) == step.content.data["hash"].as_str(), "{scope}'s start's copy isn't what it says");
    normalize(copy, scope, now)
}

/// Whether what a reader kept came from a build that opens every sealed format this one does. One that doesn't took each
/// step in a newer format for junk and kept a position past it, so what it kept lacks those steps and nothing says so.
/// That's read again from the bucket, where every step stays (`read_again`), and serves until then: it's still how a
/// device reaches the bucket at all.
fn whole(kept: &Value) -> bool {
    kept["sealed"] == ::key::FORMAT
}

/// `global`'s scope database as a reader has it: the freeze, the last position applied (or passed over), and the
/// database there.
pub struct Truth {
    pub freeze: Freeze,
    pub at: u64,
    pub db: ScopeDb,
    /// Whether it read every step: false until what an older build kept is read again (`whole`).
    pub whole: bool,
}

impl Truth {
    /// From the steps a reader has, by position from the first (none for one that will never be a step): from the
    /// network's first freeze (`first`), each step after taken. None before there's one. Fails there if it's a newer
    /// release's.
    pub fn read(steps: &[Option<Given>], root: Option<Hash>, now: u64) -> Result<Option<Truth>> {
        let mut reader = Reader::new(root);
        for given in steps {
            reader.take(given.as_ref(), now)?;
        }
        Ok(reader.truth)
    }

    /// Step `at` of `global`'s database before its first freeze: the freeze, if its copy comes to the hash it names,
    /// names the network `root` (when the reader pins one), and was signed by the recovery key it names. Any other step
    /// is nothing. One whose copy a newer release wrote stops the reader here (`Newer`): it's never passed over for
    /// another.
    pub fn first(at: u64, given: Option<&Given>, root: Option<Hash>, now: u64) -> Result<Option<Truth>> {
        let Some(g) = given.filter(|g| g.step.content.is(Change::Freeze)) else { return Ok(None) };
        match Truth::of_freeze(g, at, now) {
            Ok(t) if t.db.recovery()? == Some(g.step.author()) && root.map_or(Ok(true), |r| t.db.root().map(|n| n == Some(r)))? => Ok(Some(t)),
            Ok(_) => Ok(None),
            Err(err) if err.is::<Newer>() => Err(err.context(format!("{YOURS}'s freeze at step {at} is a newer release's: this device reads who's who no further"))),
            Err(err) => {
                tracing::info!("global's step {at} isn't its freeze: {err:#}");
                Ok(None)
            }
        }
    }

    /// The freeze `given` is at `at`, whoever signed it: its copy, if it comes to the hash it names.
    fn of_freeze(given: &Given, at: u64, now: u64) -> Result<Truth> {
        let freeze = Freeze::of(&given.step, at)?;
        let copy = connection(given.copy.as_deref().context("a freeze carries its copy")?)?;
        ensure!(database::stored::state_hash(&copy)? == freeze.hash, "a freeze's copy isn't what it says");
        Ok(Truth { freeze, at, db: normalize(copy, &Scope::Global, now)?, whole: true })
    }

    /// Takes step `at`, the next, as of `now`: applied if it counts, passed over if not (or none). → the database before
    /// it, if it counted.
    pub fn take(&mut self, at: u64, step: Option<&Step>, now: u64) -> Result<Option<ScopeDb>> {
        ensure!(at == self.at + 1, "step {at} isn't the next");
        self.at = at;
        let Some(step) = step else { return Ok(None) };
        match apply(&self.db, None, &Scope::Global, step) {
            Ok(a) => Ok(Some(std::mem::replace(&mut self.db, ScopeDb::open(a.db, now)?))),
            Err(err) if err.is::<Newer>() => {
                self.at = at - 1;
                Err(err)
            }
            Err(err) => {
                tracing::info!("global's step {at} doesn't count: {err:#}");
                Ok(None)
            }
        }
    }

    /// As a reader keeps it, to start from again: `{ freeze, freeze_at, at, hash, db }`, the database's bytes base64, and
    /// `sealed`, the newest format it opens, once it read every step (`whole`).
    pub fn keep(&mut self) -> Result<Value> {
        let bytes = self.db.bytes()?;
        let mut kept = json!({ "freeze": self.freeze.content().data, "freeze_at": self.freeze.at, "at": self.at, "hash": database::stored::state_hash(self.db.conn())?, "db": data_encoding::BASE64.encode(&bytes) });
        if self.whole {
            kept["sealed"] = json!(::key::FORMAT);
        }
        Ok(kept)
    }

    /// What `keep` kept, checked against the hash it kept with it, in this release's form.
    pub fn kept(kept: &Value, now: u64) -> Result<Truth> {
        let bytes = data_encoding::BASE64.decode(kept["db"].as_str().context("no database kept")?.as_bytes())?;
        let conn = connection(&bytes)?;
        ensure!(Some(database::stored::state_hash(&conn)?.as_str()) == kept["hash"].as_str(), "who's who this device kept isn't what it was");
        let freeze = Freeze::from_data(&kept["freeze"], kept["freeze_at"].as_u64().context("no freeze kept")?)?;
        Ok(Truth { freeze, at: kept["at"].as_u64().context("no position kept")?, db: normalize(conn, &Scope::Global, now)?, whole: whole(kept) })
    }
}

/// After its writer wrote step `at` of a scope's database: a copy of the database it leaves (`db`'s bytes), sealed
/// as its steps are, at every `COPY_EVERY`th step; and the newest hint moved on, as a database's (`stored::hint`). Only
/// saves readers work, so a link that fails it fails nothing. The hint is anyone's to write, so nothing here trusts it.
pub async fn written(bucket: &impl ::bucket::Bucket, keys: &database::stored::Keys, scope: &Scope, at: u64, bytes: Vec<u8>) -> Result<()> {
    let prefix = prefix(&scope.to_string());
    let epoch = *keys.keys().next_back().context("no key")?;
    if at % database::stored::COPY_EVERY == 0 {
        let name = ::bucket::names::copy_name(&prefix, at);
        bucket.create(&name, &seal(keys, &name, &bytes)?).await?;
    }
    database::stored::hint(bucket, &prefix, at, at - at % database::stored::COPY_EVERY, epoch).await
}

/// Where the device that let a new one in had read `global`'s scope database to (its Welcome): its freeze's position,
/// the last step it had taken, and the database's state hash there, in its release's form. The new device starts from
/// the copy at or before that step, takes the steps after it up to there, and keeps what it made only if it comes to
/// that hash: the steps from the freeze on are never all read again. Said `<freeze>.<at>.<hash>`, as auth passes it on
/// beside the network's root (`auth/worker.js`), which a device that knows none of it never reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    pub freeze: u64,
    pub at: u64,
    pub hash: String,
}

impl Welcome {
    /// A Welcome as it's said: none for one that isn't. `cloudflare/shared/crypto.js`'s `waited` reads it the same way.
    pub fn parse(said: &str) -> Option<Welcome> {
        let number = |n: &str| (!n.is_empty() && n.len() <= 16 && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())).then(|| n.parse::<u64>().ok()).flatten();
        let mut parts = said.splitn(3, '.');
        let (freeze, at, hash) = (number(parts.next()?)?, number(parts.next()?)?, parts.next()?);
        (at >= freeze && at < 1 << 53 && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).then(|| Welcome { freeze, at, hash: hash.to_string() })
    }

    /// The copy a new device starts from: the newest at or before `at` the bucket keeps (one every `stored::COPY_EVERY`
    /// steps), or none, for the freeze's own.
    pub fn copy(&self) -> Option<u64> {
        let c = self.at - self.at % database::stored::COPY_EVERY;
        (c > self.freeze).then_some(c)
    }
}

impl std::fmt::Display for Welcome {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.freeze, self.at, self.hash)
    }
}

impl Truth {
    /// Where this reader is, as it welcomes a new device.
    pub fn welcome(&self) -> Result<Welcome> {
        Ok(Welcome { freeze: self.freeze.at, at: self.at, hash: database::stored::state_hash(self.db.conn())? })
    }

    /// Who's who as `welcome` says it: from `freeze`, the step at its freeze's position, and `start`, the copy at
    /// `Welcome::copy` (its bytes, opened), or none to start from the freeze's own, then `after`, the steps after that
    /// one up to the Welcome's, each taken as any reader takes it. Fails unless it comes to the Welcome's hash, in this
    /// release's form: a freeze, copy or step a node or another device swapped is never used.
    pub fn welcomed(welcome: &Welcome, freeze: &Given, start: Option<&[u8]>, after: &[Option<Given>], now: u64) -> Result<Truth> {
        ensure!(u64::from(freeze.step.seq()) == welcome.freeze, "that isn't the freeze it was welcomed with");
        let anchored = Truth::of_freeze(freeze, welcome.freeze, now).context("the step it was welcomed with isn't a freeze")?;
        let (at, db) = match (welcome.copy(), start) {
            (Some(at), Some(bytes)) => (at, normalize(connection(bytes)?, &Scope::Global, now)?),
            (_, None) => (welcome.freeze, anchored.db),
            (None, Some(_)) => bail!("no copy comes before the freeze's"),
        };
        ensure!(after.len() as u64 == welcome.at - at, "not every step up to where it was welcomed is here");
        let mut t = Truth { freeze: anchored.freeze, at, db, whole: true };
        for given in after {
            t.take(t.at + 1, given.as_ref().map(|g| &g.step), now)?;
        }
        ensure!(database::stored::state_hash(t.db.conn())? == welcome.hash, "who's who from that copy isn't what the device that let this one in said");
        Ok(t)
    }
}

/// `global`'s steps as the bucket holds them, sealed, by position: what a reader takes (`Reader::add`), fetched from the
/// bucket itself (`fetch`) or passed on by a node (`to_json`, `from_json`). With a Welcome, the step at its freeze's
/// position and the copy at `Welcome::copy` (`start`), when the bucket has them.
#[derive(Default)]
pub struct Sealed {
    pub welcome: Option<Welcome>,
    pub freeze: Option<Vec<u8>>,
    pub start: Option<(u64, Vec<u8>)>,
    pub steps: BTreeMap<u64, Vec<u8>>,
}

impl Sealed {
    /// As a node passes them on, and a browser gives them: `{ steps: [{ at, sealed }], welcome, freeze: { at, sealed },
    /// start: { at, sealed } }`, base64url. One whose bytes don't decode isn't there.
    pub fn from_json(v: &Value) -> Sealed {
        let bytes = |s: &Value| data_encoding::BASE64URL_NOPAD.decode(s["sealed"].as_str()?.as_bytes()).ok();
        let welcome = v["welcome"].as_str().and_then(Welcome::parse);
        let freeze = welcome.as_ref().and_then(|w| v.get("freeze").filter(|f| f["at"].as_u64() == Some(w.freeze))).and_then(bytes);
        let start = v.get("start").and_then(|s| Some((s["at"].as_u64()?, bytes(s)?)));
        let steps = v["steps"].as_array().into_iter().flatten().filter_map(|s| Some((s["at"].as_u64()?, bytes(s)?))).collect();
        Sealed { welcome, freeze, start, steps }
    }

    /// As `from_json` reads them, with `at`, the last position there from the first fetched.
    pub fn to_json(&self, from: u64) -> Value {
        let sealed = |at: u64, b: &[u8]| json!({ "at": at, "sealed": data_encoding::BASE64URL_NOPAD.encode(b) });
        let mut out = json!({});
        if let (Some(w), Some(f)) = (&self.welcome, &self.freeze) {
            out = json!({ "welcome": w.to_string(), "freeze": sealed(w.freeze, f) });
            if let Some((c, b)) = &self.start {
                out["start"] = sealed(*c, b);
            }
        }
        let mut at = self.start.as_ref().map(|(c, _)| *c).or(self.freeze.as_ref().and(self.welcome.as_ref().map(|w| w.freeze))).unwrap_or(from);
        while self.steps.contains_key(&(at + 1)) {
            at += 1;
        }
        out["at"] = json!(at);
        out["steps"] = json!(self.steps.iter().map(|(n, b)| sealed(*n, b)).collect::<Vec<_>>());
        out
    }

    pub fn epochs(&self) -> std::collections::BTreeSet<u32> {
        let all = self.steps.values().chain(&self.freeze).chain(self.start.as_ref().map(|(_, b)| b));
        all.filter_map(|b| ::key::epoch_of(b).ok()).collect()
    }
}

/// `global`'s steps in `bucket` after `from`, up to the first position with nothing, and the one after that if it's
/// there, which says whether the one missing was written (`Reader::add`). With `welcome`, from its freeze's step and
/// the copy before its step instead, if the freeze's is there.
pub async fn fetch(bucket: &impl ::bucket::Bucket, from: u64, welcome: Option<&Welcome>) -> Result<Sealed> {
    let prefix = prefix("global");
    let get = async |name: String| Ok::<_, anyhow::Error>(bucket.get(&name).await?.map(|o| o.bytes));
    let mut out = Sealed::default();
    let mut at = from;
    if let Some(w) = welcome
        && let Some(freeze) = get(::bucket::names::step_name(&prefix, w.freeze)).await?
    {
        (out.welcome, out.freeze, at) = (Some(w.clone()), Some(freeze), w.freeze);
        if let Some(c) = w.copy()
            && let Some(copy) = get(::bucket::names::copy_name(&prefix, c)).await?
        {
            (out.start, at) = (Some((c, copy)), c);
        }
    }
    loop {
        at += 1;
        let Some(step) = get(::bucket::names::step_name(&prefix, at)).await? else {
            if let Some(after) = get(::bucket::names::step_name(&prefix, at + 1)).await? {
                out.steps.insert(at + 1, after);
            }
            return Ok(out);
        };
        out.steps.insert(at, step);
    }
}

/// `global`'s scope database as one reader reads it, step by step: before it's read the network's first freeze
/// (`Truth::first`) of the network `root`, the last step it looked at; after, who's who as the steps up to there leave
/// it (`Truth`). Every device reads who's who with one, so every device passes over and counts the same steps.
pub struct Reader {
    root: Option<Hash>,
    truth: Option<Truth>,
    looked: u64,
}

impl Reader {
    /// For the network whose root is `root`, or for none, the first freeze there is.
    pub fn new(root: Option<Hash>) -> Reader {
        Reader { root, truth: None, looked: 0 }
    }

    pub fn root(&self) -> Option<Hash> {
        self.root
    }

    /// Who's who, once it's read the freeze.
    pub fn truth(&self) -> Option<&Truth> {
        self.truth.as_ref()
    }

    pub fn truth_mut(&mut self) -> Option<&mut Truth> {
        self.truth.as_mut()
    }

    pub fn into_truth(self) -> Option<Truth> {
        self.truth
    }

    pub fn at(&self) -> u64 {
        self.truth.as_ref().map_or(self.looked, |t| t.at)
    }

    /// Whether it read every step it holds, as `Truth::whole`.
    pub fn whole(&self) -> bool {
        self.truth.as_ref().is_none_or(|t| t.whole)
    }

    /// Starts from `truth`, unless it's read the freeze itself: as kept (`Truth::kept`) or worked out from a Welcome.
    pub fn resume(&mut self, truth: Truth) -> Result<()> {
        if self.truth.is_none() {
            self.root = self.root.or(truth.db.root()?);
            self.truth = Some(truth);
        }
        Ok(())
    }

    /// Where it read to, to keep and start again from (`restart`): `{ truth, looked }`.
    pub fn kept(&mut self) -> Result<Value> {
        Ok(json!({ "truth": self.truth.as_mut().map(Truth::keep).transpose()?, "looked": self.looked }))
    }

    /// Starts again from what `kept` kept: who's who, checked against its hash, or where it looked to before.
    pub fn restart(&mut self, kept: &Value, now: u64) -> Result<()> {
        match kept.get("truth").filter(|t| !t.is_null()) {
            Some(t) => self.truth = Some(Truth::kept(t, now)?),
            None => self.looked = kept["looked"].as_u64().unwrap_or(0),
        }
        Ok(())
    }

    /// Takes the next step as this reader opened it (none for one that will never be a step), as of `now`: before it's
    /// read the freeze, the network's first freeze or nothing; after, applied if it counts. Fails at a newer release's,
    /// which is never passed over. → who's who before it, and after, if it changed (before the freeze, one of nobody).
    pub fn take(&mut self, given: Option<&Given>, now: u64) -> Result<Option<(ScopeDb, &ScopeDb)>> {
        let at = self.at() + 1;
        let was = match &mut self.truth {
            Some(t) => t.take(at, given.map(|g| &g.step), now)?,
            None => {
                self.truth = Truth::first(at, given, self.root, now)?;
                self.looked = at;
                match &self.truth {
                    Some(t) => {
                        self.root = t.db.root()?;
                        Some(ScopeDb::empty(&Scope::Global, now)?)
                    }
                    None => None,
                }
            }
        };
        Ok(was.zip(self.truth.as_ref().map(|t| &t.db)))
    }

    /// `keys`, each step key by epoch, that it opens a step with now: before it's read the freeze, any; after, with
    /// `epochs`, the epochs' own keys they came from, only those whose hash who's who says that epoch has.
    fn usable(&self, keys: &database::stored::Keys, epochs: Option<&BTreeMap<u32, [u8; 32]>>) -> Result<database::stored::Keys> {
        let (Some(t), Some(epochs)) = (&self.truth, epochs) else { return Ok(keys.clone()) };
        let mut out = database::stored::Keys::new();
        for (e, k) in keys {
            if let Some(opens) = epochs.get(e)
                && t.db.epoch_hash(&Scope::Global, *e)? == Some(::key::epoch_hash(opens))
            {
                out.insert(*e, *k);
            }
        }
        Ok(out)
    }

    /// Takes `sealed`'s steps from the next on, opened with `keys` (`usable`), as of `now`, calling `each` with each
    /// one's position, what it was, and who's who before and after it if it changed (`take`). With a Welcome, before
    /// it's read the freeze, it starts from there (`welcomed`); one that doesn't check out is passed over, and it reads
    /// from where it was. A step is junk, passed over, if it doesn't open or isn't one, or is under an epoch above every
    /// one made, as the steps before it leave it (before the freeze, above every key it has); it stops at one under an
    /// epoch made whose key it hasn't, and at the first position missing, failing if the step after it is one: a step
    /// gone was deleted, and reading on would pass over what it changed. → the epoch it stopped at, if it did.
    pub fn add(&mut self, sealed: &Sealed, keys: &database::stored::Keys, epochs: Option<&BTreeMap<u32, [u8; 32]>>, now: u64, mut each: impl FnMut(u64, Option<&Given>, Option<(ScopeDb, &ScopeDb)>) -> Result<()>) -> Result<Option<u32>> {
        if self.truth.is_none()
            && let Some(w) = &sealed.welcome
        {
            match self.welcomed(w, sealed, keys, epochs, now) {
                Ok(t) => self.resume(t)?,
                Err(err) => tracing::info!("where this device was welcomed to ({w}): {err:#}: it reads from the freeze"),
            }
        }
        loop {
            let at = self.at() + 1;
            let Some(bytes) = sealed.steps.get(&at) else { break };
            let keys = self.usable(keys, epochs)?;
            let made = match &self.truth {
                Some(t) => t.db.epoch(&Scope::Global)?,
                None => keys.keys().next_back().copied().unwrap_or(0),
            };
            let given = match open(&keys, made, &Scope::Global, at, bytes) {
                Opened::Step(g) => Some(g),
                Opened::Junk => None,
                Opened::Waits(e) => return Ok(Some(e)),
            };
            let changed = self.take(given.as_ref(), now)?;
            each(at, given.as_ref(), changed)?;
        }
        let next = self.at() + 1;
        gap(keys, &Scope::Global, next, sealed.steps.get(&(next + 1)).map(Vec::as_slice))?;
        Ok(None)
    }

    pub async fn read(&mut self, bucket: &impl ::bucket::Bucket, keys: &database::stored::Keys, now: u64) -> Result<Option<u32>> {
        let sealed = fetch(bucket, self.at(), None).await?;
        self.add(&sealed, keys, None, now, |_, _, _| Ok(()))
    }

    /// Who's who as `welcome` says it, from `sealed`: its freeze's step, and the copy at `Welcome::copy` with the steps
    /// after it up to the Welcome's, or, if that copy isn't there or doesn't come to its hash, every step after the
    /// freeze's up to there (`Truth::welcomed`), each opened with `keys`, unchecked, so each of `epochs` who's who there
    /// made must be the one it made.
    fn welcomed(&self, welcome: &Welcome, sealed: &Sealed, keys: &database::stored::Keys, epochs: Option<&BTreeMap<u32, [u8; 32]>>, now: u64) -> Result<Truth> {
        let made = keys.keys().next_back().copied().unwrap_or(0);
        let opened = |at: u64, bytes: &[u8]| match open(keys, made, &Scope::Global, at, bytes) {
            Opened::Step(g) => Ok(Some(g)),
            Opened::Junk => Ok(None),
            Opened::Waits(e) => bail!("global's step {at} is under epoch {e}, whose key this device wasn't given"),
        };
        let after = |from: u64| (from + 1..=welcome.at).map(|at| opened(at, sealed.steps.get(&at).with_context(|| format!("global's step {at} wasn't given"))?)).collect::<Result<Vec<_>>>();
        let freeze = opened(welcome.freeze, sealed.freeze.as_deref().context("no freeze")?)?.context("the step it was welcomed with isn't a freeze")?;
        let from_copy = sealed.start.as_ref().filter(|(c, _)| Some(*c) == welcome.copy()).map(|(c, bytes)| {
            let name = ::bucket::names::copy_name(&prefix("global"), *c);
            let key = keys.get(&::key::epoch_of(bytes)?).context("its copy is under a key this device wasn't given")?;
            Truth::welcomed(welcome, &freeze, Some(&::key::open(key, &name, bytes)?), &after(*c)?, now)
        });
        let truth = match from_copy {
            Some(Ok(t)) => t,
            tried => {
                if let Some(Err(err)) = tried {
                    tracing::info!("global's copy before step {}: {err:#}: read from its freeze", welcome.at);
                }
                Truth::welcomed(welcome, &freeze, None, &after(welcome.freeze)?, now)?
            }
        };
        for (e, epoch) in epochs.into_iter().flatten() {
            ensure!(truth.db.epoch_hash(&Scope::Global, *e)?.is_none_or(|h| h == ::key::epoch_hash(epoch)), "a key for this didn't check out, so it wasn't used");
        }
        Ok(truth)
    }
}

/// Who's who read from `bucket` into `reader`, from where it is, by a device that holds no key of `global`'s yet but
/// those sealed to it there (`to`, its key in hex, opened by `opens`): each epoch its steps are sealed under, and
/// each who's who made, sealed to it. With `welcome`, from there, or if that doesn't check out (then none), from the
/// first.
/// → the epochs, each only once who's who says it's the one it made (none before it's read the freeze), and the steps'
/// key in each.
/// The names an epoch sealed to `to` may be under: a device's its own; the recovery key's its own after a replacement,
/// then the one every epoch was first sealed under.
fn names(to: &str, opens: &impl ::key::Opens) -> Vec<String> {
    match to {
        "recovery" => vec![::key::recovery_to(&opens.public()), to.to_string()],
        to => vec![to.to_string()],
    }
}

pub async fn read_sealed_to(bucket: &impl ::bucket::Bucket, reader: &mut Reader, to: &str, opens: &impl ::key::Opens, welcome: &mut Option<Welcome>, now: u64) -> Result<(BTreeMap<u32, [u8; 32]>, database::stored::Keys)> {
    let mut sealed = fetch(bucket, reader.at(), welcome.as_ref().filter(|_| reader.truth.is_none())).await?;
    let (mut epochs, mut tried, mut keys) = (BTreeMap::new(), std::collections::BTreeSet::new(), database::stored::Keys::new());
    let mut fetched = true;
    loop {
        let made = match &reader.truth {
            Some(t) => t.db.epoch_hashes(&Scope::Global)?.into_keys().collect::<Vec<_>>(),
            None => Vec::new(),
        };
        let wanted: Vec<u32> = sealed.epochs().into_iter().chain(made).filter(|n| tried.insert(*n)).collect();
        if wanted.is_empty() && !fetched {
            break;
        }
        for n in &wanted {
            for to in names(to, opens) {
                if let Some(o) = bucket.get(&database::keys::epoch_name("global", *n, &to)).await?
                    && let Some(key) = ::key::open_sealed_by(opens, &o.bytes).await.ok().and_then(|k| <[u8; 32]>::try_from(k).ok())
                {
                    epochs.insert(*n, key);
                    break;
                }
            }
        }
        if !wanted.is_empty() {
            keys = database::keys::database_keys(bucket, "global", ::key::APP, CoreDb::Scope.name(), &epochs, false).await?;
        }
        let added = reader.add(&sealed, &keys, Some(&epochs), now, |_, _, _| Ok(()));
        fetched = sealed.welcome.is_some() && reader.truth.is_none();
        if fetched {
            *welcome = None;
            sealed = fetch(bucket, reader.at(), None).await?;
        } else {
            added?;
        }
    }
    if let Some(t) = &reader.truth {
        let mut kept = BTreeMap::new();
        for (n, k) in epochs {
            if t.db.epoch_hash(&Scope::Global, n)? == Some(::key::epoch_hash(&k)) {
                kept.insert(n, k);
            }
        }
        epochs = kept;
    } else {
        epochs.clear();
    }
    keys.retain(|n, _| epochs.contains_key(n));
    Ok((epochs, keys))
}

/// Who's who read again from the first step (`read_sealed_to`), for a reader of the network `root` at position `at` that
/// isn't `whole`, if it reaches that far, and the epochs it opened: none if it doesn't, so what's kept stays.
pub async fn read_again(bucket: &impl ::bucket::Bucket, root: Option<Hash>, at: u64, to: &str, opens: &impl ::key::Opens, now: u64) -> Result<Option<(Reader, BTreeMap<u32, [u8; 32]>)>> {
    let mut fresh = Reader::new(root);
    let (epochs, _) = read_sealed_to(bucket, &mut fresh, to, opens, &mut None, now).await?;
    Ok((fresh.truth.is_some() && fresh.at() >= at).then_some((fresh, epochs)))
}

pub struct Applied {
    pub db: Connection,
}

/// Applies `step` to `db`, `scope`'s database as the steps before it left it, with `global`'s as it is now (for a
/// scope other than `global`, whose permissions are `global`'s): → the database it leaves, or why it doesn't count.
pub fn apply(db: &ScopeDb, global: Option<&ScopeDb>, scope: &Scope, step: &Step) -> Result<Applied> {
    ensure!(step.scope() == *scope, "a step of another scope");
    let who = global.unwrap_or(db);
    if step.content.is(Change::Defaults) {
        return defaults(db, who, scope, step);
    }
    ensure!(version(db.conn())? <= READS, Newer);
    ensure!(step.epoch() <= who.epoch(scope)?, "it names an epoch its scope hasn't reached");
    crate::rules::signed(who, scope, step)?;
    let out = crate::scope::fresh(db.conn(), scope)?;
    crate::rules::apply(who, &out, scope, step)?;
    if *scope == Scope::Global {
        crate::scope::derive(&out)?;
    }
    named(&out, step)?;
    if *scope == Scope::Global {
        forget_old_releases(&out)?;
    }
    Ok(Applied { db: out })
}

/// A scope's own database (not `global`'s) as one reader reads it, step by step: from its start (`ScopeReader::start`),
/// the last position it applied or passed over, and the database there. Every node reads a scope with one.
pub struct ScopeReader {
    pub at: u64,
    pub db: ScopeDb,
    /// As `Truth::whole`.
    pub whole: bool,
}

impl ScopeReader {
    /// The start of `scope`'s database, step 0, as the bucket holds it, opened with `keys`: none if there's none, or it
    /// doesn't open.
    pub async fn fetch_start(bucket: &impl ::bucket::Bucket, keys: &database::stored::Keys, scope: &Scope) -> Result<Option<Given>> {
        Ok(match bucket.get(&start_name(scope)).await? {
            Some(o) => match open(keys, u32::MAX, scope, 0, &o.bytes) {
                Opened::Step(given) => Some(given),
                _ => None,
            },
            None => None,
        })
    }

    /// Before its first step: its database from its start (`started`), with who's who `truth`.
    pub fn start(truth: &Truth, scope: &Scope, start: Option<&Given>, now: u64) -> Result<ScopeReader> {
        Ok(ScopeReader { at: 0, db: started(&truth.freeze, &truth.db, scope, start, now)?, whole: true })
    }

    /// Takes the steps of `read` after its own position, `read` read from `from` on (`read`): each applied if it counts
    /// with `global`, `global`'s database as it is now, passed over if not. A newer release's step stops it before it
    /// (`Newer`), as it stops `global`'s readers (`Truth::take`): it's never passed over, which a reader of that release
    /// wouldn't do. → whether it took any.
    pub fn take(&mut self, from: u64, read: &Read, global: &ScopeDb, scope: &Scope, now: u64) -> Result<bool> {
        let skip = usize::try_from(self.at.saturating_sub(from)).unwrap_or(usize::MAX);
        ensure!(from <= self.at, "{scope}'s steps from {from} skip some after {}", self.at);
        let mut took = false;
        for step in read.steps.iter().skip(skip) {
            if let Some(given) = step {
                match apply(&self.db, Some(global), scope, &given.step) {
                    Ok(a) => self.db = ScopeDb::open(a.db, now)?,
                    Err(err) if err.is::<Newer>() => return Err(err),
                    Err(err) => tracing::info!("{scope}'s step {} doesn't count: {err:#}", self.at + 1),
                }
            }
            self.at += 1;
            took = true;
        }
        Ok(took)
    }

    /// As a reader keeps it, to start from again (`kept`): `{ at, hash, db }`, the database's bytes base64, and `sealed`,
    /// the newest format it opens, once it read every step (`whole`).
    pub fn keep(&mut self) -> Result<Value> {
        let mut kept = json!({ "at": self.at, "hash": database::stored::state_hash(self.db.conn())?, "db": data_encoding::BASE64.encode(&self.db.bytes()?) });
        if self.whole {
            kept["sealed"] = json!(::key::FORMAT);
        }
        Ok(kept)
    }

    /// What `keep` kept, checked against the hash it kept with it, in this release's form.
    pub fn kept(kept: &Value, scope: &Scope, now: u64) -> Result<ScopeReader> {
        let bytes = data_encoding::BASE64.decode(kept["db"].as_str().context("no database")?.as_bytes())?;
        let conn = connection(&bytes)?;
        ensure!(Some(database::stored::state_hash(&conn)?.as_str()) == kept["hash"].as_str(), "it isn't what it was");
        Ok(ScopeReader { at: kept["at"].as_u64().context("no position")?, db: normalize(conn, scope, now)?, whole: whole(kept) })
    }
}

/// A database's version: the `READS` of the release whose types, roles, rules and tables it has.
pub fn version(db: &Connection) -> Result<u32> {
    Ok(db.query_row("pragma user_version", [], |r| r.get(0))?)
}

/// What a reader that reads less than a step's release says: it stops there, and its device needs updating.
#[derive(Debug)]
pub struct Newer;

impl std::fmt::Display for Newer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "update this device: {YOURS}'s who's who is from a newer release than this one reads")
    }
}

impl std::error::Error for Newer {}

/// This release's types, roles and rules as a `Change::Defaults` step says them, with its version.
pub fn release_defaults() -> Result<Content> {
    let db = Connection::open_in_memory()?;
    db.execute_batch(permissions::SCHEMA)?;
    db.execute_batch(permissions::DEFAULTS)?;
    let mut data = rules_of(&db)?;
    data["reads"] = json!(READS);
    Ok(content::node(Change::Defaults, data))
}

/// A database's types, roles and rules, as a `Change::Defaults` step says them.
pub fn rules_of(db: &Connection) -> Result<serde_json::Value> {
    let rows = |sql: &str| -> Result<Vec<serde_json::Value>> {
        let mut q = db.prepare(sql)?;
        let n = q.column_count();
        let out = q.query_map([], |r| (0..n).map(|i| r.get::<_, rusqlite::types::Value>(i)).collect::<rusqlite::Result<Vec<_>>>())?;
        out.map(|r| {
            Ok(serde_json::Value::Array(
                r?.into_iter()
                    .map(|v| match v {
                        rusqlite::types::Value::Integer(i) => serde_json::json!(i),
                        rusqlite::types::Value::Text(t) => serde_json::json!(t),
                        _ => serde_json::Value::Null,
                    })
                    .collect(),
            ))
        })
        .collect()
    };
    Ok(serde_json::json!({
        "types": rows("SELECT name, permissions FROM types ORDER BY name")?,
        "roles": rows("SELECT type, name, permissions, gives FROM roles WHERE builtin = 1 ORDER BY type, name")?,
        "rules": rows(&format!("SELECT subject, relation, object FROM relations WHERE {RULE} ORDER BY 1, 2, 3"))?,
    }))
}


/// A `Change::Defaults` step: by an admin's device, the release's types, roles and rules as it says them, in place of those
/// the database had; the network's own roles stay.
fn defaults(db: &ScopeDb, who: &ScopeDb, scope: &Scope, step: &Step) -> Result<Applied> {
    // Whose it is first: one that doesn't count is passed over, whatever release it names.
    ensure!(who.client(&step.author())?.is_some_and(|c| c.user == step.user()), "only a user's own device writes for them");
    allowed!(who.may(step.user(), "change-roles", scopes::NETWORK), "only the network's admins say the release's roles");
    let reads = u32::try_from(step.content.data["reads"].as_u64().context("a release's roles name its version")?)?;
    ensure!(reads <= READS, Newer);
    ensure!(reads > version(db.conn())?, "a release's roles only ever move on");
    let out = crate::scope::fresh(db.conn(), scope)?;
    let data = &step.content.data;
    let list = |k: &str| data[k].as_array().cloned().unwrap_or_default();
    let tx = out.unchecked_transaction()?;
    tx.execute_batch(&format!("DELETE FROM types; DELETE FROM roles WHERE builtin = 1; DELETE FROM relations WHERE {RULE};"))?;
    for t in list("types") {
        tx.execute("INSERT OR REPLACE INTO types VALUES (?, ?)", rusqlite::params![t[0].as_str(), t[1].as_str()])?;
    }
    for r in list("roles") {
        tx.execute("INSERT OR REPLACE INTO roles VALUES (?, ?, ?, ?, 1)", rusqlite::params![r[0].as_str(), r[1].as_str(), r[2].as_str(), r[3].as_str().unwrap_or("[]")])?;
    }
    for r in list("rules") {
        let (subject, object) = (r[0].as_str().unwrap_or(""), r[2].as_str().unwrap_or(""));
        if subject.starts_with(['#', '@']) || subject.contains('*') || object.contains('*') {
            tx.execute("INSERT OR IGNORE INTO relations VALUES (?, ?, ?, NULL)", rusqlite::params![subject, r[1].as_str(), object])?;
        }
    }
    tx.pragma_update(None, "user_version", reads)?;
    tx.commit()?;
    Ok(Applied { db: out })
}

/// What a counted step names that no other table keeps: its blobs, and a release.
fn named(db: &Connection, step: &Step) -> Result<()> {
    let pinned = step.content.app == ::key::APP && !step.content.is(Change::Keep);
    for blob in &step.content.blobs {
        db.execute("INSERT INTO blobs VALUES (?1, ?2) ON CONFLICT (blob) DO UPDATE SET pinned = max(pinned, ?2)", rusqlite::params![blob, pinned])?;
    }
    if matches!(step.content.change(), Some(Change::NodeRelease | Change::ServiceRelease)) {
        db.execute(
            "INSERT INTO releases VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![step.seq(), step.content.kind, serde_json::to_string(&step.content)?, ::key::hex(&step.author()), i64::try_from(step.time()).unwrap_or(i64::MAX)],
        )?;
    }
    Ok(())
}

/// How many releases of each platform's build of the node, and of each service's, keep their builds: the newest a
/// device behind updates to is among them.
const RELEASES_KEPT: i64 = 5;

/// Forgets the builds that only releases older than their platform's (and service's) last `RELEASES_KEPT` name: no
/// newer release names them, and no deploy's bundle is one. A forgotten blob goes from the bucket as any other does
/// (`Node::take_forgotten`). As each step of `global` is applied, so a database whose release steps came before this
/// rule forgets them at its next step, where a node notes what it forgot.
fn forget_old_releases(db: &Connection) -> Result<()> {
    let old = format!(
        "WITH ranked AS (
            SELECT content, row_number() OVER (PARTITION BY json_extract(content, '$.data.target'), json_extract(content, '$.data.service') ORDER BY at DESC) AS n
            FROM releases),
        old AS (
            SELECT b.value AS blob FROM ranked, json_each(ranked.content, '$.blobs') b GROUP BY b.value HAVING min(n) > {RELEASES_KEPT}
            EXCEPT SELECT bundle FROM deploys)"
    );
    db.execute_batch(&format!("{old} UPDATE blobs SET pinned = 0 WHERE blob IN old; {old} INSERT OR IGNORE INTO forgotten SELECT blob FROM old;"))?;
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use crate::steps::*;
    use crate::scopes::Role;
    use crate::content;

    /// `global`'s database as a network starts (`content::first`), and the steps after it.
    #[derive(Clone)]
    struct History {
        start: Value,
        steps: Vec<Step>,
    }

    impl History {
        fn write(&mut self, key: &SigningKey, user: &str, content: Content) -> Step {
            let s = sign(key, &Scope::Global, self.steps.len() as u64 + 2, user, 1, 0, content).unwrap();
            self.steps.push(s.clone());
            s
        }
    }

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";
    const ROOT: Hash = Hash::from_bytes([5; 32]);

    /// `global`'s database after `h`'s steps, each applied if it counts.
    fn global_of(h: &History) -> ScopeDb {
        let mut db = first(ROOT, &h.start, 0).unwrap();
        for s in &h.steps {
            if let Ok(a) = apply(&db, None, &Scope::Global, s) {
                db = ScopeDb::open(a.db, 0).unwrap();
            }
        }
        db
    }

    /// Andre's Mac starts the network, its recovery key kept offline, and lists Bob's laptop, Bob an admin too.
    fn two_admins() -> (History, SigningKey, SigningKey, SigningKey) {
        use ::key::Endpoint;
        use crate::content::device;
        use crate::scopes::Kind;
        let (mac, laptop, recovery) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]), SigningKey::from_bytes(&[7; 32]));
        let mut h = History { start: content::first("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &recovery.verifying_key().to_bytes(), &"e".repeat(64)), steps: Vec::new() };
        h.write(&mac, ANDRE, device(&Endpoint::of(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Admin))));
        (h, mac, laptop, recovery)
    }

    /// Each platform's (and service's) last `RELEASES_KEPT` releases keep their builds; one only older releases name is
    /// forgotten, so it goes from the bucket, unless a newer release names it too or a deploy's bundle is it.
    #[test]
    fn only_the_last_releases_of_each_platform_keep_their_builds() -> anyhow::Result<()> {
        let (mut h, mac, _, _) = two_admins();
        let blob = |n: u8| format!("{n:02x}").repeat(32);
        let release = |kind: &str, data: Value, b: String| Content { blobs: vec![b], ..Content::new(::key::APP, kind, data) };
        let linux = |n: u8| release("node-release", json!({ "target": "x86_64-unknown-linux-gnu" }), blob(n));
        h.write(&mac, ANDRE, release("service-release", json!({ "target": "x86_64-unknown-linux-gnu", "service": "whatsapp" }), blob(90)));
        h.write(&mac, ANDRE, content::app("tracker", &format!("user:{ANDRE}")));
        for n in 0..8 {
            h.write(&mac, ANDRE, linux(n));
        }
        h.write(&mac, ANDRE, release("node-release", json!({ "target": "aarch64-apple-darwin" }), blob(80)));
        h.write(&mac, ANDRE, content::deploy("tracker", &blob(1), []));
        // The newest release names an old one's build again.
        h.write(&mac, ANDRE, linux(0));
        let db = global_of(&h);
        let forgotten = db.state()?.forgotten;
        for n in [2, 3] {
            assert!(!db.names_blob(&blob(n))? && forgotten.contains(&blob(n)), "a build only releases older than the last {RELEASES_KEPT} name is forgotten");
        }
        assert!(db.names_blob(&blob(0))?, "one a newer release names again stays");
        assert!(db.names_blob(&blob(1))?, "one a deploy's bundle is stays");
        assert!((4..8).all(|n| db.names_blob(&blob(n)).unwrap()), "the last releases' builds stay");
        assert!(db.names_blob(&blob(80))? && db.names_blob(&blob(90))?, "each platform and service counts its own releases");
        Ok(())
    }

    /// Who's who is the network's first freeze: signed by the recovery key its copy names, its copy checked against the
    /// hash it names, and naming the network the reader pins. Anyone else's, an admin's own device's too, is nothing,
    /// as is one whose copy isn't what it says, or another network's; one a newer release wrote stops the reader there,
    /// never passed over. A device let in starts from its Welcome, whose hash pins it.
    #[test]
    fn a_reader_takes_only_the_recovery_key_s_freeze() -> anyhow::Result<()> {
        let (h, mac, _, recovery) = two_admins();
        let key = recovery.verifying_key().to_bytes();
        let theirs = first_freeze(&recovery, ROOT, &h.start, ANDRE, 0).unwrap();
        assert!(first_freeze(&mac, ROOT, &h.start, ANDRE, 0).is_err(), "only the recovery key it names signs it");
        let admins = Given { step: sign(&mac, &Scope::Global, 1, ANDRE, 1, 0, theirs.step.content.clone()).unwrap(), copy: theirs.copy.clone() };
        assert!(Truth::read(&[Some(admins.clone())], Some(ROOT), 0).unwrap().is_none(), "an admin's is nothing");
        let at2 = Given { step: sign(&recovery, &Scope::Global, 2, ANDRE, 1, 0, theirs.step.content.clone()).unwrap(), copy: theirs.copy.clone() };
        let t = Truth::read(&[Some(admins), Some(at2.clone())], Some(ROOT), 0).unwrap().unwrap();
        assert_eq!(t.at, 2);
        assert_eq!(t.db.recovery()?, Some(key));
        assert!(Truth::read(&[Some(theirs.clone())], Some(Hash::from_bytes([6; 32])), 0).unwrap().is_none(), "nor another network's");
        assert!(Truth::read(&[Some(theirs.clone())], None, 0).unwrap().is_some(), "though a reader that pins none takes the first");
        let mut swapped = at2;
        let mut db = connection(swapped.copy.as_deref().unwrap()).unwrap();
        db.execute_batch("UPDATE principals SET name = 'Mallory'").unwrap();
        swapped.copy = Some(db.serialize("main").unwrap().to_vec());
        assert!(Truth::read(&[None, Some(swapped.clone())], Some(ROOT), 0).unwrap().is_none(), "nor one whose copy isn't what it says");
        db.pragma_update(None, "user_version", READS + 1).unwrap();
        let newer = Freeze { at: 1, hash: database::stored::state_hash(&db).unwrap(), logs: [("global".to_string(), 1)].into() };
        let newer = Given { step: sign(&recovery, &Scope::Global, 1, ANDRE, 1, 0, newer.content()).unwrap(), copy: Some(db.serialize("main").unwrap().to_vec()) };
        assert!(Truth::read(&[Some(newer), Some(theirs.clone())], Some(ROOT), 0).err().unwrap().is::<Newer>(), "a newer release's stops it");
        // A Welcome's hash pins who's who, whoever signed the freeze it names.
        let mut t = Truth::read(&[Some(theirs.clone())], Some(ROOT), 0).unwrap().unwrap();
        let step = sign(&mac, &Scope::Global, 2, ANDRE, 1, 0, content::setting("name", Some("Ours"))).unwrap();
        assert!(t.take(2, Some(&step), 0).unwrap().is_some());
        let w = t.welcome().unwrap();
        let after = [Some(Given { step, copy: None })];
        assert_eq!(database::stored::state_hash(Truth::welcomed(&w, &theirs, None, &after, 0).unwrap().db.conn()).unwrap(), w.hash);
        assert!(Truth::welcomed(&Welcome { hash: "0".repeat(64), ..w.clone() }, &theirs, None, &after, 0).is_err(), "nor another hash");
        assert!(Truth::welcomed(&w, &theirs, None, &[None], 0).is_err(), "nor a step held back");
        Ok(())
    }

    #[test]
    fn a_step_checks_only_as_it_was_signed() {
        let key = SigningKey::from_bytes(&[1; 32]);
        let (h, sig, b) = sign(&key, &Scope::Global, 5, ANDRE, 1, 5, content::setting("n", Some("5"))).unwrap().encode();
        assert!(Step::decode(&h, &sig, &b).is_ok());
        assert!(Step::decode(&h, &sig, br#"{"app":"node","kind":"setting","data":{}}"#).is_err(), "not with another body");
        let other = String::from_utf8(h.clone()).unwrap().replace("\"time\":5", "\"time\":6");
        assert!(Step::decode(other.as_bytes(), &sig, &b).is_err(), "not with another time");
        let spaced = String::from_utf8(h).unwrap().replace(',', ", ");
        assert!(Step::decode(spaced.as_bytes(), &sig, &b).is_err(), "only as canonical JSON");
    }

    /// A scope the freeze names starts from its start, step 0, as it was written, by a device the network listed, its
    /// copy checked against its hash; without one it can't be read, and a scope the freeze doesn't name starts empty.
    #[test]
    fn a_scope_starts_from_its_start() -> anyhow::Result<()> {
        let (h, mac, _, _) = two_admins();
        let global = global_of(&h);
        let scope = Scope::User(ANDRE.into());
        let conn = crate::scope::empty(&scope).unwrap();
        conn.execute_batch(r#"INSERT INTO added VALUES ('notes', 1, 'user:a000000000000001', 0); INSERT INTO pages VALUES ('user', '{"items":["notes"]}');"#).unwrap();
        let mut db = ScopeDb::open(conn, 0).unwrap();
        let freeze = Freeze { at: 2, hash: "0".repeat(64), logs: [("global".to_string(), 9), (scope.to_string(), 4)].into() };
        let given = start(&mac, &scope, ANDRE, 1, 0, &mut db).unwrap();
        let given = decode(&scope, 0, &encoded(&given)).unwrap();
        let hash = |d: &ScopeDb| database::stored::state_hash(d.conn()).unwrap();
        assert_eq!(hash(&started(&freeze, &global, &scope, Some(&given), 0).unwrap()), hash(&db));
        assert!(started(&freeze, &global, &scope, None, 0).is_err(), "none yet: it waits for one");
        let stranger = start(&SigningKey::from_bytes(&[9; 32]), &scope, ANDRE, 1, 0, &mut db).unwrap();
        assert!(started(&freeze, &global, &scope, Some(&stranger), 0).is_err(), "nor one from outside the network");
        let mut swapped = given.clone();
        let mut c = connection(swapped.copy.as_deref().unwrap()).unwrap();
        c.execute_batch("PRAGMA user_version = 1").unwrap();
        swapped.copy = Some(c.serialize("main").unwrap().to_vec());
        assert!(started(&freeze, &global, &scope, Some(&swapped), 0).is_err(), "nor a copy that isn't what it says");
        let other = Scope::User(BOB.into());
        assert!(started(&freeze, &global, &other, None, 0).unwrap().state()?.added.is_empty(), "a scope with no frozen log starts empty");
        Ok(())
    }

    #[test]
    fn a_scope_s_reader_stops_at_a_newer_release_s_step() {
        let (h, mac, _, _) = two_admins();
        let global = global_of(&h);
        let scope = Scope::User(ANDRE.into());
        let db = || ScopeDb::empty(&scope, 0).unwrap();
        let mut newer = release_defaults().unwrap();
        newer.data["reads"] = json!(READS + 1);
        let newer = Some(Given { step: sign(&mac, &scope, 2, ANDRE, 1, 0, newer).unwrap(), copy: None });
        let read = |steps| Read { steps, waits: None };
        let mut r = ScopeReader { at: 0, db: db(), whole: true };
        assert!(r.take(0, &read(vec![None, newer, None]), &global, &scope, 0).err().unwrap().is::<Newer>() && r.at == 1, "stopped before it");
        let mut r = ScopeReader { at: 0, db: db(), whole: true };
        assert!(r.take(0, &read(vec![None, None]), &global, &scope, 0).unwrap() && r.at == 2);
    }

    /// A newest hint is anyone's to write: one far above the last step jams no reader, which reads on to the steps'
    /// own end. Nor does anything else past a position with nothing, unless it's a step; nor a step under an epoch
    /// above every one made, which a reader passes over once it's applied those before it.
    #[test]
    fn keyless_objects_jam_no_reader() {
        use ::bucket::Bucket;
        let bucket = ::bucket::Memory::default();
        let prefix = prefix("global");
        let keys: database::stored::Keys = [(1, [9; 32])].into();
        let (_, mac, _, _) = two_admins();
        let step = |at: u64| {
            let name = ::bucket::names::step_name(&prefix, at);
            seal(&keys, &name, &encode(&sign(&mac, &Scope::Global, at, ANDRE, 1, 0, content::setting("name", Some("x"))).unwrap(), None)).unwrap()
        };
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            bucket.create(&::bucket::names::newest_name(&prefix), br#"{"at": 1000000000, "copy": 0, "epoch": 1}"#).await.unwrap();
            assert!(read(&bucket, &keys, &Scope::Global, 0, 1).await.unwrap().steps.is_empty(), "nothing written, nothing read, and no failure");
            bucket.create(&::bucket::names::step_name(&prefix, 3), b"x").await.unwrap();
            assert!(read(&bucket, &keys, &Scope::Global, 0, 1).await.unwrap().steps.is_empty(), "nor junk after a position with nothing");
            bucket.create(&::bucket::names::step_name(&prefix, 2), &step(2)).await.unwrap();
            assert!(read(&bucket, &keys, &Scope::Global, 0, 1).await.is_err(), "but a step after one with nothing stops it: that one was written");
            let add = async |r: &mut Reader| r.add(&fetch(&bucket, r.at(), None).await.unwrap(), &keys, None, 0, |_, _, _| Ok(()));
            assert!(add(&mut Reader::new(None)).await.is_err(), "as it stops who's who's reader");
            // Junk at 1, under an epoch no one made.
            let junk = ::bucket::names::step_name(&prefix, 1);
            bucket.create(&junk, &::key::seal(0xFFFF_FFFF, &[3; 32], &junk, b"junk")).await.unwrap();
            let r = read(&bucket, &keys, &Scope::Global, 0, u32::MAX).await.unwrap();
            assert!(r.steps.is_empty() && r.waits == Some(u32::MAX), "it waits only for an epoch that was made");
            let r = read(&bucket, &keys, &Scope::Global, 0, 1).await.unwrap();
            assert!(r.steps.len() == 3 && r.steps[0].is_none() && r.waits.is_none(), "and passes over one above every epoch made");
            assert!(matches!(open(&keys, 1, &Scope::Global, 1, &bucket.get(&junk).await.unwrap().unwrap().bytes), Opened::Junk));
            let r = read(&bucket, &keys, &Scope::Global, 1, 1).await.unwrap();
            assert!(r.steps.len() == 2 && r.steps[0].is_some() && r.steps[1].is_none(), "and reads on past it, passing over the junk after");
            let mut reader = Reader::new(None);
            assert!(add(&mut reader).await.unwrap().is_none() && reader.at() == 3, "who's who's reader, before its freeze, passes over junk under an epoch above every key it has");
        });
    }

    /// A release's roles that don't count stop no one: whatever release it names, a `Change::Defaults` step by a device that
    /// isn't an admin's own is passed over; only an admin's of a newer release stops a reader.
    #[test]
    fn only_an_admin_s_newer_release_stops_a_reader() {
        let (h, mac, laptop, _) = two_admins();
        let mut h = h;
        h.write(&mac, ANDRE, content::role(BOB, Role::Member));
        let db = global_of(&h);
        let step = |key: &SigningKey, user: &str| {
            let mut content = release_defaults().unwrap();
            content.data["reads"] = json!(999);
            sign(key, &Scope::Global, 1, user, 1, 0, content).unwrap()
        };
        let stranger = SigningKey::from_bytes(&[9; 32]);
        for (key, user) in [(&laptop, BOB), (&stranger, ANDRE)] {
            let err = apply(&db, None, &Scope::Global, &step(key, user)).err().unwrap();
            assert!(!err.is::<Newer>(), "passed over, not a newer release: {err:#}");
        }
        assert!(apply(&db, None, &Scope::Global, &step(&mac, ANDRE)).err().unwrap().is::<Newer>());
    }

    /// A release's types, roles and rules move a database on only to a newer release's, by an admin's own device, and a
    /// reader of an older one stops there rather than read on as if it didn't; and from the first that reads the scope
    /// database, each `READS` is held to the tables, types, roles and rules it reads, so a release that changes them
    /// reads one more.
    #[test]
    fn a_release_s_roles_only_move_on_and_an_older_reader_stops_at_a_newer_one() {
        use ::key::Endpoint;
        use crate::scopes::Kind;
        let mac = SigningKey::from_bytes(&[1; 32]);
        // A database an older release made.
        let db = first(ROOT, &content::first("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)), 0).unwrap();
        db.conn().pragma_update(None, "user_version", READS - 1).unwrap();
        let step = |at: u64, reads: u32| {
            let mut content = release_defaults().unwrap();
            content.data["reads"] = json!(reads);
            sign(&mac, &Scope::Global, at, ANDRE, 1, 0, content).unwrap()
        };
        let newer = apply(&db, None, &Scope::Global, &step(1, READS + 1)).err().unwrap();
        assert!(newer.is::<Newer>(), "a newer release's stops this reader");
        let db = ScopeDb::open(apply(&db, None, &Scope::Global, &step(1, READS)).unwrap().db, 0).unwrap();
        assert_eq!(version(db.conn()).unwrap(), READS);
        assert!(apply(&db, None, &Scope::Global, &step(2, READS)).is_err(), "the same release's again changes nothing");
        // What each `READS` reads, from the first that reads the scope database (2): add `(READS, hash)`
        // when a release changes them, and read one more.
        const PINNED: &[(u32, &str)] = &[(2, "1a26a7b14a1b8998d5edcf6b6d00db14a698c6109eb865f7bd8e7c86dcd295a4"), (3, "e614e4c705368711133be27dd44ff51c24ff89da7fe059001ad3990f0b454a67"), (4, "faa5b6936f41b2763b4a4a4533d82323bb60e068ba3822ed6bafbf4e2921fd6b"), (5, "faa5b6936f41b2763b4a4a4533d82323bb60e068ba3822ed6bafbf4e2921fd6b"), (6, "faa5b6936f41b2763b4a4a4533d82323bb60e068ba3822ed6bafbf4e2921fd6b"), (7, "623223311093180cd17e55f5a9933c62c2b900d16936ed9bb902b30999c175a7"), (8, "5088091c0646f689ee10f3190a3c1a933d9b2e34ac27c254c7df547f7573f688"), (9, "eed18d0f11ad255a9e8786c88fbe560a15a1949739b802105766ed1c1f555c4e"), (10, "eed18d0f11ad255a9e8786c88fbe560a15a1949739b802105766ed1c1f555c4e"), (11, "ba4b7c2281b445b48330bf8b0825f0e55e2eee85d7aca7dbddc4f3f2d5a6fa83"), (12, "681acdc557538974d9dc1b0583acdd6b1af036d11f767b788a865d780561a925"), (13, "5fa1374ca77f51360ebe0a1877aa961d124c2fc241a85943075eda4fc190aebb"), (14, "5fa1374ca77f51360ebe0a1877aa961d124c2fc241a85943075eda4fc190aebb"), (15, "5fa1374ca77f51360ebe0a1877aa961d124c2fc241a85943075eda4fc190aebb"), (16, "9c427f37e7a5465e7b932816526cffc54f7bb8bf9d20bf1515ef4c15634e6ba0")];
        let mut h = blake3::Hasher::new();
        h.update(crate::scope::SCHEMA.as_bytes());
        h.update(permissions::SCHEMA.as_bytes());
        h.update(rules_of(db.conn()).unwrap().to_string().as_bytes());
        let hash = h.finalize().to_hex().to_string();
        assert!(PINNED.contains(&(READS, hash.as_str())), "the scope database's tables, types, roles or rules changed: read one more in READS, and pin ({}, \"{hash}\")", READS + 1);
    }

    /// Who may write what as a step, by the rows at that step: a member makes a group but changes no setting, a removed
    /// device's next step counts for nothing, nor one naming an epoch not reached.
    #[test]
    fn a_step_counts_only_if_its_signer_may_make_that_change_there() -> anyhow::Result<()> {
        use ::key::Endpoint;
        use crate::content::{device, group, remove_device, setting};
        let node = |kind: &str, data: Value| Content::new(::key::APP, kind, data);
        use crate::scopes::Kind;
        let (mac, laptop, phone) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]), SigningKey::from_bytes(&[3; 32]));
        let mut db = first(ROOT, &content::first("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)), 0).unwrap();
        let mut at = 0;
        let mut step = |db: &mut ScopeDb, key: &SigningKey, user: &str, epoch: u32, content: Content| -> bool {
            at += 1;
            let s = sign(key, &Scope::Global, at, user, epoch, 0, content).unwrap();
            let s = decode(&Scope::Global, at, &encode(&s, None)).unwrap().step;
            match apply(db, None, &Scope::Global, &s) {
                Ok(a) => {
                    *db = ScopeDb::open(a.db, 0).unwrap();
                    true
                }
                Err(_) => false,
            }
        };
        assert!(step(&mut db, &mac, ANDRE, 1, device(&Endpoint::of(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Member)))));
        assert!(step(&mut db, &laptop, BOB, 1, group("c000000000000003", "Band")), "a member makes a group");
        assert!(!step(&mut db, &laptop, BOB, 1, setting("name", Some("Bob's"))), "but changes no setting");
        assert!(!step(&mut db, &laptop, BOB, 1, device(&Endpoint::of(&phone), ANDRE, "phone", &[Kind::Client], None)), "nor lists another's device");
        assert!(!step(&mut db, &mac, ANDRE, 2, setting("name", Some("Ours"))), "nor names an epoch not reached");
        assert!(step(&mut db, &mac, ANDRE, 1, remove_device(&laptop.verifying_key().to_bytes())));
        assert!(!step(&mut db, &laptop, BOB, 1, node("rename", json!({ "user": BOB, "name": "Robert" }))), "a removed device's next step counts for nothing");
        assert!(db.group("c000000000000003")?.is_some() && db.setting("name")?.is_none());
        // Signed for one place, it's nowhere else.
        let s = sign(&mac, &Scope::Global, 9, ANDRE, 1, 0, setting("name", Some("x"))).unwrap();
        assert!(decode(&Scope::Global, 10, &encode(&s, None)).is_err() && decode(&Scope::User(ANDRE.into()), 9, &encode(&s, None)).is_err());
        Ok(())
    }
}
