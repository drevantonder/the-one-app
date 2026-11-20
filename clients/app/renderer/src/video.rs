//! A page's `video` natively (renderer.ts's `video`): a screen's live video over WebRTC, as the web's browser
//! connects and plays one (clients/web/renderer/video.js). WebRTC is str0m's and H.264 OpenH264's, as WebRTC's sender
//! has them (apps/webrtc/server/bin/stream), so the two ends are one stack, built from source for every kind of
//! device, with no browser and no libwebrtc. Each connection is a thread of its own: it reaches the screen only
//! through a TURN server's relay, over TCP, as the sender does, offers to receive video and the data channel both ends
//! make (`input`, negotiated, id 0), takes the screen's answer, and decodes each frame that comes into the latest,
//! which the window draws (`Video::tile`). What it says the page hears as events (`offer`, `state`, `size`).

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::channel::{ChannelConfig, ChannelId};
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event as RtcEvent, IceConnectionState, Input as RtcInput, Output, Rtc};
use turn_client_proto::api::{TurnClientApi, TurnConfig, TurnEvent, TurnPollRet, TurnRecvRet};
use turn_client_proto::prelude::DelayedTransmitBuild;
use turn_client_proto::stun::agent::Transmit;
use turn_client_proto::tcp::TurnClientTcp;
use turn_client_proto::types::{TransportType, TurnCredentials};

use crate::geometry::{Bounds, DevicePixels, Size};
use crate::render::atlas::{Atlas, AtlasKey, AtlasTile};

/// How long the TURN server has to give an allocation, as the sender's.
const ALLOCATING: Duration = Duration::from_secs(4);

/// What a connection was given: its ICE servers, as `RTCConfiguration` has them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ice {
    ice_servers: Vec<IceServer>,
}

#[derive(Deserialize)]
struct IceServer {
    #[serde(deserialize_with = "one_or_many")]
    urls: Vec<String>,
    username: Option<String>,
    credential: Option<String>,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Urls {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Urls::deserialize(d)? {
        Urls::One(url) => vec![url],
        Urls::Many(urls) => urls,
    })
}

/// What reaches a connection's thread: bytes from the TURN server, or what its page did.
enum Command {
    Turn(Vec<u8>),
    TurnClosed,
    Answer(String),
    Send(String),
    Close,
}

/// What a connection has for its window: the latest frame, BGRA, as the atlas takes it, how many have come, and the
/// events its page is to hear.
#[derive(Default)]
struct Shared {
    frame: Option<(Size<u32>, Vec<u8>)>,
    came: u64,
    told: Vec<(&'static str, Value)>,
}

/// A `video` node: its connection, the last made, and what's drawn of it.
#[derive(Default)]
pub(crate) struct Video {
    commands: Option<Sender<Command>>,
    shared: Arc<Mutex<Shared>>,
    /// Looked at only: nothing done on it is sent.
    pub watch: bool,
    /// Its data channel is open, so what's done on it goes down it.
    pub connected: bool,
    /// Its video's own size, and where it was last drawn, in the window.
    pub size: Option<Size<u32>>,
    pub drawn: Option<Bounds<f32>>,
    /// It's been in its page, so it's closed once it's out of it (`Screen::sync_videos`).
    pub shown: bool,
    copied: u64,
}

impl Drop for Video {
    fn drop(&mut self) {
        self.close();
    }
}

impl Video {
    /// A new connection with `ice`, the last one closed; `wake` wakes its window as it has something for it. No `ice`
    /// is none.
    pub fn connect(&mut self, ice: &Value, wake: Option<Arc<dyn Fn() + Send + Sync>>) {
        self.close();
        self.connected = false;
        if ice.is_null() {
            return;
        }
        let (commands, received) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        self.shared = shared.clone();
        self.commands = Some(commands.clone());
        let ice = ice.clone();
        let _ = std::thread::Builder::new().name("video".into()).spawn(move || {
            let out = Out { shared, wake };
            match connection(ice, commands, &received, &out) {
                Ok(why) => eprintln!("A video's connection ended: {why}"),
                Err(why) => eprintln!("A video's connection failed: {why:#}"),
            }
            out.tell("state", json!({"state": "lost"}));
        });
    }

    fn close(&mut self) {
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(Command::Close);
        }
    }

    /// The screen's answer to its offer.
    pub fn answer(&self, sdp: &str) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::Answer(sdp.to_owned()));
        }
    }

    /// Sends what's done on it down its data channel, as JSON (core/ui/src/drawing-primitives/input.ts's `Input`),
    /// unless it's only looked at.
    pub fn send(&self, input: Value) {
        if let (false, true, Some(commands)) = (self.watch, self.connected, &self.commands) {
            let _ = commands.send(Command::Send(input.to_string()));
        }
    }

    /// The events its page is to hear since it was last asked, and whether a frame came.
    pub fn take(&mut self) -> (Vec<(&'static str, Value)>, bool) {
        let mut shared = lock(&self.shared);
        let told = std::mem::take(&mut shared.told);
        for (name, told) in &told {
            match *name {
                "state" => self.connected = told["state"] == "connected",
                "size" => {
                    self.size = Some(Size {
                        width: told["width"].as_u64().unwrap_or(0) as u32,
                        height: told["height"].as_u64().unwrap_or(0) as u32,
                    })
                }
                _ => {}
            }
        }
        (told, shared.came != self.copied)
    }

    /// Its latest frame, in its tile of the atlas, written there as it came, and its size, drawn in `content`.
    pub fn tile(&mut self, id: crate::tree::Id, content: Bounds<f32>, atlas: &Atlas) -> Option<(AtlasTile, Size<u32>)> {
        self.drawn = Some(content);
        let shared = lock(&self.shared);
        let (size, bgra) = shared.frame.as_ref()?;
        let size = *size;
        if size.width == 0 || size.height == 0 {
            return None;
        }
        use std::hash::{Hash as _, Hasher as _};
        let mut key = std::collections::hash_map::DefaultHasher::new();
        ("video", id, size.width, size.height).hash(&mut key);
        let padded = Size { width: DevicePixels(size.width as i32 + 2), height: DevicePixels(size.height as i32 + 2) };
        let empty = vec![0u8; (size.width as usize + 2) * (size.height as usize + 2) * 4];
        let tile = atlas
            .get_or_insert_with(&AtlasKey::Image(key.finish()), &mut || {
                Ok(Some((padded, std::borrow::Cow::Borrowed(&empty[..]))))
            })
            .ok()??;
        if self.copied != shared.came {
            self.copied = shared.came;
            atlas.write(
                &tile,
                Size { width: DevicePixels(size.width as i32), height: DevicePixels(size.height as i32) },
                bgra.clone(),
            );
        }
        Some((tile, size))
    }

    /// Where `at`, in the window, is on its screen, in its video's pixels, where it's drawn there as big as fits, in
    /// the middle of its box.
    pub fn at(&self, at: crate::geometry::Point<f32>) -> Option<(u32, u32)> {
        self.drawn?.contains(&at).then(|| self.at_clamped(at))?
    }

    /// Where `at` is on its screen, or its nearest edge, where it's outside it, as a press held there is.
    pub fn at_clamped(&self, at: crate::geometry::Point<f32>) -> Option<(u32, u32)> {
        let (drawn, size) = (self.drawn?, self.size?);
        if size.width == 0 || size.height == 0 {
            return None;
        }
        let scale = (drawn.size.width / size.width as f32).min(drawn.size.height / size.height as f32);
        let x = (at.x - drawn.origin.x - (drawn.size.width - size.width as f32 * scale) / 2.0) / scale;
        let y = (at.y - drawn.origin.y - (drawn.size.height - size.height as f32 * scale) / 2.0) / scale;
        Some((
            x.clamp(0.0, size.width as f32 - 1.0).round() as u32,
            y.clamp(0.0, size.height as f32 - 1.0).round() as u32,
        ))
    }
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a connection's thread gives its window, which is woken to take it.
struct Out {
    shared: Arc<Mutex<Shared>>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Out {
    /// An event its page is to hear.
    fn tell(&self, name: &'static str, told: Value) {
        lock(&self.shared).told.push((name, told));
        self.woken();
    }

    /// A frame, the latest.
    fn frame(&self, size: Size<u32>, bgra: Vec<u8>) {
        let mut shared = lock(&self.shared);
        shared.frame = Some((size, bgra));
        shared.came += 1;
        drop(shared);
        self.woken();
    }

    fn woken(&self) {
        if let Some(wake) = &self.wake {
            wake();
        }
    }
}

/// One connection, from its offer until it's closed or lost: gives why it ended.
fn connection(
    ice: Value,
    commands: Sender<Command>,
    received: &Receiver<Command>,
    out: &Out,
) -> anyhow::Result<String> {
    let started = Instant::now();
    let ice: Ice = serde_json::from_value(ice).context("its ICE servers")?;
    let mut turn = Turn::connect(&ice, commands)?;
    let Some(relay) = turn.allocate(received)? else { return Ok("it was closed".into()) };

    let mut rtc = Rtc::builder().clear_codecs().enable_h264(true).build(Instant::now());
    rtc.add_local_candidate(Candidate::relayed(relay, turn.local, "udp")?);
    let mut change = rtc.sdp_api();
    let mid = change.add_media(MediaKind::Video, Direction::RecvOnly, None, None, None);
    // Both ends make the data channel (id 0), so it opens with the connection, with nothing more to say.
    let channel = change.add_channel_with_config(ChannelConfig {
        label: "input".into(),
        negotiated: Some(0),
        ..Default::default()
    });
    let (offer, pending) = change.apply().ok_or_else(|| anyhow!("there was nothing to offer"))?;
    out.tell("offer", json!({"sdp": offer.to_sdp_string()}));
    let offered = started.elapsed();

    let timings = Timings { started, offered, answered: None };
    let mut session = Session {
        rtc,
        turn,
        relay,
        mid,
        channel,
        decoder: openh264::decoder::Decoder::new()?,
        size: None,
        pending: Some(pending),
        ended: None,
        timings,
    };
    session.run(received, out)
}

struct Session {
    rtc: Rtc,
    turn: Turn,
    relay: SocketAddr,
    mid: Mid,
    channel: ChannelId,
    decoder: openh264::decoder::Decoder,
    size: Option<(usize, usize)>,
    pending: Option<SdpPendingOffer>,
    ended: Option<String>,
    timings: Timings,
}

/// When its connection began, and how long after its offer was made and the screen's answer came, which the log says
/// once it's connected.
struct Timings {
    started: Instant,
    offered: Duration,
    answered: Option<Duration>,
}

impl Session {
    fn run(&mut self, received: &Receiver<Command>, out: &Out) -> anyhow::Result<String> {
        loop {
            if let Some(why) = self.ended.take() {
                return Ok(why);
            }
            let at = self.drain(out)?;
            while self.turn.client.poll_event().is_some() {}
            let wait = at.saturating_duration_since(Instant::now()).min(self.turn.wait());
            let command = if wait.is_zero() {
                received.try_recv().map_err(|_| RecvTimeoutError::Timeout)
            } else {
                received.recv_timeout(wait)
            };
            match command {
                Ok(Command::Turn(bytes)) => {
                    for (peer, data) in self.turn.receive(&bytes) {
                        let Ok(contents) = data.as_slice().try_into() else { continue };
                        let receive = Receive { proto: Protocol::Udp, source: peer, destination: self.relay, contents };
                        self.rtc.handle_input(RtcInput::Receive(Instant::now(), receive))?;
                        self.drain(out)?;
                    }
                    self.turn.flush()?;
                }
                Ok(Command::Answer(sdp)) => {
                    let Some(pending) = self.pending.take() else { continue };
                    self.timings.answered = Some(self.timings.started.elapsed());
                    // What the screen reaches the relay from, which the relay takes from only once permitted.
                    for ip in candidates(&sdp) {
                        self.turn.permit(ip);
                    }
                    self.rtc
                        .sdp_api()
                        .accept_answer(pending, SdpAnswer::from_sdp_string(&sdp).context("the screen's answer")?)?;
                    self.turn.flush()?;
                }
                Ok(Command::Send(input)) => {
                    if let Some(mut channel) = self.rtc.channel(self.channel) {
                        channel.write(false, input.as_bytes())?;
                    }
                }
                Ok(Command::TurnClosed) => return Ok("the TURN server closed its connection".into()),
                Ok(Command::Close) | Err(RecvTimeoutError::Disconnected) => return Ok("it was closed".into()),
                Err(RecvTimeoutError::Timeout) => {
                    self.rtc.handle_input(RtcInput::Timeout(Instant::now()))?;
                    self.turn.flush()?;
                }
            }
        }
    }

    /// Takes everything str0m has to do now, until the time it next has something: gives that time.
    fn drain(&mut self, out: &Out) -> anyhow::Result<Instant> {
        loop {
            match self.rtc.poll_output()? {
                Output::Timeout(at) => return Ok(at),
                Output::Transmit(transmit) => self.turn.send(transmit.destination, &transmit.contents)?,
                Output::Event(event) => match event {
                    RtcEvent::ChannelOpen(id, _) if id == self.channel => {
                        let Timings { started, offered, answered } = self.timings;
                        let ms = |at: Duration| at.as_millis();
                        eprintln!(
                            "a video connected in {} ms: offered at {} ms, through TURN, and answered at {} ms",
                            ms(started.elapsed()),
                            ms(offered),
                            answered.map_or(0, ms)
                        );
                        out.tell("state", json!({"state": "connected"}));
                    }
                    RtcEvent::ChannelClose(id) if id == self.channel => {
                        self.ended = Some("the screen closed its data channel".into())
                    }
                    RtcEvent::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                        self.ended = Some("the screen went".into())
                    }
                    RtcEvent::MediaData(data) => self.decode(&data.data, out),
                    _ => {}
                },
            }
        }
    }

    /// Decodes a frame into the latest, or asks for a whole one where it can't be.
    fn decode(&mut self, frame: &[u8], out: &Out) {
        use openh264::formats::YUVSource as _;
        let decoded = match self.decoder.decode(frame) {
            Ok(Some(yuv)) => yuv,
            Ok(None) => return,
            Err(_) => {
                if let Some(stream) = self.rtc.direct_api().stream_rx_by_mid(self.mid, None) {
                    stream.request_keyframe(KeyframeRequestKind::Pli);
                }
                return;
            }
        };
        let (width, height) = decoded.dimensions();
        let mut pixels = vec![0u8; width * height * 4];
        decoded.write_rgba8(&mut pixels);
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        if self.size != Some((width, height)) {
            self.size = Some((width, height));
            out.tell("size", json!({"width": width, "height": height}));
        }
        out.frame(Size { width: width as u32, height: height as u32 }, pixels);
    }
}

/// The IP addresses of the candidates in an SDP.
fn candidates(sdp: &str) -> HashSet<IpAddr> {
    sdp.lines()
        .filter_map(|line| line.strip_prefix("a=candidate:"))
        .filter_map(|candidate| candidate.split_whitespace().nth(4)?.parse().ok())
        .collect()
}

/// A TURN server's relay, reached over TCP, by its IPv4 address, as WebRTC's sender reaches it (its `session.rs`).
struct Turn {
    client: TurnClientTcp,
    stream: TcpStream,
    server: SocketAddr,
    local: SocketAddr,
    base: Instant,
    bound: HashSet<SocketAddr>,
}

impl Turn {
    /// A TCP connection to the first TURN server over TCP it was given, with its bytes read on a thread of their own.
    fn connect(ice: &Ice, commands: Sender<Command>) -> anyhow::Result<Self> {
        let (url, server) = ice
            .ice_servers
            .iter()
            .flat_map(|server| server.urls.iter().map(move |url| (url, server)))
            .find(|(url, _)| url.starts_with("turn:") && url.ends_with("transport=tcp"))
            .ok_or_else(|| anyhow!("no TURN server over TCP was given"))?;
        let host = url["turn:".len()..].split('?').next().unwrap_or_default();
        let address = host
            .to_socket_addrs()
            .with_context(|| format!("couldn't find {host}"))?
            .find(SocketAddr::is_ipv4)
            .ok_or_else(|| anyhow!("{host} has no IPv4 address"))?;
        let stream = TcpStream::connect_timeout(&address, ALLOCATING)
            .with_context(|| format!("couldn't reach TURN at {address}"))?;
        stream.set_nodelay(true)?;
        let mut reading = stream.try_clone()?;
        std::thread::Builder::new().name("video turn".into()).spawn(move || {
            let mut buffer = vec![0; 65536];
            loop {
                match reading.read(&mut buffer) {
                    Ok(0) | Err(_) => break drop(commands.send(Command::TurnClosed)),
                    Ok(n) => {
                        if commands.send(Command::Turn(buffer[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        })?;
        let local = stream.local_addr()?;
        let credentials =
            TurnCredentials::new(server.username.as_deref().unwrap_or(""), server.credential.as_deref().unwrap_or(""));
        let client = TurnClientTcp::allocate(local, address, TurnConfig::new(credentials));
        Ok(Self { client, stream, server: address, local, base: Instant::now(), bound: HashSet::new() })
    }

    fn now(&self) -> turn_client_proto::stun::Instant {
        turn_client_proto::stun::Instant::from_std(self.base)
    }

    /// Waits for the TURN server to give the relay's address, or none where it's closed first.
    fn allocate(&mut self, received: &Receiver<Command>) -> anyhow::Result<Option<SocketAddr>> {
        let until = Instant::now() + ALLOCATING;
        loop {
            self.flush()?;
            while let Some(event) = self.client.poll_event() {
                match event {
                    TurnEvent::AllocationCreated(_, relay) => return Ok(Some(relay)),
                    TurnEvent::AllocationCreateFailed(_) => bail!("the TURN server refused an allocation"),
                    _ => {}
                }
            }
            let wait = until.saturating_duration_since(Instant::now()).min(self.wait());
            match received.recv_timeout(wait) {
                Ok(Command::Turn(bytes)) => drop(self.receive(&bytes)),
                Ok(Command::TurnClosed) => bail!("the TURN server closed its connection"),
                Ok(Command::Close) | Err(RecvTimeoutError::Disconnected) => return Ok(None),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) if Instant::now() >= until => {
                    bail!("it couldn't reach TURN within {} seconds", ALLOCATING.as_secs())
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// How long until the client has something to do: a request to send again, or the allocation to refresh.
    fn wait(&mut self) -> Duration {
        match self.client.poll(self.now()) {
            TurnPollRet::WaitUntil(at) => at.to_std(self.base).saturating_duration_since(Instant::now()),
            _ => Duration::from_secs(1),
        }
    }

    /// Sends the client's requests.
    fn flush(&mut self) -> anyhow::Result<()> {
        let now = self.now();
        let _ = self.client.poll(now);
        while let Some(transmit) = self.client.poll_transmit(now) {
            self.stream.write_all(transmit.data.as_ref())?;
        }
        Ok(())
    }

    /// The peers' data in bytes from the TURN server, the rest being the client's own.
    fn receive(&mut self, bytes: &[u8]) -> Vec<(SocketAddr, Vec<u8>)> {
        let now = self.now();
        let mut data = vec![];
        if let TurnRecvRet::PeerData(peer) =
            self.client.recv(Transmit::new(bytes, TransportType::Tcp, self.server, self.local), now)
        {
            data.push((peer.peer, peer.data().to_vec()));
        }
        while let Some(peer) = self.client.poll_recv(now) {
            data.push((peer.peer, peer.data().to_vec()));
        }
        data
    }

    fn permit(&mut self, ip: IpAddr) {
        let _ = self.client.create_permission(TransportType::Udp, ip, self.now());
    }

    /// Sends `data` to `peer` through the relay, over a channel once the peer has one. One the relay doesn't let
    /// through yet is let through, and its packet dropped, as ICE sends again.
    fn send(&mut self, peer: SocketAddr, data: &[u8]) -> anyhow::Result<()> {
        let now = self.now();
        if !self.client.have_permission(TransportType::Udp, peer.ip()) {
            self.permit(peer.ip());
            return self.flush();
        }
        if self.bound.insert(peer) {
            let _ = self.client.bind_channel(TransportType::Udp, peer, now);
            self.flush()?;
        }
        if let Ok(Some(transmit)) = self.client.send_to(TransportType::Udp, peer, data, now) {
            self.stream.write_all(&transmit.data.build())?;
        }
        Ok(())
    }
}
