//! Files in a tool's inputs and results on the command line, and printing a result.

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

/// Files up to this size go inline, as base64, in a tool's input.
pub const FILE_LIMIT: usize = 10 << 20;

/// A local file as an input: `{ name, type, size, base64 }`.
pub fn read_file(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    ensure!(bytes.len() <= FILE_LIMIT, "{} is over {} MiB", path.display(), FILE_LIMIT >> 20);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Ok(json!({ "name": name, "type": mime(&name), "size": bytes.len(), "base64": data_encoding::BASE64.encode(&bytes) }))
}

pub fn mime(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "zip" => "application/zip",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "md" => "text/markdown",
        "txt" | "log" => "text/plain",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// A file in a result: an object with `base64` and a `name` or `type` (`mimeType`).
pub fn as_file(v: &Value) -> Option<(String, String, Vec<u8>)> {
    let data = v.get("base64").or_else(|| v.get("data").filter(|_| v.get("mimeType").is_some()))?.as_str()?;
    let ty = v.get("mimeType").or_else(|| v.get("type")).and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let name = v.get("name").and_then(Value::as_str).map(str::to_string);
    if name.is_none() && v.get("type").or_else(|| v.get("mimeType")).is_none() {
        return None;
    }
    let bytes = data_encoding::BASE64.decode(data.as_bytes()).ok()?;
    Some((name.unwrap_or_default(), ty, bytes))
}

/// Prints a tool's result: text as it is, files saved (to `out`, or a temporary file named for them) and their
/// paths printed, anything else as JSON. With `out`, a result that isn't a file is saved there instead.
pub(super) fn show(app: &str, tool: &str, ok: &Value, out: Option<&str>, json: bool) -> Result<()> {
    let mut out = out.map(std::path::PathBuf::from);
    let mut save = |i: usize, name: &str, ty: &str, bytes: &[u8]| -> Result<String> {
        let path = match out.take() {
            Some(p) => p,
            None => {
                let ext = ty.split('/').nth(1).unwrap_or("bin").split(['+', ';']).next().unwrap_or("bin");
                let name = if name.is_empty() { format!("{app}-{tool}-{i}.{ext}") } else { name.replace('/', "_") };
                std::env::temp_dir().join(name)
            }
        };
        std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
        Ok(path.display().to_string())
    };
    if let Some(content) = ok["content"].as_array() {
        let mut shown = Vec::new();
        for (i, c) in content.iter().enumerate() {
            match (c["type"].as_str(), as_file(c)) {
                (Some("image" | "audio"), Some((name, ty, bytes))) => shown.push(json!({ "file": save(i, &name, &ty, &bytes)?, "type": ty })),
                (Some("resource"), _) if c["resource"]["blob"].is_string() => {
                    let r = &c["resource"];
                    let bytes = data_encoding::BASE64.decode(r["blob"].as_str().unwrap_or("").as_bytes())?;
                    let name = r["uri"].as_str().and_then(|u| u.rsplit('/').next()).unwrap_or("");
                    shown.push(json!({ "file": save(i, name, r["mimeType"].as_str().unwrap_or(""), &bytes)?, "type": r["mimeType"] }));
                }
                _ => shown.push(json!({ "text": c["text"].as_str().or(c["resource"]["text"].as_str()).unwrap_or("") })),
            }
        }
        for s in &shown {
            match (json, s["text"].as_str()) {
                (true, _) => {}
                (false, Some(text)) => println!("{text}"),
                (false, None) => println!("{}", s["file"].as_str().unwrap_or("")),
            }
        }
        if json {
            println!("{}", Value::Array(shown));
        }
        return Ok(());
    }
    if let Some((name, ty, bytes)) = as_file(ok) {
        let path = save(0, &name, &ty, &bytes)?;
        match json {
            true => println!("{}", json!({ "file": path, "name": name, "type": ty, "size": bytes.len() })),
            false => println!("{path}"),
        }
        return Ok(());
    }
    let text = match ok {
        _ if json => serde_json::to_string(ok)?,
        Value::String(s) => s.clone(),
        Value::Null => return Ok(()),
        // Laid out for a person at a terminal; compact for an agent's shell or a pipe.
        other if std::io::stdout().is_terminal() => serde_json::to_string_pretty(other)?,
        other => serde_json::to_string(other)?,
    };
    match out {
        Some(path) => {
            std::fs::write(&path, format!("{text}\n")).with_context(|| format!("write {}", path.display()))?;
            println!("{}", path.display());
        }
        None => println!("{text}"),
    }
    Ok(())
}
