//! Bundles: what a deploy names. A tar holding `app.js`, the built ES module, `src/`, the files it was built from, and
//! for an app with a service, `services.json`, the hash of each platform's build of it (`services`).
//! Plain ustar, written the same way every time, so the same files give the same blob. Each app's latest is kept in
//! the bucket too, sealed, for a node that has no other way to get it (the Cloudflare node).

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

/// An app folder's files, as written: everything but `node_modules`, dot files and its service's source (`service/`,
/// built apart), sorted. A link in it to a file or folder of the same git repository is followed, so apps beside each
/// other share code with a link (`apps/gmail/google` → `../google`); a link out of the repository, or to a folder that
/// holds itself, is refused.
pub fn sources(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let dir = dir.canonicalize()?;
    let repo = dir.ancestors().find(|a| a.join(".git").exists());
    let mut files = Vec::new();
    walk(&dir, "", repo, 0, &mut files)?;
    files.retain(|(path, _)| !path.starts_with("service/"));
    files.sort();
    Ok(files)
}

/// `dir`'s files, each named from the app's folder by `prefix` and its own name; `links` how many links led here.
fn walk(dir: &Path, prefix: &str, repo: Option<&Path>, links: usize, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    ensure!(links < 8, "{prefix} links to a folder that links back: an app's links go to other folders");
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let rel = format!("{prefix}{name}");
        let path = entry.path();
        let linked = entry.file_type()?.is_symlink();
        let target = if linked { path.canonicalize().with_context(|| format!("{rel} links to nothing"))? } else { path };
        if linked {
            ensure!(repo.is_some_and(|repo| target.starts_with(repo)), "{rel} links out of its git repository: an app is built from its own folder and what that links to in the same repository");
        }
        if target.is_dir() {
            walk(&target, &format!("{rel}/"), repo, links + usize::from(linked), out)?;
        } else {
            out.push((rel, std::fs::read(&target)?));
        }
    }
    Ok(())
}

/// The built module a bundle holds.
pub fn module(bundle: &[u8]) -> Result<String> {
    let (_, bytes) = unpack(bundle)?.into_iter().find(|(path, _)| path == "app.js").context("the bundle has no app.js")?;
    Ok(String::from_utf8(bytes)?)
}

/// What names an app's service's builds in its bundle: `{ "<platform>": "<blob hash>" }`, the platform as releases say
/// it (`x86_64-unknown-linux-gnu`). The deploy names the bundle, so it names them too, signed by whoever deployed it;
/// a node runs a build of the service only if it's the one its app's latest deploy names, or a release (`oned`).
pub const SERVICES: &str = "services.json";

/// The builds of its app's service a bundle names, by platform: none if it names none.
pub fn services(bundle: &[u8]) -> std::collections::BTreeMap<String, String> {
    let file = unpack(bundle).ok().and_then(|files| files.into_iter().find(|(path, _)| path == SERVICES));
    file.and_then(|(_, bytes)| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

pub fn pack(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for (path, bytes) in files {
        // ustar keeps a path of up to 255 bytes as a prefix of up to 155 and a name of up to 100, split at a `/`.
        let split = if path.len() < 100 { Some(0) } else { path.match_indices('/').map(|(i, _)| i).find(|&i| i <= 155 && path.len() - i - 1 < 100) };
        let split = split.with_context(|| format!("{path}: too long for a bundle"))?;
        ensure!(!path.is_empty(), "a file in a bundle needs a path");
        let (prefix, name) = if split == 0 { ("", path.as_str()) } else { (&path[..split], &path[split + 1..]) };
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        header[100..107].copy_from_slice(b"0000644");
        header[108..115].copy_from_slice(b"0000000");
        header[116..123].copy_from_slice(b"0000000");
        header[124..135].copy_from_slice(format!("{:011o}", bytes.len()).as_bytes());
        header[136..147].copy_from_slice(b"00000000000");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|b| *b as u32).sum();
        header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(bytes);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    Ok(out)
}

pub fn unpack(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    let mut at = 0;
    while at + 512 <= bytes.len() {
        let header = &bytes[at..at + 512];
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let field = |range: std::ops::Range<usize>| String::from_utf8_lossy(&header[range]).trim_matches(|c| c == '\0' || c == ' ').to_string();
        let name = field(0..100);
        let prefix = field(345..500);
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let size = usize::from_str_radix(&field(124..136), 8).context("bad tar size")?;
        let start = at + 512;
        ensure!(start + size <= bytes.len(), "the bundle is cut short");
        match header[156] {
            b'0' | 0 => files.push((path, bytes[start..start + size].to_vec())),
            b'5' => {}
            other => bail!("bundles hold only files, not type {}", other as char),
        }
        at = start + size.div_ceil(512) * 512;
    }
    Ok(files)
}
