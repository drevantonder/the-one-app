//! One viewer's connection: str0m's WebRTC, reaching the viewer only through a TURN server's relay, over TCP.
//!
//! TURN over TCP, by its IPv4 address: allocations over UDP to Cloudflare's get no answer from some machines, by
//! the port they're from, and one over IPv6 can go nowhere, where TCP over IPv4 always answers.
use crate::{Config, Event, Source, input::Input};
use anyhow::{Context, anyhow, bail};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use str0m::change::SdpOffer;
use str0m::channel::{ChannelConfig, ChannelId};
use str0m::format::Codec;
use str0m::media::{MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event as RtcEvent, IceConnectionState, Input as RtcInput, Output, Rtc};
use turn_client_proto::api::{TurnClientApi, TurnConfig, TurnEvent, TurnPollRet, TurnRecvRet};
use turn_client_proto::prelude::DelayedTransmitBuild;
use turn_client_proto::stun::agent::Transmit;
use turn_client_proto::tcp::TurnClientTcp;
use turn_client_proto::types::{TransportType, TurnCredentials};

/// How long the TURN server has to give an allocation, and the viewer to connect once answered.
const ALLOCATING: Duration = Duration::from_secs(4);
const CONNECTING: Duration = Duration::from_secs(30);

/// Runs the stream until the viewer goes, `answer`ing the offer once it can: gives why it ended.
pub fn run(config: Config, answer: &mut dyn FnMut(Result<String, String>)) -> anyhow::Result<String> {
    let (events, received) = mpsc::channel();
    let mut turn = Turn::connect(&config, events.clone())?;
    let relay = turn.allocate(&received)?;

    let mut rtc = Rtc::builder().clear_codecs().enable_h264(true).build(Instant::now());
    rtc.add_local_candidate(Candidate::relayed(relay, turn.local, "udp")?);
    let offer = SdpOffer::from_sdp_string(&config.offer).context("the viewer's offer")?;
    let sdp = rtc.sdp_api().accept_offer(offer)?.to_sdp_string();
    // Its data channel, which both ends make, so it opens with the connection.
    let channel = rtc.direct_api().create_data_channel(ChannelConfig {
        label: "input".into(),
        negotiated: Some(0),
        ..Default::default()
    });
    // What the viewer reaches the relay from, which the relay takes from only once permitted.
    for ip in candidates(&config.offer) {
        turn.permit(ip);
    }
    turn.flush()?;

    let source = crate::start(config.source, events)?;
    answer(Ok(sdp));
    let mut session = Session { rtc, turn, relay, source, channel, mid: None, connected: false, started: Instant::now(), ended: None };
    session.drain()?;
    session.run(&received)
}

/// The IP addresses of the candidates in an SDP.
fn candidates(sdp: &str) -> HashSet<IpAddr> {
    sdp.lines()
        .filter_map(|line| line.strip_prefix("a=candidate:"))
        .filter_map(|candidate| candidate.split_whitespace().nth(4)?.parse().ok())
        .collect()
}

struct Turn {
    client: TurnClientTcp,
    stream: TcpStream,
    server: SocketAddr,
    local: SocketAddr,
    base: Instant,
    bound: HashSet<SocketAddr>,
}

impl Turn {
    /// A TCP connection to the first TURN server over TCP the viewer was given, with its bytes read on a thread of
    /// their own into `events`.
    fn connect(config: &Config, events: Sender<Event>) -> anyhow::Result<Self> {
        let (url, server) = config
            .ice_servers
            .iter()
            .flat_map(|server| server.urls.iter().map(move |url| (url, server)))
            .find(|(url, _)| url.starts_with("turn:") && url.ends_with("transport=tcp"))
            .ok_or_else(|| anyhow!("no TURN server over TCP was given"))?;
        let host = url["turn:".len()..].split('?').next().unwrap();
        let address = host
            .to_socket_addrs()
            .with_context(|| format!("couldn't find {host}"))?
            .find(SocketAddr::is_ipv4)
            .ok_or_else(|| anyhow!("{host} has no IPv4 address"))?;
        let stream = TcpStream::connect_timeout(&address, ALLOCATING).with_context(|| format!("couldn't reach TURN at {address}"))?;
        stream.set_nodelay(true)?;
        let mut reading = stream.try_clone()?;
        std::thread::spawn(move || {
            let mut buffer = vec![0; 65536];
            loop {
                match reading.read(&mut buffer) {
                    Ok(0) | Err(_) => break events.send(Event::Ended("the TURN server closed its connection".into())),
                    Ok(n) => {
                        if events.send(Event::Turn(buffer[..n].to_vec())).is_err() {
                            break Ok(());
                        }
                    }
                }
            }
        });
        let local = stream.local_addr()?;
        let credentials = TurnCredentials::new(server.username.as_deref().unwrap_or(""), server.credential.as_deref().unwrap_or(""));
        let client = TurnClientTcp::allocate(local, address, TurnConfig::new(credentials));
        Ok(Self { client, stream, server: address, local, base: Instant::now(), bound: HashSet::new() })
    }

    fn now(&self) -> turn_client_proto::stun::Instant {
        turn_client_proto::stun::Instant::from_std(self.base)
    }

    /// Waits for the TURN server to give the relay's address.
    fn allocate(&mut self, events: &mpsc::Receiver<Event>) -> anyhow::Result<SocketAddr> {
        let until = Instant::now() + ALLOCATING;
        loop {
            self.flush()?;
            while let Some(event) = self.client.poll_event() {
                match event {
                    TurnEvent::AllocationCreated(_, relay) => return Ok(relay),
                    TurnEvent::AllocationCreateFailed(_) => bail!("the TURN server refused an allocation"),
                    _ => {}
                }
            }
            let wait = until.saturating_duration_since(Instant::now()).min(self.wait());
            match events.recv_timeout(wait) {
                Ok(Event::Turn(bytes)) => {
                    let _ = self.receive(&bytes);
                }
                Ok(Event::Ended(why)) => bail!(why),
                Ok(Event::Frame(_)) => {}
                Err(RecvTimeoutError::Timeout) if Instant::now() >= until => bail!("it couldn't reach TURN within {} seconds", ALLOCATING.as_secs()),
                Err(_) => {}
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
        if let TurnRecvRet::PeerData(peer) = self.client.recv(Transmit::new(bytes, TransportType::Tcp, self.server, self.local), now) {
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

    /// Sends `data` to `peer` through the relay, over a channel once the peer has one, which costs 4 bytes a packet
    /// where an indication costs 36. One the relay doesn't let through yet is let through, and its packet dropped,
    /// as ICE sends again.
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

struct Session {
    rtc: Rtc,
    turn: Turn,
    relay: SocketAddr,
    source: Box<dyn Source>,
    channel: ChannelId,
    mid: Option<Mid>,
    connected: bool,
    started: Instant,
    ended: Option<String>,
}

impl Session {
    fn run(&mut self, events: &mpsc::Receiver<Event>) -> anyhow::Result<String> {
        loop {
            if let Some(why) = self.ended.take() {
                return Ok(why);
            }
            if !self.connected && self.started.elapsed() > CONNECTING {
                return Ok("the viewer never connected".into());
            }
            let rtc = self.drain()?;
            // What happens to the relay's permissions and channels is the client's own business.
            while self.turn.client.poll_event().is_some() {}
            // What str0m has to send now (packets of a frame, a frame at a time) goes at once, not a wait later.
            let wait = rtc.saturating_duration_since(Instant::now()).min(self.turn.wait());
            let event = if wait.is_zero() { events.try_recv().map_err(|_| RecvTimeoutError::Timeout) } else { events.recv_timeout(wait) };
            match event {
                Ok(Event::Turn(bytes)) => {
                    for (peer, data) in self.turn.receive(&bytes) {
                        let Ok(contents) = data.as_slice().try_into() else { continue };
                        let receive = Receive { proto: Protocol::Udp, source: peer, destination: self.relay, contents };
                        self.rtc.handle_input(RtcInput::Receive(Instant::now(), receive))?;
                        self.drain()?;
                    }
                    self.turn.flush()?;
                }
                Ok(Event::Frame(frame)) => self.write(frame)?,
                Ok(Event::Ended(why)) => return Ok(why),
                Err(RecvTimeoutError::Timeout) => {
                    self.rtc.handle_input(RtcInput::Timeout(Instant::now()))?;
                    self.turn.flush()?;
                }
                Err(RecvTimeoutError::Disconnected) => return Ok("its source ended".into()),
            }
        }
    }

    /// Takes everything str0m has to do now, until the time it next has something: gives that time.
    fn drain(&mut self) -> anyhow::Result<Instant> {
        loop {
            match self.rtc.poll_output()? {
                Output::Timeout(at) => return Ok(at),
                Output::Transmit(transmit) => self.turn.send(transmit.destination, &transmit.contents)?,
                Output::Event(event) => match event {
                    RtcEvent::Connected => {
                        eprintln!("the viewer connected");
                        self.connected = true;
                        self.source.keyframe();
                    }
                    RtcEvent::IceConnectionStateChange(state) => {
                        eprintln!("ICE is {state:?}");
                        if state == IceConnectionState::Disconnected {
                            self.ended = Some("the viewer went".into());
                        }
                    }
                    RtcEvent::MediaAdded(media) => self.mid = Some(media.mid),
                    RtcEvent::KeyframeRequest(_) => self.source.keyframe(),
                    RtcEvent::ChannelData(data) if data.id == self.channel => match serde_json::from_slice::<Input>(&data.data) {
                        Ok(input) => self.source.input(input),
                        Err(why) => eprintln!("the viewer sent what isn't input: {why}"),
                    },
                    RtcEvent::ChannelClose(id) if id == self.channel => self.ended = Some("the viewer closed its data channel".into()),
                    _ => {}
                },
            }
        }
    }

    /// Sends a frame, timed as it's sent, once the viewer is there to see it.
    fn write(&mut self, frame: Vec<u8>) -> anyhow::Result<()> {
        let (true, Some(mid)) = (self.connected, self.mid) else { return Ok(()) };
        let Some(writer) = self.rtc.writer(mid) else { return Ok(()) };
        // The H.264 the viewer takes that's sent a NAL unit or more a packet, constrained baseline best.
        let h264: Vec<_> = writer.payload_params().filter(|p| p.spec().codec == Codec::H264 && p.spec().format.packetization_mode == Some(1)).collect();
        let Some(pt) = h264.iter().find(|p| p.spec().format.profile_level_id == Some(0x42e01f)).or(h264.first()).map(|p| p.pt()) else {
            bail!("the viewer doesn't take H.264");
        };
        let now = Instant::now();
        let time = MediaTime::from_90khz((now - self.started).as_micros() as u64 * 9 / 100);
        writer.write(pt, now, time, frame)?;
        self.drain()?;
        Ok(())
    }
}
