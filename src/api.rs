//! The local HTTP API: how this machine's shells, CLI and MCP reach its node, as its local user; how a client device
//! reaches it over iroh (src/node/net.rs), as its user. A sign-in reaches no route here, ever: it only lets a device ask
//! auth to be let in, and a device that's let in is a client device.
//!
//! A request that isn't a client device's is this machine's own programs', over its loopback with the
//! token in the node's home (`local-token`, which only its user reads), or refused. A file put from elsewhere is read back
//! only by whoever put it or may use a scope that names it, up to 512 MiB each and 2 GiB per user that nothing keeps
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
pub mod openapi;
mod reply;
mod serve;
mod tools;

pub use agents::serve_guide;
pub use calls::app_route;
pub(crate) use calls::call_from_cloudflare_node;
pub use guard::{FromClient, guarded};
pub(crate) use guard::Me;
pub use local::{LOCAL_TOKEN, agent, as_this_machine, local_token, token_refused};
pub(crate) use reply::{Body, Error, Reply, done, given, tagged};
pub use serve::{serve, serve_until};

/// Where this machine's node serves its local API: the config's port, or `<CLI>_ADDR`.
pub fn addr() -> String {
    std::env::var(crate::config::env("ADDR")).unwrap_or_else(|_| format!("127.0.0.1:{}", crate::config::PORT))
}

pub fn router(node: Node) -> Router {
    guarded(routes(), node)
}

/// Who may ask a route. This machine's own programs may ask any; the rest only what its route says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// Also the network's client devices over iroh, each as their user, with what that user may do.
    Clients,
    /// Only this machine's own programs.
    ThisMachine,
}

/// The local API's routes, each with who may ask it (`Who`): what `guarded` lets in.
#[derive(Default)]
pub struct Routes {
    router: Router<Node>,
    who: Vec<(String, Who)>,
    listed: Vec<(String, Who, openapi::Op)>,
}

impl Routes {
    pub fn route(mut self, path: &str, who: Who, handler: axum::routing::MethodRouter<Node>) -> Routes {
        self.router = self.router.route(path, handler);
        self.who.push((path.to_string(), who));
        self
    }

    /// What the route before says of itself in the OpenAPI document, one for each method it answers.
    pub fn says(mut self, op: openapi::Op) -> Routes {
        let (path, who) = self.who.last().expect("a route to say it of").clone();
        self.listed.push((path, who, op));
        self
    }

    pub fn merge(mut self, other: Routes) -> Routes {
        self.router = self.router.merge(other.router);
        self.who.extend(other.who);
        self.listed.extend(other.listed);
        self
    }

    /// Every route's path, with who may ask it.
    pub fn who(&self) -> &[(String, Who)] {
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
        .merge(tools::routes())
        .merge(openapi::routes())
        .merge(crate::node::vault::routes::routes())
        .merge(crate::node::backups::routes::routes())
        .merge(crate::node::storage::routes::routes())
        .merge(crate::node::devices::routes::routes())
        .merge(crate::node::settings_routes())
        .merge(crate::node::widgets::routes::routes())
        .merge(crate::node::apps::routes::routes())
        .merge(crate::node::services::routes::routes())
        .merge(crate::node::releases::routes::routes())
        .merge(crate::node::blobs::routes::routes())
        .merge(crate::node::screens::routes::routes())
        .merge(crate::insights::routes::routes())
}

/// Whether a client device may ask `path` (`Who::Clients`), as the node's routes match it.
pub fn clients_may(path: &str) -> bool {
    let asked: Vec<&str> = path.split('/').collect();
    calls::app_route(path)
        || routes().who().iter().filter(|(r, w)| r != calls::APP_ROUTE && matches!(w, Who::Clients)).any(|(route, _)| {
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
    }

    /// Every route refuses whoever its `Who` doesn't let in, and a client device goes on to whether it's one of the
    /// network's. A sign-in reaches no route at all: if one were ever let through to be checked, its bad token would
    /// answer 401 here rather than the refusal every other stranger gets.
    #[tokio::test(flavor = "multi_thread")]
    async fn each_route_refuses_whom_its_who_says() {
        let dir = std::env::temp_dir().join(format!("one-routes-{}", rand::random::<u64>()));
        let node = crate::node::Node::start(&dir, false).await.unwrap();
        let routes = super::routes();
        let who: Vec<(String, Who)> = routes.who().to_vec();
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
        let asked = tokio::task::spawn_blocking(move || {
            // A refusal can close its connection (`refuse`), so none is kept for the next request.
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_idle_connections(0).build().into();
            let mut asked = Vec::new();
            for (route, who) in who {
                let path: String = route.split('/').map(|s| if s.starts_with('{') { "x" } else { s }).collect::<Vec<_>>().join("/");
                for asker in [Asker::Nobody, Asker::SignIn, Asker::Client] {
                    let said: Vec<(u16, serde_json::Value)> = ["GET", "POST"]
                        .into_iter()
                        .filter_map(|method| {
                            let mut req = ureq::http::Request::builder().method(method).uri(format!("http://{at}{path}")).header("content-type", "application/json");
                            req = match asker {
                                Asker::Nobody => req,
                                Asker::SignIn => req.header("authorization", "Bearer not-a-sign-in"),
                                Asker::Client => req.header("x-test-client", "1"),
                            };
                            let mut resp = agent.run(req.body("{}".to_string()).unwrap()).unwrap();
                            let status = resp.status().as_u16();
                            (status != 405).then(|| (status, resp.body_mut().read_json().unwrap_or_default()))
                        })
                        .collect();
                    assert!(!said.is_empty(), "{route} takes neither GET nor POST");
                    asked.extend(said.into_iter().map(|(status, body)| (route.clone(), who, asker, status, body)));
                }
            }
            asked
        })
        .await
        .unwrap();
        for (route, who, asker, status, body) in asked {
            let refused = status == 403 && body["code"] != crate::protocol::code::NOT_LET_IN;
            let (want_refused, then) = match (who, asker) {
                (_, Asker::Nobody | Asker::SignIn) => (true, None),
                (Who::Clients, Asker::Client) => (false, Some(403)),
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
