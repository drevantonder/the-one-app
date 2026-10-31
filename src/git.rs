//! `git-remote-<cli>`: git's remote helper for `<cli>://<repo>` URLs, on a machine with a node. Local git builds the
//! packs and indexes them; this keeps them as blobs and moves branches by calling the repos app through the local
//! API. See git's gitremote-helpers(7) for the protocol on stdin and stdout.

use std::io::{BufRead, Write};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

/// The app that keeps repos.
const APP: &str = "repos";

/// Runs the helper for `url`, talking to git over stdin and stdout.
pub fn remote(url: &str) -> Result<()> {
    let repo = url.split_once("://").map(|(_, r)| r).unwrap_or(url).trim_end_matches('/').trim_end_matches(".git").to_string();
    ensure!(!repo.is_empty(), "which repo? {}://<repo>", crate::brand::CLI);
    let api = Api(crate::api::addr());
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut lines = stdin.lock().lines();
    while let Some(line) = lines.next() {
        let line = line?;
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            [] => break,
            ["capabilities"] => write!(out, "fetch\npush\noption\n\n")?,
            ["option", ..] => writeln!(out, "unsupported")?,
            ["list"] | ["list", "for-push"] => {
                let r = api.repo(&repo)?;
                for b in branches(&r) {
                    writeln!(out, "{} refs/heads/{}", b.1, b.0)?;
                }
                if branches(&r).iter().any(|b| b.0 == "main") {
                    writeln!(out, "@refs/heads/main HEAD")?;
                }
                writeln!(out)?;
            }
            ["fetch", ..] => {
                let mut wanted = vec![words[1].to_string()];
                for line in lines.by_ref() {
                    let line = line?;
                    match line.split_whitespace().nth(1) {
                        Some(sha) if line.starts_with("fetch ") => wanted.push(sha.to_string()),
                        _ => break,
                    }
                }
                fetch(&api, &repo, &wanted)?;
                writeln!(out)?;
            }
            ["push", spec] => {
                let mut specs = vec![spec.to_string()];
                for line in lines.by_ref() {
                    let line = line?;
                    match line.strip_prefix("push ") {
                        Some(spec) => specs.push(spec.to_string()),
                        None => break,
                    }
                }
                for spec in specs {
                    let dst = spec.rsplit(':').next().unwrap_or("").to_string();
                    match push(&api, &repo, &spec) {
                        Ok(()) => writeln!(out, "ok {dst}")?,
                        Err(err) => writeln!(out, "error {dst} {}", format!("{err:#}").replace('\n', " "))?,
                    }
                }
                writeln!(out)?;
            }
            other => bail!("git asked for {other:?}, which this helper doesn't do"),
        }
        out.flush()?;
    }
    Ok(())
}

/// A repo's branches: (name, head).
fn branches(repo: &Value) -> Vec<(String, String)> {
    repo["branches"].as_array().into_iter().flatten().filter_map(|b| Some((b["branch"].as_str()?.to_string(), b["head"].as_str()?.to_string()))).collect()
}

/// Makes the commits `wanted` local: indexes each of the repo's packs this repo hasn't had yet.
fn fetch(api: &Api, repo: &str, wanted: &[String]) -> Result<()> {
    if wanted.iter().all(|sha| has(sha)) {
        return Ok(());
    }
    let r = api.repo(repo)?;
    let seen_path = std::path::PathBuf::from(text(&["rev-parse", "--git-dir"])?).join(crate::brand::CLI).join("packs");
    let seen = std::fs::read_to_string(&seen_path).unwrap_or_default();
    let mut kept = seen.clone();
    for pack in r["packs"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if seen.lines().any(|l| l == pack) {
            continue;
        }
        let bytes = api.blob(pack)?;
        git(&["index-pack", "--stdin"], Some(&bytes)).with_context(|| format!("index pack {pack}"))?;
        kept.push_str(pack);
        kept.push('\n');
    }
    std::fs::create_dir_all(seen_path.parent().unwrap())?;
    std::fs::write(&seen_path, kept)?;
    let missing: Vec<&String> = wanted.iter().filter(|sha| !has(sha)).collect();
    ensure!(missing.is_empty(), "the repo's packs don't hold {missing:?}");
    Ok(())
}

/// Pushes one refspec, `[+]<src>:refs/heads/<branch>` (an empty src deletes the branch).
fn push(api: &Api, repo: &str, spec: &str) -> Result<()> {
    let (force, spec) = match spec.strip_prefix('+') {
        Some(s) => (true, s),
        None => (false, spec),
    };
    let (src, dst) = spec.split_once(':').context("a refspec is src:dst")?;
    let branch = dst.strip_prefix("refs/heads/").context("only branches can be pushed")?;
    let r = api.repo(repo).ok();
    let remote: Vec<(String, String)> = r.as_ref().map(branches).unwrap_or_default();
    let base = remote.iter().find(|b| b.0 == branch).map(|b| b.1.clone());
    let head = match src {
        "" => None,
        src => Some(text(&["rev-parse", "--verify", &format!("{src}^{{commit}}")])?),
    };
    if let (Some(base), Some(head), false) = (&base, &head, force) {
        ensure!(has(base), "fetch first: {branch} is at {}, which you don't have", &base[..12]);
        let ok = Command::new("git").args(["merge-base", "--is-ancestor", base, head]).status()?.success();
        ensure!(ok, "{branch} has commits you don't (non-fast-forward): fetch and merge, or push with +");
    }
    let mut pack = None;
    if let Some(head) = &head {
        // Everything the remote's branches already hold that this repo has is left out; the pack is never thin.
        let mut revs = format!("{head}\n");
        for (_, h) in remote.iter().filter(|(_, h)| has(h)) {
            revs.push_str(&format!("^{h}\n"));
        }
        let bytes = git(&["pack-objects", "--revs", "--stdout", "-q"], Some(revs.as_bytes()))?;
        // A pack's header is "PACK", a version, and how many objects it holds.
        if bytes.len() >= 12 && bytes[8..12] != [0, 0, 0, 0] {
            pack = Some(api.put(bytes)?);
        }
    }
    let server = api.get("/v1/state")?["server"]["key"].as_str().unwrap_or("").to_string();
    api.post("/v1/call", json!({ "app": APP, "op": "push", "server": server, "input": { "repo": repo, "branch": branch, "base": base, "head": head, "pack": pack } }))?;
    Ok(())
}

fn has(sha: &str) -> bool {
    Command::new("git").args(["cat-file", "-e", &format!("{sha}^{{commit}}")]).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Runs git, feeding it `input`; its output. Its output never reaches git's end of this helper's stdout.
fn git(args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut child = Command::new("git").args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let input = input.map(|i| i.to_vec());
    let writer = std::thread::spawn(move || {
        if let Some(input) = input {
            let _ = stdin.write_all(&input);
        }
    });
    let out = child.wait_with_output()?;
    let _ = writer.join();
    ensure!(out.status.success(), "git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    Ok(out.stdout)
}

/// What git says, as one trimmed line.
fn text(args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(git(args, None)?)?.trim().to_string())
}

/// The node's local API.
struct Api(String);

impl Api {
    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder().http_status_as_error(false).build().into()
    }

    fn reply(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
        let mut resp = resp.with_context(|| format!("no node is running here: start one with {} serve", crate::brand::CLI))?;
        let body: Value = resp.body_mut().with_config().limit(64 << 20).read_json()?;
        if let Some(err) = body.get("error") {
            bail!("{}", err.as_str().unwrap_or("failed"));
        }
        Ok(body)
    }

    fn get(&self, path: &str) -> Result<Value> {
        Self::reply(Self::agent().get(format!("http://{}{path}", self.0)).call())
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        Self::reply(Self::agent().post(format!("http://{}{path}", self.0)).send_json(&body))
    }

    fn repo(&self, repo: &str) -> Result<Value> {
        match self.post("/v1/call", json!({ "app": APP, "op": "get", "input": { "repo": repo } })) {
            Ok(r) => Ok(r["ok"].clone()),
            Err(err) if format!("{err:#}").contains("no repo") => Ok(json!({ "branches": [], "packs": [] })),
            Err(err) => Err(err),
        }
    }

    fn put(&self, bytes: Vec<u8>) -> Result<String> {
        Ok(Self::reply(Self::agent().post(format!("http://{}/v1/blobs", self.0)).send(&bytes[..]))?["blob"].as_str().unwrap_or("").to_string())
    }

    fn blob(&self, hash: &str) -> Result<Vec<u8>> {
        let mut resp = Self::agent().get(format!("http://{}/v1/blobs/{hash}", self.0)).call()?;
        ensure!(resp.status().is_success(), "no blob {hash}: {}", resp.body_mut().read_to_string().unwrap_or_default());
        Ok(resp.body_mut().with_config().limit(1 << 30).read_to_vec()?)
    }
}
