//! Code mode on this device: `one run <file>`, and `execute` and `call` in `one mcp --code`. A script runs in
//! OpenCode's interpreter under a pinned Bun (`core/codemode/run.js`), and each tool it calls comes back here and is
//! asked of the network as this client asks any tool, so what this device fills in of a tool's inputs
//! (`client::InputResolver`) is filled in for a script's calls too, and for `call`'s. A server does the same for both
//! over the web (`OneCore.CodeMode`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use client::Client;
use serde_json::{Map, Value, json};
use sha2::Digest;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

/// The runner's files, and the npm packages it needs, pinned by their lockfile.
const RUNNER: [(&str, &str); 3] = [
    ("package.json", include_str!("../../../core/codemode/package.json")),
    ("bun.lock", include_str!("../../../core/codemode/bun.lock")),
    ("run.js", include_str!("../../../core/codemode/run.js")),
];

/// The Bun a script runs under, as `mise.toml` pins it, and each kind of machine's npm package of it with its
/// sha512, as npm's registry gives it.
const BUN: &str = "1.4.4";
const BUN_PACKAGES: [(&str, &str, &str); 5] = [
    (
        "linux-x86_64",
        "linux-x64",
        "CHbbkmHVM28T1GFq+6GL4Ao4ujwwYwj1CxFFwEFzLanPfNsId9AhbIP0Q4reWuWBkGimUkoQygrK7Rgjtj3vog==",
    ),
    (
        "linux-aarch64",
        "linux-aarch64",
        "ZTVtpf9zmHSBVPX1sBwe5agGb/rN34yPGg/Hby8e2Ax2uCUA2URZKlZH7qLntK7qaFJCMXh9v0GWNYN2zINz1w==",
    ),
    (
        "macos-aarch64",
        "darwin-aarch64",
        "zW/hxwc8sJAILGwB5eTC4i48ouGXf0V4lrxmEOk3JIE6JkYVW1TZh1E2pIempqsezgW1sckTIIDudFON8bJ2Mw==",
    ),
    (
        "macos-x86_64",
        "darwin-x64",
        "oKGcxLsvh4oHjAo28nPm8c/xN0o/OmGyj1IHYc94vprkWsW2hNg2keEZt5f40cI5Hvu4jDlSRTR0UNpv1UIAxw==",
    ),
    (
        "windows-x86_64",
        "windows-x64",
        "a3LNctBVO8PjePsFZfwDleF2JXYf2BfQ1u/PLRnBDLnC2Riml+4+LgJpKj4xRGhB92KweGyQI5PXaTkP9qypMQ==",
    ),
];

/// The script's own limits are the runner's: ten minutes, and a minute for its last call to come back.
const STOPPED: Duration = Duration::from_secs(11 * 60);

/// Runs `code`, calling the tools the network lists for whoever this client asks as. Gives whether it ended well, and
/// what it returned or what went wrong, with what it logged.
pub async fn run(client: &Arc<Client>, code: &str) -> Result<(bool, String)> {
    ensure!(!code.trim().is_empty(), "Give the script as code");
    let (listed, skills) =
        tokio::join!(client.request("tools/list", json!({})), client.request("skills/list", json!({})));
    let tools: Vec<Value> = listed?["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| {
            json!({
                "name": tool["name"],
                "plugin": tool["_meta"]["plugin"],
                "description": tool["description"],
                "input": tool["inputSchema"],
                "output": tool["outputSchema"],
            })
        })
        .collect();
    // What each plugin is for, which its own skill says (`skill://<plugin>/<its own name>/SKILL.md`), as a server
    // gives a script (`OneCore.Search.described/1`).
    let namespaces: Map<String, Value> = skills?["skills"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|skill| {
            let path = skill["uri"].as_str()?.strip_prefix("skill://")?.strip_suffix("/SKILL.md")?;
            let [plugin, _own] = path.split('/').collect::<Vec<_>>()[..] else { return None };
            Some((plugin.to_owned(), skill["frontmatter"]["description"].clone()))
        })
        .collect();
    let (bun, folder) = runner().await?;
    let mut child = Command::new(bun)
        .arg("run.js")
        .current_dir(folder)
        .env("DO_NOT_TRACK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let to = Arc::new(Mutex::new(child.stdin.take().context("the runner has a stdin")?));
    let mut from = BufReader::new(child.stdout.take().context("the runner has a stdout")?).lines();
    to.lock()
        .await
        .write_all(format!("{}\n", json!({"code": code, "tools": tools, "namespaces": namespaces})).as_bytes())
        .await?;
    let listening = async {
        while let Some(line) = from.next_line().await? {
            let Ok(response) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(done) = response.get("done") {
                return Ok(Some((done["ok"] == true, done["text"].as_str().unwrap_or_default().to_owned())));
            }
            let (client, to) = (client.clone(), to.clone());
            // Each call runs beside the others, as the script makes them.
            tokio::spawn(async move {
                let called = client
                    .request("tools/call", json!({"name": response["name"], "arguments": response["input"]}))
                    .await;
                let reply = match called {
                    Ok(result) if result["isError"] == true => {
                        json!({"reply": response["call"], "error": texts(&result["content"])})
                    }
                    Ok(result) => json!({"reply": response["call"], "ok": value(&result)}),
                    Err(error) => json!({"reply": response["call"], "error": format!("{error:#}")}),
                };
                // A script that has stopped listening has finished, or been stopped.
                let _ = to.lock().await.write_all(format!("{reply}\n").as_bytes()).await;
            });
        }
        anyhow::Ok(None)
    };
    let done = tokio::time::timeout(STOPPED, listening).await;
    let _ = child.start_kill();
    match done {
        Ok(Ok(Some(done))) => Ok(done),
        Ok(Ok(None)) => {
            let out = child.wait_with_output().await?;
            bail!("The script stopped without an answer: {}", String::from_utf8_lossy(&out.stderr).trim())
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Ok((false, "The script ran past its time and was stopped.".to_owned())),
    }
}

/// `call`'s answer, as MCP's result: each of `calls`, one `{path, input}` or a list of them, asked of the network at
/// once by its path, as search gives it (`tools.gmail.search`) or without its `tools.`. Each answers, in order, with
/// its `path`, `ok`, and its `value` or `error`, and what isn't text of its answer, such as a picture, comes after.
pub async fn call(client: &Arc<Client>, calls: &Value) -> Result<Value> {
    let listed = client.request("tools/list", json!({})).await?;
    let mut named = HashMap::new();
    for tool in listed["tools"].as_array().into_iter().flatten() {
        let name = tool["name"].as_str().unwrap_or_default().to_owned();
        let dotted = match tool["_meta"]["plugin"].as_str() {
            Some(plugin) => format!("{plugin}.{}", name.strip_prefix(&format!("{plugin}_")).unwrap_or(&name)),
            None => name.clone(),
        };
        named.insert(path(&dotted), name.clone());
        named.insert(dotted, name);
    }
    let calls = match calls {
        Value::Array(calls) => calls.clone(),
        call => vec![call.clone()],
    };
    let asked: Vec<_> = calls
        .into_iter()
        .map(|request| {
            let (client, name) = (client.clone(), request["path"].as_str().and_then(|path| named.get(path)).cloned());
            tokio::spawn(async move {
                let path = request["path"].clone();
                let Some(name) = name else {
                    let why = format!(
                        "{} isn't a tool's path. search gives each exactly, as tools.gmail.search.",
                        path.as_str().map_or_else(|| path.to_string(), str::to_owned)
                    );
                    return (path, Err(why));
                };
                let called = client.request("tools/call", json!({"name": name, "arguments": request["input"]})).await;
                match called {
                    Ok(result) if result["isError"] == true => (path, Err(texts(&result["content"]))),
                    Ok(result) => (path, Ok(result)),
                    Err(error) => (path, Err(format!("{error:#}"))),
                }
            })
        })
        .collect();
    let (mut answers, mut files, mut failed) = (Vec::new(), Vec::new(), 0);
    let count = asked.len();
    for answer in asked {
        match answer.await? {
            (path, Ok(result)) => {
                answers.push(json!({"path": path, "ok": true, "value": value(&result)}));
                // What isn't text of its answer, as a picture is, stays MCP's content, for whoever called to see.
                let parts = result["content"].as_array().into_iter().flatten();
                files.extend(parts.filter(|part| part["type"] != "text").cloned());
            }
            (path, Err(why)) => {
                failed += 1;
                answers.push(json!({"path": path, "ok": false, "error": why}));
            }
        }
    }
    let structured = json!({"calls": answers});
    let content: Vec<Value> =
        std::iter::once(json!({"type": "text", "text": structured.to_string()})).chain(files).collect();
    Ok(json!({"content": content, "structuredContent": structured, "isError": failed == count}))
}

/// A tool's path in a script, from its path there, as OpenCode writes it: `tools.gmail.search`,
/// `tools["google-calendar"].events` (`OneCore.Search.path/1`).
fn path(dotted: &str) -> String {
    let identifier = |segment: &str| {
        let mut chars = segment.chars();
        chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
            && chars.all(|next| next.is_ascii_alphanumeric() || next == '_' || next == '$')
    };
    dotted.split('.').fold("tools".to_owned(), |path, segment| {
        if identifier(segment) { format!("{path}.{segment}") } else { format!("{path}[{}]", json!(segment)) }
    })
}

/// A call's answer as JSON (`structuredContent`). A tool of a version deployed before tools said what they answer has
/// none, so its text is read: text that's JSON is that, and other text is a string.
fn value(result: &Value) -> Value {
    if let Some(structured) = result.get("structuredContent") {
        return structured.clone();
    }
    let content = &result["content"];
    let text = texts(content);
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn texts(content: &Value) -> String {
    let parts = content.as_array().map(Vec::as_slice).unwrap_or_default();
    parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n")
}

/// The pinned Bun, and the runner's folder with its packages installed, each made once in this device's home.
async fn runner() -> Result<(PathBuf, PathBuf)> {
    let home = client::home()?;
    let bun = bun(&home).await?;
    let mut hash = sha2::Sha256::new();
    for (path, text) in RUNNER {
        hash.update(format!("{path}\0{}\0{text}", text.len()));
    }
    let hash = data_encoding::HEXLOWER.encode(&hash.finalize());
    let folder = home.join("codemode").join(&hash[..16]);
    if folder.join("node_modules/@opencode/codemode").is_dir() {
        return Ok((bun, folder));
    }
    // Made beside it, then moved into place, so two at once can't see half of one.
    let making = folder.with_extension(std::process::id().to_string());
    tokio::fs::create_dir_all(&making).await?;
    for (path, text) in RUNNER {
        tokio::fs::write(making.join(path), text).await?;
    }
    let installed = Command::new(&bun)
        .args(["install", "--frozen-lockfile", "--production"])
        .current_dir(&making)
        .env("DO_NOT_TRACK", "1")
        .output()
        .await?;
    if !installed.status.success() {
        let _ = tokio::fs::remove_dir_all(&making).await;
        bail!("Code mode's packages didn't install:\n{}", String::from_utf8_lossy(&installed.stderr).trim());
    }
    if tokio::fs::rename(&making, &folder).await.is_err() {
        let _ = tokio::fs::remove_dir_all(&making).await;
    }
    Ok((bun, folder))
}

/// The pinned Bun, downloaded from npm's registry the first time and checked against its sha512.
async fn bun(home: &Path) -> Result<PathBuf> {
    let packages: HashMap<&str, (&str, &str)> =
        BUN_PACKAGES.iter().map(|(target, package, sha)| (*target, (*package, *sha))).collect();
    let target = client::target();
    let (package, integrity) = packages
        .get(target.as_str())
        .with_context(|| format!("Code mode can't run on {target}: Bun has no build for it"))?;
    let folder = home.join(format!("bun-{BUN}"));
    let program = folder.join(format!("bun{}", std::env::consts::EXE_SUFFIX));
    if program.exists() {
        return Ok(program);
    }
    let url = format!("https://registry.npmjs.org/@oven/bun-{package}/-/bun-{package}-{BUN}.tgz");
    let tarball = client::web()?.get(&url).send().await?.error_for_status()?.bytes().await?;
    let digest = data_encoding::BASE64.encode(&sha2::Sha512::digest(&tarball));
    ensure!(digest == *integrity, "Bun didn't download whole from {url}");
    // Unpacked beside it and moved into place in one step, so two at once can't see half of one.
    let making = folder.with_extension(std::process::id().to_string());
    tokio::fs::create_dir_all(&making).await?;
    tokio::fs::write(making.join("package.tgz"), &tarball).await?;
    let member = format!("package/bin/bun{}", std::env::consts::EXE_SUFFIX);
    let unpacked = Command::new("tar")
        .args(["-xzf", "package.tgz", "--strip-components=2", &member])
        .current_dir(&making)
        .status()
        .await?;
    ensure!(unpacked.success(), "Bun didn't unpack");
    tokio::fs::remove_file(making.join("package.tgz")).await?;
    if tokio::fs::rename(&making, &folder).await.is_err() {
        let _ = tokio::fs::remove_dir_all(&making).await;
    }
    Ok(program)
}
