//! The local HTTP API: how this machine's shells, CLI and MCP reach its node, as its local user; how a client device
//! reaches it over iroh (app-runtime/src/net.rs), as its user. A sign-in reaches no route here, ever: it only lets a device ask
//! auth to be let in, and a device that's let in is a client device.
//!
//! A request that isn't a client device's is this machine's own programs', over its loopback with the
//! token in the node's home (`local-token`, which only its user reads), or refused. A file put from elsewhere is read back
//! only by whoever put it or may use a scope that names it, up to 512 MiB each and 2 GiB per user that nothing keeps
//! yet, deleted an hour on (or once its request is answered) unless something does.

use axum::Router;
use axum::routing::post;

use ::api::Who;
use ::api::runtime::Rt;
use crate::Host;

mod agents;
mod calls;
mod guard;
mod in_process;
mod mcp;
mod network;
pub mod openapi;
mod tools;

pub use agents::serve_guide;
pub use calls::{APP_SAYS, app_route};
pub(crate) use calls::call_from_cloudflare_node;
pub use guard::{FromClient, guarded};
pub use in_process::InProcess;

/// The API's routes in this program: over the runtime its handlers read (`State<Node>` reads it as this one).
pub type Routes = ::api::Routes<Rt>;

pub fn router(host: Host) -> Router {
    guarded(routes(), host)
}

/// The route `path` is asked by, as the API names it (`/api/blobs/{id}`), if it's one of its: a literal route before one
/// with a parameter, as axum takes them.
pub fn route_of(path: &str) -> Option<&'static str> {
    static ROUTES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let routes = ROUTES.get_or_init(|| routes().who.into_iter().map(|(path, _)| path).collect());
    let asked: Vec<&str> = path.split('/').collect();
    let fits = |route: &str, literal: bool| {
        let parts: Vec<&str> = route.split('/').collect();
        let rest = parts.last().is_some_and(|p| p.starts_with("{*"));
        (if rest { asked.len() >= parts.len() } else { asked.len() == parts.len() })
            && parts.iter().zip(&asked).all(|(p, a)| p == a || (!literal && p.starts_with('{') && !a.is_empty()))
    };
    [true, false].into_iter().find_map(|literal| routes.iter().find(|r| fits(r, literal))).map(String::as_str)
}

/// Every route of the local API, unguarded: `guarded` lets in who each says.
pub fn routes() -> Routes {
    use Who::*;
    Routes::default()
        .route("/mcp", Clients, post(mcp::http))
        .merge(agents::routes())
        .merge(network::routes())
        .merge(calls::routes())
        .merge(tools::routes())
        .merge(openapi::routes())
        .merge(crate::storage::routes::routes())
        .merge(crate::settings_routes())
        .merge(crate::holds_routes())
        .merge(crate::apps::routes::routes())
        .merge(crate::services::routes::routes())
        .merge(crate::releases::routes::routes())
        .merge(crate::blobs::routes::routes())
        .merge(crate::screens::routes::routes())
        .merge(crate::runtime::routes())
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
        let host = crate::Host::start(&dir).await.unwrap();
        let routes = super::routes();
        let who: Vec<(String, Who)> = routes.who().to_vec();
        // Only a client device's requests over iroh carry `FromClient`, which the node puts on them: here, a header.
        let router = super::guarded(routes, host.clone()).layer(axum::middleware::from_fn(|mut req: axum::extract::Request, next: axum::middleware::Next| async move {
            if req.headers().contains_key("x-test-client") {
                req.extensions_mut().insert(super::FromClient([7; 32], u64::MAX));
            }
            next.run(req).await
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = listener.local_addr().unwrap();
        tokio::spawn(::api::serve(listener, router));
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
            let refused = status == 403 && body["code"] != custom_build::protocol::code::NOT_LET_IN;
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
