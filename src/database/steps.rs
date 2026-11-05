//! A scope database's steps, once its log is frozen (`switch`): the database is the truth, and every change to it is a
//! step at the next free position of its own in the bucket, which the bucket keeps in one order.
//!
//! A step is one change, signed by the device that made it as its person: the same content an entry of the log carried
//! (`members`' kinds, and a scope's own), for one position, under `entry::STEP_DOMAIN`, so it's never taken for an
//! entry nor an entry for it. Its position is signed in it (its `seq`), so a step can't be written twice, or anywhere
//! but where its writer meant it: one that loses its position to another is signed again for the next. A node only
//! carries a client's step to the bucket: it can hold one back, but change none.
//!
//! Every reader applies each step to the database as the steps before it left it, and counts it only if all three of
//! the protocol's rules hold there:
//! 1. the install's first step and its recovery key: the database's first copy is replay's at the frozen log's end
//!    (`anchor`), which every reader builds itself; and only the recovery key lets a device in by itself;
//! 2. epochs never go down: a step names its scope's epoch its writer knew, never one above the database's;
//! 3. its signer holds the permission the change needs, at that step (`allowed`): `may`, `may_give`, `may_take` and
//!    `may_make` over the database's rows; and is one of its person's own devices still listed, or for a scope's own
//!    state, a device that runs the scope's calls for them.
//!
//! A step that doesn't count is passed over, its position taken, as a database's are: nothing it says happens. What a
//! counted one changes is what replay's entry of the same content changed (`Members` and a scope's own state, read from
//! the database and built back into it), so the database a step leaves is the one replay would have made.
//!
//! Its steps are sealed as the scope's log was, with the key of the log in each epoch (`log::seal`), under their own
//! names, so whoever was handed the log's key reads them.
//!
//! ```text
//! <prefix>/<n>               step n, from 1: one signed step, its header, signature and body (`encode`)
//! ```

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::SigningKey;
use rusqlite::Connection;

use crate::entry::{Content, Entry, Ext, Scope, unhex};
use crate::members::{self, Members, Principal, Role, ScopeState};
use crate::permissions;

use super::scopes::{self, ScopeDb};

/// The node's database of a scope that its steps are: its `scope`.
pub const NAME: &str = "scope";

/// Where a scope's database's steps are in the bucket.
pub fn prefix(scope: &str) -> String {
    format!("{}/{}/{}", crate::bucket::folder(scope), members::APP, NAME)
}

/// The first position a step is written at.
pub const FIRST: u64 = 1;

/// A step of `scope`'s database for position `at`, signed by `key` as `user`, the scope at `epoch`.
pub fn sign(key: &SigningKey, scope: &Scope, at: u64, user: &str, epoch: u32, time: u64, content: Content) -> Result<Entry> {
    let at = u32::try_from(at).context("that database has more steps than a step can name")?;
    Ok(Entry::create_step(key, Ext { log: scope.to_string(), time, user: user.into(), epoch, previous: Vec::new() }, at, content))
}

/// A step as the bucket holds it, before it's sealed: its header, signature and body, as CBOR.
pub fn encode(step: &Entry) -> Vec<u8> {
    let (h, s, b) = step.encode();
    let mut out = Vec::new();
    ciborium::into_writer(&ciborium::Value::Array(vec![ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b)]), &mut out).expect("cbor in memory");
    out
}

/// A step as `encode` wrote it, its signature checked, for position `at` of `scope`'s database.
pub fn decode(scope: &Scope, at: u64, bytes: &[u8]) -> Result<Entry> {
    let ciborium::Value::Array(hsb) = ciborium::from_reader(bytes)? else { bail!("not a step") };
    let [ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b)] = hsb.as_slice() else { bail!("not a step") };
    let step = Entry::decode_step(h, s, b)?;
    ensure!(step.log() == scope.to_string() && u64::from(step.seq()) == at, "a step for another place");
    Ok(step)
}

/// `global`'s scope database as a reader that holds no copy of its own makes it, at `now`: the one the log's freeze
/// anchors, built from the entries it follows (refused if that isn't what the freeze says it comes to), then each of
/// `steps`, by position from the first, applied if it counts there (none: one that isn't a step, passed over).
pub fn truth(frozen: &members::Frozen, steps: &[Option<Entry>], now: u64) -> Result<ScopeDb> {
    let mut db = ScopeDb::build(&frozen.members, &Scope::Global, &frozen.state, now)?;
    ensure!(crate::db::state_hash(db.conn())? == frozen.hash, "who's who at your One App's freeze isn't what its admin's device said it was: this device used none of it");
    for step in steps.iter().flatten() {
        if let Ok(a) = apply(&db, None, &Scope::Global, step) {
            db = ScopeDb::open(a.db, now)?;
        }
    }
    Ok(db)
}

/// A step as a node gives it (`GET /v1/steps/global`'s `{ at, step }`, or as a bucket holds it opened), for position
/// `at` of `scope`'s database: none if it isn't one, which every reader passes over.
pub fn given(scope: &Scope, at: u64, opened: Option<&[u8]>) -> Option<Entry> {
    opened.and_then(|b| decode(scope, at, b).ok())
}

/// Reads `scope`'s steps after `from` from the bucket, opened with the keys of its log by epoch: each by position,
/// none for one that isn't a step or doesn't open. Stops at the first position with nothing, or one under an epoch
/// above every key given, which waits for that epoch's key.
pub async fn read(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, scope: &Scope, from: u64) -> Result<Vec<Option<Entry>>> {
    let prefix = prefix(&scope.to_string());
    let mut out = Vec::new();
    loop {
        let at = from + out.len() as u64 + 1;
        let name = crate::db::step_name(&prefix, at);
        let Some(o) = bucket.get(&name).await? else { break };
        let epoch = crate::keys::epoch_of(&o.bytes).ok();
        if epoch.is_some_and(|e| keys.keys().next_back().is_some_and(|newest| e > *newest)) {
            break;
        }
        let opened = epoch.and_then(|e| keys.get(&e)).and_then(|k| crate::keys::open(k, &name, &o.bytes).ok());
        out.push(given(scope, at, opened.as_deref()));
    }
    Ok(out)
}

/// What a step that counts made of the database it was applied to: the database it leaves.
pub struct Applied {
    pub db: Connection,
}

/// Applies `step` to `db`, `scope`'s database as the steps before it left it, with `global`'s as it is now (for a
/// scope other than `global`, whose permissions are `global`'s): → the database it leaves, or why it doesn't count.
pub fn apply(db: &ScopeDb, global: Option<&ScopeDb>, scope: &Scope, step: &Entry) -> Result<Applied> {
    ensure!(step.scope() == *scope, "a step of another scope");
    let who = global.unwrap_or(db);
    if step.content.app == members::APP && step.content.kind == DEFAULTS {
        return defaults(db, who, scope, step);
    }
    let mut m = who.members();
    let mut state = db.state();
    ensure!(step.epoch() <= m.epoch(scope), "it names an epoch its scope hasn't reached");
    allowed(who, scope, step)?;
    let member = members::is_member_entry(step);
    if *scope == Scope::Global && member {
        m.try_step(step)?;
        if matches!(step.content.kind.as_str(), "install" | "uninstall") {
            members::applies_in_scope(&m, &state, step)?;
            state = apply_own(&m, state, step)?;
        }
    } else {
        ensure!(m.counts(step), "{} can't write in {scope} for that person", crate::entry::short(&step.author()));
        if step.content.app == members::APP {
            state = apply_own(&m, state, step)?;
        }
    }
    let built = super::scope::build(&if *scope == Scope::Global { m } else { Members::default() }, scope, &state)?;
    carry(db.conn(), &built)?;
    named(&built, step)?;
    Ok(Applied { db: built })
}

/// The kind of step that says the release's types, roles and rules (`permissions::DEFAULTS`), as rows: the first an
/// admin's device writes once the log is frozen, and again whenever a release changes them. Readers never run their
/// own release's: each database keeps the rows its steps said.
pub const DEFAULTS: &str = "defaults";

/// The release's types, roles and rules as `DEFAULTS` steps say them: `{ types, roles, rules }`, rows as arrays.
pub fn release_defaults() -> Result<Content> {
    let db = Connection::open_in_memory()?;
    db.execute_batch(permissions::SCHEMA)?;
    permissions::defaults(&db)?;
    Ok(Content::new(members::APP, DEFAULTS, rules_of(&db)?))
}

/// A database's types, roles and rules, as a `DEFAULTS` step says them.
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

/// A row of `relations` that's a rule: the release's, never given or taken (`permissions`).
const RULE: &str = "(substr(subject, 1, 1) IN ('#', '@') OR instr(subject, '*') > 0 OR instr(object, '*') > 0)";

/// A `DEFAULTS` step: by an admin's device, the release's types, roles and rules as it says them, in place of those
/// the database had; the install's own roles stay.
fn defaults(db: &ScopeDb, who: &ScopeDb, scope: &Scope, step: &Entry) -> Result<Applied> {
    ensure!(who.client(&step.author()).is_some_and(|c| c.user == step.user()), "only a person's own device writes for them");
    ensure!(who.may(step.user(), "change-roles", scopes::INSTALL), "only the install's admins say the release's roles");
    let out = super::scope::build(&if *scope == Scope::Global { who.members() } else { Members::default() }, scope, &db.state())?;
    carry(db.conn(), &out)?;
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
    tx.commit()?;
    Ok(Applied { db: out })
}

/// What only steps say, which building from replay's members leaves out, copied from the database before: what they
/// named, and the release's types, roles and rules the steps said (`DEFAULTS`), not this reader's own release's.
fn carry(from: &Connection, to: &Connection) -> Result<()> {
    to.execute_batch(&format!("DELETE FROM types; DELETE FROM roles WHERE builtin = 1; DELETE FROM relations WHERE {RULE};"))?;
    let mut rules = from.prepare(&format!("SELECT subject, relation, object, until FROM relations WHERE {RULE}"))?;
    let mut rows = rules.query([])?;
    while let Some(r) = rows.next()? {
        to.execute("INSERT OR IGNORE INTO relations VALUES (?, ?, ?, ?)", rusqlite::params![r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<i64>>(3)?])?;
    }
    for table in ["types", "roles"] {
        let mut q = from.prepare(&format!("SELECT * FROM {table}{}", if table == "roles" { " WHERE builtin = 1" } else { "" }))?;
        let n = q.column_count();
        let insert = format!("INSERT OR REPLACE INTO {table} VALUES ({})", vec!["?"; n].join(", "));
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            let values: Vec<rusqlite::types::Value> = (0..n).map(|i| row.get(i)).collect::<rusqlite::Result<_>>()?;
            to.execute(&insert, rusqlite::params_from_iter(values))?;
        }
    }
    for table in ["blobs", "releases"] {
        let mut q = from.prepare(&format!("SELECT * FROM {table}"))?;
        let n = q.column_count();
        let insert = format!("INSERT INTO {table} VALUES ({})", vec!["?"; n].join(", "));
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            let values: Vec<rusqlite::types::Value> = (0..n).map(|i| row.get(i)).collect::<rusqlite::Result<_>>()?;
            to.execute(&insert, rusqlite::params_from_iter(values))?;
        }
    }
    Ok(())
}

/// What a counted step names that no other table keeps: its blobs, and a release.
fn named(db: &Connection, step: &Entry) -> Result<()> {
    let pinned = step.content.app == members::APP && step.content.kind != "keep";
    for blob in &step.content.blobs {
        db.execute("INSERT INTO blobs VALUES (?1, ?2) ON CONFLICT (blob) DO UPDATE SET pinned = max(pinned, ?2)", rusqlite::params![blob, pinned])?;
    }
    if step.content.app == members::APP && members::RELEASES.contains(&step.content.kind.as_str()) {
        db.execute(
            "INSERT INTO releases VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![step.seq(), step.content.kind, serde_json::to_string(&step.content)?, crate::entry::hex(&step.author()), i64::try_from(step.time()).unwrap_or(i64::MAX)],
        )?;
    }
    Ok(())
}

/// A scope's own state after `step`, a node entry of its own: replay's rules for it, and what it does.
fn apply_own(m: &Members, mut state: ScopeState, step: &Entry) -> Result<ScopeState> {
    members::apply_in_scope(m, &mut state, step)?;
    Ok(state)
}

/// Rule 3: whether `step`'s signer, as its person, holds the permission its change needs in `db` (`global`'s), before
/// it. Replay's own rules for the change are checked too, as it's applied (`apply`): this is the engine's.
pub fn allowed(db: &ScopeDb, scope: &Scope, step: &Entry) -> Result<()> {
    let data = &step.content.data;
    let w = step.user();
    let text = |k: &str| data[k].as_str().map(str::to_string).with_context(|| format!("no {k}"));
    let may = |verb: &str, object: &str| db.may(w, verb, object);
    let role = |k: &str| -> Result<Role> { Ok(serde_json::from_value(data[k].clone())?) };
    let ok = match (step.content.app.as_str(), step.content.kind.as_str()) {
        // Rule 1: the recovery key lets a device in, and hands over to the next; nothing else.
        _ if db.recovery() == Some(step.author()) => matches!(step.content.kind.as_str(), "device" | "recovery") && *scope == Scope::Global,
        (members::APP, "genesis") => false,
        (members::APP, "device") => {
            let user = text("user")?;
            let key = unhex(&text("key")?)?;
            let kinds: Vec<members::Kind> = serde_json::from_value(data["roles"].clone()).unwrap_or_default();
            let theirs = kinds.iter().any(|k| matches!(k, members::Kind::Client | members::Kind::Service));
            let verb = if theirs { "add-devices" } else { "add-machines" };
            match (db.user_ever(&user), db.machine_ever(&key)) {
                // Someone new, with their first device: given a role in the install by someone who may give it.
                (None, None) => db.may_give(w, &user, super::scope::role(role("role")?), scopes::INSTALL),
                (Some(_), None) => db.user(&user).is_some() && maker(db, w, verb, &scopes::person(&user), &scopes::person(&user), "owner", &scopes::device(&key)),
                // Listed again by its person's own device, to name the device it's part of.
                (Some(_), Some(_)) => may("add-devices", &scopes::person(&user)),
                (None, Some(_)) => false,
            }
        }
        (members::APP, "role") => {
            let user = text("user")?;
            let was = db.user(&user).context("no such person")?.role;
            let to = role("role")?;
            to == was || (db.may_give(w, &user, super::scope::role(to), scopes::INSTALL) && db.may_take(w, &user, super::scope::role(was), scopes::INSTALL))
        }
        (members::APP, "remove") => match text("user") {
            Ok(user) => db.user(&user).is_some_and(|u| db.may_take(w, &user, super::scope::role(u.role), scopes::INSTALL)),
            Err(_) => may("remove", &scopes::device(&unhex(&text("key")?)?)),
        },
        (members::APP, "epoch") => {
            let s = Scope::parse(&text("scope")?)?;
            may("remove", &scopes::object(&s))
        }
        (members::APP, "runs") => may("keep", &scopes::person(w)),
        (members::APP, "grant") | (members::APP, "vault") => may("keep", &scopes::object(&Scope::parse(&text("scope")?)?)),
        (members::APP, "group") => may("make-groups", scopes::INSTALL),
        (members::APP, "invite") => db.may_give(w, &text("user")?, super::scope::role(role("role")?), &scopes::group(&text("group")?)),
        // Taking up an invite: the role the invite gave, which was checked as it was given.
        (members::APP, "accept") => db.roles().get(&(w.to_string(), Scope::Group(text("group")?))).is_some_and(|(_, invited)| *invited),
        (members::APP, "member") => {
            let (g, user) = (text("group")?, text("user")?);
            let object = scopes::group(&g);
            let was = db.group_role(&user, &g).or_else(|| db.group(&g).and_then(|grp| grp.invited.get(&user).copied()));
            let to: Option<Role> = serde_json::from_value(data["role"].clone())?;
            let took = was.is_none_or(|r| Some(r) == to || db.may_take(w, &user, super::scope::role(r), &object));
            took && to.is_none_or(|r| Some(r) == was || db.may_give(w, &user, super::scope::role(r), &object))
        }
        (members::APP, "rename") => {
            if let Ok(key) = text("key") {
                may("rename", &scopes::device(&unhex(&key)?))
            } else if let Ok(user) = text("user") {
                may("rename", &scopes::person(&user))
            } else {
                may("rename", &scopes::group(&text("group")?))
            }
        }
        (members::APP, "setting") => may("change-settings", scopes::INSTALL),
        (members::APP, "recovery") => false,
        (members::APP, "app") => {
            let name = text("name")?;
            let owner = Principal::parse(&text("owner")?)?;
            let to = scopes::owner_object(&owner);
            match db.app(&name) {
                None => {
                    let holders = match &owner {
                        Principal::User(u) => scopes::person(u),
                        Principal::Group(g) => format!("{}#admin", scopes::group(g)),
                        Principal::Admins => format!("{}#admin", scopes::INSTALL),
                    };
                    maker(db, w, "add-apps", &to, &holders, "owner", &scopes::app(&name))
                }
                // Deleting, restoring, erasing or giving it away; or saying where its code is.
                Some(a) => {
                    let moves = a.owner != owner;
                    let retires = data.get("deleted").is_some() || data["erased"] == true || moves;
                    (!retires || may("delete", &scopes::app(&name))) && (!moves || may("add-apps", &to)) && (data.get("source").is_none() || may("deploy", &scopes::app(&name)))
                }
            }
        }
        (members::APP, "deploy") => may("deploy", &scopes::app(&text("app")?)),
        (members::APP, "service") => may("switch", &scopes::device(&unhex(&text("device")?)?)),
        (members::APP, "install" | "uninstall" | "schedule" | "vault-request") => may("add-apps", &scopes::object(scope)) || runs_it(db, scope, step),
        (members::APP, "keep" | "forget") => may("use", &scopes::object(scope)) || runs_it(db, scope, step),
        // A release, which every node checks is signed with the release key before it takes it.
        (members::APP, kind) if members::RELEASES.contains(&kind) => true,
        (members::APP, "arrange") => *scope == Scope::User(w.to_string()) && may("use", &scopes::object(scope)),
        // An app's own entries, and the release's: as replay counts them, by who signed them.
        (members::APP, _) => false,
        _ => true,
    };
    ensure!(ok, "{} may not {} there", w, step.content.kind);
    Ok(())
}

/// Whether `maker` may make `object`, new, with its first row `(subject, relation, object)` (`permissions::may_make`).
fn maker(db: &ScopeDb, maker: &str, verb: &str, container: &str, subject: &str, relation: &str, object: &str) -> bool {
    permissions::may_make(db.conn(), &scopes::person(maker), verb, container, subject, relation, object, db.now()).unwrap_or(false)
}

/// Whether a step of a scope's own state was signed by a device that runs the scope's calls, for a person who may use
/// the scope: a node writes what a call it ran did.
fn runs_it(db: &ScopeDb, scope: &Scope, step: &Entry) -> bool {
    db.runs_for(&step.author(), scope) && db.may(step.user(), "use", &scopes::object(scope))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// That `entries`, replay's in its order, applied as steps from the genesis on, leave the database replay's
    /// `m` builds: what each counted for and changed is replay's.
    /// Histories written one after another (each entry after every one before it): those where replay's order is
    /// the one the steps would have, with nothing written at once, which a database's one order never has.
    pub(crate) fn agrees(entries: &[Entry], m: &Members) {
        let mut before: std::collections::HashMap<crate::entry::Hash, std::collections::HashSet<crate::entry::Hash>> = Default::default();
        for (i, e) in entries.iter().enumerate() {
            let mut all: std::collections::HashSet<_> = e.follows_all().collect();
            for f in e.follows_all() {
                all.extend(before.get(&f).cloned().unwrap_or_default());
            }
            if !entries[..i].iter().all(|p| all.contains(&p.hash())) {
                return;
            }
            before.insert(e.hash(), all);
        }
        // Nor a removal that says its remover had seen less than it had, which replay takes back from before it: a
        // step's place is where it was written, so what came before a removal counts.
        let last_removal = entries.iter().rposition(|e| e.content.kind == "remove").unwrap_or(0);
        if entries[..last_removal].iter().any(|e| m.cut(e)) {
            return;
        }
        let genesis = &entries[..1];
        let start = Members::replay(Some(genesis[0].hash()), genesis, &Default::default());
        let mut db = ScopeDb::build(&start, &Scope::Global, &ScopeState::default(), 0).unwrap();
        for e in &entries[1..] {
            match apply(&db, None, &Scope::Global, e) {
                Ok(a) => db = ScopeDb::open(a.db, 0).unwrap(),
                Err(err) => tracing::debug!("{} doesn't count: {err:#}", e.content.kind),
            }
        }
        let replayed = ScopeDb::build(m, &Scope::Global, &ScopeState::default(), 0).unwrap();
        let hash = |d: &ScopeDb| crate::db::state_hash(d.conn()).unwrap();
        if hash(&db) != hash(&replayed) {
            let dump = |d: &ScopeDb| {
                let mut out = Vec::new();
                for t in ["install", "principals", "invites", "devices", "relations", "epochs", "sealed", "owed", "keeps", "apps", "deploys", "services", "settings", "vault", "vault_opens", "recovery", "runs", "runs_chosen"] {
                    let mut q = d.conn().prepare(&format!("SELECT * FROM {t} ORDER BY 1")).unwrap();
                    let n = q.column_count();
                    let rows: Vec<String> = q.query_map([], |r| Ok((0..n).map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap())).collect::<Vec<_>>().join(", "))).unwrap().map(Result::unwrap).collect();
                    out.push(format!("{t}: {rows:#?}"));
                }
                out.join("\n")
            };
            panic!("steps left\n{}\nreplay made\n{}", dump(&db), dump(&replayed));
        }
    }

    /// Who may write what as a step, by the rows at that step: a member makes a group but changes no setting, a removed
    /// device's next step counts for nothing, nor one naming an epoch not reached; and a step is never an entry.
    #[test]
    fn a_step_counts_only_if_its_signer_may_make_that_change_there() {
        use crate::keys::Endpoint;
        use crate::members::{Kind, device, genesis, group, node, remove_device, setting};
        const ANDRE: &str = "a000000000000001";
        const BOB: &str = "b000000000000002";
        let (mac, laptop, phone) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]), SigningKey::from_bytes(&[3; 32]));
        let g = Entry::create(&mac, Ext { log: "global".into(), time: 0, user: ANDRE.into(), epoch: 0, previous: Vec::new() }, None, genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)));
        let m = Members::replay(Some(g.hash()), std::slice::from_ref(&g), &Default::default());
        let mut db = ScopeDb::build(&m, &Scope::Global, &ScopeState::default(), 0).unwrap();
        let mut at = 0;
        let mut step = |db: &mut ScopeDb, key: &SigningKey, user: &str, epoch: u32, content: Content| -> bool {
            at += 1;
            let s = sign(key, &Scope::Global, at, user, epoch, 0, content).unwrap();
            let s = decode(&Scope::Global, at, &encode(&s)).unwrap();
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
        assert!(step(&mut db, &mac, ANDRE, 1, remove_device(&laptop.verifying_key().to_bytes(), None)));
        assert!(!step(&mut db, &laptop, BOB, 1, node("rename", json!({ "user": BOB, "name": "Robert" }))), "a removed device's next step counts for nothing");
        assert!(db.group("c000000000000003").is_some() && db.setting("name").is_none());
        // Signed for one place, it's nowhere else; and an entry, signed the same way, is never a step.
        let s = sign(&mac, &Scope::Global, 9, ANDRE, 1, 0, setting("name", Some("x"))).unwrap();
        assert!(decode(&Scope::Global, 10, &encode(&s)).is_err() && decode(&Scope::User(ANDRE.into()), 9, &encode(&s)).is_err());
        assert!(decode(&Scope::Global, 0, &encode(&g)).is_err());
    }
}
