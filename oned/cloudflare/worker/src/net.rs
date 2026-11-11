//! oned on Cloudflare's iroh endpoint (`Node` in worker.js, a Durable Object): the endpoint of its own key, as every
//! app runtime is, with no socket. The relay hands it the datagrams sent to its key, by RPC (`deliver`), and passes on what it
//! sends, so to every other endpoint it's one more behind the relay. It speaks what app runtimes speak (app-runtime/src/net.rs): to
//! client devices, HTTP/1.1 on a stream of its own per request (`<cli>/http/1`), answered by `worker.js` as an app runtime's
//! local API answers; to computers' app runtimes, an ask on a stream, its answer the same stream back (`<cli>/1`). Its
//! connections live as long as the object, which ends itself once nobody asks (worker.js), so it holds nothing while
//! asleep.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use iroh::endpoint::transports::{CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit};
use iroh::endpoint::{Connection, QuicTransportConfig, presets};
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMode, SecretKey, TransportAddr};
use iroh_base::CustomAddr;
use js_sys::{Function, Promise, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// This transport's kind of address: an endpoint's key, reached through the relay that hands this one its datagrams.
const THROUGH_RELAY: u64 = 0x6f6e;
/// The most a client's request may carry: a call and its input. A Worker has 128 MB.
const ASK_LIMIT: usize = 4 << 20;
const HEAD_LIMIT: usize = 64 << 10;
const ANSWER_LIMIT: usize = 16 << 20;
const STREAMS: u32 = 8;
/// How many connections others may have open to it at once, and how many of their asks it reads at once.
const CONNECTIONS: usize = 32;
const ASKS: usize = 16;
/// How many datagrams the relay may hand it before iroh reads them: more are dropped, as a full socket drops them.
const INBOX_MOST: usize = 4096;

fn alpn() -> Vec<u8> {
    format!("{}/1", custom_build::config::CLI).into_bytes()
}

/// What client devices speak to app runtimes.
fn http_alpn() -> Vec<u8> {
    format!("{}/http/1", custom_build::config::CLI).into_bytes()
}

fn through_relay(key: &[u8]) -> CustomAddr {
    CustomAddr::from_parts(THROUGH_RELAY, key)
}

/// Datagrams the relay handed in, until iroh reads them.
struct Inbox {
    packets: VecDeque<(CustomAddr, Vec<u8>)>,
    waker: Option<Waker>,
}

static INBOX: Mutex<Inbox> = Mutex::new(Inbox { packets: VecDeque::new(), waker: None });

thread_local! {
    /// The endpoint, what passes its datagrams to the relay, and the connections to other endpoints, by key.
    static NET: RefCell<Option<Net>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct Net {
    endpoint: Endpoint,
    conns: Rc<RefCell<HashMap<[u8; 32], Connection>>>,
    /// Connections others opened that are open now, and their asks being read or answered now.
    incoming: Rc<std::cell::Cell<usize>>,
    asks: Rc<std::cell::Cell<usize>>,
}

impl Net {
    /// Keeps `conn` as the one to `remote` until it closes, then forgets it, unless another replaced it.
    fn keep(&self, remote: [u8; 32], conn: &Connection) {
        self.conns.borrow_mut().insert(remote, conn.clone());
        let (conns, conn) = (self.conns.clone(), conn.clone());
        wasm_bindgen_futures::spawn_local(async move {
            conn.closed().await;
            let mut conns = conns.borrow_mut();
            if conns.get(&remote).is_some_and(|c| c.stable_id() == conn.stable_id()) {
                conns.remove(&remote);
            }
        });
    }
}

thread_local! {
    /// The object's `send(dst, segment size, contents)`, which passes datagrams this endpoint sends to the relay, and its
    /// `asked(key, method, path, headers, body)`, which answers a client's request: the object now running, as one that
    /// ended may leave the endpoint in the isolate for the next.
    static SEND: RefCell<Option<Function>> = const { RefCell::new(None) };
    static ASKED: RefCell<Option<Function>> = const { RefCell::new(None) };
}

#[derive(Debug)]
struct Relayed {
    me: CustomAddr,
    addrs: n0_watcher::Watchable<Vec<CustomAddr>>,
}

#[derive(Debug)]
struct Sender;

impl CustomTransport for Relayed {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        Ok(Box::new(Relayed { me: self.me.clone(), addrs: self.addrs.clone() }))
    }
}

impl CustomEndpoint for Relayed {
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<CustomAddr>> {
        self.addrs.watch()
    }

    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(Sender)
    }

    fn max_transmit_segments(&self) -> NonZeroUsize {
        NonZeroUsize::new(20).unwrap()
    }

    fn poll_recv(&mut self, cx: &mut Context, bufs: &mut [io::IoSliceMut<'_>], metas: &mut [noq_udp::RecvMeta], infos: &mut [RecvInfo]) -> Poll<io::Result<usize>> {
        let mut inbox = INBOX.lock().unwrap();
        let mut n = 0;
        while n < bufs.len() {
            let Some((from, data)) = inbox.packets.pop_front() else { break };
            if bufs[n].len() < data.len() {
                continue;
            }
            bufs[n][..data.len()].copy_from_slice(&data);
            metas[n].len = data.len();
            metas[n].stride = data.len();
            infos[n] = RecvInfo::new(from, Some(self.me.clone()));
            n += 1;
        }
        if n > 0 {
            return Poll::Ready(Ok(n));
        }
        inbox.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl CustomSender for Sender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        addr.id() == THROUGH_RELAY && addr.data().len() == 32
    }

    fn poll_send(&self, _cx: &mut Context, dst: &CustomAddr, _src: Option<&CustomAddr>, t: &Transmit<'_>) -> Poll<io::Result<()>> {
        SEND.with(|s| {
            if let Some(send) = s.borrow().as_ref() {
                let args = [Uint8Array::from(dst.data()).into(), JsValue::from(t.segment_size.unwrap_or(0) as u32), Uint8Array::from(t.contents).into()];
                let _ = send.apply(&JsValue::NULL, &args.iter().collect());
            }
        });
        Poll::Ready(Ok(()))
    }
}

fn thrown(err: impl std::fmt::Display) -> JsError {
    JsError::new(&err.to_string())
}

/// Starts the endpoint of the key `seed`: datagrams it sends go to `send(dst, segment size, contents)`, and each request
/// a client device opens a stream for goes to `asked(key (hex), method, path, headers (names and values, one after the
/// other), body (bytes))`, a promise of `{ status, headers, body }`, the same. → its key, hex.
#[wasm_bindgen]
pub async fn endpoint_start(seed: Vec<u8>, send: Function, asked: Function) -> Result<String, JsError> {
    SEND.with(|s| *s.borrow_mut() = Some(send));
    ASKED.with(|a| *a.borrow_mut() = Some(asked));
    if let Some(net) = NET.with(|n| n.borrow().clone()).filter(|n| !n.endpoint.is_closed()) {
        return Ok(data_encoding::HEXLOWER.encode(net.endpoint.id().as_bytes()));
    }
    let key = SecretKey::from_bytes(&seed.try_into().map_err(|_| JsError::new("the endpoint's key is 32 bytes"))?);
    let me = through_relay(key.public().as_bytes());
    let transport = QuicTransportConfig::builder().max_concurrent_bidi_streams(STREAMS.into()).max_concurrent_uni_streams(0u32.into()).build();
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![http_alpn()])
        .transport_config(transport)
        .relay_mode(RelayMode::Disabled)
        .add_custom_transport(Arc::new(Relayed { me: me.clone(), addrs: n0_watcher::Watchable::new(vec![me]) }))
        .bind()
        .await
        .map_err(thrown)?;
    let net = Net { endpoint: endpoint.clone(), conns: Default::default(), incoming: Default::default(), asks: Default::default() };
    NET.with(|n| *n.borrow_mut() = Some(net.clone()));
    let id = data_encoding::HEXLOWER.encode(endpoint.id().as_bytes());
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(incoming) = endpoint.accept().await {
            if net.incoming.get() >= CONNECTIONS {
                incoming.refuse();
                continue;
            }
            let net = net.clone();
            net.incoming.set(net.incoming.get() + 1);
            wasm_bindgen_futures::spawn_local(async move {
                let accepted = match incoming.accept() {
                    Ok(accepting) => accepting.await.ok(),
                    Err(_) => None,
                };
                // A client's connection is only ever its requests': what this app runtime asks machines goes on its own.
                if let Some(conn) = accepted {
                    let remote = *conn.remote_id().as_bytes();
                    answer(&net, conn, remote).await;
                }
                net.incoming.set(net.incoming.get() - 1);
            });
        }
    });
    Ok(id)
}

/// Answers each request on a connection a client device opened, each on its own stream, a few at once.
async fn answer(net: &Net, conn: Connection, remote: [u8; 32]) {
    let remote = data_encoding::HEXLOWER.encode(&remote);
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        if net.asks.get() >= ASKS {
            let _ = recv.stop(1u32.into());
            let _ = send.reset(1u32.into());
            continue;
        }
        net.asks.set(net.asks.get() + 1);
        let (remote, asks) = (remote.clone(), net.asks.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let _done = Done(asks);
            let response = match request(&mut recv).await {
                Ok((method, path, headers, body)) => respond(&remote, &method, &path, &headers, &body).await,
                Err(why) => reply(400, &[("content-type".into(), "application/json".into())], json(&why).as_bytes()),
            };
            let _ = send.write_all(&response).await;
            let _ = send.finish();
        });
    }
}

/// A request read off a stream: its method, path, headers and body (as long as its length says, and no longer than
/// `ASK_LIMIT`).
async fn request(recv: &mut iroh::endpoint::RecvStream) -> Result<(String, String, Vec<(String, String)>, Vec<u8>), String> {
    let mut buf = Vec::new();
    loop {
        let mut list = [httparse::EMPTY_HEADER; 64];
        let mut parsed = httparse::Request::new(&mut list);
        match parsed.parse(&buf).map_err(|e| format!("that isn't a request: {e}"))? {
            httparse::Status::Complete(len) => {
                let headers: Vec<(String, String)> = parsed.headers.iter().map(|h| (h.name.to_ascii_lowercase(), String::from_utf8_lossy(h.value).into_owned())).collect();
                let length: usize = headers.iter().find(|h| h.0 == "content-length").map_or(Ok(0), |h| h.1.trim().parse()).map_err(|_| "a request's length is a number".to_string())?;
                if length > ASK_LIMIT {
                    return Err(format!("a request sends at most {} MiB", ASK_LIMIT >> 20));
                }
                let (method, path) = (parsed.method.unwrap_or("").to_string(), parsed.path.unwrap_or("").to_string());
                let mut body = buf.split_off(len);
                while body.len() < length {
                    match recv.read_chunk(length - body.len()).await.map_err(|e| e.to_string())? {
                        Some(chunk) => body.extend_from_slice(&chunk),
                        None => return Err("the request ended early".into()),
                    }
                }
                body.truncate(length);
                return Ok((method, path, headers, body));
            }
            httparse::Status::Partial if buf.len() > HEAD_LIMIT => return Err("the request's head is too long".into()),
            httparse::Status::Partial => match recv.read_chunk(HEAD_LIMIT).await.map_err(|e| e.to_string())? {
                Some(chunk) => buf.extend_from_slice(&chunk),
                None => return Err("the stream ended before a request".into()),
            },
        }
    }
}

/// What `worker.js` answers a request, as an HTTP/1.1 response.
async fn respond(remote: &str, method: &str, path: &str, headers: &[(String, String)], body: &[u8]) -> Vec<u8> {
    let failed = |why: String| reply(503, &[("content-type".into(), "application/json".into())], json(&why).as_bytes());
    let Some(asked) = ASKED.with(|a| a.borrow().clone()) else { return failed("oned on Cloudflare isn't started".into()) };
    let flat = js_sys::Array::new();
    for (name, value) in headers {
        flat.push(&name.into());
        flat.push(&value.into());
    }
    let args = js_sys::Array::of5(&remote.into(), &method.into(), &path.into(), &flat, &Uint8Array::from(body));
    let promise = match asked.apply(&JsValue::NULL, &args) {
        Ok(p) => Promise::from(p),
        Err(err) => return failed(format!("{err:?}")),
    };
    let out = match JsFuture::from(promise).await {
        Ok(out) => out,
        Err(err) => return failed(js_sys::Error::from(err).message().into()),
    };
    let get = |k: &str| js_sys::Reflect::get(&out, &k.into()).unwrap_or(JsValue::UNDEFINED);
    let status = get("status").as_f64().unwrap_or(500.0) as u16;
    let list: Vec<String> = js_sys::Array::from(&get("headers")).iter().filter_map(|v| v.as_string()).collect();
    let headers: Vec<(String, String)> = list.chunks(2).filter(|p| p.len() == 2).map(|p| (p[0].clone(), p[1].clone())).collect();
    reply(status, &headers, &Uint8Array::new(&get("body")).to_vec())
}

/// An HTTP/1.1 response: nothing a header says can start another.
fn reply(status: u16, headers: &[(String, String)], body: &[u8]) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", if status < 400 { "OK" } else { "Refused" });
    for (name, value) in headers {
        let framing = ["content-length", "transfer-encoding", "connection"].contains(&name.to_ascii_lowercase().as_str());
        if framing || name.bytes().chain(value.bytes()).any(|b| b == b'\r' || b == b'\n' || b == 0) {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    out
}

fn json(why: &str) -> String {
    serde_json::json!({ "error": why }).to_string()
}

/// Counts an ask as answered once it's dropped, however it ended.
struct Done(Rc<std::cell::Cell<usize>>);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

/// Closes the endpoint, telling those connected to it that its connections are gone, so their next ask makes a new
/// one at once; the object ends next.
#[wasm_bindgen]
pub async fn endpoint_close() {
    let Some(net) = NET.with(|n| n.borrow_mut().take()) else { return };
    net.conns.borrow_mut().clear();
    let _ = n0_future::time::timeout(Duration::from_secs(1), net.endpoint.close()).await;
    INBOX.lock().unwrap().packets.clear();
}

/// Datagrams the relay was given for this endpoint, from `src`: one, or `segment_size` long each, back to back.
#[wasm_bindgen]
pub fn endpoint_deliver(src: &[u8], segment_size: u32, contents: &[u8]) {
    let from = through_relay(src);
    let mut inbox = INBOX.lock().unwrap();
    let step = if segment_size == 0 { contents.len() } else { segment_size as usize };
    for chunk in contents.chunks(step.max(1)) {
        if inbox.packets.len() >= INBOX_MOST {
            break;
        }
        inbox.packets.push_back((from.clone(), chunk.to_vec()));
    }
    if let Some(waker) = inbox.waker.take() {
        waker.wake();
    }
}

/// Sends the endpoint `to` (hex) `message`, on the connection open to it or a new one made within `connect_ms`: → its
/// answer within `limit_ms`, and how long making the connection took (ms), 0 if it was open. Throws with
/// `unreached: true` if it never reached it.
#[wasm_bindgen]
pub async fn endpoint_ask(to: String, message: String, connect_ms: u32, limit_ms: u32) -> Result<JsValue, JsValue> {
    let unreached = |why: String| -> JsValue {
        let err = js_sys::Error::new(&why);
        let _ = js_sys::Reflect::set(&err, &"unreached".into(), &true.into());
        err.into()
    };
    let net = NET.with(|n| n.borrow().clone()).ok_or_else(|| unreached("the endpoint isn't started".into()))?;
    let key: [u8; 32] = data_encoding::HEXLOWER.decode(to.as_bytes()).ok().and_then(|k| k.try_into().ok()).ok_or_else(|| unreached(format!("{to} isn't a key")))?;
    let mut fresh = false;
    let mut made = 0.0;
    let (mut send, mut recv) = loop {
        let open = net.conns.borrow().get(&key).filter(|c| !fresh && c.close_reason().is_none()).cloned();
        let conn = match open {
            Some(conn) => conn,
            None => {
                let id = PublicKey::from_bytes(&key).map_err(|e| unreached(e.to_string()))?;
                let started = js_sys::Date::now();
                let addr = EndpointAddr::from_parts(id, [TransportAddr::Custom(through_relay(&key))]);
                let conn = match n0_future::time::timeout(Duration::from_millis(connect_ms.into()), net.endpoint.connect(addr, &alpn())).await {
                    Ok(Ok(conn)) => conn,
                    Ok(Err(err)) => return Err(unreached(format!("couldn't be reached: {err}"))),
                    Err(_) => return Err(unreached("isn't connected".into())),
                };
                made = js_sys::Date::now() - started;
                fresh = true;
                net.keep(key, &conn);
                conn
            }
        };
        match conn.open_bi().await {
            Ok(stream) => break stream,
            Err(_) if !fresh => fresh = true,
            Err(err) => return Err(unreached(format!("couldn't be reached: {err}"))),
        }
    };
    let sent = async {
        send.write_all(message.as_bytes()).await?;
        send.finish()?;
        anyhow::Ok(())
    };
    sent.await.map_err(|e| unreached(format!("couldn't be reached: {e}")))?;
    let answer = match n0_future::time::timeout(Duration::from_millis(limit_ms.into()), recv.read_to_end(ANSWER_LIMIT)).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(err)) => return Err(js_sys::Error::new(&format!("its answer didn't come: {err}")).into()),
        Err(_) => return Err(js_sys::Error::new("it didn't answer in time").into()),
    };
    let out = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&out, &"answer".into(), &String::from_utf8_lossy(&answer).as_ref().into());
    let _ = js_sys::Reflect::set(&out, &"connect".into(), &made.into());
    Ok(out.into())
}
