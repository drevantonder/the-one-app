//! An iOS Simulator's screen, idb's (fbidb.io): the H.264 its companion (`idb_companion`, started here for the
//! simulator and ended with the stream) encodes from the simulator's own display, a frame a message, sent on as it
//! comes, never decoded; what the viewer does is played on the simulator through the companion's HID, one event after another.
//! Both are its gRPC (idb's `idb.proto`, of which `proto` has what's used here).
//!
//! idb can't be asked for a whole frame: its video starts when the viewer connects, and starts again when the viewer
//! asks for one, which begins with one.
//!
//! Buttons: 1 is a finger, which taps, or swipes if it moved; 2 Home; 3 a long press; 4 to 7 the wheel, a swipe.
use crate::{Event, Source, input::Input};
use anyhow::{Context, anyhow, bail};
use serde::Deserialize;
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_stream::wrappers::UnboundedReceiverStream;

#[derive(Deserialize)]
pub struct Config {
    /// idb's companion's path.
    companion: String,
    udid: String,
    /// The simulator's points for each of its screen's pixels, which its touches are in.
    points: f64,
}

pub struct Idb {
    hid: UnboundedSender<proto::HidEvent>,
    video: UnboundedSender<()>,
    companion: Child,
    points: f64,
    at: (f64, f64),
    /// Where a finger went down, when, and with which button.
    down: Option<((f64, f64), Instant, u8)>,
}

impl Idb {
    pub fn start(config: Config, events: Sender<Event>) -> anyhow::Result<Self> {
        // It says its port as JSON once it's listening.
        let mut companion = Command::new(&config.companion)
            .args(["--udid", &config.udid, "--grpc-port", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .context("couldn't start idb's companion")?;
        let stdout = companion.stdout.take().unwrap();
        let (said, port) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(port) = serde_json::from_str::<serde_json::Value>(&line).ok().and_then(|v| v["grpc_port"].as_u64()) {
                    let _ = said.send(port);
                }
            }
        });
        let port = port.recv_timeout(Duration::from_secs(30)).map_err(|_| anyhow!("idb's companion didn't start"))?;

        let (hid, hid_events) = unbounded_channel();
        let (video, restarts) = unbounded_channel();
        std::thread::spawn(move || {
            let why = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|runtime| runtime.block_on(companion_calls(port, hid_events, restarts, &events)));
            let why = why.map_or_else(|why| format!("idb's companion: {why:#}"), |()| "idb's companion ended".into());
            let _ = events.send(Event::Ended(why));
        });
        Ok(Self { hid, video, companion, points: config.points, at: (0.0, 0.0), down: None })
    }

    fn send(&self, events: impl IntoIterator<Item = proto::HidEvent>) {
        for event in events {
            let _ = self.hid.send(event);
        }
    }

    /// A point of the video's pixels, in the simulator's points.
    fn point(&self, (x, y): (f64, f64)) -> proto::Point {
        proto::Point { x: (x * self.points).round(), y: (y * self.points).round() }
    }

    fn swipe(&self, from: proto::Point, to: proto::Point, duration: f64) {
        self.send([proto::event(proto::hid_event::Event::Swipe(proto::Swipe { start: Some(from), end: Some(to), delta: 0.0, duration }))]);
    }

    /// Presses and lets go of the keys `codes`, in order and then the other way.
    fn keys(&self, codes: &[u64]) {
        let press = |code, direction| proto::press(proto::press_action::Action::Key(proto::Key { keycode: code }), direction);
        self.send(codes.iter().map(|&c| press(c, proto::DOWN)).chain(codes.iter().rev().map(|&c| press(c, proto::UP))));
    }
}

impl Source for Idb {
    fn input(&mut self, input: Input) {
        match input {
            Input::Move { x, y } => self.at = (x, y),
            Input::Button { b: b @ (1 | 3), down: true, .. } => self.down = Some((self.at, Instant::now(), b)),
            Input::Button { b: 1 | 3, down: false, .. } => {
                let Some(((x0, y0), since, b)) = self.down.take() else { return };
                let (x1, y1) = self.at;
                let held = since.elapsed().as_secs_f64();
                if (x1 - x0).abs() + (y1 - y0).abs() > 12.0 {
                    self.swipe(self.point((x0, y0)), self.point((x1, y1)), held.max(0.1));
                } else {
                    let touch = |direction| proto::press(proto::press_action::Action::Touch(proto::Touch { point: Some(self.point((x1, y1))) }), direction);
                    let long = b == 3 || held > 0.5;
                    let delay = proto::event(proto::hid_event::Event::Delay(proto::Delay { duration: 1.0 }));
                    self.send([Some(touch(proto::DOWN)), long.then_some(delay), Some(touch(proto::UP))].into_iter().flatten());
                }
            }
            Input::Button { b: 2, down: true, .. } => {
                let home = |direction| proto::press(proto::press_action::Action::Button(proto::Button { button: proto::HOME }), direction);
                self.send([home(proto::DOWN), home(proto::UP)]);
            }
            Input::Button { b: b @ 4..=7, down: true, .. } => {
                // A step of the wheel is a swipe of 60 points the other way, as a finger scrolls.
                let (dx, dy) = [(0.0, 60.0), (0.0, -60.0), (60.0, 0.0), (-60.0, 0.0)][b as usize - 4];
                let from = self.point(self.at);
                let to = proto::Point { x: from.x + dx, y: from.y + dy };
                self.swipe(from, to, 0.1);
            }
            Input::Button { .. } => {}
            Input::Key { key, mods } => {
                if let Some(code) = keycode(&key).or_else(|| keycode(&key.to_lowercase())) {
                    let mut codes: Vec<u64> = mods.iter().filter_map(|m| modifier(m)).collect();
                    codes.push(code);
                    self.keys(&codes);
                }
            }
            Input::Text { text } => {
                for c in text.chars() {
                    match typed(c) {
                        Some((code, true)) => self.keys(&[SHIFT, code]),
                        Some((code, false)) => self.keys(&[code]),
                        None => eprintln!("idb has no key for {c:?}"),
                    }
                }
            }
        }
    }

    fn keyframe(&mut self) {
        let _ = self.video.send(());
    }
}

impl Drop for Idb {
    fn drop(&mut self) {
        let _ = self.companion.kill();
        let _ = self.companion.wait();
    }
}

/// The companion's HID, taking the events as they come, and its video, started as it's asked for: until either ends.
async fn companion_calls(
    port: u64,
    hid: tokio::sync::mpsc::UnboundedReceiver<proto::HidEvent>,
    mut restarts: tokio::sync::mpsc::UnboundedReceiver<()>,
    events: &Sender<Event>,
) -> anyhow::Result<()> {
    use tonic::codegen::http::uri::PathAndQuery;
    let channel = tonic::transport::Endpoint::from_shared(format!("http://127.0.0.1:{port}"))?.connect().await?;
    let mut playing = tonic::client::Grpc::new(channel.clone());
    playing.ready().await?;
    let mut hid = tokio::spawn(async move {
        playing
            .client_streaming::<_, _, proto::HidResponse, _>(
                tonic::Request::new(UnboundedReceiverStream::new(hid)),
                PathAndQuery::from_static("/idb.CompanionService/hid"),
                tonic_prost::ProstCodec::default(),
            )
            .await
    });

    // Nothing until the viewer connects, and asks for its first whole frame.
    if restarts.recv().await.is_none() {
        return Ok(());
    }
    loop {
        let started = Instant::now();
        let (control, requests) = unbounded_channel();
        let start = proto::VideoStreamRequest {
            control: Some(proto::video_stream_request::Control::Start(proto::Start { fps: 30, format: 0, compression_quality: 0.2, scale_factor: 1.0, ..Default::default() })),
        };
        control.send(start)?;
        let mut video = tonic::client::Grpc::new(channel.clone());
        video.ready().await?;
        let mut responses = video
            .streaming::<_, _, proto::VideoStreamResponse, _>(
                tonic::Request::new(UnboundedReceiverStream::new(requests)),
                PathAndQuery::from_static("/idb.CompanionService/video_stream"),
                tonic_prost::ProstCodec::default(),
            )
            .await?
            .into_inner();
        loop {
            tokio::select! {
                response = responses.message() => match response? {
                    Some(proto::VideoStreamResponse { output: Some(proto::video_stream_response::Output::Payload(proto::Payload { source: Some(proto::payload::Source::Data(data)) })) }) => {
                        if events.send(Event::Frame(data)).is_err() {
                            return Ok(());
                        }
                    }
                    Some(_) => {}
                    None => bail!("its video ended"),
                },
                // A whole frame asked for, at most once a second: the video starts again, which begins with one.
                restart = restarts.recv() => match restart {
                    None => return Ok(()),
                    Some(()) if started.elapsed() < Duration::from_secs(1) => {}
                    Some(()) => {
                        let _ = control.send(proto::VideoStreamRequest { control: Some(proto::video_stream_request::Control::Stop(proto::Stop {})) });
                        break;
                    }
                },
                ended = &mut hid => bail!("its HID ended: {:?}", ended.map(|r| r.err())),
            }
        }
    }
}

/// The USB HID usage of each key X names, which is how idb presses one.
fn keycode(name: &str) -> Option<u64> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        return typed(c).filter(|(_, shifted)| !shifted).map(|(code, _)| code);
    }
    Some(match name {
        "Return" => 40,
        "Escape" => 41,
        "BackSpace" => 42,
        "Tab" => 43,
        "space" => 44,
        "Delete" => 76,
        "Right" => 79,
        "Left" => 80,
        "Down" => 81,
        "Up" => 82,
        "Home" => 74,
        "End" => 77,
        "Prior" => 75,
        "Next" => 78,
        _ => return None,
    })
}

const SHIFT: u64 = 225;

/// Control, Shift, Alt and ⌘, as X names them.
fn modifier(name: &str) -> Option<u64> {
    Some(match name {
        "Control_L" => 224,
        "Shift_L" => SHIFT,
        "Alt_L" => 226,
        "Super_L" => 227,
        _ => return None,
    })
}

/// The key that types a character on a US keyboard, and whether it takes Shift: HID usages from 30 on, in order.
fn typed(c: char) -> Option<(u64, bool)> {
    const PLAIN: &str = "1234567890\n\x1b\x08\t -=[]\\\0;'`,./";
    const SHIFTED: &str = "!@#$%^&*()\0\0\0\0\0_+{}|\0:\"~<>?";
    match c {
        'a'..='z' => Some((4 + (c as u64 - 'a' as u64), false)),
        'A'..='Z' => Some((4 + (c as u64 - 'A' as u64), true)),
        '\0' => None,
        _ => PLAIN.chars().position(|p| p == c).map(|i| (30 + i as u64, false)).or_else(|| SHIFTED.chars().position(|p| p == c).map(|i| (30 + i as u64, true))),
    }
}

/// What's used here of idb's gRPC, `idb.proto` at v1.7.8, by its fields' numbers.
mod proto {
    pub const DOWN: i32 = 0;
    pub const UP: i32 = 1;
    pub const HOME: i32 = 1;

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Point {
        #[prost(double, tag = "1")]
        pub x: f64,
        #[prost(double, tag = "2")]
        pub y: f64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct HidEvent {
        #[prost(oneof = "hid_event::Event", tags = "1, 2, 3")]
        pub event: Option<hid_event::Event>,
    }

    pub mod hid_event {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Event {
            #[prost(message, tag = "1")]
            Press(super::Press),
            #[prost(message, tag = "2")]
            Swipe(super::Swipe),
            #[prost(message, tag = "3")]
            Delay(super::Delay),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Press {
        #[prost(message, optional, tag = "1")]
        pub action: Option<PressAction>,
        #[prost(int32, tag = "2")]
        pub direction: i32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PressAction {
        #[prost(oneof = "press_action::Action", tags = "1, 2, 3")]
        pub action: Option<press_action::Action>,
    }

    pub mod press_action {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Action {
            #[prost(message, tag = "1")]
            Touch(super::Touch),
            #[prost(message, tag = "2")]
            Button(super::Button),
            #[prost(message, tag = "3")]
            Key(super::Key),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Touch {
        #[prost(message, optional, tag = "1")]
        pub point: Option<Point>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Button {
        #[prost(int32, tag = "1")]
        pub button: i32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Key {
        #[prost(uint64, tag = "1")]
        pub keycode: u64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Swipe {
        #[prost(message, optional, tag = "1")]
        pub start: Option<Point>,
        #[prost(message, optional, tag = "2")]
        pub end: Option<Point>,
        #[prost(double, tag = "5")]
        pub delta: f64,
        #[prost(double, tag = "6")]
        pub duration: f64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Delay {
        #[prost(double, tag = "1")]
        pub duration: f64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct HidResponse {}

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct VideoStreamRequest {
        #[prost(oneof = "video_stream_request::Control", tags = "1, 2")]
        pub control: Option<video_stream_request::Control>,
    }

    pub mod video_stream_request {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Control {
            #[prost(message, tag = "1")]
            Start(super::Start),
            #[prost(message, tag = "2")]
            Stop(super::Stop),
        }
    }

    /// H.264 is format 0.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Start {
        #[prost(string, tag = "1")]
        pub file_path: String,
        #[prost(uint64, tag = "2")]
        pub fps: u64,
        #[prost(int32, tag = "3")]
        pub format: i32,
        #[prost(double, tag = "4")]
        pub compression_quality: f64,
        #[prost(double, tag = "5")]
        pub scale_factor: f64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Stop {}

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct VideoStreamResponse {
        #[prost(oneof = "video_stream_response::Output", tags = "1, 2")]
        pub output: Option<video_stream_response::Output>,
    }

    pub mod video_stream_response {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Output {
            #[prost(bytes, tag = "1")]
            LogOutput(Vec<u8>),
            #[prost(message, tag = "2")]
            Payload(super::Payload),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Payload {
        #[prost(oneof = "payload::Source", tags = "2")]
        pub source: Option<payload::Source>,
    }

    pub mod payload {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Source {
            #[prost(bytes, tag = "2")]
            Data(Vec<u8>),
        }
    }

    pub fn event(event: hid_event::Event) -> HidEvent {
        HidEvent { event: Some(event) }
    }

    pub fn press(action: press_action::Action, direction: i32) -> HidEvent {
        event(hid_event::Event::Press(Press { action: Some(PressAction { action: Some(action) }), direction }))
    }
}
