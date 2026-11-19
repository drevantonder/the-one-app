//! An Android phone's screen: its own H.264, from scrcpy's server running in it (github.com/Genymobile/scrcpy),
//! sent on as it comes, never decoded; what the viewer does goes to that server's control socket, which injects it
//! as touches, keys and text. The app pushes scrcpy's server to the phone first, at `/data/local/tmp/scrcpy-server.jar`.
//!
//! Buttons: 1 is a finger, 2 Home, 3 Back, 4 to 7 the wheel.
use crate::{Event, Source, input::Input};
use anyhow::{Context, bail};
use serde::Deserialize;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Deserialize)]
pub struct Config {
    /// adb's path.
    adb: String,
    /// The phone's, `emulator-5560`.
    serial: String,
    /// scrcpy's server's version, which it checks it's started with.
    version: String,
}

pub struct Scrcpy {
    control: TcpStream,
    server: Child,
    /// The video's width and height, which a touch says it's in.
    size: Arc<Mutex<(u16, u16)>>,
    at: (i32, i32),
    held: bool,
}

/// scrcpy's pointer id for a generic finger, -2.
const FINGER: u64 = 0xFFFF_FFFF_FFFF_FFFE;

impl Scrcpy {
    pub fn start(config: Config, events: Sender<Event>) -> anyhow::Result<Self> {
        let adb = |args: &[&str]| {
            let mut command = Command::new(&config.adb);
            command.args(["-s", &config.serial]).args(args).stdin(Stdio::null());
            command
        };
        // scrcpy's server connects to this end through adb (`reverse`): the video's socket, then the control's.
        let scid = format!("{:08x}", rand() & 0x7FFF_FFFF);
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let tunnel = format!("localabstract:scrcpy_{scid}");
        if !adb(&["reverse", &tunnel, &format!("tcp:{port}")]).status()?.success() {
            bail!("adb couldn't reach the phone");
        }
        // Its video is at most 1600 pixels tall, H.264's constrained baseline, which every browser decodes, and its
        // encoder makes a frame only when the screen changes.
        let server = adb(&[
            "shell",
            "CLASSPATH=/data/local/tmp/scrcpy-server.jar",
            "app_process",
            "/",
            "com.genymobile.scrcpy.Server",
            &config.version,
            &format!("scid={scid}"),
            "log_level=warn",
            "tunnel_forward=false",
            "audio=false",
            "control=true",
            "video_codec=h264",
            "max_size=1600",
            "max_fps=60",
            "video_bit_rate=6000000",
            "video_codec_options=profile=1",
            "send_device_meta=false",
            "send_dummy_byte=false",
            "stay_awake=true",
            "cleanup=true",
        ])
        .stdout(Stdio::null())
        .spawn()?;
        let (accepted, sockets) = mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..2 {
                let _ = accepted.send(listener.accept().map(|(socket, _)| socket));
            }
        });
        let take = || sockets.recv_timeout(Duration::from_secs(15)).context("scrcpy's server didn't connect").and_then(|s| Ok(s?));
        let video = take()?;
        let control = take()?;
        let _ = adb(&["reverse", "--remove", &tunnel]).status();

        let size = Arc::new(Mutex::new((0, 0)));
        let sized = size.clone();
        std::thread::spawn(move || {
            let why = frames(video, &sized, &events).map_or_else(|why| format!("the phone's video ended: {why}"), |()| "the phone's video ended".into());
            let _ = events.send(Event::Ended(why));
        });
        Ok(Self { control, server, size, at: (0, 0), held: false })
    }

    fn send(&mut self, message: &[u8]) {
        if let Err(why) = self.control.write_all(message) {
            eprintln!("scrcpy's control: {why}");
        }
    }

    /// A finger down (0), up (1) or moved (2).
    fn touch(&mut self, action: u8) {
        let (width, height) = *self.size.lock().unwrap();
        let mut m = vec![2, action];
        m.extend(FINGER.to_be_bytes());
        m.extend(self.at.0.to_be_bytes());
        m.extend(self.at.1.to_be_bytes());
        m.extend(width.to_be_bytes());
        m.extend(height.to_be_bytes());
        m.extend((if action == 1 { 0u16 } else { 0xFFFF }).to_be_bytes());
        m.extend(0u32.to_be_bytes()); // the action's button
        m.extend(0u32.to_be_bytes()); // the buttons held
        self.send(&m);
    }

    /// Android's key `code` pressed and let go, with X's modifiers `mods` as Android's meta state.
    fn key(&mut self, code: u32, mods: &[String]) {
        let meta = mods.iter().fold(0u32, |meta, m| {
            meta | match m.as_str() {
                "Shift_L" => 0x41,
                "Control_L" => 0x3000,
                "Alt_L" => 0x12,
                "Super_L" => 0x30000,
                _ => 0,
            }
        });
        for action in [0u8, 1] {
            let mut m = vec![0, action];
            m.extend(code.to_be_bytes());
            m.extend(0u32.to_be_bytes()); // repeat
            m.extend(meta.to_be_bytes());
            self.send(&m);
        }
    }
}

impl Source for Scrcpy {
    fn input(&mut self, input: Input) {
        match input {
            Input::Move { x, y } => {
                self.at = (x as i32, y as i32);
                if self.held {
                    self.touch(2);
                }
            }
            Input::Button { b: 1, down, .. } => {
                self.held = down;
                self.touch(if down { 0 } else { 1 });
            }
            Input::Button { b: 3, down, .. } => self.send(&[4, if down { 0 } else { 1 }]),
            Input::Button { b: 2, down: true, .. } => self.key(3, &[]),
            Input::Button { b: b @ 4..=7, down: true, .. } => {
                // One step of the wheel, as scrcpy's own sends it: 1 of its range of 16, up and left positive.
                let (h, v): (i16, i16) = [(0, 1), (0, -1), (1, 0), (-1, 0)][b as usize - 4];
                let (width, height) = *self.size.lock().unwrap();
                let mut m = vec![3];
                m.extend(self.at.0.to_be_bytes());
                m.extend(self.at.1.to_be_bytes());
                m.extend(width.to_be_bytes());
                m.extend(height.to_be_bytes());
                m.extend((h * 2048).to_be_bytes());
                m.extend((v * 2048).to_be_bytes());
                m.extend(0u32.to_be_bytes());
                self.send(&m);
            }
            Input::Button { .. } => {}
            Input::Key { key, mods } => {
                if let Some(code) = keycode(&key).or_else(|| keycode(&key.to_lowercase())) {
                    self.key(code, &mods);
                }
            }
            Input::Text { text } => {
                for (i, line) in text.split('\n').enumerate() {
                    if i > 0 {
                        self.key(66, &[]);
                    }
                    if !line.is_empty() {
                        let mut m = vec![1];
                        m.extend((line.len() as u32).to_be_bytes());
                        m.extend(line.as_bytes());
                        self.send(&m);
                    }
                }
            }
        }
    }

    /// scrcpy's server starts its encoder again, which begins with a whole frame.
    fn keyframe(&mut self) {
        self.send(&[17]);
    }
}

impl Drop for Scrcpy {
    fn drop(&mut self) {
        let _ = self.control.shutdown(std::net::Shutdown::Both);
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

/// Android's key code for a key X names.
fn keycode(name: &str) -> Option<u32> {
    let mut letters = name.chars();
    match (letters.next(), letters.next()) {
        (Some(c @ 'a'..='z'), None) => return Some(29 + (c as u32 - 'a' as u32)),
        (Some(c @ '0'..='9'), None) => return Some(7 + (c as u32 - '0' as u32)),
        _ => {}
    }
    Some(match name {
        "Return" => 66,
        "BackSpace" => 67,
        "Tab" => 61,
        "Escape" => 4,
        "Delete" => 112,
        "Left" => 21,
        "Right" => 22,
        "Up" => 19,
        "Down" => 20,
        "Home" => 122,
        "End" => 123,
        "Prior" => 92,
        "Next" => 93,
        "space" => 62,
        _ => return None,
    })
}

/// Packets off the video socket: a session's size, a config (SPS and PPS, put before the next frame), a frame.
fn frames(mut video: TcpStream, size: &Mutex<(u16, u16)>, events: &Sender<Event>) -> anyhow::Result<()> {
    let mut codec = [0; 4];
    video.read_exact(&mut codec)?;
    let mut config = vec![];
    loop {
        let mut header = [0; 12];
        video.read_exact(&mut header)?;
        let head = u64::from_be_bytes(header[..8].try_into()?);
        let length = u32::from_be_bytes(header[8..].try_into()?);
        if head >> 63 == 1 {
            // A session's: its flags, width and height, in the same 12 bytes.
            *size.lock().unwrap() = ((head & 0xFFFF_FFFF) as u16, length as u16);
            continue;
        }
        let mut data = vec![0; length as usize];
        video.read_exact(&mut data)?;
        if head >> 62 & 1 == 1 {
            config = data;
            continue;
        }
        let mut frame = std::mem::take(&mut config);
        frame.extend(data);
        if events.send(Event::Frame(frame)).is_err() {
            return Ok(());
        }
    }
}

/// A number no other stream's is likely to be, for scrcpy's id of a session.
fn rand() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    hasher.finish() as u32
}
