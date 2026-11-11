//! The names this build's programs run by, which every program that starts or finds another reads.

use crate::config::CLI;

pub fn oned() -> String {
    format!("{CLI}d")
}

/// The name onet runs by on a computer (`<cli>t`), which oned starts it as.
pub fn onet() -> String {
    format!("{CLI}t")
}

/// The name git runs this program by for `<CLI>::` addresses.
pub fn git_helper() -> String {
    format!("git-remote-{CLI}")
}
