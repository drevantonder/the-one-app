//! iroh's relay protocol for the relay Worker: what checks an endpoint's handshake and reads and writes its frames.
//!
//! http.rs, key_cache.rs and protos/{common,handshake,relay}.rs are iroh-relay 1.4.1's own (github.com/n0-computer/iroh,
//! MIT OR Apache-2.0), copied unmodified: the crate keeps its server half `pub(crate)` behind a `server` feature that
//! needs tokio's network runtime, which doesn't build for WebAssembly. This file is only the few crate-level items
//! they import, and what the Worker calls.
#![allow(unexpected_cfgs, dead_code, unused_imports)]

use bytes::{BufMut, Bytes, BytesMut};
use iroh_base::PublicKey;
use wasm_bindgen::prelude::*;

pub mod http;
mod key_cache;
pub use key_cache::KeyCache;

pub mod protos {
    pub mod common;
    pub mod handshake;
    pub mod relay;
    /// The one item handshake.rs imports from streams.rs, whose bodies are tokio's or the browser's WebSocket.
    pub mod streams {
        pub type StreamError = n0_error::AnyError;
        pub trait BytesStreamSink:
            n0_future::Stream<Item = Result<bytes::Bytes, StreamError>> + n0_future::Sink<bytes::Bytes, Error = StreamError> + Unpin
        {
        }
    }
}

/// lib.rs's trait, as handshake.rs names it.
pub trait ExportKeyingMaterial {
    fn export_keying_material<T: AsMut<[u8]>>(&self, output: T, label: &[u8], context: Option<&[u8]>) -> Option<T>;
}

/// What handshake.rs's server half names from server.rs: only its `authorize_with` uses them, which the Worker doesn't.
pub mod server {
    use std::sync::Arc;
    pub enum Access {
        Allow,
        Deny { reason: Option<String> },
    }
    pub struct ClientRequest;
    pub trait DynAccessControl {
        fn on_connect<'a>(&'a self, _r: &'a ClientRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = Access> + 'a>>;
    }
    pub struct OnDisconnectGuard;
    impl OnDisconnectGuard {
        pub fn for_access_control(_a: Arc<dyn DynAccessControl>, _r: &ClientRequest) -> Self {
            Self
        }
    }
}

use protos::common::FrameType;
use protos::handshake::{ClientAuth, ServerChallenge};
use protos::relay::{ClientToRelayMsg, RelayToClientMsg};

fn frame<T: serde::Serialize>(tag: FrameType, body: &T) -> Vec<u8> {
    let mut out = BytesMut::new();
    tag.write_to(&mut out);
    postcard::to_io(body, out.writer()).expect("serializes").into_inner().to_vec()
}

/// A new challenge: its 16 bytes, then the ServerChallenge frame that sends it.
#[wasm_bindgen]
pub fn challenge() -> Vec<u8> {
    let c = ServerChallenge::new(&mut rand::rng());
    let mut out = c.challenge.to_vec();
    out.extend(frame(FrameType::ServerChallenge, &c));
    out
}

/// Checks a ClientAuth frame against the challenge it answers, as iroh does: → the endpoint's key.
#[wasm_bindgen]
pub fn verify(challenge: &[u8], msg: &[u8]) -> Result<Vec<u8>, JsError> {
    let mut msg = Bytes::copy_from_slice(msg);
    let tag = FrameType::from_bytes(&mut msg).map_err(|e| JsError::new(&e.to_string()))?;
    if tag != FrameType::ClientAuth {
        return Err(JsError::new("expected ClientAuth"));
    }
    let auth: ClientAuth = postcard::from_bytes(&msg).map_err(|e| JsError::new(&e.to_string()))?;
    let challenge = ServerChallenge { challenge: challenge.try_into().map_err(|_| JsError::new("challenge"))? };
    auth.verify(&challenge).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(auth.public_key.as_bytes().to_vec())
}

#[wasm_bindgen]
pub fn confirm() -> Vec<u8> {
    frame(FrameType::ServerConfirmsAuth, &())
}

/// ServerDeniesAuth is `{ reason: String }`, which postcard writes as the string alone.
#[wasm_bindgen]
pub fn deny(reason: &str) -> Vec<u8> {
    frame(FrameType::ServerDeniesAuth, &reason)
}

thread_local!(static KEYS: KeyCache = KeyCache::new(1024));

/// A frame from the endpoint `src`: `[0, dst (32 bytes), the frame for dst]` to pass on, `[1, frame]` to answer it
/// with, or `[2]` for nothing to do.
#[wasm_bindgen]
pub fn from_client(src: &[u8], msg: &[u8]) -> Result<Vec<u8>, JsError> {
    let src = PublicKey::try_from(src).map_err(|e| JsError::new(&e.to_string()))?;
    let msg = KEYS.with(|k| ClientToRelayMsg::from_bytes(Bytes::copy_from_slice(msg), k)).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(match msg {
        ClientToRelayMsg::Datagrams { dst_endpoint_id, datagrams } => {
            let mut out = vec![0u8];
            out.extend_from_slice(dst_endpoint_id.as_bytes());
            RelayToClientMsg::Datagrams { remote_endpoint_id: src, datagrams }.write_to(&mut out);
            out
        }
        ClientToRelayMsg::Ping(data) => {
            let mut out = vec![1u8];
            RelayToClientMsg::Pong(data).write_to(&mut out);
            out
        }
        _ => vec![2u8],
    })
}

/// The frame telling an endpoint that `id`, which it sent to, isn't connected.
#[wasm_bindgen]
pub fn endpoint_gone(id: &[u8]) -> Result<Vec<u8>, JsError> {
    let id = PublicKey::try_from(id).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(RelayToClientMsg::EndpointGone(id).write_to(Vec::new()))
}

/// The datagrams in a client's Datagrams frame, for an endpoint that isn't a socket (the serverless node): its
/// segment size (2 bytes, 0 for one datagram), then their contents.
#[wasm_bindgen]
pub fn datagrams_of(msg: &[u8]) -> Result<Vec<u8>, JsError> {
    let msg = KEYS.with(|k| ClientToRelayMsg::from_bytes(Bytes::copy_from_slice(msg), k)).map_err(|e| JsError::new(&e.to_string()))?;
    let ClientToRelayMsg::Datagrams { datagrams, .. } = msg else { return Err(JsError::new("not datagrams")) };
    let mut out = datagrams.segment_size.map_or(0, |s| s.get()).to_be_bytes().to_vec();
    out.extend_from_slice(&datagrams.contents);
    Ok(out)
}

/// The frame a socket gets for datagrams from `src`, an endpoint that isn't a socket.
#[wasm_bindgen]
pub fn to_client(src: &[u8], segment_size: u16, contents: &[u8]) -> Result<Vec<u8>, JsError> {
    let src = PublicKey::try_from(src).map_err(|e| JsError::new(&e.to_string()))?;
    let datagrams = protos::relay::Datagrams { ecn: None, segment_size: std::num::NonZeroU16::new(segment_size), contents: Bytes::copy_from_slice(contents) };
    Ok(RelayToClientMsg::Datagrams { remote_endpoint_id: src, datagrams }.write_to(Vec::new()))
}
