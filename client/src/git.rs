//! Git's remote helper for `one::` addresses (gitremote-helpers): git runs this program as `git-remote-one
//! <remote> <repo>` for `git clone one::users/alice/made/repos/notes`, and every fetch and push after.
//!
//! It runs git's own HTTP transport (`git remote-http`) at an address on this machine that only it answers, for as
//! long as that runs, and carries each request git makes there to a server as one call of the tool `repos_git`,
//! signed as everything this device asks is. So git speaks git, the device's sign-in is the only one, and a repo is
//! reached wherever this device reaches its network. What's too large for a call goes through the bucket, as any
//! large file does.

use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::{env, fs};

use anyhow::{Context, Result, bail};
use data_encoding::BASE64;
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode};
use serde_json::{Value, json};

use crate::Client;

/// The program git runs for `one::` addresses, which is this one by another name.
pub const HELPER: &str = "git-remote-one";
/// Larger than this, what git sends goes through the bucket, as the server sends what's larger back.
const INLINE: usize = 4 << 20;

/// Runs as git's remote helper, with what git gave it: the remote's name and the repo.
pub async fn helper(args: &[String]) -> Result<ExitCode> {
    let [remote, repo] = args else {
        bail!("{HELPER} is run by git, for addresses like one::users/alice/made/repos/notes")
    };
    let repo = repo.trim_start_matches("one::").trim_matches('/').to_owned();
    let client = Client::open(crate::home()?).await?;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let secret = iroh::SecretKey::generate().public().to_string();
    let carried = Arc::new(Carried { client: client.clone(), repo: repo.clone(), secret: secret.clone() });
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let carried = carried.clone();
            let serve = hyper::service::service_fn(move |request| {
                let carried = carried.clone();
                async move { Ok::<_, Infallible>(carried.answer(request).await) }
            });
            tokio::spawn(
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), serve),
            );
        }
    });
    // Git is told, in its environment where nothing else on the machine reads it, the header that says a request
    // is from it, and to go to this machine straight, whatever proxy it's set up with.
    let count: usize = env::var("GIT_CONFIG_COUNT").ok().and_then(|count| count.parse().ok()).unwrap_or(0);
    let status = tokio::process::Command::new("git")
        .args(["remote-http", remote, &format!("http://127.0.0.1:{port}/{repo}")])
        .env("GIT_CONFIG_COUNT", (count + 2).to_string())
        .env(format!("GIT_CONFIG_KEY_{count}"), "http.extraHeader")
        .env(format!("GIT_CONFIG_VALUE_{count}"), format!("One-Secret: {secret}"))
        .env(format!("GIT_CONFIG_KEY_{}", count + 1), "http.proxy")
        .env(format!("GIT_CONFIG_VALUE_{}", count + 1), "")
        // A partial clone's transport would fetch a missing object through this helper, which would ask again.
        .env("GIT_NO_LAZY_FETCH", "1")
        .status()
        .await
        .context("couldn't run git's HTTP transport")?;
    client.close().await;
    Ok(ExitCode::from(status.code().unwrap_or(128).clamp(0, 255) as u8))
}

/// Puts `git-remote-one` beside `one` where `one` is found on the PATH, so git finds it: the same program, which
/// knows it's the helper by the name it's run as. A folder this can't write in, as a system's, is left to whoever
/// installed `one` there, so this says nothing if it can't.
pub fn beside() {
    let Ok(me) = env::current_exe().and_then(fs::canonicalize) else { return };
    let Some(path) = env::var_os("PATH") else { return };
    let program = |name: &str| format!("{name}{}", env::consts::EXE_SUFFIX);
    for folder in env::split_paths(&path) {
        let one = folder.join(program("one"));
        if fs::canonicalize(&one).is_ok_and(|found| found == me) {
            link(&one, &folder.join(program(HELPER)));
            return;
        }
    }
}

/// A symbolic link to `one`, which follows it as it's updated.
#[cfg(unix)]
fn link(_one: &Path, helper: &Path) {
    if fs::symlink_metadata(helper).is_err() {
        let _ = std::os::unix::fs::symlink(Path::new("one"), helper);
    }
}

/// A hard link to `one.exe`: Windows lets only an administrator make a symbolic link. A hard link is the program as
/// it was when it was linked, so one that an update replaced since is linked again; while git runs the old one,
/// Windows keeps it, and it's linked the next time.
#[cfg(windows)]
fn link(one: &Path, helper: &Path) {
    let same = match (fs::metadata(one), fs::metadata(helper)) {
        (Ok(one), Ok(helper)) => one.len() == helper.len() && one.modified().ok() == helper.modified().ok(),
        _ => false,
    };
    if !same && (fs::remove_file(helper).is_ok() || !helper.exists()) {
        let _ = fs::hard_link(one, helper);
    }
}

/// What carries git's requests for one repo.
struct Carried {
    client: Arc<Client>,
    repo: String,
    secret: String,
}

impl Carried {
    async fn answer(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        match self.carry(request).await {
            Ok((kind, body)) => Response::builder()
                .header("content-type", kind)
                .header("cache-control", "no-cache")
                .body(Full::new(body.into())),
            // Git shows the text of only some refusals, so this says each on git's own way to its person.
            Err((status, why)) => {
                eprintln!("{why:#}");
                Response::builder().status(status).body(Full::new(Bytes::new()))
            }
        }
        .expect("a response is made of what was given")
    }

    async fn carry(&self, request: Request<Incoming>) -> Result<(String, Vec<u8>), (StatusCode, anyhow::Error)> {
        let header = |name: &str| request.headers().get(name).and_then(|value| value.to_str().ok()).map(str::to_owned);
        if header("one-secret").as_deref() != Some(&self.secret) {
            return Err((StatusCode::FORBIDDEN, anyhow::anyhow!("This is for the git this ran.")));
        }
        let wanted = request.uri().path_and_query().map(|path| path.as_str()).unwrap_or_default();
        let Some(path) = wanted.strip_prefix(&format!("/{}/", self.repo)).map(str::to_owned) else {
            return Err((StatusCode::NOT_FOUND, anyhow::anyhow!("{wanted} isn't this repo's")));
        };
        let mut inputs = json!({"repo": self.repo, "path": path});
        for (input, header) in [("protocol", header("git-protocol")), ("encoding", header("content-encoding"))] {
            if let Some(value) = header {
                inputs[input] = json!(value);
            }
        }
        let gone = |error: anyhow::Error| (StatusCode::BAD_GATEWAY, error);
        let body = request.into_body().collect().await.map_err(|error| gone(error.into()))?.to_bytes();
        if body.len() > INLINE {
            inputs["upload"] = json!(self.client.upload("repos", body.to_vec()).await.map_err(gone)?);
        } else if !body.is_empty() {
            inputs["body"] = json!(BASE64.encode(&body));
        }
        let asked = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                           "params": {"name": "repos_git", "arguments": inputs}});
        let answer = self.client.ask(asked.to_string().as_bytes()).await.map_err(gone)?;
        let answer: Value = serde_json::from_slice(&answer).map_err(|error| gone(error.into()))?;
        let result = &answer["result"];
        let part = &result["content"][0];
        if let Some(why) = answer["error"]["message"].as_str() {
            return Err((StatusCode::FORBIDDEN, anyhow::anyhow!("{why}")));
        }
        if result["isError"] == true {
            return Err((StatusCode::FORBIDDEN, anyhow::anyhow!("{}", part["text"].as_str().unwrap_or("refused"))));
        }
        match part["type"].as_str() {
            Some("resource") => {
                let kind = part["resource"]["mimeType"].as_str().unwrap_or("application/octet-stream").to_owned();
                let body = BASE64.decode(part["resource"]["blob"].as_str().unwrap_or_default().as_bytes());
                Ok((kind, body.map_err(|error| gone(error.into()))?))
            }
            Some("resource_link") => {
                let kind = part["mimeType"].as_str().unwrap_or("application/octet-stream").to_owned();
                let link = part["uri"].as_str().unwrap_or_default();
                let fetched = async {
                    crate::web()?.get(link).send().await?.error_for_status()?.bytes().await.map_err(anyhow::Error::from)
                };
                Ok((kind, fetched.await.map_err(gone)?.to_vec()))
            }
            _ => Err(gone(anyhow::anyhow!("the server's answer isn't git's"))),
        }
    }
}
