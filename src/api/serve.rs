//! What serves the local API on a port.

use axum::Router;

/// Serves the local API on `listener`, knowing each request's peer, which `from_this_machine` checks.
pub async fn serve(listener: tokio::net::TcpListener, router: Router) -> std::io::Result<()> {
    // Tapped only so its connections still say their peer.
    let listener = axum::serve::ListenerExt::tap_io(listener, |_| {});
    axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await
}
