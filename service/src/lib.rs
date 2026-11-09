//! A service in Rust: a program of its own, which its device's oned starts (`oned <name>`, src/services/endpoint.rs) and tells
//! where to register (`ONE_ADDR`), where the token it sends is (`ONE_HOME`) and where its data goes (`<NAME>_DATA`).
//! It registers, long-polls for calls and answers each on a thread of its own, registering again whenever it's
//! forgotten, until oned stops it. What it says to its node, and the node to it, is `service/wire.json`, which WhatsApp's service
//! in Go and the Mac's in Swift (`service/swift`) say as well; tests/contracts.rs holds them to it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The commit it was built from, as `one-dev update` and the Mac's build give it.
pub const BUILD: &str = match option_env!("ONE_BUILD") {
    Some(b) => b,
    None => "dev",
};

/// Who a call is for, as its node says.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Me {
    pub user: String,
    /// Their device's name.
    pub name: String,
    /// The agent that asked (an MCP client's name, like "Claude Code"), when one did.
    pub agent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Call {
    pub id: u64,
    /// The function it runs.
    pub function: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub caller: Me,
    /// The app whose `ctx.services` made the call, if one did.
    #[serde(default)]
    pub from: Option<String>,
}

/// What a call gives back: its answer, and blobs the caller's scope keeps.
#[derive(Debug, Default)]
pub struct Answer {
    pub ok: Value,
    pub blobs: Vec<String>,
}

impl Answer {
    pub fn new(ok: Value) -> Self {
        Answer { ok, blobs: Vec::new() }
    }
}

/// A service's answer that it can't serve this call here (a model it lacks), so the call goes on to the next device.
#[derive(Debug)]
pub struct Elsewhere(pub String);

impl std::fmt::Display for Elsewhere {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Elsewhere {}

/// A service: its name, which is its app's, and how it answers. What the app is (its title, tools and instructions) the
/// app says; a service says only what's about its machine.
pub struct Service {
    pub name: &'static str,
    /// The functions about its own machine (Shell's `run`), which a call tries on the caller's device first.
    pub prefer_here: &'static [&'static str],
    /// Runs one call, on a thread of its own.
    pub call: fn(host: &Host, call: &Call) -> Result<Answer>,
    /// Starts what it does between calls, once, before it registers.
    pub start: Option<fn(host: &Host)>,
}

/// What a service has: its folder, and its node.
#[derive(Clone)]
pub struct Host {
    pub name: &'static str,
    pub dir: PathBuf,
    node: String,
    agent: ureq::Agent,
}

impl Host {
    pub fn get(&self, path: &str) -> Result<Value> {
        Ok(answer(self.agent.get(format!("{}{path}", self.node)).call())?.unwrap_or_default())
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        Ok(answer(self.agent.post(format!("{}{path}", self.node)).send_json(body))?.unwrap_or_default())
    }

    pub fn post_within(&self, path: &str, body: &Value, limit: Duration) -> Result<Value> {
        Ok(answer(self.agent.post(format!("{}{path}", self.node)).config().timeout_global(Some(limit)).build().send_json(body))?.unwrap_or_default())
    }

    /// The port of its node's loopback.
    pub fn port(&self) -> u16 {
        self.node.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(0)
    }

    pub fn for_test(dir: &std::path::Path) -> Host {
        Host { name: "test", dir: dir.to_path_buf(), node: "http://127.0.0.1:1".into(), agent: ureq::Agent::new_with_defaults() }
    }

    /// Keeps bytes, read from `body` as they come, as a blob the caller's scope keeps once the call names it in its
    /// answer: its hash and size.
    pub fn put_blob(&self, body: impl std::io::Read + Send + 'static) -> Result<(String, u64)> {
        let resp = self.agent.post(format!("{}/v1/blobs", self.node)).header("content-type", "application/octet-stream").send(ureq::SendBody::from_owned_reader(body));
        let put = answer(resp)?.unwrap_or_default();
        let blob = put["blob"].as_str().filter(|b| !b.is_empty()).ok_or_else(|| anyhow!("the node kept no blob"))?;
        Ok((blob.to_string(), put["size"].as_u64().unwrap_or(0)))
    }

    /// A blob's bytes, as they come, from whichever device has it.
    pub fn get_blob(&self, hash: &str) -> Result<impl std::io::Read + Send + 'static> {
        let resp = self.agent.get(format!("{}/v1/blobs/{hash}", self.node)).call()?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.into_body().read_to_string().unwrap_or_default();
            bail!("{status}: {}", body.trim());
        }
        Ok(resp.into_body().into_reader())
    }

    /// `text` with the Vault's fields (`{{name}}`) filled in by the node for call `id`, which only `host` may be sent.
    pub fn fill(&self, id: u64, text: &str, host: &str) -> Result<String> {
        let out = self.post(&format!("/v1/services/{}/calls/{id}/vault", self.name), &json!({ "text": text, "host": host })).map_err(|err| anyhow!("the Vault: {err}"))?;
        Ok(out["text"].as_str().unwrap_or_default().to_string())
    }
}

/// A call's input as `T`, where none is an empty one and a field that's null is one not given, as Go's services read them.
pub fn input<T: serde::de::DeserializeOwned + Default>(input: &Value) -> Result<T> {
    fn given(v: &Value) -> Value {
        match v {
            Value::Object(o) => Value::Object(o.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), given(v))).collect()),
            Value::Array(a) => Value::Array(a.iter().map(given).collect()),
            v => v.clone(),
        }
    }
    Ok(if input.is_null() { T::default() } else { serde_json::from_value(given(input)).map_err(|err| anyhow!("bad input: {err}"))? })
}

fn answer(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Option<Value>> {
    let mut resp = resp?;
    if resp.status() == 204 {
        return Ok(None);
    }
    let body: Value = resp.body_mut().read_json().unwrap_or_default();
    if let Some(err) = body["error"].as_str().filter(|e| !e.is_empty()) {
        bail!("{err}");
    }
    if !resp.status().is_success() {
        bail!("{}", resp.status());
    }
    Ok(Some(body))
}

/// The folder the service `name` keeps its data in, as oned says (`<NAME>_DATA`).
pub fn dir(name: &str) -> PathBuf {
    std::env::var_os(format!("{}_DATA", name.to_uppercase().replace('-', "_"))).map(PathBuf::from).unwrap_or_default()
}

/// Answers `service`'s calls, as oned's environment says, until oned stops this program. Call it before starting a
/// thread.
pub fn serve(service: Service) -> Result<()> {
    let name = service.name;
    let at = std::env::var("ONE_ADDR").map_err(|_| anyhow!("{name} is a service: its device's oned starts it"))?;
    let home = std::env::var_os("ONE_HOME").map(PathBuf::from).ok_or_else(|| anyhow!("ONE_HOME isn't set"))?;
    let dir = dir(name);
    ensure!(!dir.as_os_str().is_empty(), "{name}'s oned says where its data goes");
    std::fs::create_dir_all(&dir)?;
    // Not passed on to what it starts (Shell's commands): `one` there would take the service's folder as its home.
    for var in ["ONE_ADDR", "ONE_HOME", "ONE_CLI"] {
        // SAFETY: before this program starts a thread of its own; nothing reads the environment meanwhile.
        unsafe { std::env::remove_var(var) };
    }
    // Read for each request, as Go's service does, so a node that starts after it still lets it in.
    let token = home.join("local-token");
    let as_this_program = move |mut req: ureq::http::Request<ureq::SendBody>, next: ureq::middleware::MiddlewareNext| {
        if let Some(value) = std::fs::read_to_string(&token).ok().and_then(|t| t.trim().parse().ok()) {
            req.headers_mut().insert("x-one-local-token", value);
        }
        next.handle(req)
    };
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).middleware(as_this_program.clone()).build().into();
    let poll: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(40))).middleware(as_this_program).build().into();
    let host = Arc::new(Host { name, dir, node: format!("http://{at}"), agent });
    if let Some(start) = service.start {
        start(&host);
    }
    let prefer: Value = service.prefer_here.iter().map(|f| (f.to_string(), json!("here"))).collect::<serde_json::Map<_, _>>().into();
    let describe = json!({ "version": BUILD, "prefer": prefer });
    loop {
        if let Err(err) = host.post(&format!("/v1/services/{name}"), &describe) {
            eprintln!("registering: {err:#}");
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }
        loop {
            let call = match answer(poll.get(format!("{}/v1/services/{name}/calls", host.node)).call()) {
                Ok(Some(call)) => match serde_json::from_value::<Call>(call) {
                    Ok(call) => call,
                    Err(err) => {
                        eprintln!("a call it can't read: {err}");
                        continue;
                    }
                },
                Ok(None) => continue,
                Err(err) => {
                    eprintln!("waiting for calls: {err:#}");
                    std::thread::sleep(Duration::from_secs(1));
                    break;
                }
            };
            let (host, run) = (host.clone(), service.call);
            std::thread::spawn(move || {
                // One that panics is answered as failed, not left for its caller to wait out.
                let reply = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&host, &call))) {
                    Ok(Ok(answer)) if answer.blobs.is_empty() => json!({ "ok": answer.ok }),
                    Ok(Ok(answer)) => json!({ "ok": answer.ok, "blobs": answer.blobs }),
                    Ok(Err(err)) if err.is::<Elsewhere>() => json!({ "elsewhere": err.to_string() }),
                    Ok(Err(err)) => json!({ "error": format!("{err:#}") }),
                    Err(_) => json!({ "error": format!("{} failed: it panicked", host.name) }),
                };
                if let Err(err) = host.post(&format!("/v1/services/{}/calls/{}", host.name, call.id), &reply) {
                    eprintln!("answering {}: {err:#}", call.function);
                }
            });
        }
    }
}
