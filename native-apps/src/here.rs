//! On a computer the window is a view of the computer's onet (`<cli>t`, which its oned keeps running), holding no key of
//! its own: the device is the computer, and onet holds its one key. The window asks onet as one of the computer's own
//! programs, on its loopback with the token in its home, as the CLI does: every request of the page, where the computer
//! stands with a network, and asking to be let in with a sign-in made here, which names the computer's key. On a
//! computer with no `one`, the window sets it up first (`platform::set_up`), from the network's signed record. While
//! oned starts onet again (an update), the window waits for it, and asks again what only reads (`api::loopback`).

use std::time::Duration;

use serde_json::Value;

/// Where this computer's onet answers: `ADDR` (a throwaway's), or the API's own port.
fn addr() -> String {
    crate::env("ADDR").unwrap_or_else(|| format!("127.0.0.1:{}", custom_build::config::PORT))
}

fn token() -> Result<String, String> {
    std::fs::read_to_string(platform::home().join("local-token")).map(|t| t.trim().to_string()).map_err(|_| format!("This computer's {} isn't running.", onet()))
}

/// onet's name here (`<cli>t`).
fn onet() -> String {
    format!("{}t", custom_build::config::CLI)
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(secs))).http_status_as_error(false).build().into()
}

/// Asks onet `path`, `body` posted or a GET, waiting at most `secs` → its status, and what it said. Blocks.
pub fn ask(path: &str, body: Option<Value>, secs: u64) -> Result<(u16, Value), String> {
    let token = token()?;
    let url = format!("http://{}{path}", addr());
    let agent = agent(secs);
    let mut r = ::api::loopback::while_refused(|| match &body {
        Some(body) => agent.post(&url).header(custom_build::protocol::header::LOCAL_TOKEN, &token).send_json(body),
        None => agent.get(&url).header(custom_build::protocol::header::LOCAL_TOKEN, &token).call(),
    })
    .map_err(|_| format!("This computer's {} isn't answering.", onet()))?;
    Ok((r.status().as_u16(), r.body_mut().read_json().unwrap_or_default()))
}

/// What onet said, or why it didn't: its `error`, or a status that isn't a success.
pub async fn asked(path: &'static str, body: Option<Value>, secs: u64) -> Result<Value, String> {
    let (status, out) = tauri::async_runtime::spawn_blocking(move || ask(path, body, secs)).await.map_err(|e| e.to_string())??;
    match out["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None if !(200..300).contains(&status) => Err("That didn’t work. Try again.".into()),
        None => Ok(out),
    }
}

/// Where this computer stands with a network, as its onet says (`{ in, address, user, join, code }`); none if onet
/// isn't answering. Blocks, for at most 2 s once it answers (`::api::loopback::BACK_WITHIN` while it starts again).
pub fn stands() -> Option<Value> {
    ask("/api/join", None, 2).ok().filter(|(status, _)| *status == 200).map(|(_, out)| out)
}

/// onet answering, its program set up first where it's missing (`platform::set_up`, from the network at `address`) or
/// started where it isn't running → where the computer stands.
pub async fn ready(address: &str) -> Result<Value, String> {
    if let Some(stands) = tauri::async_runtime::spawn_blocking(stands).await.map_err(|e| e.to_string())? {
        return Ok(stands);
    }
    let address = address.to_string();
    tauri::async_runtime::spawn_blocking(move || crate::platform::set_up(&address)).await.map_err(|e| e.to_string())??;
    // oned starts onet as it starts, which then takes a moment to answer: nothing says when, so it's asked again.
    for _ in 0..60 {
        if let Some(stands) = tauri::async_runtime::spawn_blocking(stands).await.map_err(|e| e.to_string())? {
            return Ok(stands);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(format!("This computer's {} didn't start. Its log says why: {} logs this device --service onet", onet(), custom_build::config::CLI))
}

/// A response of onet's, its body coming as it sends it.
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: tokio::sync::mpsc::Receiver<Result<Vec<u8>, String>>,
}

/// A request of the page's, to onet, which answers it or passes it on as this device. One lost as onet stopped is asked
/// again if it only reads, and else answered as failed, which the page says and never sends again.
pub async fn fetch(method: String, path: String, headers: Vec<(String, String)>, body: Vec<u8>) -> Result<Answer, String> {
    let (answered, answer) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let token = match token() {
            Ok(token) => token,
            Err(e) => return drop(answered.send(Err(e))),
        };
        let send = || {
            let mut request = ureq::http::Request::builder().method(method.as_str()).uri(format!("http://{}{path}", addr())).header(custom_build::protocol::header::LOCAL_TOKEN, &token);
            for (name, value) in &headers {
                request = request.header(name, value);
            }
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
            agent.run(request.body(body.clone())?)
        };
        let apps = || ask(api::path::APPS, None, 30).ok().map(|(_, apps)| apps);
        let sent = match ::api::loopback::while_refused(&send) {
            Err(err) if ::api::loopback::lost(&err) && ::api::loopback::reads(&method, &path, &body, apps) => ::api::loopback::while_refused(&send),
            Err(err) if ::api::loopback::lost(&err) => return drop(answered.send(Ok(failed(::api::loopback::lost_write())))),
            sent => sent,
        };
        let response = match sent {
            Ok(response) => response,
            Err(_) => return drop(answered.send(Err(format!("This computer's {} isn't answering.", onet())))),
        };
        let headers = response.headers().iter().filter_map(|(n, v)| Some((n.to_string(), v.to_str().ok()?.to_string()))).collect();
        let (parts, body) = tokio::sync::mpsc::channel(4);
        let status = response.status().as_u16();
        if answered.send(Ok(Answer { status, headers, body })).is_err() {
            return;
        }
        let mut reader = response.into_body().into_reader();
        let mut buf = vec![0; 64 << 10];
        loop {
            let part = match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) => return,
                Ok(n) => Ok(buf[..n].to_vec()),
                Err(e) => Err(e.to_string()),
            };
            let failed = part.is_err();
            // The page stopped reading it.
            if parts.blocking_send(part).is_err() || failed {
                return;
            }
        }
    });
    answer.await.map_err(|e| e.to_string())?
}

/// What onet would have answered had it not stopped: that it failed, saying `error`.
fn failed(error: String) -> Answer {
    let (part, body) = tokio::sync::mpsc::channel(1);
    let _ = part.try_send(Ok(serde_json::json!({ "error": error }).to_string().into_bytes()));
    Answer { status: 502, headers: vec![("content-type".into(), "application/json".into())], body }
}
