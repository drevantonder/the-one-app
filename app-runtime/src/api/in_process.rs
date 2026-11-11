//! The node's own routes, asked in the node itself (`api::Client`), as the request its MCP answers asked them.

use std::io::Read;
use std::time::Duration;

use anyhow::Result;
use ::api::client::{Answer, Via};

use crate::Host;

/// The node's own routes, asked in the node itself as the request its MCP answers asked them: with that request's proof
/// of who asks (the client device its connection proved, or this machine's own program), which
/// `only_this_machine` checks for each, as it would over HTTP.
pub struct InProcess {
    router: axum::Router,
    client: Option<super::FromClient>,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    headers: axum::http::HeaderMap,
    runtime: tokio::runtime::Handle,
}

impl InProcess {
    /// As a request that came with `client` (over iroh), from `peer` (over TCP), and `headers`. Called in the runtime.
    pub fn new(host: Host, client: Option<super::FromClient>, peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>, headers: &axum::http::HeaderMap) -> InProcess {
        let proof = [axum::http::header::AUTHORIZATION.as_str(), custom_build::protocol::header::LOCAL_TOKEN, custom_build::protocol::header::READS];
        let headers = headers.iter().filter(|(n, _)| proof.contains(&n.as_str())).map(|(n, v)| (n.clone(), v.clone())).collect();
        InProcess { router: super::guarded(super::routes(), host), client, peer, headers, runtime: tokio::runtime::Handle::current() }
    }
}

impl Via for InProcess {
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, _wait: Option<Duration>) -> Result<Answer> {
        use hyper::service::Service;
        let mut req = axum::http::Request::builder().method(method).uri(path).body(axum::body::Body::from(body.unwrap_or_default()))?;
        req.headers_mut().extend(self.headers.clone());
        for (name, value) in headers {
            req.headers_mut().insert(axum::http::HeaderName::try_from(*name)?, value.parse()?);
        }
        if let Some(client) = self.client {
            req.extensions_mut().insert(client);
        }
        if let Some(peer) = self.peer {
            req.extensions_mut().insert(peer);
        }
        let service = hyper_util::service::TowerToHyperService::new(self.router.clone());
        let (parts, body) = self.runtime.block_on(service.call(req))?.into_parts();
        // Its body as it comes: the node's events stream until whoever reads them stops.
        let (sending, chunks) = std::sync::mpsc::channel();
        self.runtime.spawn(async move {
            use tokio_stream::StreamExt;
            let mut body = body.into_data_stream();
            while let Some(Ok(chunk)) = body.next().await {
                if sending.send(chunk).is_err() {
                    break;
                }
            }
        });
        let trace = parts.headers.get(custom_build::protocol::header::TRACE).and_then(|h| h.to_str().ok()).map(str::to_string);
        Ok(Answer { status: parts.status.as_u16(), trace, body: Box::new(Streamed { chunks, part: Default::default() }) })
    }
}

struct Streamed {
    chunks: std::sync::mpsc::Receiver<axum::body::Bytes>,
    part: axum::body::Bytes,
}

impl Read for Streamed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.part.is_empty() {
            match self.chunks.recv() {
                Ok(chunk) => self.part = chunk,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.part.len());
        buf[..n].copy_from_slice(&self.part[..n]);
        self.part = self.part.slice(n..);
        Ok(n)
    }
}
