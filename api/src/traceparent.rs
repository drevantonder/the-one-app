//! W3C's trace context, as every device, Worker and client names a trace and where a call failed in it.

/// W3C's ids: 16 random bytes for a trace and 8 for a span, as lowercase hex.
pub mod ids {
    /// A new W3C span id.
    pub fn span() -> String {
        data_encoding::HEXLOWER.encode(&rand::random::<[u8; 8]>())
    }

    /// A new W3C trace id.
    pub fn trace() -> String {
        data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>())
    }
}

/// `traceparent`'s trace and span, if it's W3C's and has them: what another device sent, so nothing else is kept.
pub fn parent_of(text: &str) -> Option<(String, String)> {
    let hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    let ["00", trace, span, _] = text.split('-').collect::<Vec<_>>()[..] else { return None };
    (hex(trace, 32) && hex(span, 16)).then(|| (trace.to_string(), span.to_string()))
}

/// Whether `trace` is one of those. By its id, so every span of a trace agrees, on any device.
pub fn sampled(trace: &str) -> bool {
    u64::from_str_radix(trace.get(trace.len().saturating_sub(8)..).unwrap_or(""), 16).is_ok_and(|n| n % custom_build::protocol::SAMPLED_ONE_IN == 0)
}

/// Where a call failed: the order it goes through them.
pub mod stage {
    /// Whether it can run at all: its scopes, whether its app's deleted, a lab that's off.
    pub const CHECK: &str = "check";
    /// The loop guard that stopped its app.
    pub const GUARD: &str = "guard";
    pub const KEYS: &str = "keys";
    pub const LEASE: &str = "lease";
    pub const RUN: &str = "run";
    pub const PREPARE: &str = "prepare";
    pub const WRITE: &str = "write";
    /// It took longer than it may.
    pub const LIMIT: &str = "limit";
    /// Going from device to device to find one that will run it.
    pub const ROUTE: &str = "route";
    pub const ALL: &[&str] = &[CHECK, GUARD, KEYS, LEASE, RUN, PREPARE, WRITE, LIMIT, ROUTE];
}

/// A device that didn't answer in time, as insights lists a failure.
pub const TIMEOUT: &str = "timeout";
/// A device that couldn't be reached, and one that went away while it was being asked.
pub const UNREACHABLE: &str = "unreachable";
