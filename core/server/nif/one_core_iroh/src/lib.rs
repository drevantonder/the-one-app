//! The server's NIF for being reached by key: it opens the server's endpoint, hands what clients ask to the
//! server's Elixir with the key of whoever asked, and sends its answers back. A client asks one message a stream
//! (`client`), and is told something the same way: one message on a stream the server opens, over the connection
//! the client has open if it has one, and otherwise over one the server makes to it. A server asks another as a
//! client does, and asks a device that's watching the same way, over a connection it makes to the run watching.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use client::{Network, RequestError};
use data_encoding::HEXLOWER;
use iroh::endpoint::{Connection, VarInt};
use iroh::{Endpoint, EndpointAddr};
use rustler::{Atom, Binary, Encoder, Env, LocalPid, NewBinary, OwnedEnv, Resource, ResourceArc, Term};
use tokio::runtime::Runtime;
use tokio::sync::{RwLock, oneshot};
use tokio::time::{Instant, MissedTickBehavior, interval_at};

mod outside;

mod atoms {
    rustler::atoms! { ok, ask, closed, connected, gone, stopped, unreachable }
}

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| Runtime::new().expect("tokio starts"));

/// A server's endpoint, open for as long as the server holds this, with the clients that have a connection open to
/// it and the ones watching, by their keys, and its connections to other servers by theirs, each with the address
/// it was made to. Where the ones watching are is kept in the file `watching` beside its key too, so a server that
/// starts again still tells them what changes. A server found gone leaves that address with no connection, and a
/// connection that closed for want of anything to ask leaves nothing. `asks` is what's being answered (`Asks`).
/// Nothing looks inside it after a panic, which is what Rustler needs to hear before it hands one to Elixir.
struct Reached {
    endpoint: AssertUnwindSafe<Endpoint>,
    clients: AssertUnwindSafe<Clients>,
    watchers: AssertUnwindSafe<Mutex<HashMap<String, Watcher>>>,
    watching: AssertUnwindSafe<PathBuf>,
    servers: AssertUnwindSafe<Servers>,
    asks: AssertUnwindSafe<Asks>,
}

/// What's being answered: each message holds it for reading while it is, and a server that's stopping holds it for
/// writing, so it takes nothing new.
type Asks = Arc<RwLock<()>>;

impl Reached {
    /// Keeps where the clients that are watching are, and the device each runs for, as `watchers` has them now.
    fn keep(&self, watchers: &HashMap<String, Watcher>) {
        let stored: HashMap<&String, serde_json::Value> = watchers
            .iter()
            .map(|(key, watcher)| {
                let mut stored = serde_json::to_value(&watcher.address).unwrap_or_default();
                stored["device"] = watcher.device.clone().into();
                (key, stored)
            })
            .collect();
        if let Ok(stored) = serde_json::to_vec(&stored) {
            let _ = std::fs::write(&*self.watching, stored);
        }
    }
}

/// The connection each client last made to this server, by the client's key.
type Clients = Arc<Mutex<HashMap<String, Connection>>>;

/// This server's connection to each other server, by its key, with the address it was made to.
type Servers = Mutex<HashMap<String, (String, Option<Arc<client::Reached>>)>>;

/// A client that wants to hear what changes: where it said a server reaches it, the device it runs for, if it proved
/// one, and the connection this server made to it there to tell it something or ask it, while that's open. One thing
/// at a time is said over it, so that many things to say at once make one connection.
#[derive(Clone)]
struct Watcher {
    address: EndpointAddr,
    device: Option<String>,
    open: Arc<tokio::sync::Mutex<Option<Arc<client::Reached>>>>,
}

impl Watcher {
    /// The connection to this client, made again if it isn't open.
    async fn reached(&self, endpoint: &Endpoint) -> Option<Arc<client::Reached>> {
        let mut open = self.open.lock().await;
        if !open.as_ref().is_some_and(|open| open.open()) {
            *open = client::Reached::connect(endpoint, self.address.clone(), client::CONNECT).await.ok().map(Arc::new);
        }
        open.clone()
    }
}

#[rustler::resource_impl]
impl Resource for Reached {}

impl Drop for Reached {
    fn drop(&mut self) {
        let endpoint = self.endpoint.clone();
        RUNTIME.spawn(async move { endpoint.close().await });
    }
}

/// One message asked, holding the way its answer goes back, and what the call it asked for says it's doing, which its
/// progress notifications carry (`stage`). Dropped unanswered, whoever asked is told nothing came.
struct Ask {
    answer: Mutex<Option<oneshot::Sender<Vec<u8>>>>,
    stage: Arc<Mutex<Option<String>>>,
}

#[rustler::resource_impl]
impl Resource for Ask {}

/// The key kept in `key_file`, made on first use, as others know it.
#[rustler::nif(schedule = "DirtyIo")]
fn key(key_file: String) -> Result<String, String> {
    let key = client::key(Path::new(&key_file)).map_err(|error| format!("{error:#}"))?;
    Ok(key.public().to_string())
}

/// The signature of `message` by the key kept in `key_file`.
#[rustler::nif(schedule = "DirtyIo")]
fn sign(key_file: String, message: Binary) -> Result<String, String> {
    let key = client::key(Path::new(&key_file)).map_err(|error| format!("{error:#}"))?;
    Ok(HEXLOWER.encode(&key.sign(message.as_slice()).to_bytes()))
}

/// `data` encrypted to the machine whose key is `to`, for what `about` says (`client::sealed`).
#[rustler::nif]
fn seal<'a>(env: Env<'a>, to: &str, about: &str, data: Binary) -> Result<Binary<'a>, String> {
    let to: iroh::PublicKey = to.parse().map_err(|_| format!("{to} isn't a key"))?;
    let sealed = client::sealed::seal(&to, about, data.as_slice()).map_err(|error| format!("{error:#}"))?;
    let mut binary = NewBinary::new(env, sealed.len());
    binary.as_mut_slice().copy_from_slice(&sealed);
    Ok(binary.into())
}

/// What `one serve run` told the server in `line`, read as `client::serve::Told` and given as the JSON it's read as,
/// or why it's not one.
#[rustler::nif]
fn told(line: &str) -> Result<String, String> {
    Ok(serde_json::from_str::<client::serve::Told>(line).map_err(|error| error.to_string())?.to_string())
}

/// The line the server says `message` in, the JSON of a `client::serve::Said`, which `one serve run` reads, or why it
/// isn't one.
#[rustler::nif]
fn said(message: &str) -> Result<String, String> {
    Ok(serde_json::from_str::<client::serve::Said>(message).map_err(|error| error.to_string())?.to_string())
}

/// Opens the endpoint whose key is kept in `key_file`, and sends each message asked to `asked` as
/// `{:ask, ask, from, message}`, where `from` is the key of whoever asked. With a `network`, its address and the token
/// shown there, it's reached through the relay at that address too, and hears what outsiders ask there, whose `from`
/// is `nil`. It gives the endpoint once it's connected there, since until then the network's other servers know it as
/// off and don't pass it what they're asked: its address, as the JSON a client is told it in, and the endpoint.
#[rustler::nif(schedule = "DirtyIo")]
fn start(
    key_file: String,
    network: Option<(String, String)>,
    handler: LocalPid,
) -> Result<(ResourceArc<Reached>, String), String> {
    RUNTIME.block_on(async {
        let network = network.map(|(address, token)| Network { address, token });
        let key = client::key(Path::new(&key_file)).map_err(|error| format!("{error:#}"))?;
        // It listens on the port it did before it started again, so a connection another server still holds from
        // its last run reaches it: iroh sends what it has for a machine only where a connection it still has with it
        // last was, and one a server left behind when it stopped lasts until it times out. Another program may have
        // taken the port meanwhile, and then it's any that's free.
        let port_file = Path::new(&key_file).with_file_name("port");
        let port = std::fs::read_to_string(&port_file).ok().and_then(|port| port.trim().parse().ok()).unwrap_or(0);
        let endpoint = match client::endpoint(key.clone(), network.as_ref(), port).await {
            Err(_) if port != 0 => client::endpoint(key, network.as_ref(), 0).await,
            bound => bound,
        }
        .map_err(|error| format!("{error:#}"))?;
        if let Some(socket) = endpoint.bound_sockets().into_iter().find(SocketAddr::is_ipv4) {
            std::fs::write(&port_file, socket.port().to_string()).map_err(|error| error.to_string())?;
        }
        let address = client::address(&endpoint, network.as_ref()).map_err(|error| format!("{error:#}"))?;
        if let Some(network) = network {
            endpoint.online().await;
            // It listens there while the endpoint is open: one closed, as the server closes it as it stops
            // (`OneCore.Server`), leaves the relay to the next.
            let listen = outside::listen(network, endpoint.secret_key().clone(), handler);
            tokio::spawn(endpoint.closed().run_until(listen));
        }
        let address = serde_json::to_string(&address).map_err(|error| error.to_string())?;
        let clients = Clients::default();
        let asks = Asks::default();
        tokio::spawn(accept(endpoint.clone(), handler, clients.clone(), asks.clone()));
        let watching = Path::new(&key_file).with_file_name("watching");
        let stored = std::fs::read(&watching).ok();
        let stored: HashMap<String, serde_json::Value> =
            stored.and_then(|stored| serde_json::from_slice(&stored).ok()).unwrap_or_default();
        let watchers = stored
            .into_iter()
            .filter_map(|(key, stored)| {
                let device = stored["device"].as_str().map(str::to_owned);
                let address = serde_json::from_value(stored).ok()?;
                Some((key, Watcher { address, device, open: Arc::default() }))
            })
            .collect();
        let reached = Reached {
            endpoint: AssertUnwindSafe(endpoint),
            clients: AssertUnwindSafe(clients),
            watchers: AssertUnwindSafe(Mutex::new(watchers)),
            watching: AssertUnwindSafe(watching),
            servers: AssertUnwindSafe(Mutex::default()),
            asks: AssertUnwindSafe(asks),
        };
        Ok((ResourceArc::new(reached), address))
    })
}

/// Hands `message` to the server's Elixir, with the key of whoever asked it if they have one, and waits for its
/// answer. There's none if the Elixir is gone: `REFUSED` if it was before it took the message, and `STOPPED` if it
/// went after, so it may have done what was asked.
async fn ask(
    handler: &LocalPid,
    from: Option<&str>,
    message: &[u8],
    stage: Arc<Mutex<Option<String>>>,
) -> Result<Vec<u8>, VarInt> {
    let (reply_sender, reply_receiver) = oneshot::channel();
    let ask = ResourceArc::new(Ask { answer: Mutex::new(Some(reply_sender)), stage });
    let sent = OwnedEnv::new().send_and_clear(handler, |env| {
        let mut binary = NewBinary::new(env, message.len());
        binary.as_mut_slice().copy_from_slice(message);
        (atoms::ask(), ask, from, Binary::from(binary))
    });
    sent.map_err(|_| client::REFUSED)?;
    reply_receiver.await.map_err(|_| client::STOPPED)
}

async fn accept(endpoint: Endpoint, handler: LocalPid, clients: Clients, asks: Asks) {
    while let Some(incoming) = endpoint.accept().await {
        // A server that's stopping takes no new connection, so whoever connects reaches another.
        if asks.try_read().is_err() {
            incoming.refuse();
            continue;
        }
        let (clients, asks) = (clients.clone(), asks.clone());
        tokio::spawn(async move {
            let Ok(connection) = incoming.await else { return };
            let from = connection.remote_id().to_string();
            clients.lock().expect("no connection panics").insert(from.clone(), connection.clone());
            while let Ok((mut send, mut receive)) = connection.accept_bi().await {
                let (from, asks) = (from.clone(), asks.clone());
                tokio::spawn(async move {
                    let Ok(message) = receive.read_to_end(client::LIMIT).await else { return };
                    // Whoever asks nothing only asks whether this server is there, which this says, while it's
                    // stopping too, as it answers what it took already.
                    if message.is_empty() {
                        let _ = send.finish();
                        return;
                    }
                    // A stream ended unanswered is reset, so whoever asked knows it wasn't answered, and whether it
                    // was taken.
                    let Ok(_answering) = asks.try_read_owned() else { return drop(send.reset(client::REFUSED)) };
                    let stage = Arc::new(Mutex::new(None));
                    let response = ask(&handler, Some(&from), &message, stage.clone());
                    tokio::pin!(response);
                    // While it runs, whoever asked with a progress token is told so every `THERE`, which is how
                    // they know it's still running, however long it takes (`client::Reached::ask`). How far it
                    // is, is the seconds it's run, since nothing says how far a call is, and what it's doing is
                    // what the call last said (`stage`).
                    let token = client::progress_token(&message);
                    let started = Instant::now();
                    let mut every = interval_at(started + client::THERE, client::THERE);
                    every.set_missed_tick_behavior(MissedTickBehavior::Delay);
                    // Whoever asked going before its answer, by stopping the stream or losing the connection, is
                    // told the Elixir as the network's edge tells it of a client over the web.
                    let stopped = send.stopped();
                    tokio::pin!(stopped);
                    let (mut telling, mut there) = (token.is_some(), true);
                    let response = loop {
                        tokio::select! {
                            response = &mut response => break response,
                            _ = every.tick(), if telling => {
                                let ran = started.elapsed().as_secs();
                                let doing = stage.lock().expect("no answer panics").clone();
                                let token = token.as_ref().expect("only told with a token");
                                telling = send.write_all(&client::progress(token, ran, doing.as_deref())).await.is_ok();
                            }
                            _ = &mut stopped, if there => {
                                (telling, there) = (false, false);
                                if let Some(gone) = gone(&message) {
                                    let _ = ask(&handler, Some(&from), &gone, Arc::default()).await;
                                }
                            }
                        }
                    };
                    match response {
                        Ok(response) if send.write_all(&response).await.is_ok() => drop(send.finish()),
                        Ok(_) => {}
                        Err(unanswered) => drop(send.reset(unanswered)),
                    }
                });
            }
            // A connection the client made after this one stays.
            let mut clients = clients.lock().expect("no connection panics");
            if clients.get(&from).is_some_and(|last| last.stable_id() == connection.stable_id()) {
                clients.remove(&from);
            }
        });
    }
}

/// What says whoever asked `message` went away before its answer: `notifications/one/gone` for its id, with what it
/// said of itself, as the network's edge says it for a client over the web (`OneCore.Call.State.gone/1`). Nothing for
/// a notification, which isn't answered.
fn gone(message: &[u8]) -> Option<Vec<u8>> {
    let request: serde_json::Value = serde_json::from_slice(message).ok()?;
    let params = serde_json::json!({"requestId": request.get("id")?, "_meta": request["params"]["_meta"]});
    Some(serde_json::json!({"jsonrpc": "2.0", "method": "notifications/one/gone", "params": params}).to_string().into_bytes())
}

/// Keeps where the client whose key is `client` says a server reaches it while it wants to hear what changes, the
/// JSON a client is told an address in, and the device it proved it runs for, or with no address, forgets it. A client
/// says only where it is itself. A device watches from its latest run, as its app runs once, so a run of the same
/// device from before is forgotten.
#[rustler::nif(schedule = "DirtyIo")]
fn watch(reached: ResourceArc<Reached>, client: String, address: Option<String>, device: Option<String>) -> Atom {
    let address = address.and_then(|address| serde_json::from_str::<EndpointAddr>(&address).ok());
    let mut watchers = reached.watchers.lock().expect("no connection panics");
    let changed = match address.filter(|address| address.id.to_string() == client) {
        // A client says it each time it connects, which changes nothing once it's said.
        Some(address)
            if watchers.get(&client).is_some_and(|watcher| watcher.address == address && watcher.device == device) =>
        {
            false
        }
        Some(address) => {
            if device.is_some() {
                watchers.retain(|_, watcher| watcher.device != device);
            }
            watchers.insert(client, Watcher { address, device, open: Arc::default() });
            true
        }
        None => watchers.remove(&client).is_some(),
    };
    if changed {
        reached.keep(&watchers);
    }
    atoms::ok()
}

/// Sends `message` to every client that's watching: over the connection it has open to this server, or else over
/// one made to it for this. A client that can't be reached where it said has gone, and is told nothing more.
#[rustler::nif]
fn notify(reached: ResourceArc<Reached>, message: Binary) -> Atom {
    let message: Arc<[u8]> = message.as_slice().into();
    let watchers = reached.watchers.lock().expect("no connection panics").clone();
    for (key, watcher) in watchers {
        let (reached, message) = (reached.clone(), message.clone());
        RUNTIME.spawn(async move {
            let asking = reached.clients.lock().expect("no connection panics").get(&key).cloned();
            if let Some(asking) = asking
                && client::notify(&asking, &message).await
            {
                return;
            }
            let mut open = watcher.open.lock().await;
            if let Some(open) = open.as_ref().filter(|open| open.open())
                && client::notify(open.connection(), &message).await
            {
                return;
            }
            *open = client::Reached::connect(&reached.endpoint, watcher.address.clone(), client::CONNECT).await.ok().map(Arc::new);
            match open.as_ref() {
                Some(open) => drop(client::notify(open.connection(), &message).await),
                None => {
                    let mut watchers = reached.watchers.lock().expect("no connection panics");
                    // It may have said where it is again since.
                    if watchers.get(&key).is_some_and(|now| Arc::ptr_eq(&now.open, &watcher.open)) {
                        watchers.remove(&key);
                        reached.keep(&watchers);
                    }
                }
            }
        });
    }
    atoms::ok()
}

/// The devices that are watching, by their keys: each a device a run of its client proved it runs for.
#[rustler::nif]
fn devices(reached: ResourceArc<Reached>) -> Vec<String> {
    let watchers = reached.watchers.lock().expect("no connection panics");
    watchers.values().filter_map(|watcher| watcher.device.clone()).collect()
}

/// Asks the device whose key is `device` one message, the run of its client that's watching, over the connection this
/// server has made to it, or one it makes now, and sends `to` the message `{reference, outcome}`: `{:ok, answer}`;
/// `:unreachable` if no run of it is watching, or the one that is can't be reached, so it was asked nothing, and has
/// gone, as `notify` finds; or `:stopped` if it stopped answering after it was asked, so it may have done it.
#[rustler::nif]
fn ask_client(reached: ResourceArc<Reached>, device: String, message: Binary, to: LocalPid, reference: Term) -> Atom {
    let watching = reached.watchers.lock().expect("no connection panics").clone();
    let watcher = watching.into_iter().find(|(_, watcher)| watcher.device.as_ref() == Some(&device));
    let message = message.as_slice().to_vec();
    let reference = Reference::keep(reference);
    RUNTIME.spawn(async move {
        let open = match &watcher {
            Some((_, watcher)) => watcher.reached(&reached.endpoint).await,
            None => None,
        };
        if let (None, Some((key, watcher))) = (&open, &watcher) {
            let mut watchers = reached.watchers.lock().expect("no connection panics");
            // It may have said where it is again since.
            if watchers.get(key).is_some_and(|now| Arc::ptr_eq(&now.open, &watcher.open)) {
                watchers.remove(key);
                reached.keep(&watchers);
            }
        }
        let response = match open {
            Some(open) => open.ask(&message, |_| {}).await,
            None => Err(RequestError::Unreachable),
        };
        reference.send(&to, |env| match response {
            Ok(response) => {
                let mut binary = NewBinary::new(env, response.len());
                binary.as_mut_slice().copy_from_slice(&response);
                (atoms::ok(), Binary::from(binary)).encode(env)
            }
            Err(RequestError::Stopped) => atoms::stopped().encode(env),
            Err(RequestError::Unreachable) => atoms::unreachable().encode(env),
        });
    });
    atoms::ok()
}

/// Stops taking what's asked: whoever asks or connects from now on is refused, so they ask another server. Once
/// every message taken is answered, or `within` milliseconds have passed, the endpoint closes, which whoever is
/// connected hears at once.
#[rustler::nif(schedule = "DirtyIo")]
fn close(reached: ResourceArc<Reached>, within: u64) -> Atom {
    RUNTIME.block_on(async {
        let _locked = tokio::time::timeout(Duration::from_millis(within), reached.asks.write()).await;
        reached.endpoint.close().await;
    });
    atoms::ok()
}

/// Sends `message` back to whoever asked `ask`. An empty answer only ends the stream.
#[rustler::nif]
fn reply(ask: ResourceArc<Ask>, message: Binary) -> Atom {
    if let Some(reply_sender) = ask.answer.lock().expect("no answer panics").take() {
        // Whoever asked may have gone, and then nobody is left to tell.
        let _ = reply_sender.send(message.as_slice().to_vec());
    }
    atoms::ok()
}

/// Says what the call `ask` asked for is doing, which its progress notifications carry from now on.
#[rustler::nif]
fn stage(ask: ResourceArc<Ask>, doing: String) -> Atom {
    *ask.stage.lock().expect("no answer panics") = Some(doing);
    atoms::ok()
}

/// Connects to the server at `address`, the JSON a client is told it in, and sends `to` the message
/// `{reference, {:connected, path}}`, the path being `direct` or `relay` (`client::Reached::path`), or
/// `{reference, {:unreachable, why, relay}}`, `relay` being how this server reaches its network's relay just then
/// (`client::relay`). A server last found gone at this same address is given only as long as one that's there takes
/// to say so.
#[rustler::nif]
fn connect(reached: ResourceArc<Reached>, address: String, to: LocalPid, reference: Term) -> Atom {
    let reference = Reference::keep(reference);
    RUNTIME.spawn(async move {
        let connected = async {
            let server = serde_json::from_str::<EndpointAddr>(&address)
                .map_err(|error| format!("{address} isn't an address: {error}"))?;
            let key = server.id.to_string();
            let gone_there = reached
                .servers
                .lock()
                .expect("no connection panics")
                .get(&key)
                .is_some_and(|(was, open)| *was == address && open.is_none());
            let within = if gone_there { client::THERE } else { client::CONNECT };
            let open = client::Reached::connect(&reached.endpoint, server, within).await.map_err(|why| why.to_string())?;
            let path = open.path();
            reached.servers.lock().expect("no connection panics").insert(key, (address, Some(Arc::new(open))));
            Ok::<_, String>(path)
        };
        let outcome = connected.await;
        let relay = client::relay(&reached.endpoint);
        reference.send(&to, |env| match outcome {
            Ok(path) => (atoms::connected(), path).encode(env),
            Err(why) => (atoms::unreachable(), why, relay).encode(env),
        });
    });
    atoms::ok()
}

/// Asks the server whose key is `server` one message over its open connection, and sends `to` the message
/// `{reference, outcome}`: `{:ok, answer}`; `:closed` if no connection to it is open, or `{:gone, address}` if the
/// one that was open, made to `address`, no longer answers, so either way it was asked nothing; or `:stopped` if it
/// stopped answering after it was asked, so it may have done what was asked. What that server says the call is doing,
/// in its progress notifications, is said of `relay` too, the message this server is answering by asking it.
#[rustler::nif]
fn ask_server(
    reached: ResourceArc<Reached>,
    server: String,
    message: Binary,
    to: LocalPid,
    reference: Term,
    relay: Option<ResourceArc<Ask>>,
) -> Atom {
    let open = reached.servers.lock().expect("no connection panics").get(&server).cloned();
    let message = message.as_slice().to_vec();
    let reference = Reference::keep(reference);
    let relay = relay.map(|ask| ask.stage.clone());
    RUNTIME.spawn(async move {
        let relayed = |line: &[u8]| {
            if let (Some(relay), Some(doing)) = (&relay, client::progress_message(line)) {
                *relay.lock().expect("no answer panics") = Some(doing);
            }
        };
        let response = match open {
            Some((address, Some(open))) => match open.ask(&message, relayed).await {
                Ok(response) => Ok(response),
                Err(unanswered) => {
                    let mut servers = reached.servers.lock().expect("no connection panics");
                    // Another connection may have been made since this one was asked, and that one stays.
                    if let Some((_address, Some(now))) = servers.get(&server)
                        && Arc::ptr_eq(now, &open)
                    {
                        match open.gone() {
                            true => servers.insert(server, (address.clone(), None)),
                            false => servers.remove(&server),
                        };
                    }
                    Err(Some((unanswered, open.gone().then_some(address))))
                }
            },
            _ => Err(None),
        };
        reference.send(&to, |env| match response {
            Ok(response) => {
                let mut binary = NewBinary::new(env, response.len());
                binary.as_mut_slice().copy_from_slice(&response);
                (atoms::ok(), Binary::from(binary)).encode(env)
            }
            Err(Some((RequestError::Stopped, _gone))) => atoms::stopped().encode(env),
            Err(Some((RequestError::Unreachable, Some(address)))) => (atoms::gone(), address).encode(env),
            Err(_) => atoms::closed().encode(env),
        });
    });
    atoms::ok()
}

/// A reference Elixir made, kept to mark the one message that answers it.
struct Reference(OwnedEnv, rustler::env::SavedTerm);

impl Reference {
    fn keep(reference: Term) -> Self {
        let env = OwnedEnv::new();
        let saved = env.save(reference);
        Self(env, saved)
    }

    // The server's Elixir going away is the only way this fails, and then nobody is left to tell.
    fn send(mut self, to: &LocalPid, outcome: impl for<'a> FnOnce(Env<'a>) -> Term<'a>) {
        let saved = self.1;
        let _ = self.0.send_and_clear(to, |env| (saved.load(env), outcome(env)));
    }
}

include!(concat!(env!("OUT_DIR"), "/init.rs"));
