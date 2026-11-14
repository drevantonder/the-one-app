//! The server's add-on for being reached by key: it opens the server's endpoint, hands what clients ask to the
//! server's Elixir with the key of whoever asked, and sends its answers back. A client asks one message a stream
//! (`client`), and is told something the same way: one message on a stream the server opens. A server asks another
//! as a client does.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use client::{Network, Unanswered};
use data_encoding::HEXLOWER;
use iroh::{Endpoint, EndpointAddr};
use rustler::{Atom, Binary, Encoder, Env, LocalPid, NewBinary, OwnedEnv, Resource, ResourceArc, Term};
use tokio::runtime::Runtime;
use tokio::sync::{broadcast, oneshot};

mod outside;

mod atoms {
    rustler::atoms! { ok, ask, closed, gone, stopped, unreachable }
}

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| Runtime::new().expect("tokio starts"));

/// A server's endpoint, open for as long as the server holds this, where what it tells its clients is sent from,
/// and its connections to other servers by their keys, each with the address it was made to. A server found gone
/// leaves that address with no connection, and a connection that closed for want of anything to ask leaves nothing.
/// Nothing looks inside it after a panic, which is what Rustler needs to hear before it hands one to Elixir.
struct Reached {
    endpoint: AssertUnwindSafe<Endpoint>,
    told: AssertUnwindSafe<broadcast::Sender<Vec<u8>>>,
    servers: AssertUnwindSafe<Mutex<HashMap<String, (String, Option<Arc<client::Reached>>)>>>,
}

#[rustler::resource_impl]
impl Resource for Reached {}

impl Drop for Reached {
    fn drop(&mut self) {
        let endpoint = self.endpoint.clone();
        RUNTIME.spawn(async move { endpoint.close().await });
    }
}

/// One message asked, holding the way its answer goes back. Dropped unanswered, whoever asked is told nothing came.
struct Ask(Mutex<Option<oneshot::Sender<Vec<u8>>>>);

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

/// Opens the endpoint whose key is kept in `key_file`, and sends each message asked to `asked` as
/// `{:ask, ask, from, message}`, where `from` is the key of whoever asked. With a `network`, its address and the pass
/// shown there, it's reached through the relay at that address too, and hears what outsiders ask there, whose `from`
/// is `nil`. Gives the endpoint and its address, as the JSON a client is told it in.
#[rustler::nif(schedule = "DirtyIo")]
fn start(
    key_file: String,
    network: Option<(String, String)>,
    asked: LocalPid,
) -> Result<(ResourceArc<Reached>, String), String> {
    RUNTIME.block_on(async {
        let network = network.map(|(address, pass)| Network { address, pass });
        let endpoint =
            client::endpoint(Path::new(&key_file), network.as_ref()).await.map_err(|error| format!("{error:#}"))?;
        // iroh lists the machine's addresses on its networks. A client on this machine reaches it with no network too.
        let here = endpoint.bound_sockets().into_iter().filter(SocketAddr::is_ipv4);
        let mut address = here.fold(endpoint.addr(), |address, socket| {
            address.with_ip_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, socket.port())))
        });
        if let Some(network) = network {
            // A client is told the relay now, before the endpoint has connected to it.
            address = address.with_relay_url(network.address.parse().map_err(|_| "that isn't an address")?);
            tokio::spawn(outside::hear(network, endpoint.secret_key().clone(), asked));
        }
        let address = serde_json::to_string(&address).map_err(|error| error.to_string())?;
        // A client that falls this many messages behind misses some, and hears the next.
        let (told, _) = broadcast::channel(64);
        tokio::spawn(accept(endpoint.clone(), asked, told.clone()));
        let reached = Reached {
            endpoint: AssertUnwindSafe(endpoint),
            told: AssertUnwindSafe(told),
            servers: AssertUnwindSafe(Mutex::new(HashMap::new())),
        };
        Ok((ResourceArc::new(reached), address))
    })
}

/// Hands `message` to the server's Elixir, with the key of whoever asked it if they have one, and waits for its
/// answer. There's none if the Elixir is gone.
async fn ask(asked: &LocalPid, from: Option<&str>, message: &[u8]) -> Option<Vec<u8>> {
    let (answer, answered) = oneshot::channel();
    let ask = ResourceArc::new(Ask(Mutex::new(Some(answer))));
    let sent = OwnedEnv::new().send_and_clear(asked, |env| {
        let mut binary = NewBinary::new(env, message.len());
        binary.as_mut_slice().copy_from_slice(message);
        (atoms::ask(), ask, from, Binary::from(binary))
    });
    sent.ok()?;
    answered.await.ok()
}

async fn accept(endpoint: Endpoint, asked: LocalPid, told: broadcast::Sender<Vec<u8>>) {
    while let Some(incoming) = endpoint.accept().await {
        let mut told = told.subscribe();
        tokio::spawn(async move {
            let Ok(connection) = incoming.await else { return };
            let from = connection.remote_id().to_string();
            loop {
                tokio::select! {
                    stream = connection.accept_bi() => {
                        let Ok((mut send, mut receive)) = stream else { return };
                        let from = from.clone();
                        tokio::spawn(async move {
                            let Ok(message) = receive.read_to_end(client::LIMIT).await else { return };
                            // Whoever asks nothing only asks whether this server is there, which this says.
                            if message.is_empty() {
                                let _ = send.finish();
                            } else if let Some(answer) = ask(&asked, Some(&from), &message).await
                                && send.write_all(&answer).await.is_ok()
                            {
                                let _ = send.finish();
                            }
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
    if let Some(answer) = ask.0.lock().expect("no answer panics").take() {
        // Whoever asked may have gone, and then nobody is left to tell.
        let _ = answer.send(message.as_slice().to_vec());
    }
    atoms::ok()
}

/// Connects to the server at `address`, the JSON a client is told it in, and sends `to` the message
/// `{reference, :ok}`, or `{reference, :unreachable}` if it doesn't answer in time. A server last found gone at
/// this same address is given only as long as one that's there takes to say so.
#[rustler::nif]
fn connect(reached: ResourceArc<Reached>, address: String, to: LocalPid, reference: Term) -> Atom {
    let reference = Reference::keep(reference);
    RUNTIME.spawn(async move {
        let connected = async {
            let server = serde_json::from_str::<EndpointAddr>(&address).ok()?;
            let key = server.id.to_string();
            let gone_there = reached
                .servers
                .lock()
                .expect("no connection panics")
                .get(&key)
                .is_some_and(|(was, open)| *was == address && open.is_none());
            let within = if gone_there { client::THERE } else { client::CONNECT };
            let open = Arc::new(client::Reached::connect(&reached.endpoint, server, within).await?);
            let connection = open.connection().clone();
            reached.servers.lock().expect("no connection panics").insert(key, (address, Some(open)));
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
/// `{reference, outcome}`: `{:ok, answer}`; `:closed` if no connection to it is open, or `{:gone, address}` if the
/// one that was open, made to `address`, no longer answers, so either way it was asked nothing; or `:stopped` if it
/// stopped answering after it was asked, so it may have done what was asked.
#[rustler::nif]
fn ask_server(reached: ResourceArc<Reached>, server: String, message: Binary, to: LocalPid, reference: Term) -> Atom {
    let open = reached.servers.lock().expect("no connection panics").get(&server).cloned();
    let message = message.as_slice().to_vec();
    let reference = Reference::keep(reference);
    RUNTIME.spawn(async move {
        let answer = match open {
            Some((address, Some(open))) => match open.ask(&message).await {
                Ok(answer) => Ok(answer),
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
        reference.send(&to, |env| match answer {
            Ok(answer) => {
                let mut binary = NewBinary::new(env, answer.len());
                binary.as_mut_slice().copy_from_slice(&answer);
                (atoms::ok(), Binary::from(binary)).encode(env)
            }
            Err(Some((Unanswered::Stopped, _gone))) => atoms::stopped().encode(env),
            Err(Some((Unanswered::Unreachable, Some(address)))) => (atoms::gone(), address).encode(env),
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

rustler::init!("Elixir.OneCore.Iroh");
