//! Bundles: what a deploy names. A tar holding `app.js`, the built ES module, and `src/`, the files it was built from.
//! See SPEC.md, "Apps". Plain ustar, written the same way every time, so the same files give the same blob.

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

/// An app folder's files, as written: everything but `node_modules` and dot files, sorted.
pub fn sources(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
            out.push((rel, std::fs::read(&path)?));
        }
    }
    Ok(())
}

/// The built module a bundle holds.
pub fn module(bundle: &[u8]) -> Result<String> {
    let (_, bytes) = unpack(bundle)?.into_iter().find(|(path, _)| path == "app.js").context("the bundle has no app.js")?;
    Ok(String::from_utf8(bytes)?)
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

#[test]
fn round_trips() {
    let files = vec![("app.js".to_string(), b"export default {}".to_vec()), ("src/app.js".to_string(), b"export default {}".to_vec())];
    let packed = pack(&files).unwrap();
    assert_eq!(unpack(&packed).unwrap(), files);
    assert_eq!(pack(&files).unwrap(), packed, "the same files make the same bundle");
    let long = vec![(format!("src/{}/app.js", "a".repeat(120)), b"x".to_vec())];
    assert_eq!(unpack(&pack(&long).unwrap()).unwrap(), long, "long paths keep a prefix");
}
