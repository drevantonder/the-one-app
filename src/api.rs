//! The local HTTP API: how this machine's shells, CLI and MCP reach its node, as its local user; how a client device
//! reaches it over iroh (src/node/net.rs), as its person; and how someone signed in at the network's address
//! (this machine's CLI and MCP, after `sign-in`) reaches it, as themselves, for what their sign-in opens.
//!
//! A request with an access token from auth for the network's address (never a remote MCP client's, which is for
//! `<address>/mcp`) is its person's, and so is its call's; someone it's for who isn't a person of the network yet is
//! let in as a member by a device whose own person is an admin, since sign-in lets in only those an admin invited
//! or people at the network's domain. A request without one is this machine's own programs', over its loopback with the
//! token in the node's home (`local-token`, which only its user reads), or refused, but for a picture a window's app
//! list names by its content (`/v1/images/<name>`), anyone's who has that name. A file put from elsewhere is read back
//! only by whoever put it or may use a scope that names it, up to 512 MiB each and 2 GiB per person that nothing keeps
//! yet, deleted an hour on (or once its request is answered) unless something does.

use axum::Router;
use axum::routing::post;

use crate::node::Node;

pub mod client;
pub use client::Client;
mod agents;
mod calls;
mod guard;
mod local;
mod network;
mod reply;
mod serve;

pub use agents::serve_guide;
pub use calls::app_route;
pub(crate) use calls::call_from_cloudflare_node;
pub use guard::{ENDPOINT_HEADER, FromClient, guarded};
pub(crate) use guard::{Me, SignedIns};
pub use local::{LOCAL_TOKEN, LOCAL_TOKEN_HEADER, agent, as_this_machine, local_token};
pub(crate) use reply::{Body, Error, Reply, done, given, tagged};
pub use serve::{serve, serve_until};

/// Where this machine's node serves its local API: the config's port, or `<CLI>_ADDR`.
pub fn addr() -> String {
    std::env::var(format!("{}_ADDR", crate::config::CLI.to_uppercase())).unwrap_or_else(|_| format!("127.0.0.1:{}", crate::config::PORT))
}

pub fn router(node: Node) -> Router {
    guarded(routes(), node)
}

/// Who may ask a route. This machine's own programs may ask any; the rest only what its route says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// Anyone who has its name: a picture named by its content.
    Anyone,
    /// Also someone signed in at the network's address, and the network's client devices over iroh, each as their
    /// person, with what that person may do.
    SignedIn,
    /// Also the network's client devices over iroh, as their person, but no sign-in from elsewhere: MCP.
    Clients,
    /// Only this machine's own programs.
    ThisMachine,
}

/// The local API's routes, each with who may ask it (`Who`): what `guarded` lets in.
#[derive(Default)]
pub struct Routes {
    router: Router<Node>,
    who: Vec<(&'static str, Who)>,
}

impl Routes {
    pub fn route(mut self, path: &'static str, who: Who, handler: axum::routing::MethodRouter<Node>) -> Routes {
        self.router = self.router.route(path, handler);
        self.who.push((path, who));
        self
    }

    pub fn merge(mut self, other: Routes) -> Routes {
        self.router = self.router.merge(other.router);
        self.who.extend(other.who);
        self
    }

    /// Every route's path, with who may ask it.
    pub fn who(&self) -> &[(&'static str, Who)] {
        &self.who
    }
}

/// Every route of the local API, unguarded: `guarded` lets in who each says.
pub fn routes() -> Routes {
    use Who::*;
    Routes::default()
        .route("/mcp", Clients, post(crate::mcp::http))
        .merge(agents::routes())
        .merge(network::routes())
        .merge(calls::routes())
        .merge(crate::node::vault::routes::routes())
        .merge(crate::node::backups::routes::routes())
        .merge(crate::node::storage::routes::routes())
        .merge(crate::node::devices::routes::routes())
        .merge(crate::node::groups::routes::routes())
        .merge(crate::node::widgets::routes::routes())
        .merge(crate::node::push::routes::routes())
        .merge(crate::node::apps::routes::routes())
        .merge(crate::node::services::routes::routes())
        .merge(crate::node::releases::routes::routes())
        .merge(crate::node::blobs::routes::routes())
        .merge(crate::node::screens::routes::routes())
}

/// Whether a client device may ask `path` (`Who::SignedIn` or `Who::Clients`), as the node's routes match it.
pub fn clients_may(path: &str) -> bool {
    let asked: Vec<&str> = path.split('/').collect();
    calls::app_route(path)
        || routes().who().iter().filter(|(r, w)| *r != calls::APP_ROUTE && matches!(w, Who::SignedIn | Who::Clients)).any(|(route, _)| {
            let route: Vec<&str> = route.split('/').collect();
            route.len() == asked.len() && route.iter().zip(&asked).all(|(r, a)| r == a || (r.starts_with('{') && !a.is_empty()))
        })
}
pub fn port() -> u16 {
    port_of(&addr())
}

pub fn port_of(addr: &str) -> u16 {
    addr.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(crate::config::PORT)
}

#[cfg(test)]
mod tests {
    use super::Who;

    /// Who asks, as `only_this_machine` tells them apart.
    #[derive(Clone, Copy, Debug)]
    enum Asker {
        Nobody,
        SignIn,
        Client,
        ThisMachine,
    }

    /// Every route refuses whoever its `Who` doesn't let in, and lets the rest on to who they are: a sign-in to its
    /// token's check, a client device to whether it's one of the network's, this machine's own programs anywhere.
    #[tokio::test(flavor = "multi_thread")]
    async fn each_route_refuses_whom_its_who_says() {
        let dir = std::env::temp_dir().join(format!("one-routes-{}", rand::random::<u64>()));
        let node = crate::node::Node::start(&dir, false).await.unwrap();
        let routes = super::routes();
        let who: Vec<(&str, Who)> = routes.who().to_vec();
        // Only a client device's requests over iroh carry `FromClient`, which the node puts on them: here, a header.
        let router = super::guarded(routes, node.clone()).layer(axum::middleware::from_fn(|mut req: axum::extract::Request, next: axum::middleware::Next| async move {
            if req.headers().contains_key("x-test-client") {
                req.extensions_mut().insert(super::FromClient([7; 32], u64::MAX));
            }
            next.run(req).await
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = listener.local_addr().unwrap();
        tokio::spawn(super::serve(listener, router));
        let token = node.local_token().to_string();
        let asked = tokio::task::spawn_blocking(move || {
            // A refusal can close its connection (`refuse`), so none is kept for the next request.
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_idle_connections(0).build().into();
            let mut asked = Vec::new();
            for (route, who) in who {
                let path: String = route.split('/').map(|s| if s.starts_with('{') { "x" } else { s }).collect::<Vec<_>>().join("/");
                for asker in [Asker::Nobody, Asker::SignIn, Asker::Client, Asker::ThisMachine] {
                    let said: Vec<(u16, serde_json::Value)> = ["GET", "POST"]
                        .into_iter()
                        .filter_map(|method| {
                            let mut req = ureq::http::Request::builder().method(method).uri(format!("http://{at}{path}")).header("content-type", "application/json");
                            req = match asker {
                                Asker::Nobody => req,
                                Asker::SignIn => req.header("authorization", "Bearer not-a-sign-in"),
                                Asker::Client => req.header("x-test-client", "1"),
                                Asker::ThisMachine => req.header(super::LOCAL_TOKEN_HEADER, &token).header("authorization", "Bearer not-a-sign-in"),
                            };
                            let mut resp = agent.run(req.body("{}".to_string()).unwrap()).unwrap();
                            let status = resp.status().as_u16();
                            (status != 405).then(|| (status, resp.body_mut().read_json().unwrap_or_default()))
                        })
                        .collect();
                    assert!(!said.is_empty(), "{route} takes neither GET nor POST");
                    asked.extend(said.into_iter().map(|(status, body)| (route, who, asker, status, body)));
                }
            }
            asked
        })
        .await
        .unwrap();
        for (route, who, asker, status, body) in asked {
            let refused = status == 403 && body["code"] != crate::protocol::code::NOT_LET_IN;
            let (want_refused, then) = match (who, asker) {
                (Who::Anyone, _) => (false, None),
                (_, Asker::Nobody) => (true, None),
                (Who::SignedIn, Asker::SignIn) | (_, Asker::ThisMachine) => (false, Some(401)),
                (_, Asker::SignIn) => (true, None),
                (Who::SignedIn | Who::Clients, Asker::Client) => (false, Some(403)),
                (Who::ThisMachine, Asker::Client) => (true, None),
            };
            assert_eq!(refused, want_refused, "{route} ({who:?}) as {asker:?}: {status} {body}");
            if let Some(then) = then {
                assert_eq!(status, then, "{route} ({who:?}) as {asker:?} goes on to who they are: {body}");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
