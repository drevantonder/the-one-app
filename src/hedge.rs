//! Requests from a device or node to Cloudflare (the bucket, and the Workers at the network's address), routed around a
//! connection that has gone quiet. Home routers' NAT sometimes kills a connection, mixing up two devices behind it that
//! used the same source port; the reset that says so comes after half a minute, or in a container behind one, never.
//! So a request waits for the first byte of its answer only as long as that host's answers usually take, its limit,
//! learned as TCP learns its retransmission timeout (RFC 6298). Past it, a request that comes to the same if the server
//! gets it twice is sent again on a new connection and whichever answers first is taken; one that mustn't fails then,
//! and its connection is dropped. How often a limit passes, and which side won, is counted for insights (`take_late`).
//! The same limit is learned of anything else a first byte is waited for by name (`limit`, `learn`): another device's
//! "taken" (src/node/calls.rs), and a node's first answer to a client (onet).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;
#[cfg(not(target_family = "wasm"))]
use std::{
    io::Read,
    sync::{Arc, mpsc},
    time::Instant,
};

#[cfg(not(target_family = "wasm"))]
use ureq::http::{Request, Response, request::Parts};
#[cfg(not(target_family = "wasm"))]
use ureq::{Agent, Body, SendBody};

/// A limit is never under this, so jitter alone doesn't send a healthy request twice, nor over this, so a dead
/// connection never holds a request long.
const LEAST: Duration = Duration::from_millis(250);
const MOST: Duration = Duration::from_secs(5);
#[cfg(not(target_family = "wasm"))]
/// The least a request never sent twice waits: failing a healthy write costs far more than sending a read twice.
const NEVER_LEAST: Duration = Duration::from_secs(2);
/// A host's limit before any answer of its has been timed.
const FIRST: Duration = Duration::from_secs(3);

/// Whether a request may reach the server twice: a read, or a write of a name bound to its bytes, comes to the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Twice {
    Safe,
    Never,
}

/// What's learned of the time to a host's first byte: its smoothed mean and deviation, in ms.
#[derive(Clone, Copy, Debug, Default)]
struct Estimate {
    mean: f64,
    deviation: f64,
    timed: bool,
}

impl Estimate {
    fn learn(&mut self, took: Duration) {
        let ms = took.as_secs_f64() * 1000.0;
        if !self.timed {
            (self.mean, self.deviation, self.timed) = (ms, ms / 2.0, true);
            return;
        }
        self.deviation = 0.75 * self.deviation + 0.25 * (self.mean - ms).abs();
        self.mean = 0.875 * self.mean + 0.125 * ms;
    }

    fn limit(&self) -> Duration {
        match self.timed {
            false => FIRST,
            true => Duration::from_secs_f64((self.mean + 4.0 * self.deviation) / 1000.0).clamp(LEAST, MOST),
        }
    }
}

/// Each name's estimate: a host's by `<host> <reached> <method>`, as a write's answer comes later than a read's.
static ESTIMATES: Mutex<BTreeMap<String, Estimate>> = Mutex::new(BTreeMap::new());

/// How long the first byte of what `name` answers is waited for before it's taken as stalled.
pub fn limit(name: &str) -> Duration {
    ESTIMATES.lock().unwrap().get(name).copied().unwrap_or_default().limit()
}

/// Learns how long the first byte of what `name` answered took.
pub fn learn(name: &str, took: Duration) {
    ESTIMATES.lock().unwrap().entry(name.to_string()).or_default().learn(took);
}
#[cfg(not(target_family = "wasm"))]
/// How many requests passed their limit since `take_late` last read them, by what they reached and how each ended.
static LATE: Mutex<BTreeMap<(&'static str, &'static str), u64>> = Mutex::new(BTreeMap::new());

#[cfg(not(target_family = "wasm"))]
/// Requests that passed their limit since this last asked, for insights: what each reached, how it ended (`original`,
/// the first connection answered after all; `hedge`, the new one did; `neither`; or `failed`, one never sent twice),
/// and how many.
pub fn take_late() -> Vec<(&'static str, &'static str, u64)> {
    std::mem::take(&mut *LATE.lock().unwrap()).into_iter().map(|((reached, ended), n)| (reached, ended, n)).collect()
}

#[cfg(not(target_family = "wasm"))]
fn late(reached: &'static str, ended: &'static str) {
    *LATE.lock().unwrap().entry((reached, ended)).or_default() += 1;
}

#[cfg(not(target_family = "wasm"))]
/// Sends `request` with `agent`, as `reached` says what it reaches for insights (`bucket`, or a Worker's name): → its
/// answer, once its first byte came, and whether it was sent twice.
pub fn run(agent: &Agent, reached: &'static str, request: Request<Vec<u8>>, twice: Twice) -> Result<(Response<Body>, bool), ureq::Error> {
    let (parts, body) = request.into_parts();
    let key = format!("{} {reached} {}", parts.uri.authority().map(|a| a.as_str()).unwrap_or_default(), parts.method);
    let limit = limit(&key);
    let learn = |took| learn(&key, took);
    match twice {
        Twice::Never => never(agent, reached, parts, body, limit.max(NEVER_LEAST), learn).map(|r| (r, false)),
        Twice::Safe => safe(agent, reached, parts, Arc::new(body), limit, learn),
    }
}

#[cfg(not(target_family = "wasm"))]
/// One that mustn't reach the server twice: its connection is dropped once its limit passes, and the next request
/// takes a new one.
fn never(agent: &Agent, reached: &'static str, parts: Parts, body: Vec<u8>, limit: Duration, learn: impl Fn(Duration)) -> Result<Response<Body>, ureq::Error> {
    let moved = Arc::new(Mutex::new(Instant::now()));
    let request = build(&parts, Arc::new(body), {
        let moved = moved.clone();
        move || *moved.lock().unwrap() = Instant::now()
    });
    let request = agent.configure_request(request).timeout_connect(Some(limit)).timeout_recv_response(Some(limit)).build();
    match agent.run(request) {
        Ok(response) => {
            learn(moved.lock().unwrap().elapsed());
            Ok(response)
        }
        Err(ureq::Error::Timeout(ureq::Timeout::Connect | ureq::Timeout::RecvResponse)) => {
            late(reached, "failed");
            Err(ureq::Error::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("no answer began within {} ms, as the {reached}'s answers usually do, so its connection was taken for dead and dropped: try again, on a new one", limit.as_millis()))))
        }
        Err(err) => Err(err),
    }
}

#[cfg(not(target_family = "wasm"))]
enum Event {
    /// A side's request moved on: it started, or more of its body went.
    Moved(usize),
    Answered(usize, Result<Response<Body>, ureq::Error>),
}

#[cfg(not(target_family = "wasm"))]
/// One that may reach the server twice: sent again on a new connection once its limit passes after it last moved,
/// and whichever answers first taken. The other is left to end by its agent's own timeout.
fn safe(agent: &Agent, reached: &'static str, parts: Parts, body: Arc<Vec<u8>>, limit: Duration, learn: impl Fn(Duration)) -> Result<(Response<Body>, bool), ureq::Error> {
    let (tx, rx) = mpsc::channel();
    let start = |side: usize, agent: Agent| {
        let (tx, parts, body) = (tx.clone(), parts.clone(), body.clone());
        std::thread::spawn(move || {
            let moved = tx.clone();
            let request = build(&parts, body, move || drop(moved.send(Event::Moved(side))));
            let _ = tx.send(Event::Answered(side, agent.run(request)));
        });
    };
    start(0, agent.clone());
    let (mut moved, mut hedged, mut waiting) = ([Instant::now(); 2], false, 1);
    loop {
        let event = match hedged {
            false => rx.recv_timeout(limit.saturating_sub(moved[0].elapsed())),
            true => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match event {
            Ok(Event::Moved(side)) => moved[side] = Instant::now(),
            Ok(Event::Answered(side, Ok(response))) => {
                learn(moved[side].elapsed());
                if hedged {
                    late(reached, ["original", "hedge"][side]);
                }
                return Ok((response, hedged));
            }
            Ok(Event::Answered(side, Err(err @ ureq::Error::StatusCode(_)))) => {
                learn(moved[side].elapsed());
                return Err(err);
            }
            Ok(Event::Answered(_, Err(err))) => {
                waiting -= 1;
                if waiting == 0 {
                    if hedged {
                        late(reached, "neither");
                    }
                    return Err(err);
                }
            }
            // Home routers' NAT can kill a connection without a word (see the top): a new one routes around it.
            Err(mpsc::RecvTimeoutError::Timeout) => {
                (hedged, waiting, moved[1]) = (true, waiting + 1, Instant::now());
                start(1, Agent::new_with_config(agent.config().clone()));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => unreachable!("this holds a sender while it waits"),
        }
    }
}

#[cfg(not(target_family = "wasm"))]
/// `parts` with `body`, calling `moved` as it starts and as its body goes: a GET or HEAD has none, anything else says
/// its length, as S3 wants.
fn build(parts: &Parts, body: Arc<Vec<u8>>, mut moved: impl FnMut() + Send + 'static) -> Request<SendBody<'static>> {
    moved();
    let mut request = Request::from_parts(parts.clone(), ());
    let sent = match parts.method.as_str() {
        "GET" | "HEAD" => SendBody::none(),
        _ => {
            request.headers_mut().insert("content-length", body.len().into());
            SendBody::from_owned_reader(Watched { body, at: 0, moved: Box::new(moved) })
        }
    };
    request.map(|()| sent)
}

#[cfg(not(target_family = "wasm"))]
/// A body as it's read to be sent, saying each time more goes.
struct Watched {
    body: Arc<Vec<u8>>,
    at: usize,
    moved: Box<dyn FnMut() + Send>,
}

#[cfg(not(target_family = "wasm"))]
impl Read for Watched {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = (&self.body[self.at..]).read(buf)?;
        self.at += n;
        (self.moved)();
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_is_the_mean_and_four_deviations_learned_as_tcp_does_within_its_bounds() {
        let mut e = Estimate::default();
        assert_eq!(e.limit(), FIRST);
        e.learn(Duration::from_millis(100));
        assert_eq!((e.mean, e.deviation), (100.0, 50.0));
        assert_eq!(e.limit(), Duration::from_millis(300));
        e.learn(Duration::from_millis(20));
        assert_eq!((e.mean, e.deviation), (90.0, 57.5));
        assert_eq!(e.limit(), Duration::from_millis(320));
        for _ in 0..100 {
            e.learn(Duration::from_millis(20));
        }
        assert_eq!(e.limit(), LEAST);
        for _ in 0..100 {
            e.learn(Duration::from_secs(6));
        }
        assert_eq!(e.limit(), MOST);
    }

    /// A server that accepts a connection and never answers, as one a home router's NAT killed: → its address, and how
    /// many requests reached it.
    fn silent() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let at = format!("http://{}", listener.local_addr().unwrap());
        let reached = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = reached.clone();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for conn in listener.incoming().flatten() {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                held.push(conn);
            }
        });
        (at, reached)
    }

    #[test]
    fn a_write_that_mustnt_land_twice_is_never_sent_again_and_fails_at_its_limit() {
        let (at, reached) = silent();
        let agent: Agent = Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into();
        let request = Request::builder().method("PUT").uri(format!("{at}/lease")).body(b"mine".to_vec()).unwrap();
        let asked = Instant::now();
        let err = run(&agent, "bucket", request, Twice::Never).unwrap_err();
        assert!(asked.elapsed() < FIRST.max(NEVER_LEAST) + Duration::from_secs(1), "it waited {:?}", asked.elapsed());
        assert!(err.to_string().contains("taken for dead"), "{err}");
        assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
