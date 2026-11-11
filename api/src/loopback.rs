//! A computer's own programs ask its onet on its loopback, and onet asks its app runtime there, and oned starts both
//! again whenever it starts again itself (an update): for those few seconds each refuses connections, and nothing says
//! when it's back. So whatever asks one asks again for a moment while it's refused (`while_refused`), whatever it asks,
//! as a refused request was never sent. One sent before it stopped (`lost`) may have been done: only one that only reads
//! is asked again (`reads`), and anything else says it failed (`lost_write`), and is never sent again.

use std::time::{Duration, Instant};

use serde_json::Value;

/// How long a refused request is asked again: oned has onet and its app runtime back within seconds of stopping them.
pub const BACK_WITHIN: Duration = Duration::from_secs(15);

/// How long it waits between asks.
pub const EVERY: Duration = Duration::from_millis(250);

/// Nothing is listening where it asked, so nothing was sent.
pub fn refused(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::ConnectionRefused
}

/// Whether a request of this program's waited out `BACK_WITHIN` and was still refused: then nothing is coming back, and
/// what it asks next isn't waited for again, until one is answered.
static GAVE_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `send`, again while it's refused, for up to `BACK_WITHIN`: then whatever it gave.
pub fn while_refused<T>(mut send: impl FnMut() -> Result<T, ureq::Error>) -> Result<T, ureq::Error> {
    use std::sync::atomic::Ordering::Relaxed;
    let until = Instant::now() + BACK_WITHIN;
    loop {
        match send() {
            Err(ureq::Error::Io(err)) if refused(&err) && !GAVE_UP.load(Relaxed) => match Instant::now() < until {
                true => std::thread::sleep(EVERY),
                false => {
                    GAVE_UP.store(true, Relaxed);
                    return Err(ureq::Error::Io(err));
                }
            },
            Err(ureq::Error::Io(err)) if refused(&err) => return Err(ureq::Error::Io(err)),
            done => {
                GAVE_UP.store(false, Relaxed);
                return done;
            }
        }
    }
}

/// What it asked had been sent when the program answering it stopped.
pub fn lost(err: &ureq::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(err, ureq::Error::Io(e) if matches!(e.kind(), UnexpectedEof | ConnectionReset | ConnectionAborted | BrokenPipe))
}

/// Whether `method` of `path` with `body` only reads: a GET or HEAD, or a call to a tool the apps' list (`/api/apps`, which
/// `apps` asks, only for a call) says only reads. An app runtime just started lists an app's tools only once it has
/// loaded it, and nothing says when, so it's asked again till then, for up to `BACK_WITHIN`.
pub fn reads(method: &str, path: &str, body: &[u8], mut apps: impl FnMut() -> Option<Value>) -> bool {
    if matches!(method, "GET" | "HEAD") {
        return true;
    }
    if path.split('?').next() != Some(crate::path::CALL) {
        return false;
    }
    let Ok(call) = serde_json::from_slice::<Value>(body) else { return false };
    let until = Instant::now() + BACK_WITHIN;
    loop {
        let listed = apps().and_then(|apps| apps.as_array()?.iter().find(|a| a["name"] == call["app"])?.get("tools")?.as_array().cloned());
        match listed {
            Some(tools) => return tools.iter().any(|t| t["name"] == call["tool"] && t["reads"] == true),
            None if Instant::now() < until => std::thread::sleep(EVERY),
            None => return false,
        }
    }
}

/// What a request that doesn't only read, lost as its onet stopped, says.
pub fn lost_write() -> String {
    format!("This computer's {}t stopped before it answered, as it does when it updates, so that may or may not have been done: check before asking again.", custom_build::config::CLI)
}
