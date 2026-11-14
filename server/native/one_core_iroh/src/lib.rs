//! The server's add-on for being reached by key: it opens the server's endpoint, hands what clients ask to the
//! server's Elixir, and sends its answers back. A client asks one message a stream (`client`).

use std::net::{Ipv4Addr, SocketAddr};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use iroh::Endpoint;
use iroh::endpoint::SendStream;
use rustler::{Atom, Binary, LocalPid, NewBinary, OwnedEnv, Resource, ResourceArc};
use tokio::runtime::Runtime;

mod atoms {
    rustler::atoms! { ok, ask }
}

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| Runtime::new().expect("tokio starts"));

/// A server's endpoint, open for as long as the server holds this. Nothing looks inside it after a panic, which is
/// what Rustler needs to hear before it hands one to Elixir.
struct Reached(AssertUnwindSafe<Endpoint>);

#[rustler::resource_impl]
impl Resource for Reached {}

impl Drop for Reached {
    fn drop(&mut self) {
        let endpoint = self.0.clone();
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
        tokio::spawn(accept(endpoint.clone(), asked));
        Ok((ResourceArc::new(Reached(AssertUnwindSafe(endpoint))), address))
    })
}

async fn accept(endpoint: Endpoint, asked: LocalPid) {
    while let Some(incoming) = endpoint.accept().await {
        tokio::spawn(async move {
            let Ok(connection) = incoming.await else { return };
            while let Ok((send, mut receive)) = connection.accept_bi().await {
                tokio::spawn(async move {
                    let Ok(message) = receive.read_to_end(client::LIMIT).await else { return };
                    let ask = ResourceArc::new(Ask(Mutex::new(Some(send))));
                    // The server's Elixir going away is the only way this fails, and then nobody is left to tell.
                    let _ = OwnedEnv::new().send_and_clear(&asked, |env| {
                        let mut binary = NewBinary::new(env, message.len());
                        binary.as_mut_slice().copy_from_slice(&message);
                        (atoms::ask(), ask, Binary::from(binary))
                    });
                });
            }
        });
    }
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

rustler::init!("Elixir.OneCore.Iroh");
