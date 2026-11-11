//! Where a database's objects are in the bucket, by its prefix: its newest hint, its steps, the signatures beside older
//! ones, its copies, and the mark that its steps may have been tidied.


/// What says a database's steps may have been deleted (`tidy`): created before the first goes.
pub fn tidied_name(prefix: &str) -> String {
    format!("{prefix}/{}", custom_build::protocol::TIDIED)
}

/// A database's objects in the bucket, by its prefix: its newest hint, step `n`, and its copy after step `n`.
pub fn newest_name(prefix: &str) -> String {
    format!("{prefix}/{}", custom_build::protocol::NEWEST)
}

/// A step's position as its name says it.
pub fn position_text(n: u64) -> String {
    format!("{n:0width$}", width = custom_build::protocol::STEP_DIGITS)
}

pub fn step_name(prefix: &str, n: u64) -> String {
    format!("{prefix}/{}", position_text(n))
}

pub fn copies_folder(prefix: &str) -> String {
    format!("{prefix}/{}", custom_build::protocol::COPY)
}

pub fn copy_name(prefix: &str, n: u64) -> String {
    format!("{}/{}", copies_folder(prefix), position_text(n))
}

pub fn sig_name(prefix: &str, n: u64) -> String {
    format!("{}.sig", step_name(prefix, n))
}

/// Whether the steps at `prefix` may have signatures beside them, as an app database's older steps do, rather than the
/// scope database's (`database::steps`), whose steps are signed in them.
pub fn signed_beside(prefix: &str) -> bool {
    !keeps_every_step(prefix)
}

/// A database's prefix and position, from the name of one of its steps or the signature beside one.
pub fn position(name: &str) -> Option<(&str, u64)> {
    let (prefix, last) = name.rsplit_once('/')?;
    let n = last.strip_suffix(".sig").unwrap_or(last).parse::<u64>().ok().filter(|n| *n > 0)?;
    [step_name(prefix, n), sig_name(prefix, n)].contains(&name.to_string()).then_some((prefix, n))
}

/// If `name` is a database's step, or the signature beside one: the signature beside it, and the `n` steps after it,
/// each with the signature beside it: what's read next. A scope's log's steps have none.
pub fn following(name: &str, n: u32) -> Vec<String> {
    let Some((prefix, at)) = position(name) else { return Vec::new() };
    let signed = signed_beside(prefix);
    let names = (at..=at + u64::from(n)).flat_map(|i| [Some(step_name(prefix, i)), signed.then(|| sig_name(prefix, i))]).flatten();
    names.filter(|other| other.as_str() > name).collect()
}

/// Where one of the core's own databases is in `scope`'s folder (`database_prefix`).
pub fn core_prefix(db: key::CoreDb, scope: &str) -> String {
    crate::names::database_prefix(scope, key::APP, db.name())
}

/// Where a database of an app's in a scope keeps its steps, copies and hint (`database::stored`).
pub fn database_prefix(scope: &str, app: &str, database: &str) -> String {
    let permanent = key::permanent(app, database);
    format!("{}/{}/{}", crate::folder(scope), permanent.app, permanent.database)
}

/// Where a blob named in `scope` is kept in the bucket, by its hash: under the scope's app runtime folder, sealed with the key
/// of the app runtime's database `blobs` there. A deploy's bundle and a release's binary are `global`'s.
pub fn blob_name(scope: &str, hash: &str) -> String {
    format!("{}/{hash}", crate::names::core_prefix(key::CoreDb::Blobs, scope))
}

/// Whether every step of the database at `prefix` is kept forever: a scope database's (`database::steps`), who's who
/// and the record of who did what. `tidy` deletes none of its steps, whoever asks.
pub fn keeps_every_step(prefix: &str) -> bool {
    let mut parts = prefix.rsplit('/');
    parts.next() == Some(key::CoreDb::Scope.name()) && parts.next() == Some(key::APP)
}
