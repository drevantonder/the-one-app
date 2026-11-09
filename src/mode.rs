//! What this process is for: the user's own network (`Own`), or a throwaway network's (`one-dev throwaway`), which has a home
//! and address of its own and must touch nothing of the real one's (`~/.local/bin`, the harnesses' configs, git's config).
//! `one-dev throwaway` tells every program it starts, by `Throwaway::tell`, and it's read here once, as the program starts: no code
//! asks the environment, settings or a flag whether this is one.
//!
//! A throwaway network's node also serves its bucket (an S3 stand-in, the folder its node made the network in) and
//! stops itself when nobody has asked it anything for an hour; `one-dev throwaway` starts it again as it was.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;


/// How long a throwaway network's node runs with nobody asking it anything.
const IDLE: Duration = Duration::from_secs(60 * 60);

pub enum Mode {
    Own,
    Throwaway(Throwaway),
}

/// A throwaway network, by its folder: its home, its programs' logs and pids, its data.
pub struct Throwaway {
    home: PathBuf,
}

const UI: &str = "ui-folder";

fn var() -> String {
    crate::config::env("THROWAWAY")
}

pub fn here() -> &'static Mode {
    static HERE: OnceLock<Mode> = OnceLock::new();
    HERE.get_or_init(|| match std::env::var_os(var()) {
        Some(home) => Mode::Throwaway(Throwaway { home: home.into() }),
        None => Mode::Own,
    })
}

impl Mode {
    /// Where the programs of services are put that a node fetches: `~/.local/bin`, but a throwaway
    /// network's in its home, so what it puts there never replaces the real network's program.
    pub fn bin(&self) -> PathBuf {
        match self {
            Mode::Throwaway(t) => t.home.join("bin"),
            Mode::Own => crate::user_home().join(".local/bin"),
        }
    }
}

impl Throwaway {
    /// Makes `command` a program of the throwaway network in `home`, whose node is at `addr`: its home and address, which
    /// it asks its node at as one of this machine's programs, and its mode.
    pub fn tell(command: &mut std::process::Command, home: &Path, addr: &str) {
        command.env(var(), home).env(crate::config::env("HOME"), home).env(crate::config::env("ADDR"), addr);
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The folder its pages and Workers are read from instead of what its build holds: a checkout's root.
    pub fn ui(&self) -> Option<PathBuf> {
        Self::ui_in(&self.home)
    }

    /// `ui`, for the throwaway network in `home`.
    pub fn ui_in(home: &Path) -> Option<PathBuf> {
        Some(std::fs::read_to_string(home.join(UI)).ok()?.trim().into())
    }

    /// Says which folder that is for the throwaway network in `home`, read by its programs.
    pub fn set_ui(home: &Path, folder: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(home)?;
        std::fs::write(home.join(UI), folder.to_string_lossy().as_bytes())
    }

    /// Serves what only a throwaway network's node does, from where it serves: its bucket, and stopping it when it's left
    /// idle. → `routes`, noting each request.
    pub fn serve_beside(&self, routes: axum::Router) -> axum::Router {
        use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
        let home = self.home.clone();
        tokio::spawn({
            let home = home.clone();
            async move {
                if let Err(err) = bucket(home).await {
                    tracing::warn!("the throwaway network's bucket stopped: {err:#}");
                }
            }
        });
        let started = std::time::Instant::now();
        let asked = std::sync::Arc::new(AtomicU64::new(0));
        tokio::spawn({
            let asked = asked.clone();
            async move {
                while started.elapsed().as_secs().saturating_sub(asked.load(Relaxed)) < IDLE.as_secs() {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
                tracing::info!("nobody asked the throwaway network anything for {} minutes: stopping", IDLE.as_secs() / 60);
                let _ = std::fs::remove_file(home.join("pid"));
                // Its Workers and browser, then its own group: it keeps its data, for `throwaway` to start it again.
                stop_others(&home);
                unsafe { libc::kill(0, libc::SIGTERM) };
            }
        });
        routes.layer(axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
            asked.store(started.elapsed().as_secs(), Relaxed);
            next.run(req)
        }))
    }
}

/// Notes a program the throwaway network in `dir` started, in a process group of its own, for `programs` to find: a
/// client's browser is `client`, which starting the network again leaves running.
pub fn record(dir: &Path, pid: u32, client: bool) -> Result<()> {
    let mut pids = std::fs::read_to_string(dir.join("pids")).unwrap_or_default();
    pids += &format!("{pid} {}{}\n", started(pid as i32).unwrap_or_default(), if client { " client" } else { "" });
    std::fs::write(dir.join("pids"), pids)?;
    Ok(())
}

/// When a process started, where this machine says (Linux's /proc): in clock ticks since it booted.
fn started(pid: i32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // After the command's name, in parentheses, which may hold spaces: the 22nd field.
    stat.rsplit_once(')')?.1.split_whitespace().nth(19).map(String::from)
}

/// The throwaway network's programs that still run, by the process groups they lead (`record`): only those that started when
/// they did, where this machine says, as another process may have taken a pid since. With `clients` false, not the
/// clients' browsers.
pub fn programs(dir: &Path, clients: bool) -> Vec<i32> {
    let pids = std::fs::read_to_string(dir.join("pids")).unwrap_or_default();
    let ours = |pid: i32, at: &str| at.is_empty() || started(pid).as_deref() == Some(at);
    pids.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next().unwrap_or("").to_string(), fields.next() == Some("client")))
        })
        .filter(|(pid, at, client)| ours(*pid, at) && (clients || !client))
        .map(|(pid, _, _)| pid)
        .collect()
}

/// Stops what the throwaway network in `home` started besides its node, which then stops itself: its Workers and browser.
pub fn stop_others(home: &Path) {
    let me = std::process::id() as i32;
    for pid in programs(home, true) {
        if pid != me {
            unsafe { libc::kill(-pid, libc::SIGTERM) };
        }
    }
    let _ = std::fs::remove_file(home.join("pids"));
}

/// The throwaway network's bucket, at `bucket-addr` in its folder: the folder its node made the network in, served S3's way
/// for the URLs its URL signer presigns (read, HEAD, list, create only if absent, replace while unchanged,
/// delete), signatures unchecked, as it's only this machine's and only a throwaway network's.
async fn bucket(home: PathBuf) -> Result<()> {
    use axum::http::{HeaderMap, Method, StatusCode, Uri};
    let Ok(at) = std::fs::read_to_string(home.join("bucket-addr")) else { return Ok(()) };
    // A node taking over from another waits for the one before to let the port go.
    let mut tries = 0;
    let listener = loop {
        match tokio::net::TcpListener::bind(at.trim()).await {
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse && tries < 600 => tries += 1,
            bound => break bound?,
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let place = crate::bucket::Place::Folder(home.join("network").join("bucket"));
    let answer = move |method: Method, uri: Uri, headers: HeaderMap, body: axum::body::Bytes| {
        let place = place.clone();
        async move {
            if method == Method::GET && matches!(uri.path(), "/throwaway" | "/throwaway/") {
                let query: Vec<(String, String)> = uri.query().unwrap_or("").split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (decode(k), decode(v))).collect();
                let get = |k: &str| query.iter().find(|(q, _)| q == k).map(|(_, v)| v.clone());
                let (prefix, delimit) = (get("prefix").unwrap_or_default(), get("delimiter").is_some());
                let dir = home.join("network").join("bucket");
                return match tokio::task::spawn_blocking(move || list(&dir, &prefix, delimit)).await {
                    Ok(Ok(xml)) => (StatusCode::OK, HeaderMap::new(), xml.into_bytes()),
                    _ => (StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new(), Vec::new()),
                };
            }
            let name = decode(uri.path().strip_prefix("/throwaway/").unwrap_or(""));
            if name.is_empty() || name.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
                return (StatusCode::BAD_REQUEST, HeaderMap::new(), Vec::new());
            }
            let header = move |h: &str| headers.get(h).and_then(|v| v.to_str().ok()).map(|v| v.trim_matches('"').to_string());
            let done = |made: bool| if made { StatusCode::OK } else { StatusCode::PRECONDITION_FAILED };
            let answered = tokio::task::spawn_blocking(move || -> Result<(StatusCode, HeaderMap, Vec<u8>)> {
                Ok(match method {
                    Method::GET | Method::HEAD => match place.get_object(&name)? {
                        Some(object) => {
                            let tag = [("etag".parse().unwrap(), format!("\"{}\"", object.tag).parse()?)].into_iter().collect();
                            (StatusCode::OK, tag, if method == Method::HEAD { Vec::new() } else { object.bytes })
                        }
                        None => (StatusCode::NOT_FOUND, HeaderMap::new(), Vec::new()),
                    },
                    Method::PUT if header("if-none-match").as_deref() == Some("*") => (done(place.create(&name, &body)?), HeaderMap::new(), Vec::new()),
                    Method::PUT => (done(place.replace(&name, &body, header("if-match").as_deref())?), HeaderMap::new(), Vec::new()),
                    Method::DELETE => (place.delete(&name).map(|_| StatusCode::NO_CONTENT)?, HeaderMap::new(), Vec::new()),
                    _ => (StatusCode::METHOD_NOT_ALLOWED, HeaderMap::new(), Vec::new()),
                })
            });
            match answered.await {
                Ok(Ok(answer)) => answer,
                _ => (StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new(), Vec::new()),
            }
        }
    };
    // Any size, as a real bucket takes: a service's build, a release's binary.
    axum::serve(listener, axum::Router::new().fallback(answer).layer(axum::extract::DefaultBodyLimit::disable())).await?;
    Ok(())
}

/// The names under `prefix` in the throwaway bucket's folder `dir`, as S3's ListObjectsV2 answers, in one page: one level
/// down with `delimit` (objects, and folders as common prefixes), else every object.
fn list(dir: &Path, prefix: &str, delimit: bool) -> Result<String> {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult><IsTruncated>false</IsTruncated>");
    let (folder, start) = prefix.rsplit_once('/').map_or(("", prefix), |(f, s)| (f, s));
    let place = crate::bucket::Place::Folder(dir.to_path_buf());
    let names: Vec<String> = match delimit {
        true => place.list(folder)?.into_iter().filter(|n| n.starts_with(start)).map(|n| if folder.is_empty() { n } else { format!("{folder}/{n}") }).collect(),
        false => place.names()?.into_iter().filter(|n| n.starts_with(prefix)).collect(),
    };
    for name in names {
        let meta = std::fs::metadata(dir.join(&name))?;
        if meta.is_dir() {
            xml += &format!("<CommonPrefixes><Prefix>{}/</Prefix></CommonPrefixes>", esc(&name));
            continue;
        }
        let written = jiff::Timestamp::try_from(meta.modified()?)?;
        xml += &format!("<Contents><Key>{}</Key><LastModified>{written}</LastModified><Size>{}</Size></Contents>", esc(&name), meta.len());
    }
    Ok(xml + "</ListBucketResult>")
}

fn decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
