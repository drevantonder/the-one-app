//! The server's add-on for being reached by key: it opens the server's endpoint, hands what clients ask to the
//! server's Elixir, and sends its answers back. A client asks one message a stream (`client`), and is told
//! something the same way: one message on a stream the server opens. A server asks another as a client does.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use iroh::endpoint::{Connection, SendStream};
use iroh::{Endpoint, EndpointAddr};
use rustler::{Atom, Binary, Encoder, Env, LocalPid, NewBinary, OwnedEnv, Resource, ResourceArc, Term};
use tokio::runtime::Runtime;
use tokio::sync::broadcast;

mod atoms {
    rustler::atoms! { ok, ask, closed, stopped, unreachable }
}

/// How long reaching another server may take before it counts as not answering. A spare waits this long before it
/// takes over, so it's short.
const CONNECT: Duration = Duration::from_secs(2);

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| Runtime::new().expect("tokio starts"));

/// A server's endpoint, open for as long as the server holds this, where what it tells its clients is sent from,
/// and its open connections to other servers by their keys. Nothing looks inside it after a panic, which is what
/// Rustler needs to hear before it hands one to Elixir.
struct Reached {
    endpoint: AssertUnwindSafe<Endpoint>,
    told: AssertUnwindSafe<broadcast::Sender<Vec<u8>>>,
    servers: Mutex<HashMap<String, Connection>>,
}

#[rustler::resource_impl]
impl Resource for Reached {}

impl Drop for Reached {
    fn drop(&mut self) {
        let endpoint = self.endpoint.clone();
        RUNTIME.spawn(async move { endpoint.close().await });
    }
}

/// One message a client asked, holding the stream its answer goes back on. Dropped unanswered, the client is told
/// the stream ended.
struct Ask(Mutex<Option<SendStream>>);

#[rustler::resource_impl]
impl Resource for Ask {}

/// Opens the endpoint whose key is kept in `key_file`, and sends each message asked to `asked` as
/// `{:ask, ask, message}`. Gives the endpoint and its address, as the JSON a client is told it in.
#[rustler::nif(schedule = "DirtyIo")]
fn start(key_file: String, asked: LocalPid) -> Result<(ResourceArc<Reached>, String), String> {
    RUNTIME.block_on(async {
        let endpoint = client::endpoint(Path::new(&key_file)).await.map_err(|error| format!("{error:#}"))?;
        // iroh lists the machine's addresses on its networks. A client on this machine reaches it with no network too.
        let here = endpoint.bound_sockets().into_iter().filter(SocketAddr::is_ipv4);
        let address = here.fold(endpoint.addr(), |address, socket| {
            address.with_ip_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, socket.port())))
        });
        let address = serde_json::to_string(&address).map_err(|error| error.to_string())?;
        // A client that falls this many messages behind misses some, and hears the next.
        let (told, _) = broadcast::channel(64);
        tokio::spawn(accept(endpoint.clone(), asked, told.clone()));
        let reached = Reached {
            endpoint: AssertUnwindSafe(endpoint),
            told: AssertUnwindSafe(told),
            servers: Mutex::new(HashMap::new()),
        };
        Ok((ResourceArc::new(reached), address))
    })
}

async fn accept(endpoint: Endpoint, asked: LocalPid, told: broadcast::Sender<Vec<u8>>) {
    while let Some(incoming) = endpoint.accept().await {
        let mut told = told.subscribe();
        tokio::spawn(async move {
            let Ok(connection) = incoming.await else { return };
            loop {
                tokio::select! {
                    stream = connection.accept_bi() => {
                        let Ok((send, mut receive)) = stream else { return };
                        tokio::spawn(async move {
                            let Ok(message) = receive.read_to_end(client::LIMIT).await else { return };
                            let ask = ResourceArc::new(Ask(Mutex::new(Some(send))));
                            // The server's Elixir going away is the only way this fails, and then nobody is left
                            // to tell.
                            let _ = OwnedEnv::new().send_and_clear(&asked, |env| {
                                let mut binary = NewBinary::new(env, message.len());
                                binary.as_mut_slice().copy_from_slice(&message);
                                (atoms::ask(), ask, Binary::from(binary))
                            });
                        });
                    }
                    message = told.recv() => {
                        let message = match message {
                            Ok(message) => message,
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => return,
                        };
                        let connection = connection.clone();
                        tokio::spawn(async move {
                            let Ok(mut send) = connection.open_uni().await else { return };
                            if send.write_all(&message).await.is_ok() {
                                let _ = send.finish();
                            }
                        });
                    }
                }
            }
        });
    }
}

/// Sends `message` to every client connected to the endpoint.
#[rustler::nif]
fn tell(reached: ResourceArc<Reached>, message: Binary) -> Atom {
    // With no client connected there's nobody to send to, which is fine.
    let _ = reached.told.send(message.as_slice().to_vec());
    atoms::ok()
}

/// Sends `message` back to whoever asked `ask`. An empty answer only ends the stream.
#[rustler::nif]
fn answer(ask: ResourceArc<Ask>, message: Binary) -> Atom {
    if let Some(mut send) = ask.0.lock().expect("no answer panics").take() {
        let answer = message.as_slice().to_vec();
        RUNTIME.spawn(async move {
            if send.write_all(&answer).await.is_ok() {
                let _ = send.finish();
            }
        });
    }
    atoms::ok()
}

/// Connects to the server at `address`, the JSON a client is told it in, and sends `to` the message
/// `{reference, :ok}`, or `{reference, :unreachable}` if it doesn't answer in time.
#[rustler::nif]
fn connect(reached: ResourceArc<Reached>, address: &str, to: LocalPid, reference: Term) -> Atom {
    let address = serde_json::from_str::<EndpointAddr>(address);
    let reference = Reference::keep(reference);
    RUNTIME.spawn(async move {
        let connected = async {
            let address = address.ok()?;
            let key = address.id.to_string();
            let connecting = reached.endpoint.connect(address, client::ALPN);
            let connection = tokio::time::timeout(CONNECT, connecting).await.ok()?.ok()?;
            reached.servers.lock().expect("no connection panics").insert(key, connection.clone());
            // What that server tells its clients isn't for this one, and is read so that it can go on telling.
            tokio::spawn(async move {
                while let Ok(mut told) = connection.accept_uni().await {
                    let _ = told.read_to_end(client::LIMIT).await;
                }
            });
            Some(atoms::ok())
        };
        let outcome = connected.await.unwrap_or(atoms::unreachable());
        reference.send(&to, |env| outcome.encode(env));
    });
    atoms::ok()
}

/// Asks the server whose key is `server` one message over its open connection, and sends `to` the message
/// `{reference, outcome}`: `{:ok, answer}`; `:closed` if no connection to it is open, so it was asked nothing; or
/// `:stopped` if it stopped answering after it was asked, so it may have done what was asked.
#[rustler::nif]
fn ask_server(reached: ResourceArc<Reached>, server: String, message: Binary, to: LocalPid, reference: Term) -> Atom {
    let open = reached.servers.lock().expect("no connection panics").get(&server).cloned();
    let message = message.as_slice().to_vec();
    let reference = Reference::keep(reference);
    RUNTIME.spawn(async move {
        let asked = async {
            let connection = open.filter(|open| open.close_reason().is_none())?;
            let (mut send, receive) = connection.open_bi().await.ok()?;
            send.write_all(&message).await.ok()?;
            send.finish().ok()?;
            Some(receive)
        };
        let answer = match asked.await {
            Some(mut receive) => receive.read_to_end(client::LIMIT).await.map_err(|_| atoms::stopped()),
            None => Err(atoms::closed()),
        };
        reference.send(&to, |env| match answer {
            Ok(answer) => {
                let mut binary = NewBinary::new(env, answer.len());
                binary.as_mut_slice().copy_from_slice(&answer);
                (atoms::ok(), Binary::from(binary)).encode(env)
            }
            Err(why) => why.encode(env),
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

rustler::init!("Elixir.OneCore.Iroh");
