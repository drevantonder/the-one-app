//! What only a throwaway network's computer serves, beside its onet (`one-dev throwaway`): its bucket (an S3 stand-in,
//! the folder its node made the network in), and stopping itself when nobody has asked it anything for an hour;
//! `one-dev throwaway` starts it again as it was.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use platform::mode::{Throwaway, stop_others};

/// How long a throwaway network's node runs with nobody asking it anything.
const IDLE: Duration = Duration::from_secs(60 * 60);

/// Serves what only a throwaway network's node does, from where it serves: its bucket, and stopping it when it's left
/// idle. → `routes`, noting each request.
pub(super) fn serve_beside(network: &Throwaway, routes: axum::Router) -> axum::Router {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    let home = network.home().to_path_buf();
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
            platform::os::ask_group_to_stop(0);
        }
    });
    routes.layer(axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
        asked.store(started.elapsed().as_secs(), Relaxed);
        next.run(req)
    }))
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
    let place = bucket::Place::Folder(home.join("network").join("bucket"));
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
    let place = bucket::Place::Folder(dir.to_path_buf());
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
