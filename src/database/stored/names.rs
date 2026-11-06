//! Where a database's objects are in the bucket, by its prefix: its newest hint, its steps, the signatures beside older
//! ones, its copies, and the mark that its steps may have been tidied.

use super::keeps_every_step;

/// What says a database's steps may have been deleted (`tidy`): created before the first goes.
pub fn tidied_name(prefix: &str) -> String {
    format!("{prefix}/{}", crate::protocol::TIDIED)
}

/// A database's objects in the bucket, by its prefix: its newest hint, step `n`, and its copy after step `n`.
pub fn newest_name(prefix: &str) -> String {
    format!("{prefix}/{}", crate::protocol::NEWEST)
}

/// A step's position as its name says it.
pub(super) fn position_text(n: u64) -> String {
    format!("{n:0width$}", width = crate::protocol::STEP_DIGITS)
}

pub fn step_name(prefix: &str, n: u64) -> String {
    format!("{prefix}/{}", position_text(n))
}

pub fn copies_folder(prefix: &str) -> String {
    format!("{prefix}/{}", crate::protocol::COPY)
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
