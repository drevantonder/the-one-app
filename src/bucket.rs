//! The bucket: where an install keeps what it holds, as objects by name, in a place: a folder on one machine's disk,
//! or S3-compatible storage. The install's own bucket must write an object only if none has its name, and replace one
//! only while it's unchanged, so two servers writing at once never both win; backups' places only need to keep what
//! they're sent.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use sha2::{Digest, Sha256};

use crate::entry::hex;

/// An object as it was read: its bytes, and the tag `replace` names it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub bytes: Vec<u8>,
    pub tag: String,
}

/// What everything that keeps an install's data reads and writes it through, here or on another runtime.
pub trait Bucket {
    /// The object with this name, if there is one.
    fn get(&self, name: &str) -> impl Future<Output = Result<Option<Object>>>;
    /// Writes an object only if none has its name: false if one does.
    fn create(&self, name: &str, bytes: &[u8]) -> impl Future<Output = Result<bool>>;
    /// Replaces an object only while it's the one `tag` names, or with none, only if there's none: false if not.
    fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> impl Future<Output = Result<bool>>;
}

/// What went wrong at a place, so a person can be told how to fix it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The place rejected the key.
    Key,
    /// It has no such bucket.
    Bucket,
    /// The key may not write there.
    Refused,
    /// The place couldn't be reached.
    Unreachable,
    /// A folder's disk couldn't be written or read.
    Disk,
}

impl Trouble {
    pub fn name(self) -> &'static str {
        match self {
            Trouble::Key => "key",
            Trouble::Bucket => "bucket",
            Trouble::Refused => "refused",
            Trouble::Unreachable => "unreachable",
            Trouble::Disk => "disk",
        }
    }

    pub fn parse(name: &str) -> Option<Trouble> {
        [Trouble::Key, Trouble::Bucket, Trouble::Refused, Trouble::Unreachable, Trouble::Disk].into_iter().find(|t| t.name() == name)
    }

    /// The trouble behind an error, if a place said what it was.
    pub fn of(err: &anyhow::Error) -> Option<Trouble> {
        err.chain().find_map(|e| e.downcast_ref::<Failed>()).map(|f| f.0)
    }
}

/// A place's failure, in words that follow the place's name ("rejected the key"), and what kind it is.
#[derive(Debug)]
pub struct Failed(pub Trouble, pub String);

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}

impl std::error::Error for Failed {}

/// A folder's I/O error, as a disk's trouble, in words.
fn disk(dir: &std::path::Path, err: std::io::Error) -> anyhow::Error {
    use std::io::ErrorKind::*;
    let said = match err.kind() {
        ReadOnlyFilesystem => "is read-only".to_string(),
        StorageFull | QuotaExceeded => "is full".to_string(),
        PermissionDenied => format!("won't let this server write to {}", dir.display()),
        _ => format!("couldn't be used: {}", err.to_string().split(" (os error").next().unwrap_or_default().to_lowercase()),
    };
    Failed(Trouble::Disk, said).into()
}

/// Where a bucket is.
#[derive(Clone, Debug)]
pub enum Place {
    /// A folder on this server's disk, like a USB disk's.
    Folder(PathBuf),
    S3(S3),
}

/// An S3-compatible bucket: `endpoint` like `https://<account>.r2.cloudflarestorage.com`, path-style.
#[derive(Clone)]
pub struct S3 {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub auth: Auth,
}

/// How an S3 request is signed: here, with the bucket's keys, or by whoever holds them, as a presigned URL.
#[derive(Clone)]
pub enum Auth {
    Keys { key_id: String, secret: String },
    Signed(Sign),
}

/// Signs a request as a presigned URL, its headers included, for a server that holds no keys.
pub type Sign = Arc<dyn Fn(&Ask) -> Result<String> + Send + Sync>;

/// A request to sign: its method, the object's name ("" for the bucket), its query, and the headers it must carry.
#[derive(Debug)]
pub struct Ask<'a> {
    pub method: &'a str,
    pub name: &'a str,
    pub query: &'a [(&'a str, &'a str)],
    pub headers: &'a [(&'a str, &'a str)],
}

impl std::fmt::Debug for S3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.auth {
            Auth::Keys { key_id, .. } => write!(f, "S3 {{ {}/{}, key {key_id} }}", self.endpoint, self.bucket),
            Auth::Signed(_) => write!(f, "S3 {{ {}/{}, signed elsewhere }}", self.endpoint, self.bucket),
        }
    }
}

/// S3-compatible services that can't write only-if-absent, which an install's own bucket needs, by their hosts.
const UNCONDITIONAL: &[(&str, &str)] = &[("backblazeb2.com", "Backblaze B2"), ("your-objectstorage.com", "Hetzner"), ("digitaloceanspaces.com", "DigitalOcean Spaces")];

impl Place {
    /// A place from its item's fields.
    pub fn from_fields(fields: &BTreeMap<String, String>) -> Result<Place> {
        let get = |k: &str| fields.get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        if let Some(path) = get("path") {
            return Ok(Place::Folder(path.into()));
        }
        let endpoint = get("endpoint").context("a place has a folder's path, or S3's endpoint")?;
        ensure!(endpoint.starts_with("https://") || endpoint.starts_with("http://"), "an endpoint is a URL, like https://s3.example.com");
        Ok(Place::S3(S3 {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket: get("bucket").context("which bucket?")?,
            region: get("region").unwrap_or_else(|| "auto".into()),
            auth: Auth::Keys { key_id: get("key_id").context("which key ID?")?, secret: get("secret").context("the secret key?")? },
        }))
    }

    /// Refuses a place that can't be an install's own bucket: S3-compatible storage known to write unconditionally.
    pub fn can_hold_an_install(&self) -> Result<()> {
        if let Place::S3(s3) = self {
            if let Some((_, name)) = UNCONDITIONAL.iter().find(|(host, _)| s3.host().ends_with(host)) {
                bail!("{name} can't write a file only if it isn't there yet, which an install's bucket needs: use Cloudflare R2, AWS S3, Google Cloud Storage or MinIO");
            }
        }
        Ok(())
    }

    /// Writes an object, whether or not one has its name.
    pub fn put(&self, name: &str, bytes: &[u8]) -> Result<()> {
        match self {
            Place::Folder(dir) => {
                let file = dir.join(name);
                let tmp = partial(dir, name, bytes)?;
                std::fs::rename(&tmp, &file).map_err(|e| disk(dir, e))
            }
            Place::S3(s3) => match s3.request("PUT", name, &[], &[], bytes)? {
                (200, _, _) => Ok(()),
                (status, _, body) => Err(s3.failed(status, &body)),
            },
        }
    }

    pub fn get(&self, name: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.get_object(name)?.map(|o| o.bytes))
    }

    /// An object with its tag.
    pub fn get_object(&self, name: &str) -> Result<Option<Object>> {
        match self {
            Place::Folder(dir) => match std::fs::read(dir.join(name)) {
                Ok(bytes) => Ok(Some(Object { tag: tag_of(&bytes), bytes })),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(disk(dir, e)),
            },
            Place::S3(s3) => match s3.request("GET", name, &[], &[], &[])? {
                (200, tag, bytes) => Ok(Some(Object { bytes, tag: tag.context("S3 gave no ETag")? })),
                (404, _, _) => Ok(None),
                (status, _, body) => Err(s3.failed(status, &body)),
            },
        }
    }

    /// Writes an object only if none has its name: false if one does.
    pub fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        match self {
            Place::Folder(dir) => {
                let tmp = partial(dir, name, bytes)?;
                // A link, unlike a rename, fails if the name is taken, and the object appears whole.
                let linked = std::fs::hard_link(&tmp, dir.join(name));
                let _ = std::fs::remove_file(&tmp);
                match linked {
                    Ok(()) => Ok(true),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
                    Err(e) => Err(disk(dir, e)),
                }
            }
            Place::S3(s3) => s3.conditional(name, bytes, ("if-none-match", "*")),
        }
    }

    /// Replaces an object only while it's the one `tag` names, or with none, only if there's none: false if not.
    pub fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
        let Some(tag) = tag else { return self.create(name, bytes) };
        match self {
            Place::Folder(dir) => {
                // One machine's folder: a lock on it makes reading the tag and writing one step.
                let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(".lock")).map_err(|e| disk(dir, e))?;
                lock.lock().map_err(|e| disk(dir, e))?;
                if self.get_object(name)?.is_none_or(|o| o.tag != tag) {
                    return Ok(false);
                }
                self.put(name, bytes)?;
                Ok(true)
            }
            Place::S3(s3) => s3.conditional(name, bytes, ("if-match", tag)),
        }
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        match self {
            Place::Folder(dir) => match std::fs::remove_file(dir.join(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(disk(dir, e)),
                _ => Ok(()),
            },
            Place::S3(s3) => match s3.request("DELETE", name, &[], &[], &[])? {
                (200 | 204 | 404, _, _) => Ok(()),
                (status, _, body) => Err(s3.failed(status, &body)),
            },
        }
    }

    /// The names directly in a folder of the place, sorted.
    pub fn list(&self, folder: &str) -> Result<Vec<String>> {
        let mut out = match self {
            Place::Folder(dir) => match std::fs::read_dir(dir.join(folder)) {
                Ok(entries) => entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| !n.ends_with(".partial") && !n.starts_with('.')).collect(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(disk(dir, e)),
            },
            Place::S3(s3) => s3.list(&format!("{}/", folder.trim_end_matches('/')))?.into_iter().map(|(name, _)| name).collect(),
        };
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// The name of every object in the place, in every folder, sorted.
    pub fn names(&self) -> Result<Vec<String>> {
        let mut out = match self {
            Place::Folder(dir) => {
                fn walk(dir: &std::path::Path, at: &str, out: &mut Vec<String>) -> std::io::Result<()> {
                    for e in std::fs::read_dir(dir.join(at))? {
                        let e = e?;
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with('.') {
                            continue;
                        }
                        let path = if at.is_empty() { name } else { format!("{at}/{name}") };
                        match e.file_type()?.is_dir() {
                            true => walk(dir, &path, out)?,
                            false => out.push(path),
                        }
                    }
                    Ok(())
                }
                let mut out = Vec::new();
                match walk(dir, "", &mut out) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    other => other.map_err(|e| disk(dir, e))?,
                }
                out
            }
            Place::S3(s3) => s3.list_all()?,
        };
        out.sort();
        Ok(out)
    }

    /// The objects in a folder of the place: each one's name, when it was last written (ms), and its size.
    pub fn objects(&self, folder: &str) -> Result<Vec<(String, u64, u64)>> {
        match self {
            Place::Folder(dir) => {
                let mut out = Vec::new();
                for name in self.list(folder)? {
                    let meta = std::fs::metadata(dir.join(folder).join(&name)).map_err(|e| disk(dir, e))?;
                    let written = meta.modified().map_err(|e| disk(dir, e))?.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                    out.push((name, written, meta.len()));
                }
                Ok(out)
            }
            Place::S3(s3) => Ok(s3.list(&format!("{folder}/"))?.into_iter().filter_map(|(name, about)| Some((name, about?.0, about?.1))).collect()),
        }
    }

    /// Writes a test file, reads it back and deletes it: each step, and what went wrong if it did. For an install's
    /// own bucket, `conditional` also checks that it refuses to write over a file, and to replace a changed one.
    pub fn try_it(&self, conditional: bool) -> Vec<(&'static str, Option<String>)> {
        let name = format!("test-{}", hex(&rand::random::<[u8; 32]>()));
        let bytes = format!("{} tried this place", crate::config::NAME).into_bytes();
        let wrote = match conditional {
            true => self.can_hold_an_install().and_then(|()| self.create(&name, &bytes)).and_then(|new| if new { Ok(()) } else { bail!("a file had its name") }),
            false => self.put(&name, &bytes),
        };
        let not_written = || anyhow!("not written");
        let read = wrote.as_ref().map_err(|_| not_written()).and_then(|_| match self.get_object(&name)? {
            Some(o) if o.bytes == bytes => Ok(o),
            Some(_) => bail!("it read back different"),
            None => bail!("it wasn't there"),
        });
        let mut steps = vec![("Wrote a test file", wrote.as_ref().err().map(|e| format!("{e:#}"))), ("Read it back", read.as_ref().err().map(|e| format!("{e:#}")))];
        if conditional {
            let refused = read.as_ref().map_err(|_| not_written()).and_then(|o| {
                ensure!(!self.create(&name, b"again")?, "it wrote over the file: it can't write only if a file isn't there, which an install's bucket needs");
                ensure!(!self.replace(&name, b"again", Some("\"stale\""))?, "it replaced a file that had changed: it can't write only if a file is unchanged");
                ensure!(self.replace(&name, b"again", Some(&o.tag))?, "it wouldn't replace an unchanged file");
                Ok(())
            });
            steps.push(("Refused to write over it", refused.err().map(|e| format!("{e:#}"))));
        }
        let deleted = wrote.as_ref().map_err(|_| not_written()).and_then(|_| self.delete(&name));
        steps.push(("Deleted it", deleted.err().map(|e| format!("{e:#}"))));
        steps
    }

    /// For a folder: whether its disk is there now. A disk that's unplugged waits, rather than fails.
    pub fn present(&self) -> bool {
        match self {
            Place::Folder(dir) => dir.parent().is_none_or(|p| p.exists()),
            Place::S3(_) => true,
        }
    }
}

/// The node's own runtime reaches a place on a thread that may wait.
impl Bucket for Place {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        let (place, name) = (self.clone(), name.to_string());
        tokio::task::spawn_blocking(move || place.get_object(&name)).await?
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        let (place, name, bytes) = (self.clone(), name.to_string(), bytes.to_vec());
        tokio::task::spawn_blocking(move || Place::create(&place, &name, &bytes)).await?
    }

    async fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
        let (place, name, bytes, tag) = (self.clone(), name.to_string(), bytes.to_vec(), tag.map(str::to_string));
        tokio::task::spawn_blocking(move || Place::replace(&place, &name, &bytes, tag.as_deref())).await?
    }
}

/// A folder object's tag: a hash of its bytes.
fn tag_of(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Writes `bytes` beside where `name` goes, in a file of its own: → its path.
fn partial(dir: &std::path::Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    let file = dir.join(name);
    std::fs::create_dir_all(file.parent().context("no folder")?).map_err(|e| disk(dir, e))?;
    let tmp = file.with_file_name(format!(".{}.{}.partial", file.file_name().and_then(|n| n.to_str()).unwrap_or(""), hex(&rand::random())));
    let mut f = std::fs::File::create(&tmp).map_err(|e| disk(dir, e))?;
    f.write_all(bytes).and_then(|()| f.sync_data()).map_err(|e| disk(dir, e))?;
    Ok(tmp)
}

impl S3 {
    fn host(&self) -> &str {
        self.endpoint.split("://").nth(1).unwrap_or(&self.endpoint)
    }

    /// A PUT that only happens if `header` holds: false if it didn't.
    fn conditional(&self, name: &str, bytes: &[u8], header: (&str, &str)) -> Result<bool> {
        match self.request("PUT", name, &[], &[header], bytes)? {
            (200, _, _) => Ok(true),
            // 409: another conditional write of it is under way, which wins or loses as this one would have.
            (412 | 409, _, _) => Ok(false),
            (status, _, body) => Err(self.failed(status, &body)),
        }
    }

    /// Says what a refused request means, in words, from S3's error code.
    fn failed(&self, status: u16, body: &[u8]) -> anyhow::Error {
        let text = String::from_utf8_lossy(body);
        let code = between(&text, "<Code>", "</Code>").unwrap_or_default();
        let (trouble, said) = match (status, code.as_str()) {
            (_, "InvalidAccessKeyId" | "SignatureDoesNotMatch") | (401, _) => (Trouble::Key, "rejected the key".to_string()),
            (_, "NoSuchBucket") => (Trouble::Bucket, format!("has no bucket {}", self.bucket)),
            (403, _) | (_, "AccessDenied") => (Trouble::Refused, format!("won't let the key write to {}", self.bucket)),
            (500.., _) => (Trouble::Unreachable, format!("said {status}{}", if code.is_empty() { String::new() } else { format!(" ({code})") })),
            _ => return anyhow!("{} said {status}{}", self.host(), if code.is_empty() { String::new() } else { format!(" ({code})") }),
        };
        Failed(trouble, said).into()
    }

    /// Every object's name in the bucket.
    fn list_all(&self) -> Result<Vec<String>> {
        Ok(self.list_under("", false)?.into_iter().map(|(name, _)| name).collect())
    }

    /// The names under `prefix`, one level down: objects', with when each was last written (ms) and its size, and
    /// folders'.
    fn list(&self, prefix: &str) -> Result<Vec<(String, Option<(u64, u64)>)>> {
        self.list_under(prefix, true)
    }

    /// The names under `prefix`, one level down with `delimit`, else every one.
    fn list_under(&self, prefix: &str, delimit: bool) -> Result<Vec<(String, Option<(u64, u64)>)>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![("list-type", "2".to_string()), ("prefix", prefix.to_string())];
            if delimit {
                query.push(("delimiter", "/".to_string()));
            }
            if let Some(t) = &token {
                query.push(("continuation-token", t.clone()));
            }
            let q: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let (status, _, body) = self.request("GET", "", &q, &[], &[])?;
            if status != 200 {
                return Err(self.failed(status, &body));
            }
            let text = String::from_utf8_lossy(&body);
            // Each object is a <Contents> with its <Key> and <LastModified>; each folder a <CommonPrefixes><Prefix>.
            for (tag, block) in [("Key", "Contents"), ("Prefix", "CommonPrefixes")] {
                let (open, close) = (format!("<{block}>"), format!("</{block}>"));
                let mut rest = text.as_ref();
                while let Some(start) = rest.find(&open) {
                    rest = &rest[start + open.len()..];
                    let Some(end) = rest.find(&close) else { break };
                    let item = &rest[..end];
                    rest = &rest[end..];
                    let Some(name) = between(item, &format!("<{tag}>"), &format!("</{tag}>")).map(|n| unescape(&n)) else { continue };
                    let written = between(item, "<LastModified>", "</LastModified>").and_then(|t| t.parse::<jiff::Timestamp>().ok()).map(|t| t.as_millisecond() as u64);
                    let size = between(item, "<Size>", "</Size>").and_then(|n| n.parse().ok());
                    if let Some(name) = name.strip_prefix(prefix).map(|n| if delimit { n.trim_end_matches('/') } else { n }).filter(|n| !n.is_empty()) {
                        out.push((name.to_string(), written.zip(size)));
                    }
                }
            }
            match between(&text, "<NextContinuationToken>", "</NextContinuationToken>") {
                Some(t) if text.contains("<IsTruncated>true</IsTruncated>") => token = Some(unescape(&t)),
                _ => break,
            }
        }
        Ok(out)
    }

    /// The URL a request goes to, signed: here with the keys, or by whoever signs for this server.
    fn url(&self, ask: &Ask) -> Result<String> {
        match &self.auth {
            Auth::Signed(sign) => sign(ask),
            Auth::Keys { key_id, secret } => {
                let stamp = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ").to_string();
                let path = match ask.name {
                    "" => format!("/{}", encode(&self.bucket, true)),
                    name => format!("/{}/{}", encode(&self.bucket, true), encode(name, false)),
                };
                Ok(format!("{}{}", self.endpoint, presign(self.host(), &path, &self.region, key_id, secret, &stamp, 900, ask)))
            }
        }
    }

    /// A request by presigned URL: → its status, ETag and body.
    fn request(&self, method: &str, name: &str, query: &[(&str, &str)], headers: &[(&str, &str)], body: &[u8]) -> Result<(u16, Option<String>, Vec<u8>)> {
        let url = self.url(&Ask { method, name, query, headers })?;
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(300))).build().into();
        let mut builder = ureq::http::Request::builder().method(method).uri(&url);
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        let host = self.host();
        let mut resp = match method {
            "PUT" => agent.run(builder.body(body.to_vec())?),
            _ => agent.run(builder.body(())?),
        }
        .map_err(|e| Failed(Trouble::Unreachable, format!("couldn't be reached ({host}: {e})")))?;
        let tag = resp.headers().get("etag").and_then(|t| t.to_str().ok()).map(str::to_string);
        let bytes = resp.body_mut().with_config().limit(4 << 30).read_to_vec()?;
        Ok((resp.status().as_u16(), tag, bytes))
    }
}

/// A request's path and query, presigned with AWS Signature Version 4 (the query-string form), its headers signed so
/// they can't be dropped: a URL signed with `if-none-match: *` can only create.
fn presign(host: &str, path: &str, region: &str, key_id: &str, secret: &str, stamp: &str, expires: u32, ask: &Ask) -> String {
    let date = &stamp[..8];
    let scope = format!("{date}/{region}/s3/aws4_request");
    let mut headers: Vec<(String, String)> = ask.headers.iter().map(|(k, v)| (k.to_lowercase(), v.trim().to_string())).collect();
    headers.push(("host".into(), host.into()));
    headers.sort();
    let signed = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let mut q: Vec<(String, String)> = ask.query.iter().map(|(k, v)| (encode(k, true), encode(v, true))).collect();
    for (k, v) in [
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_string()),
        ("X-Amz-Credential", format!("{key_id}/{scope}")),
        ("X-Amz-Date", stamp.to_string()),
        ("X-Amz-Expires", expires.to_string()),
        ("X-Amz-SignedHeaders", signed.clone()),
    ] {
        q.push((encode(k, true), encode(&v, true)));
    }
    q.sort();
    let query = q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let request = format!("{}\n{path}\n{query}\n{canonical_headers}\n{signed}\nUNSIGNED-PAYLOAD", ask.method);
    let to_sign = format!("AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}", hex(&Sha256::digest(request.as_bytes()).into()));
    let mut k = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    for part in [region, "s3", "aws4_request"] {
        k = hmac(&k, part.as_bytes());
    }
    format!("{path}?{query}&X-Amz-Signature={}", hex(&hmac(&k, to_sign.as_bytes())))
}

/// HMAC-SHA256 (RFC 2104).
fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    match key.len() > 64 {
        true => k[..32].copy_from_slice(&Sha256::digest(key)),
        false => k[..key.len()].copy_from_slice(key),
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// URI-encodes as SigV4 wants: everything but unreserved characters, and `/` too unless it's a path.
fn encode(s: &str, slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if !slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn between(text: &str, open: &str, close: &str) -> Option<String> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].to_string())
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presigning_signs_as_aws_does() {
        // AWS's own example of a presigned URL: GET /test.txt, with the example key, on 2013-05-24, for a day.
        let ask = Ask { method: "GET", name: "test.txt", query: &[], headers: &[] };
        let url = presign("examplebucket.s3.amazonaws.com", "/test.txt", "us-east-1", "AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "20130524T000000Z", 86400, &ask);
        assert!(url.ends_with("&X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"), "{url}");
        let create = Ask { method: "PUT", name: "test.txt", query: &[], headers: &[("If-None-Match", "*")] };
        let url = presign("examplebucket.s3.amazonaws.com", "/test.txt", "us-east-1", "AKIAIOSFODNN7EXAMPLE", "secret", "20130524T000000Z", 60, &create);
        assert!(url.contains("X-Amz-SignedHeaders=host%3Bif-none-match") || url.contains("X-Amz-SignedHeaders=if-none-match%3Bhost"), "{url}");
    }

    #[test]
    fn many_writers_create_one_name_and_one_wins() {
        let dir = std::env::temp_dir().join(format!("bucket-race-{}", hex(&rand::random())));
        let place = Place::Folder(dir.clone());
        let won: usize = std::thread::scope(|s| {
            let runs: Vec<_> = (0..16).map(|i| {
                let place = &place;
                s.spawn(move || place.create("x", format!("{i}").as_bytes()).unwrap() as usize)
            }).collect();
            runs.into_iter().map(|r| r.join().unwrap()).sum()
        });
        assert_eq!(won, 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

}
