//! Restoring a whole network from a backup, on any machine and with no network running: `one restore`. The
//! backups app writes a backup (`apps/backups`), and this reads one with nothing but where it went.
//!
//! A destination holds `objects/<id>`, each one blob of the network's bucket as it is, and `backups/<name>`, each a
//! backup's list of blobs: a JSON object whose `blobs` are each a blob's `key` in the bucket, the `object` holding it, its `size` and its
//! `sha256`. The name is when the backup was made, in milliseconds since 1970, so the newest is the last.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, fs};

use anyhow::{Context, Result, bail};
use futures_util::future::try_join_all;
use rusty_s3::actions::{GetObject, ListObjectsV2, PutObject, S3Action};
use rusty_s3::{Bucket, Credentials, UrlStyle};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const USAGE: &str = "one restore --from <destination> --to <bucket> [--backup <name>]\n\n\
    Restores a whole network from a backup into an empty bucket, with no network running, or carries on with one that\n\
    was cut short, into the bucket it was restoring. A destination or a bucket is a\n\
    folder, or S3-compatible storage as https://<endpoint>/<bucket>[/<folder>], whose keys are in the file\n\
    --from-keys or --to-keys names, as ACCESS_KEY_ID=... and SECRET_ACCESS_KEY=... lines (the presigner's `secrets`\n\
    has them). --backup names one of the destination's backups, the newest if left out, and with --list the\n\
    destination's backups are listed instead.";

/// How long a signed request to S3-compatible storage works for.
const SIGNED_FOR: Duration = Duration::from_secs(300);
/// How many blobs are fetched at once.
const AT_ONCE: usize = 16;

#[derive(Deserialize)]
struct Listed {
    blobs: Vec<Blob>,
}

#[derive(Deserialize, Clone)]
struct Blob {
    key: String,
    object: String,
    size: u64,
    sha256: String,
}

/// Where blobs are: a folder, or a bucket of S3-compatible storage with a folder in it.
enum Destination {
    Folder(PathBuf),
    S3 { bucket: Box<Bucket>, credentials: Credentials, folder: String },
}

impl Destination {
    fn at(address: &str, keys: Option<&String>) -> Result<Self> {
        if !(address.starts_with("https://") || address.starts_with("http://")) {
            return Ok(Self::Folder(PathBuf::from(address)));
        }
        let url: url::Url = address.parse().with_context(|| format!("{address} isn't an address"))?;
        let mut parts = url.path().trim_matches('/').splitn(2, '/');
        let name =
            parts.next().filter(|name| !name.is_empty()).context("an address names its bucket after the host")?;
        let folder = parts.next().map(|folder| format!("{}/", folder.trim_end_matches('/'))).unwrap_or_default();
        let keys = keys.with_context(|| format!("{address} needs its keys: --from-keys or --to-keys"))?;
        let text = fs::read_to_string(keys).with_context(|| format!("couldn't read {keys}"))?;
        let value = |name: &str| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix(name)?.strip_prefix('=').map(str::trim))
                .map(str::to_owned)
        };
        let credentials = Credentials::new(
            value("ACCESS_KEY_ID").with_context(|| format!("{keys} has no ACCESS_KEY_ID line"))?,
            value("SECRET_ACCESS_KEY").with_context(|| format!("{keys} has no SECRET_ACCESS_KEY line"))?,
        );
        let endpoint: url::Url = format!("{}://{}", url.scheme(), url.authority()).parse()?;
        let region = env::var("AWS_REGION").unwrap_or_else(|_| "auto".to_owned());
        Ok(Self::S3 {
            bucket: Box::new(Bucket::new(endpoint, UrlStyle::Path, name.to_owned(), region)?),
            credentials,
            folder,
        })
    }

    async fn get(&self, name: &str) -> Result<Option<Vec<u8>>> {
        match self {
            Self::Folder(folder) => match fs::read(folder.join(name)) {
                Ok(data) => Ok(Some(data)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error).with_context(|| format!("couldn't read {name}")),
            },
            Self::S3 { bucket, credentials, folder } => {
                let url = GetObject::new(bucket, Some(credentials), &format!("{folder}{name}")).sign(SIGNED_FOR);
                let got = client::web()?.get(url).send().await?;
                match got.status().as_u16() {
                    200 => Ok(Some(got.bytes().await?.to_vec())),
                    404 => Ok(None),
                    status => bail!("the storage answered {status} for {name}"),
                }
            }
        }
    }

    async fn put(&self, name: &str, data: Vec<u8>) -> Result<()> {
        match self {
            Self::Folder(folder) => {
                let file = folder.join(name);
                fs::create_dir_all(file.parent().context("a blob is in a folder")?)?;
                fs::write(&file, data).with_context(|| format!("couldn't write {}", file.display()))
            }
            Self::S3 { bucket, credentials, folder } => {
                let url = PutObject::new(bucket, Some(credentials), &format!("{folder}{name}")).sign(SIGNED_FOR);
                let put = client::web()?.put(url).body(data).send().await?;
                if !put.status().is_success() {
                    bail!("the storage answered {} for {name}", put.status())
                }
                Ok(())
            }
        }
    }

    /// The names that start with `prefix`, in order.
    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        match self {
            Self::Folder(folder) => {
                let mut names = Vec::new();
                files(folder, folder, &mut names)?;
                names.retain(|name| name.starts_with(prefix));
                names.sort();
                Ok(names)
            }
            Self::S3 { bucket, credentials, folder } => {
                let (mut names, mut next) = (Vec::new(), None::<String>);
                loop {
                    let mut action = ListObjectsV2::new(bucket, Some(credentials));
                    action.with_prefix(format!("{folder}{prefix}"));
                    if let Some(next) = &next {
                        action.with_continuation_token(next.clone());
                    }
                    let got = client::web()?.get(action.sign(SIGNED_FOR)).send().await?;
                    if !got.status().is_success() {
                        bail!("the storage answered {} to a list", got.status())
                    }
                    let listed = ListObjectsV2::parse_response(&got.text().await?)?;
                    names.extend(
                        listed
                            .contents
                            .into_iter()
                            .filter_map(|object| object.key.strip_prefix(folder.as_str()).map(str::to_owned)),
                    );
                    match listed.next_continuation_token {
                        Some(token) => next = Some(token),
                        None => return Ok(names),
                    }
                }
            }
        }
    }
}

fn files(root: &Path, at: &Path, names: &mut Vec<String>) -> Result<()> {
    let entries = match fs::read_dir(at) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        entries => entries.with_context(|| format!("couldn't read {}", at.display()))?,
    };
    for entry in entries {
        let path = entry?.path();
        // What starts with a dot is a folder bucket's own, and no blob.
        if path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')) {
            continue;
        }
        if path.is_dir() {
            files(root, &path, names)?;
        } else {
            names.push(path.strip_prefix(root)?.iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
    Ok(())
}

fn size(bytes: u64) -> String {
    match bytes {
        bytes if bytes < 1_000 => format!("{bytes} bytes"),
        bytes if bytes < 1_000_000 => format!("{:.1} KB", bytes as f64 / 1e3),
        bytes if bytes < 1_000_000_000 => format!("{:.1} MB", bytes as f64 / 1e6),
        bytes => format!("{:.2} GB", bytes as f64 / 1e9),
    }
}

/// `one restore`, with the words after it.
pub async fn restore(args: &[String]) -> Result<String> {
    let mut flags = std::collections::HashMap::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--list" => flags.insert("list", String::new()),
            flag @ ("--from" | "--to" | "--backup" | "--from-keys" | "--to-keys") => {
                flags.insert(&flag[2..], words.next().context(USAGE)?.clone())
            }
            _ => bail!("{USAGE}"),
        };
    }
    let from = Destination::at(flags.get("from").context(USAGE)?, flags.get("from-keys"))?;
    let backups = from.list("backups/").await?;
    if flags.contains_key("list") {
        return Ok(backups.iter().map(|name| name.trim_start_matches("backups/")).collect::<Vec<_>>().join("\n"));
    }
    let to = Destination::at(flags.get("to").context(USAGE)?, flags.get("to-keys"))?;

    let name = match flags.get("backup") {
        Some(name) => format!("backups/{name}"),
        None => backups.last().cloned().context("There's no backup there.")?,
    };
    let backup = name.trim_start_matches("backups/");
    let listed = from.get(&name).await?.with_context(|| format!("There's no backup {backup} there."))?;
    let listed: Listed = serde_json::from_slice(&listed).context("that backup's list of blobs isn't one")?;

    // The network's list of servers isn't restored: the first server started on the bucket is its server then, as
    // a new network's first is, since the ones it had may be gone with the bucket.
    let blobs: Vec<Blob> = listed.blobs.into_iter().filter(|blob| blob.key != "network/servers").collect();
    // A network is restored into an empty bucket, or into one that a restore of this backup was cut short in, which
    // holds only blobs it has: it carries on from there, as a bucket writes each blob whole or not at all.
    let there: std::collections::HashSet<String> = to.list("").await?.into_iter().collect();
    let keys: std::collections::HashSet<&str> = blobs.iter().map(|blob| blob.key.as_str()).collect();
    if let Some(other) = there.iter().find(|key| !keys.contains(key.as_str())) {
        bail!(
            "{} holds {other}, which backup {backup} doesn't. A network is restored into an empty bucket, or into one a \
             restore of the same backup was cut short in.",
            flags["to"]
        )
    }
    let blobs: Vec<Blob> = blobs.into_iter().filter(|blob| !there.contains(&blob.key)).collect();
    let began = Instant::now();
    for some in blobs.chunks(AT_ONCE) {
        try_join_all(some.iter().map(|blob| async {
            let data = from.get(&format!("objects/{}", blob.object)).await?;
            let data = data.with_context(|| format!("{} is missing from the backup", blob.key))?;
            if data_encoding::HEXLOWER.encode(&Sha256::digest(&data)) != blob.sha256 {
                bail!("{} isn't what was backed up: the backup was changed", blob.key)
            }
            to.put(&blob.key, data).await
        }))
        .await?;
    }
    let bytes = blobs.iter().map(|blob| blob.size).sum();
    Ok(format!(
        "Restored backup {backup}: {} blobs, {}, in {} ms{}. The first server started on it acts for the network, and \
         its other servers are added again.",
        blobs.len(),
        size(bytes),
        began.elapsed().as_millis(),
        if there.is_empty() { String::new() } else { format!(", carrying on from the {} it had restored", there.len()) }
    ))
}
