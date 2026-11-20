//! `stream <file>`: a screen as live video to one viewer over WebRTC, with what the viewer does on it back over the
//! data channel, for every app that shows a screen (WebRTC's `SKILL.md`).
//!
//! The file is JSON, `{"offer": <the viewer's SDP offer>, "iceServers": [<as RTCPeerConnection takes them>], "log":
//! <a file>, "source": {"kind": ..., ...}}`, and is removed once read. It prints the SDP answer and carries on in a
//! process of its own (`stream --serve`) until the viewer goes, saying what happened in the log. Both ends reach each other only
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
    match std::env::args().nth(1).as_deref() {
        Some("--serve") => serve(),
        Some(path) => std::process::exit(started(path)),
        None => {
            eprintln!("usage: stream <file>");
            std::process::exit(64);
        }
    }
}

/// Starts the stream in a process of its own, which carries on once it has answered, so this command ends with the
/// answer: the file goes to it on its stdin, and its answer comes back as a line of JSON on its stdout.
fn started(path: &str) -> i32 {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};

    let answered = (|| -> anyhow::Result<String> {
        let file = std::fs::read(path)?;
        let _ = std::fs::remove_file(path);
        let log = serde_json::from_slice::<serde_json::Value>(&file)?["log"].as_str().map(String::from);
        let stderr = match log.and_then(|log| std::fs::OpenOptions::new().create(true).append(true).open(log).ok()) {
            Some(log) => Stdio::from(log),
            None => Stdio::null(),
        };
        let mut command = Command::new(std::env::current_exe()?);
        command.arg("--serve").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(stderr);
        detach(&mut command);
        let mut child = command.spawn()?;
        child.stdin.take().unwrap().write_all(&file)?;
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line)?;
        let response: serde_json::Value = serde_json::from_str(&line).map_err(|_| anyhow::anyhow!("it stopped before it answered"))?;
        match (response["answer"].as_str(), response["error"].as_str()) {
            (Some(sdp), _) => Ok(sdp.to_string()),
            (_, why) => anyhow::bail!("{}", why.unwrap_or("it stopped before it answered")),
        }
    })();
    match answered {
        Ok(sdp) => {
            print!("{sdp}");
            0
        }
        Err(why) => {
            eprint!("{why:#}");
            1
        }
    }
}

/// Out of the session and the job of whatever started it, so it carries on once that ends.
#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    // DETACHED_PROCESS, CREATE_NEW_PROCESS_GROUP, CREATE_BREAKAWAY_FROM_JOB
    command.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0100_0000);
}

/// `stream --serve`: the stream itself, its config on stdin, answering on stdout and saying what happens on stderr.
fn serve() {
    use std::io::Write;

    let mut answering = Some(std::io::stdout());
    let mut answer = |response: Result<String, String>| {
        if let Some(mut out) = answering.take() {
            let response = match response {
                Ok(sdp) => serde_json::json!({"answer": sdp}),
                Err(why) => serde_json::json!({"error": why}),
            };
            let _ = writeln!(out, "{response}");
            let _ = out.flush();
            // Nothing more goes up the pipe, whose reader has gone: what's said, here or by what the stream starts,
            // goes to the log.
            #[cfg(unix)]
            unsafe {
                libc::dup2(2, 1);
            }
        }
    };
    let config = match serde_json::from_reader::<_, Config>(std::io::stdin()) {
        Ok(config) => config,
        Err(why) => return answer(Err(format!("the file isn't a stream's: {why}"))),
    };
    match session::run(config, &mut answer) {
        Ok(why) => eprintln!("the stream ended: {why}"),
        Err(why) => {
            eprintln!("the stream failed: {why:#}");
            answer(Err(format!("{why:#}")));
        }
    }
}
