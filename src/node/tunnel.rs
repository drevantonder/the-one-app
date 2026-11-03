//! The install's Cloudflare Tunnel, which the kernel's gateway reaches nodes through with no public address
//! (`kernel set node-url cloudflare:<id>`): an admin keeps its token as a Tunnel item in Everyone's Vault, and every
//! device of the install runs a connector for it, so Cloudflare sends each request to whichever is up.

use sha2::Digest;

use super::*;

/// The Vault item type that holds the install's tunnel's token.
pub const TUNNEL: &str = "Tunnel";

/// Where a connector takes the token: Cloudflare's tunnel servers, `region1.v2.argotunnel.com` and the like.
const EDGE: &str = "argotunnel.com";

/// The cloudflared every device runs, and each platform's download of it with its binary's sha256, as Cloudflare
/// publishes them.
const CLOUDFLARED: &str = "2026.10.7";
const DOWNLOADS: &[(&str, &str, &str)] = &[
    ("x86_64-unknown-linux-gnu", "cloudflared-linux-amd64", "1f99fb518b83dec2ba8d870a7cc991d82a77e6196295a3d2754ba17ab9f91872"),
    ("aarch64-unknown-linux-gnu", "cloudflared-linux-arm64", "3df43aedc9ff291d1735b5cb576cb97e0b157488796db2e32b28aecabb445775"),
    ("aarch64-apple-darwin", "cloudflared-darwin-arm64.tgz", "2136431d5eb467bf447d2ee514b713e192719b143aec1d9e09bf7dc290d5bb12"),
    ("x86_64-apple-darwin", "cloudflared-darwin-amd64.tgz", "5c55e77da7012cebca19170357e196176c48c9647cc2ead404702fa125ddbd86"),
];

impl Node {
    /// Keeps this device's connector running for the install's newest Tunnel item, or stops it when there's none or
    /// this device has left the install.
    pub(super) fn follow_tunnel(&self) {
        let wanted = self.with(|i| {
            if i.r.members.compute(&self.0.id).is_none() {
                return Ok(None);
            }
            let Some((_, global)) = i.views.get("global") else { return Ok(None) };
            let Some(item) = global.items.values().filter(|it| it.kind == TUNNEL && it.apps.contains(members::APP)).max_by_key(|it| it.made) else { return Ok(None) };
            // Secrets stay put: the token goes only to the item's own hosts.
            ensure!(item.hosts.iter().any(|h| h == EDGE), "the Tunnel item's token is for {}, not {EDGE}", item.hosts.join(", "));
            Ok(item.fields.get("token").map(|f| f.value.clone()).filter(|t| !t.is_empty()))
        });
        let wanted = wanted.unwrap_or_else(|err| {
            tracing::warn!("tunnel: {err:#}");
            None
        });
        let mut running = self.0.tunnel.lock().unwrap();
        if running.as_ref().map(|(token, _)| token) == wanted.as_ref() {
            return;
        }
        // Its connector dies with the task.
        if let Some((_, task)) = running.take() {
            task.abort();
            tracing::info!("this device's tunnel connector stopped");
        }
        if let Some(token) = wanted {
            *running = Some((token.clone(), tokio::spawn(connect(token, self.0.dir.clone()))));
        }
    }

    /// Keeps the install's tunnel's token, for an admin: every device runs a connector for it from then on.
    pub async fn set_tunnel(&self, caller: &Caller, token: &str) -> Result<Value> {
        self.with(|i| {
            ensure!(members::manages_scope(&i.r.members, &caller.user, &Scope::Global), "only admins set the install's tunnel");
            Ok(())
        })?;
        let token = token.trim();
        let tunnel = data_encoding::BASE64.decode(token.as_bytes()).ok().and_then(|t| serde_json::from_slice::<Value>(&t).ok()).and_then(|t| t["t"].as_str().map(str::to_string));
        let tunnel = tunnel.context("that isn't a Cloudflare Tunnel's token: copy it from the tunnel's page in Cloudflare, or cloudflared tunnel token <name>")?;
        let fields = json!({ "token": { "value": token, "secret": true } });
        let data = json!({ "title": "Cloudflare Tunnel", "type": TUNNEL, "fields": fields, "hosts": [EDGE], "apps": [members::APP] });
        let id = self.set_item(caller, &Scope::Global, data).await?;
        Ok(json!({ "id": id, "tunnel": tunnel }))
    }

    pub async fn tunnel_route(&self, me: &Caller, method: &str, body: &Value) -> Result<Value> {
        match method {
            "GET" => {
                let set = self.with(|i| Ok(i.views.get("global").is_some_and(|(_, g)| g.items.values().any(|it| it.kind == TUNNEL))))?;
                let running = self.0.tunnel.lock().unwrap().is_some() && running(&self.0.dir.join("cloudflared.pid")).is_some();
                Ok(json!({ "set": set, "running": running, "device": self.device_name() }))
            }
            _ => {
                ensure!(body["agent"].is_null(), "agents never handle the tunnel's token: the person pipes it in");
                self.set_tunnel(me, body["token"].as_str().unwrap_or("")).await
            }
        }
    }
}

/// Runs a connector for the tunnel `token` is for until the task is aborted, starting it again whenever it stops,
/// with its pid and log in the node's folder `dir`.
async fn connect(token: String, dir: PathBuf) {
    let pid = dir.join("cloudflared.pid");
    let mut wait = Duration::from_secs(5);
    loop {
        stop_previous(&pid);
        let run = async {
            let bin = tokio::task::spawn_blocking(cloudflared).await??;
            let mut child = tokio::process::Command::new(bin)
                .args(["tunnel", "--no-autoupdate", "run"])
                // Not on its command line, which anyone on the machine can read.
                .env("TUNNEL_TOKEN", &token)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::fs::File::create(dir.join("cloudflared.log"))?)
                .kill_on_drop(true)
                .spawn()?;
            if let Some(id) = child.id() {
                std::fs::write(&pid, id.to_string())?;
            }
            tracing::info!("this device runs a connector for the install's tunnel");
            let started = Instant::now();
            let status = child.wait().await?;
            if started.elapsed() > Duration::from_secs(60) {
                wait = Duration::from_secs(5);
            }
            Ok::<_, anyhow::Error>(status.to_string())
        };
        match run.await {
            Ok(status) => tracing::warn!("this device's tunnel connector stopped ({status}): starting it again"),
            Err(err) => tracing::warn!("this device's tunnel connector: {err:#}"),
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(600));
    }
}

/// The connector whose pid is in `pid`, if it's still running.
fn running(pid: &Path) -> Option<String> {
    let id = std::fs::read_to_string(pid).ok()?.trim().to_string();
    let comm = std::process::Command::new("ps").args(["-p", &id, "-o", "comm="]).output().ok()?;
    String::from_utf8_lossy(&comm.stdout).contains("cloudflared").then_some(id)
}

/// A connector left behind by an earlier run of the node (one that updated itself) would still be running: stop it.
fn stop_previous(pid: &Path) {
    if let Some(id) = running(pid) {
        let _ = std::process::Command::new("kill").arg(id).status();
    }
    let _ = std::fs::remove_file(pid);
}

/// The pinned cloudflared, downloaded once into the cache and checked against its pinned hash.
fn cloudflared() -> Result<PathBuf> {
    let (_, file, sha256) = DOWNLOADS
        .iter()
        .find(|(target, _, _)| *target == crate::config::TARGET)
        .ok_or_else(|| anyhow!("this device can't run a tunnel connector: cloudflared has no build for {}", crate::config::TARGET))?;
    let dir = crate::cache().join(format!("cloudflared-{CLOUDFLARED}"));
    let path = dir.join("cloudflared");
    if path.exists() {
        return Ok(path);
    }
    let url = format!("https://github.com/cloudflare/cloudflared/releases/download/{CLOUDFLARED}/{file}");
    tracing::info!("downloading {url}");
    let download = ureq::get(&url).call()?.body_mut().with_config().limit(200 << 20).read_to_vec()?;
    let binary = match file.ends_with(".tgz") {
        false => download,
        true => {
            let tmp = dir.with_extension(format!("{}.part", rand::random::<u64>()));
            std::fs::create_dir_all(&tmp)?;
            std::fs::write(tmp.join("download.tgz"), &download)?;
            let unpacked = std::process::Command::new("tar").args(["-xzf", "download.tgz", "cloudflared"]).current_dir(&tmp).status();
            let binary = std::fs::read(tmp.join("cloudflared"));
            let _ = std::fs::remove_dir_all(&tmp);
            ensure!(unpacked?.success(), "couldn't unpack cloudflared");
            binary?
        }
    };
    ensure!(data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(&binary)) == *sha256, "the download of cloudflared doesn't match its pinned hash");
    // Moved into place in one step, so another node starting at once never runs a half-written one.
    std::fs::create_dir_all(&dir)?;
    let part = dir.join(format!("cloudflared.{}.part", rand::random::<u64>()));
    std::fs::write(&part, &binary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&part, &path)?;
    Ok(path)
}
