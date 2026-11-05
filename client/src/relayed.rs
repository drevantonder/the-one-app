//! A client with no socket, for a Worker's Durable Object, which has no UDP and holds no connection of its own: what
//! its endpoint sends goes to `send`, which passes it to the relay at the install's address by RPC, and the relay hands
//! what's sent to its key back by RPC, which goes to `Inbox::deliver`. To every other endpoint it's one more behind the
//! relay. The MCP Worker's devices are reached this way (cloudflare/mcp/).

use std::collections::VecDeque;
use std::io;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use iroh::endpoint::transports::{CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit};
use iroh_base::CustomAddr;

/// This transport's kind of address: an endpoint's key, reached through the relay. The serverless node's
/// (serverless/src/net.rs) is the same.
pub const THROUGH_RELAY: u64 = 0x6f6e;

/// What passes a datagram on to the relay: the key it's for, its segment size (0 for one datagram), and its bytes.
pub type Send = Arc<dyn Fn(&[u8; 32], u16, &[u8]) + std::marker::Send + Sync>;

pub fn through_relay(key: &[u8]) -> CustomAddr {
    CustomAddr::from_parts(THROUGH_RELAY, key)
}

/// The most datagrams kept for one endpoint until iroh reads them: what comes faster is dropped, as a full socket does.
const INBOX_MOST: usize = 1024;

/// Datagrams the relay handed in for one endpoint, until iroh reads them.
#[derive(Default)]
struct Queue {
    packets: VecDeque<(CustomAddr, Vec<u8>)>,
    waker: Option<Waker>,
}

/// Where the relay's datagrams for one endpoint go in.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Queue>>);

impl Inbox {
    /// Datagrams sent to this endpoint from `src`: one, or `segment_size` long each, back to back.
    pub fn deliver(&self, src: &[u8], segment_size: u32, contents: &[u8]) {
        let from = through_relay(src);
        let mut queue = self.0.lock().unwrap();
        let step = if segment_size == 0 { contents.len() } else { segment_size as usize };
        for chunk in contents.chunks(step.max(1)) {
            if queue.packets.len() >= INBOX_MOST {
                break;
            }
            queue.packets.push_back((from.clone(), chunk.to_vec()));
        }
        if let Some(waker) = queue.waker.take() {
            waker.wake();
        }
    }
}

#[derive(Clone)]
pub(crate) struct Relayed {
    me: CustomAddr,
    addrs: n0_watcher::Watchable<Vec<CustomAddr>>,
    inbox: Inbox,
    send: Send,
}

impl std::fmt::Debug for Relayed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Relayed")
    }
}

impl Relayed {
    pub(crate) fn new(me: &[u8; 32], inbox: Inbox, send: Send) -> Relayed {
        let me = through_relay(me);
        Relayed { addrs: n0_watcher::Watchable::new(vec![me.clone()]), me, inbox, send }
    }
}

impl CustomTransport for Relayed {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        Ok(Box::new(self.clone()))
    }
}

impl CustomEndpoint for Relayed {
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<CustomAddr>> {
        self.addrs.watch()
    }

    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(Sender(self.send.clone()))
    }

    fn max_transmit_segments(&self) -> NonZeroUsize {
        NonZeroUsize::new(20).unwrap()
    }

    fn poll_recv(&mut self, cx: &mut Context, bufs: &mut [io::IoSliceMut<'_>], metas: &mut [noq_udp::RecvMeta], infos: &mut [RecvInfo]) -> Poll<io::Result<usize>> {
        let mut queue = self.inbox.0.lock().unwrap();
        let mut n = 0;
        while n < bufs.len() {
            let Some((from, data)) = queue.packets.pop_front() else { break };
            if bufs[n].len() < data.len() {
                continue;
            }
            bufs[n][..data.len()].copy_from_slice(&data);
            metas[n].len = data.len();
            metas[n].stride = data.len();
            infos[n] = RecvInfo::new(from, Some(self.me.clone()));
            n += 1;
        }
        if n > 0 {
            return Poll::Ready(Ok(n));
        }
        queue.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

struct Sender(Send);

impl std::fmt::Debug for Sender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sender")
    }
}

impl CustomSender for Sender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        addr.id() == THROUGH_RELAY && addr.data().len() == 32
    }

    fn poll_send(&self, _cx: &mut Context, dst: &CustomAddr, _src: Option<&CustomAddr>, t: &Transmit<'_>) -> Poll<io::Result<()>> {
        if let Ok(to) = <[u8; 32]>::try_from(dst.data()) {
            (self.0)(&to, t.segment_size.unwrap_or(0) as u16, t.contents);
        }
        Poll::Ready(Ok(()))
    }
}
