//! A browser profile is a Linux Chrome's, its cookies sealed with Linux's basic key (--password-store=basic), which a
//! Mac's Chrome, keyed by its keychain, can't open and drops. So on a Mac, `take_cookies` opens them here, before
//! Chrome starts, and Chrome is handed them over DevTools once it has (Storage.setCookies).

use std::path::Path;

use aes_gcm::aes::Aes128;
use aes_gcm::aes::cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use sha2::Digest;

/// The keys Linux's basic store seals with: PBKDF2-SHA1 of its password, "saltysalt", one round, 16 bytes. v10's
/// password is "peanuts", v11's empty.
fn linux_key(version: &[u8]) -> Option<[u8; 16]> {
    let hex = match version {
        b"v10" => "fd621fe5a2b402539dfa147ca9272778",
        b"v11" => "d0d0ec9c7d77d43ac54187fa4818d17f",
        _ => return None,
    };
    data_encoding::HEXLOWER.decode(hex.as_bytes()).ok()?.try_into().ok()
}

/// A sealed cookie value in the clear, if it's one Linux's basic store sealed.
fn open_cookie(sealed: &[u8], host: &str, version: i64) -> Option<String> {
    if sealed.len() < 3 + 16 || !(sealed.len() - 3).is_multiple_of(16) {
        return None;
    }
    let cipher = Aes128::new(GenericArray::from_slice(&linux_key(&sealed[..3])?));
    // CBC with an IV of 16 spaces.
    let mut prev = [b' '; 16];
    let mut plain = Vec::with_capacity(sealed.len() - 3);
    for block in sealed[3..].chunks(16) {
        let mut b = GenericArray::clone_from_slice(block);
        cipher.decrypt_block(&mut b);
        plain.extend(b.iter().zip(prev).map(|(x, p)| x ^ p));
        prev.copy_from_slice(block);
    }
    let pad = *plain.last()? as usize;
    if !(1..=16).contains(&pad) || !plain[plain.len() - pad..].iter().all(|b| *b as usize == pad) {
        return None;
    }
    plain.truncate(plain.len() - pad);
    // From the database's version 24, a value starts with its domain's SHA-256.
    let h = sha2::Sha256::digest(host.as_bytes());
    if version >= 24 && plain.starts_with(&h) {
        plain.drain(..h.len());
    }
    Some(String::from_utf8_lossy(&plain).to_string())
}

/// Opens the cookies in a chrome device's profile and deletes its cookie files, for Chrome to start without them.
/// Partitioned cookies, and ones expired or that won't open, are left out.
pub fn take_cookies(dir: &Path) -> Result<Vec<Value>> {
    let files = ["Default/Network/Cookies", "Default/Cookies"];
    let Some(db) = files.iter().map(|f| dir.join(f)).find(|f| f.exists()) else { return Ok(Vec::new()) };
    let read = read_cookies(&db);
    for f in files {
        let _ = std::fs::remove_file(dir.join(f));
        let _ = std::fs::remove_file(dir.join(format!("{f}-journal")));
    }
    read.map_err(|err| anyhow!("reading the profile's sign-ins: {err:#}"))
}

fn read_cookies(db: &Path) -> Result<Vec<Value>> {
    let conn = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let version: i64 = conn.query_row("select value from meta where key = 'version'", [], |r| r.get::<_, String>(0)).ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut stmt = conn.prepare("select * from cookies")?;
    let names: Vec<String> = stmt.column_names().map(|n| n.to_string()).collect();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let mut cookies = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let col = |name: &str| names.iter().position(|n| n == name);
        let text = |name: &str| col(name).and_then(|i| r.get::<_, Option<String>>(i).ok().flatten()).unwrap_or_default();
        let num = |name: &str| col(name).and_then(|i| r.get::<_, Option<i64>>(i).ok().flatten()).unwrap_or(0);
        if !text("top_frame_site_key").is_empty() {
            continue;
        }
        let host = text("host_key");
        let mut value = text("value");
        let sealed: Vec<u8> = col("encrypted_value").and_then(|i| r.get::<_, Option<Vec<u8>>>(i).ok().flatten()).unwrap_or_default();
        if !sealed.is_empty() {
            match open_cookie(&sealed, &host, version) {
                Some(v) => value = v,
                None => continue,
            }
        }
        let mut c = json!({ "name": text("name"), "value": value, "domain": host, "path": text("path"), "secure": num("is_secure") == 1, "httpOnly": num("is_httponly") == 1 });
        // Chrome counts microseconds from 1601.
        if num("has_expires") == 1 || col("has_expires").is_none() && num("expires_utc") > 0 {
            let expires = num("expires_utc") / 1_000_000 - 11_644_473_600;
            if expires <= now {
                continue;
            }
            c["expires"] = json!(expires);
        }
        if let Some(s) = ["None", "Lax", "Strict"].get(num("samesite") as usize).filter(|_| num("samesite") >= 0) {
            c["sameSite"] = json!(s);
        }
        if let Some(p) = ["Low", "Medium", "High"].get(num("priority") as usize).filter(|_| num("priority") >= 0) {
            c["priority"] = json!(p);
        }
        cookies.push(c);
    }
    Ok(cookies)
}
