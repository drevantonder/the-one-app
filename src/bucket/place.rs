//! Places: a folder on one machine's disk, or S3-compatible storage, read and written from a machine.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail, ensure};
use sha2::{Digest, Sha256};

use crate::keys::hex;

use super::signed::pooled;
use super::{Ask, Bucket, Failed, Object, Trouble};

fn requests() -> ureq::Agent {
    static POOL: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    pooled(&POOL, 300)
}

/// A folder's I/O error, as a disk's trouble, in words.
fn disk(dir: &std::path::Path, err: std::io::Error) -> anyhow::Error {
    use std::io::ErrorKind::*;
    let said = match err.kind() {
        ReadOnlyFilesystem => "is read-only".to_string(),
        StorageFull | QuotaExceeded => "is full".to_string(),
        PermissionDenied => format!("won't let this device write to {}", dir.display()),
        _ => format!("couldn't be used: {}", err.to_string().split(" (os error").next().unwrap_or_default().to_lowercase()),
    };
    Failed(Trouble::Disk, said).into()
}

#[derive(Clone, Debug)]
pub enum Place {
    /// A folder on this device's disk, like a USB disk's.
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

/// How an S3 request is signed: here, with the bucket's keys (a backup place's), or by the URL signer, which alone
/// holds the network's bucket's.
#[derive(Clone)]
pub enum Auth {
    Keys { key_id: String, secret: String },
    Signed(Arc<super::Signer>),
}

impl std::fmt::Debug for S3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.auth {
            Auth::Keys { key_id, .. } => write!(f, "S3 {{ {}/{}, key {key_id} }}", self.endpoint, self.bucket),
            Auth::Signed(_) => write!(f, "S3 {{ {}/{}, signed elsewhere }}", self.endpoint, self.bucket),
        }
    }
}

/// S3-compatible services that can't write only-if-absent, which a network's own bucket needs, by their hosts.
const UNCONDITIONAL: &[(&str, &str)] = &[("backblazeb2.com", "Backblaze B2"), ("your-objectstorage.com", "Hetzner"), ("digitaloceanspaces.com", "DigitalOcean Spaces")];

impl Place {
    pub fn from_fields(fields: &BTreeMap<String, String>) -> Result<Place> {
        let get = |k: &str| fields.get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        if let Some(path) = get("path") {
            return Ok(Place::Folder(path.into()));
        }
        let endpoint = get("endpoint").context("Enter a folder's path, or an S3 endpoint.")?;
        ensure!(endpoint.starts_with("https://") || endpoint.starts_with("http://"), "Enter the endpoint as a web address, like https://s3.example.com.");
        Ok(Place::S3(S3 {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket: get("bucket").context("Enter the bucket's name.")?,
            region: get("region").unwrap_or_else(|| "auto".into()),
            auth: Auth::Keys { key_id: get("key_id").context("Enter the access key ID.")?, secret: get("secret").context("Enter the secret key.")? },
        }))
    }

    /// The network's bucket, from the fields a device keeps of it: a folder's `path`, or S3's address (`endpoint`,
    /// `bucket`, `region`), reached through the URL signer at the network's address, signing with `key`. A device
    /// never keeps the bucket's credentials.
    pub fn network(fields: &BTreeMap<String, String>, address: super::signed::Address, key: ed25519_dalek::SigningKey) -> Result<Place> {
        ensure!(!fields.contains_key("key_id") && !fields.contains_key("secret"), "a device keeps only the bucket's address: the URL signer holds its key");
        let get = |k: &str| fields.get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        if let Some(path) = get("path") {
            return Ok(Place::Folder(path.into()));
        }
        let endpoint = get("endpoint").context("the network's bucket is a folder's path, or S3's endpoint")?.trim_end_matches('/').to_string();
        ensure!(endpoint.starts_with("https://") || endpoint.starts_with("http://"), "an endpoint is a URL, like https://s3.example.com");
        let bucket = get("bucket").context("which bucket?")?;
        let signer = super::Signer::new(address, key, format!("{endpoint}/{bucket}"));
        Ok(Place::S3(S3 { endpoint, bucket, region: get("region").unwrap_or_else(|| "auto".into()), auth: Auth::Signed(Arc::new(signer)) }))
    }

    /// Whether each object is there with the tag given: through the URL signer, in one trip.
    pub fn confirm(&self, objects: &[(&str, &str)]) -> Result<BTreeMap<String, bool>> {
        if let Place::S3(S3 { auth: Auth::Signed(signer), .. }) = self {
            return signer.confirm(objects);
        }
        if let Place::S3(s3) = self {
            return Ok(objects.iter().map(|(name, _)| name.to_string()).zip(s3.confirm(objects)?).collect());
        }
        let bare = |t: &str| t.trim_matches('"').to_string();
        objects.iter().map(|(name, tag)| Ok((name.to_string(), self.get_object(name)?.is_some_and(|o| bare(&o.tag) == bare(tag))))).collect()
    }

    /// Refuses a place that can't be a network's own bucket: S3-compatible storage known to write unconditionally.
    pub fn can_hold_a_network(&self) -> Result<()> {
        if let Place::S3(s3) = self {
            if let Some((_, name)) = UNCONDITIONAL.iter().find(|(host, _)| s3.host().ends_with(host)) {
                bail!("{name} can't write a file only if it isn't there yet, which a network's bucket needs: use Cloudflare R2, AWS S3, Google Cloud Storage or MinIO");
            }
        }
        Ok(())
    }

    /// Writes an object, whether or not one has its name.
    pub fn put(&self, name: &str, bytes: &[u8]) -> Result<()> {
        match self {
            Place::Folder(dir) => {
                let file = within(dir, name)?;
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

    pub fn get_object(&self, name: &str) -> Result<Option<Object>> {
        match self {
            Place::Folder(dir) => match std::fs::read(within(dir, name)?) {
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

    /// Which of `objects` (names and bytes) the place holds exactly as given: a folder's are compared, S3's by ETag.
    pub fn holds(&self, objects: &[(String, Vec<u8>)]) -> Result<Vec<bool>> {
        if let Place::Folder(_) = self {
            return objects.iter().map(|(name, bytes)| Ok(self.get(name)?.is_some_and(|b| b == *bytes))).collect();
        }
        let tags: Vec<(String, String)> = objects.iter().map(|(name, bytes)| (name.clone(), etag(bytes))).collect();
        let asked = self.confirm(&tags.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect::<Vec<_>>())?;
        Ok(objects.iter().map(|(name, _)| asked.get(name).copied().unwrap_or(false)).collect())
    }

    /// Writes an object only if none has its name: false if one does.
    pub fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        match self {
            Place::Folder(dir) => {
                let tmp = partial(dir, name, bytes)?;
                // A link, unlike a rename, fails if the name is taken, and the object appears whole.
                let linked = std::fs::hard_link(&tmp, within(dir, name)?);
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
            Place::Folder(dir) => match std::fs::remove_file(within(dir, name)?) {
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
            Place::Folder(dir) => match std::fs::read_dir(within(dir, folder.trim_end_matches('/'))?) {
                Ok(entries) => entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| !n.ends_with(".partial") && !n.starts_with('.')).collect(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(disk(dir, e)),
            },
            Place::S3(s3) => s3.list(&prefix(folder))?.into_iter().map(|(name, _)| name).collect(),
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
                    let meta = std::fs::metadata(within(dir, &format!("{}/{name}", folder.trim_end_matches('/')))?).map_err(|e| disk(dir, e))?;
                    let written = meta.modified().map_err(|e| disk(dir, e))?.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                    out.push((name, written, meta.len()));
                }
                Ok(out)
            }
            Place::S3(s3) => Ok(s3.list(&prefix(folder))?.into_iter().filter_map(|(name, about)| Some((name, about?.0, about?.1))).collect()),
        }
    }

    /// Writes a test file, reads it back and deletes it: each step, and what went wrong if it did.
    pub fn try_it(&self) -> Vec<(&'static str, Option<String>)> {
        let name = format!("test-{}", hex(&rand::random::<[u8; 32]>()));
        let bytes = format!("{} tried this place", crate::config::NAME).into_bytes();
        let wrote = self.put(&name, &bytes);
        let read = wrote.as_ref().map_err(|_| anyhow!("Skipped, since writing failed.")).and_then(|_| match self.get_object(&name)? {
            Some(o) if o.bytes == bytes => Ok(o),
            Some(_) => bail!("It came back changed."),
            None => bail!("It wasn't there."),
        });
        let deleted = wrote.as_ref().map_err(|_| anyhow!("Skipped, since writing failed.")).and_then(|_| self.delete(&name));
        vec![
            ("Wrote a test file", wrote.as_ref().err().map(|e| format!("{e:#}"))),
            ("Read it back", read.as_ref().err().map(|e| format!("{e:#}"))),
            ("Deleted it", deleted.err().map(|e| format!("{e:#}"))),
        ]
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

    async fn wait(&self, ms: u64) {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }

    fn now(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
    }

    async fn objects(&self, folder: &str) -> Result<Vec<(String, u64)>> {
        let (place, folder) = (self.clone(), folder.to_string());
        let listed = tokio::task::spawn_blocking(move || Place::objects(&place, &folder)).await??;
        Ok(listed.into_iter().map(|(name, written, _)| (name, written)).collect())
    }

    async fn delete(&self, name: &str) -> Result<()> {
        let (place, name) = (self.clone(), name.to_string());
        tokio::task::spawn_blocking(move || Place::delete(&place, &name)).await?
    }

    async fn confirm(&self, objects: &[(String, Vec<u8>)]) -> Result<Vec<bool>> {
        let (place, objects) = (self.clone(), objects.to_vec());
        tokio::task::spawn_blocking(move || Place::holds(&place, &objects)).await?
    }
}

/// The ETag S3 gives an object written by one PUT, as R2 does: its bytes' MD5, quoted.
fn etag(bytes: &[u8]) -> String {
    use md5::Digest as _;
    format!("\"{}\"", data_encoding::HEXLOWER.encode(&md5::Md5::digest(bytes)))
}

/// What names in a folder start with: nothing for the top.
fn prefix(folder: &str) -> String {
    match folder.trim_end_matches('/') {
        "" => String::new(),
        folder => format!("{folder}/"),
    }
}

/// A folder object's tag: a hash of its bytes.
fn tag_of(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Where object `name` is in the folder `dir`: always below it. A name is parts joined by `/`, none empty or starting
/// with `.`, so none climbs out of the folder or names a hidden file (a home folder's `.ssh`); "" is the folder itself.
fn within(dir: &std::path::Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() {
        return Ok(dir.to_path_buf());
    }
    let fine = |part: &str| !part.is_empty() && !part.starts_with('.') && !part.contains(['\\', '\0']);
    ensure!(name.split('/').all(fine), "{name:?} isn't an object's name: its parts are words, none starting with a dot");
    Ok(dir.join(name))
}

/// Writes `bytes` beside where `name` goes, in a file of its own: → its path.
fn partial(dir: &std::path::Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    let file = within(dir, name)?;
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

    /// Which objects have the ETag given for each, one HEAD each, where this device signs its own requests: one with the
    /// URL signer asks it for all of them in one trip.
    fn confirm(&self, tags: &[(&str, &str)]) -> Result<Vec<bool>> {
        let bare = |t: &str| t.trim_matches('"').to_string();
        tags.iter()
            .map(|(name, tag)| match self.request("HEAD", name, &[], &[], &[])? {
                (200, got, _) => Ok(got.is_some_and(|g| bare(&g) == bare(tag))),
                (404, _, _) => Ok(false),
                (status, _, body) => Err(self.failed(status, &body)),
            })
            .collect()
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

    fn failed(&self, status: u16, body: &[u8]) -> anyhow::Error {
        let text = String::from_utf8_lossy(body);
        let code = between(&text, "<Code>", "</Code>").unwrap_or_default();
        let (trouble, said) = match (status, code.as_str()) {
            (_, "InvalidAccessKeyId" | "SignatureDoesNotMatch") | (401, _) => (Trouble::Key, "rejected the key".to_string()),
            (_, "NoSuchBucket") => (Trouble::Bucket, format!("has no bucket {}", self.bucket)),
            (403, _) | (_, "AccessDenied") => (Trouble::Refused, format!("won't let the key write to {}", self.bucket)),
            (500.., _) => (Trouble::Unreachable, format!("isn't answering ({status}{})", if code.is_empty() { String::new() } else { format!(" {code}") })),
            _ => return anyhow!("{} refused it ({status}{})", self.host(), if code.is_empty() { String::new() } else { format!(" {code}") }),
        };
        Failed(trouble, said).into()
    }

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

    /// The URL a request goes to, signed: here with the keys, or by whoever signs for this device.
    fn url(&self, ask: &Ask) -> Result<String> {
        match &self.auth {
            Auth::Signed(signer) => signer.sign(ask),
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
        let agent = requests();
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
        // An answer cut short says nothing of what the request did.
        let bytes = resp.body_mut().with_config().limit(4 << 30).read_to_vec().map_err(|e| Failed(Trouble::Unreachable, format!("cut its answer short ({host}: {e})")))?;
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
