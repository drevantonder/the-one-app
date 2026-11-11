//! Logos from the sites apps and services connect to. A device fetches a `domain`'s logo itself,
//! keeps it as a blob, and gives it as an app's `image`, so shells draw it offline and devices never ask the site.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};


/// A logo is fetched again after a week; a site that gave none is asked again after a day.
const FRESH: u64 = 7 * 86_400_000;
const RETRY: u64 = 86_400_000;
const LARGEST: u64 = 1 << 20;

/// The logos this device has fetched, by domain, kept in `logos.json` beside its blobs.
pub struct Logos {
    path: PathBuf,
    known: Mutex<BTreeMap<String, Known>>,
    fetching: Mutex<HashSet<String>>,
    urls: Mutex<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Known {
    /// The logo's blob and its type, or none when the site gave none a shell can draw.
    blob: Option<String>,
    mime: Option<String>,
    /// `icon` for a finished app icon (an apple-touch-icon), which a shell draws edge to edge; `logo` for the rest,
    /// which it insets on white. Logos kept before it was known are fetched again.
    #[serde(default)]
    kind: Option<String>,
    /// When it was fetched, milliseconds since 1970.
    at: u64,
}

impl Logos {
    pub fn open(dir: &Path) -> Logos {
        let path = dir.join("logos.json");
        let known = platform::files::read_json(&path);
        Logos { path, known: Mutex::new(known), fetching: Mutex::default(), urls: Mutex::default() }
    }

    /// `domain`'s logo as a data URL and its kind (`icon` or `logo`), if this device has it, and whether it's time to
    /// fetch it (again).
    pub fn get(&self, dir: &Path, domain: &str, now: u64) -> (Option<(String, String)>, bool) {
        if !valid(domain) {
            return (None, false);
        }
        let Some(k) = self.known.lock().unwrap().get(domain).cloned() else { return (None, true) };
        let due = now.saturating_sub(k.at) > if k.blob.is_some() { FRESH } else { RETRY } || (k.blob.is_some() && k.kind.is_none());
        let kind = k.kind.clone().unwrap_or_else(|| "logo".into());
        let (Some(blob), Some(mime)) = (k.blob, k.mime) else { return (None, due) };
        if let Some(url) = self.urls.lock().unwrap().get(domain) {
            return (Some((url.clone(), kind)), due);
        }
        let Ok(bytes) = database::blob_path(dir, &blob).and_then(|p| Ok(std::fs::read(p)?)) else { return (None, true) };
        let url = format!("data:{mime};base64,{}", data_encoding::BASE64.encode(&bytes));
        self.urls.lock().unwrap().insert(domain.to_string(), url.clone());
        (Some((url, kind)), due)
    }

    /// Whether this caller should fetch `domain` now: no one else is.
    pub fn begin(&self, domain: &str) -> bool {
        self.fetching.lock().unwrap().insert(domain.to_string())
    }

    /// Keeps what fetching `domain` got, a logo or none; a logo it had stays when a fetch fails. Whether the logo changed.
    pub fn finish(&self, dir: &Path, domain: &str, logo: Option<Logo>, now: u64) -> bool {
        self.fetching.lock().unwrap().remove(domain);
        let mut known = self.known.lock().unwrap();
        let old = known.get(domain).cloned();
        let new = match logo.and_then(|l| Some((database::put_blob(dir, &l.bytes).ok()?, l.mime, l.kind))) {
            Some((blob, mime, kind)) => Known { blob: Some(blob), mime: Some(mime.to_string()), kind: Some(kind.to_string()), at: now },
            None => match &old {
                Some(k) if k.blob.is_some() => Known { at: now.saturating_sub(FRESH) + RETRY, ..k.clone() },
                _ => Known { blob: None, mime: None, kind: None, at: now },
            },
        };
        let changed = old.as_ref().map(|k| (&k.blob, &k.kind)) != Some((&new.blob, &new.kind));
        known.insert(domain.to_string(), new);
        if changed {
            self.urls.lock().unwrap().remove(domain);
        }
        let tmp = self.path.with_extension("part");
        if std::fs::write(&tmp, serde_json::to_vec(&*known).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
        changed
    }
}

/// A site's name, like `messenger.com`: no scheme, path, port or address, so a device only ever asks public sites.
pub fn valid(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    domain.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        && labels.last().is_some_and(|l| l.bytes().all(|b| b.is_ascii_lowercase()))
        && !matches!(*labels.last().unwrap(), "local" | "internal" | "localhost" | "lan" | "home" | "arpa")
}

/// A site's picture: its bytes, type, and `icon` if it's a finished app icon (an apple-touch-icon, made for a Home
/// Screen), else `logo`.
pub struct Logo {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub kind: &'static str,
}

/// Fetches `domain`'s logo, as a site gives browsers one: its page's apple-touch-icon, then `/apple-touch-icon.png`,
/// then the largest icon its page links to, then `/favicon.ico`. The first a shell can draw (PNG, JPEG, GIF or WebP;
/// an ICO's largest picture, as PNG), with its type.
pub fn fetch(domain: &str) -> Result<Logo> {
    ensure!(valid(domain), "{domain:?} isn't a site's name");
    let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).user_agent(format!("{}/{}", custom_build::config::NAME, env!("CARGO_PKG_VERSION")));
    let agent = platform::reach::agent(config, platform::reach::public);
    let home = format!("https://{domain}/");
    let (page, base) = match agent.get(&home).call() {
        Ok(mut r) => {
            use ureq::ResponseExt as _;
            let base = r.get_uri().to_string();
            (r.body_mut().with_config().limit(4 << 20).read_to_string().unwrap_or_default(), base)
        }
        Err(_) => (String::new(), home),
    };
    let mut links = links(&page);
    links.sort_by_key(|l| std::cmp::Reverse(l.size));
    let mut tries: Vec<(String, &str)> = links.iter().filter(|l| l.touch).map(|l| (resolve(&base, &l.href), "icon")).collect();
    tries.push((format!("https://{domain}/apple-touch-icon.png"), "icon"));
    tries.extend(links.iter().filter(|l| !l.touch && !l.svg).map(|l| (resolve(&base, &l.href), "logo")));
    tries.push((format!("https://{domain}/favicon.ico"), "logo"));
    let mut tried = HashSet::new();
    for (url, kind) in tries.into_iter().filter(|(u, _)| tried.insert(u.clone())) {
        let Ok(mut r) = agent.get(&url).header("accept", "image/png,image/*;q=0.8").call() else { continue };
        let Ok(bytes) = r.body_mut().with_config().limit(LARGEST).read_to_vec() else { continue };
        if let Some((bytes, mime)) = picture(&bytes) {
            return Ok(Logo { bytes, mime, kind: if kind == "icon" { "icon" } else { "logo" } });
        }
    }
    bail!("{domain} has no logo a shell can draw")
}

struct Link {
    touch: bool,
    svg: bool,
    href: String,
    /// Its `sizes`' width, or 0 when it gives none.
    size: u32,
}

/// The page's `<link>`s to icons.
fn links(page: &str) -> Vec<Link> {
    let lower = page.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = lower[at..].find("<link").map(|i| at + i) {
        let end = lower[start..].find('>').map_or(lower.len(), |i| start + i);
        let attrs = attributes(&page[start + 5..end]);
        at = end;
        let get = |name: &str| attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).unwrap_or("");
        let rel: Vec<String> = get("rel").to_ascii_lowercase().split_whitespace().map(str::to_string).collect();
        let touch = rel.iter().any(|r| r.starts_with("apple-touch-icon"));
        let href = get("href").replace("&amp;", "&");
        if !(touch || rel.iter().any(|r| r == "icon")) || href.is_empty() || href.starts_with("data:") {
            continue;
        }
        let path = href.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
        let svg = get("type").contains("svg") || path.ends_with(".svg");
        let size = get("sizes").split(|c: char| !c.is_ascii_digit()).find_map(|n| n.parse().ok()).unwrap_or(0);
        out.push(Link { touch, svg, href, size });
    }
    out
}

/// A tag's attributes, names lowercased: `name="v"`, `name='v'` or `name=v`.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = tag.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
    while !rest.is_empty() {
        let name_end = rest.find(|c: char| c == '=' || c.is_whitespace() || c == '/').unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let value = if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (value, next) = match after.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let close = after[1..].find(q).map_or(after.len(), |i| i + 1);
                    (&after[1..close], after.get(close + 1..).unwrap_or(""))
                }
                _ => {
                    let stop = after.find(char::is_whitespace).unwrap_or(after.len());
                    (&after[..stop], &after[stop..])
                }
            };
            rest = next;
            value.to_string()
        } else {
            String::new()
        };
        if !name.is_empty() {
            out.push((name, value));
        }
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
    }
    out
}

/// `href` as an address, from the page at `base`.
fn resolve(base: &str, href: &str) -> String {
    if href.starts_with("https://") || href.starts_with("http://") {
        return href.to_string();
    }
    if let Some(rest) = href.strip_prefix("//") {
        return format!("https://{rest}");
    }
    let origin_end = base.find("://").map(|i| i + 3).and_then(|s| base[s..].find('/').map(|i| s + i)).unwrap_or(base.len());
    if href.starts_with('/') {
        return format!("{}{href}", &base[..origin_end]);
    }
    let path = base.split(['?', '#']).next().unwrap_or(base);
    let dir = path.rfind('/').filter(|i| *i >= origin_end).map_or(format!("{}/", &base[..origin_end]), |i| path[..=i].to_string());
    format!("{dir}{href}")
}

/// `bytes` as a picture every shell draws, and its type: as it is, or an ICO's largest as PNG.
fn picture(bytes: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let mime = match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [0, 0, 1, 0, ..] => return ico(bytes).ok().map(|png| (png, "image/png")),
        _ => return None,
    };
    Some((bytes.to_vec(), mime))
}

/// An ICO's largest picture, as PNG.
fn ico(b: &[u8]) -> Result<Vec<u8>> {
    let u16_at = |i: usize| -> Result<u16> { Ok(u16::from_le_bytes(b.get(i..i + 2).context("short")?.try_into()?)) };
    let u32_at = |i: usize| -> Result<u32> { Ok(u32::from_le_bytes(b.get(i..i + 4).context("short")?.try_into()?)) };
    let count = u16_at(4)? as usize;
    let mut best: Option<(u32, u16, usize, usize)> = None;
    for e in 0..count {
        let at = 6 + 16 * e;
        let width = match *b.get(at).context("short")? { 0 => 256, w => w as u32 };
        let (bpp, size, offset) = (u16_at(at + 6)?, u32_at(at + 8)? as usize, u32_at(at + 12)? as usize);
        if best.is_none_or(|(w, d, _, _)| (width, bpp) > (w, d)) {
            best = Some((width, bpp, offset, size));
        }
    }
    let (_, _, offset, size) = best.context("an empty icon")?;
    let data = b.get(offset..offset.checked_add(size).context("short")?).context("short")?;
    if data.starts_with(b"\x89PNG") {
        return Ok(data.to_vec());
    }
    bmp(data)
}

/// An ICO's bitmap (a BMP's header and pixels, twice as tall for its mask), 24 or 32 bits a pixel, as PNG.
fn bmp(d: &[u8]) -> Result<Vec<u8>> {
    let i32_at = |i: usize| -> Result<i32> { Ok(i32::from_le_bytes(d.get(i..i + 4).context("short")?.try_into()?)) };
    let header = i32_at(0)? as usize;
    let (width, height) = (i32_at(4)?, i32_at(8)? / 2);
    let bpp = u16::from_le_bytes(d.get(14..16).context("short")?.try_into()?) as usize;
    ensure!((1..=256).contains(&width) && (1..=256).contains(&height), "an odd size");
    ensure!(bpp == 24 || bpp == 32, "{bpp} bits a pixel");
    ensure!(i32_at(16)? == 0, "a compressed bitmap");
    let (w, h) = (width as usize, height as usize);
    let stride = (w * bpp).div_ceil(32) * 4;
    let mask_stride = w.div_ceil(32) * 4;
    let pixels = d.get(header..header + stride * h).context("short")?;
    let mask = d.get(header + stride * h..header + stride * h + mask_stride * h);
    let alpha = bpp == 32 && pixels.chunks(4).any(|p| p[3] != 0);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in (0..h).rev() {
        for x in 0..w {
            let p = &pixels[y * stride + x * bpp / 8..];
            let hidden = mask.is_some_and(|m| m[y * mask_stride + x / 8] & (0x80 >> (x % 8)) != 0);
            let a = if alpha { p[3] } else if hidden { 0 } else { 255 };
            rgba.extend_from_slice(&[p[2], p[1], p[0], a]);
        }
    }
    png(w as u32, h as u32, &rgba)
}

/// RGBA pixels, top row first, as PNG.
fn png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut raw = Vec::with_capacity(rgba.len() + h as usize);
    for row in rgba.chunks(w as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&raw)?;
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    };
    let mut ihdr = [w.to_be_bytes(), h.to_be_bytes()].concat();
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z.finish()?);
    chunk(b"IEND", &[]);
    Ok(out)
}
