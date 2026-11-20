//! `stream <file>`: a screen as live video to one viewer over WebRTC, with what the viewer does on it back over the
//! data channel, for every app that shows a screen (WebRTC's `SKILL.md`).
//!
//! The file is JSON, `{"offer": <the viewer's SDP offer>, "iceServers": [<as RTCPeerConnection takes them>], "log":
//! <a file>, "source": {"kind": ..., ...}}`, and is removed once read. It prints the SDP answer and carries on in a
//! process of its own until the viewer goes, saying what happened in the log. Both ends reach each other only
//! through TURN (`relay`), here over TCP, so neither learns the other's addresses.
//!
//! The source is where the frames come from and where what the viewer does goes (`source`): `scrcpy`, an Android
//! phone's own H.264 from scrcpy's server; `x11`, a Linux display's, encoded here.
//!
//! The data channel (id 0, negotiated) carries what the UI library's LiveScreen sends (`input::Input`).
mod input;
mod scrcpy;
mod session;
#[cfg(target_os = "linux")]
mod x11;

use serde::Deserialize;
use std::sync::mpsc::Sender;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    offer: String,
    ice_servers: Vec<IceServer>,
    log: Option<String>,
    source: SourceConfig,
}

#[derive(Deserialize)]
pub struct IceServer {
    #[serde(deserialize_with = "one_or_many")]
    urls: Vec<String>,
    username: Option<String>,
    credential: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SourceConfig {
    Scrcpy(scrcpy::Config),
    #[cfg(target_os = "linux")]
    X11(x11::Config),
}

/// What reaches a session from elsewhere: a frame, or the source having ended.
pub enum Event {
    /// Bytes from the TURN server.
    Turn(Vec<u8>),
    /// An access unit of H.264, Annex B.
    Frame(Vec<u8>),
    Ended(String),
}

/// Where the frames come from, and what the viewer does goes.
pub trait Source {
    fn input(&mut self, input: input::Input);
    /// The viewer lost a frame, or has just connected: the next is to be a whole one.
    fn keyframe(&mut self);
}

fn start(config: SourceConfig, events: Sender<Event>) -> anyhow::Result<Box<dyn Source>> {
    Ok(match config {
        SourceConfig::Scrcpy(config) => Box::new(scrcpy::Scrcpy::start(config, events)?),
        #[cfg(target_os = "linux")]
        SourceConfig::X11(config) => Box::new(x11::X11::start(config, events)?),
    })
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

fn main() {
    let path = std::env::args().nth(1).expect("usage: stream <file>");
    let config: Config = serde_json::from_slice(&std::fs::read(&path).expect("can't read the file")).expect("the file isn't a stream's");
    let _ = std::fs::remove_file(&path);
    std::process::exit(answered(config));
}

/// The answer comes back up a pipe, and the stream carries on in a process of its own once it has, so the command
/// that started it ends.
#[cfg(unix)]
fn answered(config: Config) -> i32 {
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;

    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    // Closed in what the stream starts (adb, say), or the command would wait for them to end, not for the answer.
    for fd in fds {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    match unsafe { libc::fork() } {
        -1 => panic!("couldn't fork"),
        0 => {
            unsafe {
                libc::close(fds[0]);
                libc::setsid();
                let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
                libc::dup2(null, 0);
                if let Some(log) = &config.log {
                    let log = std::ffi::CString::new(log.as_str()).unwrap();
                    let fd = libc::open(log.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND, 0o644);
                    libc::dup2(fd, 1);
                    libc::dup2(fd, 2);
                }
            }
            let mut pipe = Some(unsafe { std::fs::File::from_raw_fd(fds[1]) });
            let mut answer = |response: Result<String, String>| {
                if let Some(mut pipe) = pipe.take() {
                    let response = match response {
                        Ok(sdp) => serde_json::json!({"answer": sdp}),
                        Err(why) => serde_json::json!({"error": why}),
                    };
                    let _ = pipe.write_all(response.to_string().as_bytes());
                }
            };
            match session::run(config, &mut answer) {
                Ok(why) => eprintln!("the stream ended: {why}"),
                Err(why) => {
                    eprintln!("the stream failed: {why:#}");
                    answer(Err(format!("{why:#}")));
                }
            }
            0
        }
        _ => {
            unsafe { libc::close(fds[1]) };
            let mut response = String::new();
            let _ = unsafe { std::fs::File::from_raw_fd(fds[0]) }.read_to_string(&mut response);
            let response: serde_json::Value =
                serde_json::from_str(&response).unwrap_or_else(|_| serde_json::json!({"error": "it stopped before it answered"}));
            match (response["answer"].as_str(), response["error"].as_str()) {
                (Some(sdp), _) => {
                    print!("{sdp}");
                    0
                }
                (_, why) => {
                    eprint!("{}", why.unwrap_or("it stopped before it answered"));
                    1
                }
            }
        }
    }
}

/// No source streams from Windows yet.
#[cfg(not(unix))]
fn answered(_config: Config) -> i32 {
    eprint!("A stream can't be sent from Windows yet");
    1
}
