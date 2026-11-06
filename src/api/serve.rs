//! What serves the local API on a port: until the node hands over to the next one.

use axum::Router;
use axum::response::Response;

/// Serves the local API on `listener`, knowing each request's peer, which `from_this_machine` checks.
pub async fn serve(listener: tokio::net::TcpListener, router: Router) -> std::io::Result<()> {
    serve_until(listener, router, std::future::pending()).await
}

/// `serve`, until `stop`: then it takes no more connections, and returns once those it has are answered, or after
/// `handoff::drain_within`. As a node hands over, it stops taking connections at once, leaving them to the next, and
/// its answers say to close theirs for five seconds before it closes those left, so clients that keep one don't send on
/// a closed one.
pub async fn serve_until(listener: tokio::net::TcpListener, router: Router, stop: impl std::future::Future<Output = ()> + Send + 'static) -> std::io::Result<()> {
    let (stopped, after) = tokio::sync::oneshot::channel::<()>();
    let router = router.layer(axum::middleware::map_response(|mut resp: Response| async move {
        if crate::handoff::stopped() {
            resp.headers_mut().insert(axum::http::header::CONNECTION, axum::http::HeaderValue::from_static("close"));
        }
        resp
    }));
    // Tapped only so its connections still say their peer.
    let listener = axum::serve::ListenerExt::tap_io(UntilHandover(listener), |_| {});
    let serving = axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(async move {
        stop.await;
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let _ = stopped.send(());
    });
    let serving = std::future::IntoFuture::into_future(serving);
    let deadline = async {
        let _ = after.await;
        tokio::time::sleep(crate::handoff::drain_within()).await;
    };
    tokio::select! {
        served = serving => served,
        _ = deadline => Ok(()),
    }
}

/// A listener that stops taking connections once the node hands over, for the next one to take.
struct UntilHandover(tokio::net::TcpListener);

impl axum::serve::Listener for UntilHandover {
    type Io = tokio::net::TcpStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        tokio::select! {
            accepted = axum::serve::Listener::accept(&mut self.0) => accepted,
            _ = crate::handoff::stopping() => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.0.local_addr()
    }
}
