//! onet on a computer, `<cli>t`: the computer's way into its network, holding its device's key (`<home>/device.json`),
//! and answering this computer's own programs (the CLI, the local MCP server, the desktop app, harnesses) on its
//! loopback, with the token in its home (`one::api::LOCAL_TOKEN`). What a client answers itself it answers
//! (`one::settings::answered`); joining with a pairing code, and what the recovery key does here, it does; every other
//! request goes, as this device, to the best machine for it: its own app runtime by its loopback, which it asks as this
//! computer's own program, or another over iroh, with the keys it opens. Until its network has an address, all of that
//! is its own app runtime's to answer. It holds the device's key for that runtime (`/api/holds`), lists it as the
//! device's the first time it runs (`list_runtime`), tells the relay what the device is on, hears an ask to update
//! it, and says to every app runtime what the computer's oned says its services are doing (`/api/services`). The computer's oned starts it: `--listen=<addr>` where it answers (the API's own address, `one::api::addr`),
//! `--runtime=<addr>` where this device's own app runtime does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use one::config::{CLI, NAME};
use one::node::NetworkFile;

use crate::Device;
use crate::keyring::Address;

mod serve;

pub use serve::OWN;

/// What `<cli>t` is, for `--help` and for an argument it doesn't take.
fn usage() -> String {
    format!("{CLI}t is {NAME}'s way into the network on this computer: {CLI}t [--listen=<address>] [--runtime=<address of this device's own app runtime>]")
}

/// The name it runs by.
pub fn name() -> String {
    one::oned::onet()
}

pub fn invoked() -> bool {
    one::os::invoked_as().is_some_and(|me| me == name())
}

/// Where this device's own app runtime answers on this computer, and its folder, with the token it's asked with
/// (`one::node::runtime_folder`), read as it's asked, as the runtime makes it as it starts.
pub struct Runtime {
    pub at: String,
    pub folder: PathBuf,
}

impl Runtime {
    /// The runtime `key` listed as this device's, as it's asked.
    pub(crate) fn local(&self, key: [u8; 32]) -> crate::Local {
        let folder = self.folder.clone();
        crate::Local { key, addr: self.at.clone(), token: Arc::new(move || one::api::local_token(&folder)) }
    }

    /// `method` of `path` asked of the runtime, as this computer's own program, with `body`: → its status, headers and
    /// body; again while it starts again (`one::loopback`). What it answers before this computer's onet reaches its
    /// network (`Onet::device`).
    async fn ask(&self, method: String, path: String, headers: Vec<(String, String)>, body: Vec<u8>) -> Result<(u16, Vec<(String, String)>, Vec<u8>)> {
        let (url, token) = (format!("http://{}{path}", self.at), one::api::local_token(&self.folder).context("this computer's own app runtime hasn't started yet")?);
        tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
            let mut answer = one::loopback::while_refused(|| {
                let mut request = ureq::http::Request::builder().method(method.as_str()).uri(&url).header(one::protocol::header::LOCAL_TOKEN, &token);
                for (name, value) in &headers {
                    request = request.header(name, value);
                }
                agent.run(request.body(body.clone())?)
            })?;
            let headers = answer.headers().iter().filter_map(|(n, v)| Some((n.to_string(), v.to_str().ok()?.to_string()))).collect();
            Ok((answer.status().as_u16(), headers, answer.body_mut().with_config().limit(64 << 20).read_to_vec()?))
        })
        .await?
    }
}

/// This computer's onet: its home, its key, what its own programs show it, and once it's in a network, its device.
pub(crate) struct Onet {
    home: PathBuf,
    key: one::keys::SigningKey,
    name: String,
    token: String,
    /// Where it answers.
    listen: String,
    /// Where this device's own app runtime answers, if it's given.
    runtime: Option<Runtime>,
    device: tokio::sync::RwLock<Option<Arc<Device>>>,
    /// Whether it keeps this computer's harnesses and git, as it does once from its first start in a network.
    keeping: std::sync::atomic::AtomicBool,
    /// Whether it has told the relay, since it started, that its runtime is the network's and it a client alone.
    told: std::sync::atomic::AtomicBool,
    /// This device asking to be let in, as a user signed in on this computer (`serve::ask`), until it's in.
    asking: tokio::sync::Mutex<Option<Arc<Device>>>,
    /// Held while it runs, so no other onet signs as this device at once.
    _lock: std::fs::File,
}

/// How often onet tells the relay what this device is on, hears an ask left for it, and sees to its listings.
const KEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

pub fn run(args: &[String]) -> Result<()> {
    let flag = |prefix: &str| args.iter().find_map(|a| a.strip_prefix(prefix)).map(str::to_string);
    if args.iter().any(|a| !a.starts_with("--listen=") && !a.starts_with("--runtime=")) {
        bail!("{}", usage());
    }
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(async {
        one::logs::keep(LOG);
        tracing_subscriber::fmt().with_writer(one::logs::Tee).with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "onet=info,one=info,warn".into())).init();
        let onet = Arc::new(Onet::open(&one::home(), flag("--listen=").unwrap_or_else(one::api::addr), flag("--runtime="))?);
        let keeping = onet.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = keeping.start().await {
                    tracing::warn!("{err:#}");
                }
                tokio::time::sleep(KEEP_EVERY).await;
            }
        });
        serve::serve(onet).await
    })
}

/// The name of onet's log on a computer (`one::logs`), which `<cli> logs <device> --service onet` reads.
const LOG: &str = "onet";

impl Onet {
    /// The onet of the computer whose home is `home`. One whose home says it's in a network, with no key, refuses to
    /// start: a new key there would make it a stranger to its own network.
    fn open(home: &Path, listen: String, runtime: Option<String>) -> Result<Onet> {
        std::fs::create_dir_all(home).with_context(|| format!("make {}", home.display()))?;
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(home.join("onet.lock"))?;
        lock.try_lock().map_err(|_| anyhow::anyhow!("another {} runs for {} already: only one signs as this device", name(), home.display()))?;
        if !home.join("device.json").exists() && NetworkFile::read(home)?.is_some() {
            bail!("{} says this computer is in a network, but its key (device.json) isn't there: a new one would make it a stranger to its network, so {} won't start. Put device.json back.", home.join("network").display(), name());
        }
        let (key, name) = one::node::device_key(home)?;
        let token = one::node::load_local_token(home)?;
        let runtime = runtime.map(|at| Runtime { at, folder: one::node::runtime_folder(home) });
        Ok(Onet { home: home.to_path_buf(), key, name, token, listen, runtime, device: Default::default(), keeping: Default::default(), told: Default::default(), asking: Default::default(), _lock: lock })
    }

    /// What it does from its start, and again every so often (`KEEP_EVERY`), if its home says it's in a network: lists
    /// its app runtime as its device's (`list_runtime`); and once the network has an address, keeps it in its home for
    /// `<cli> update` and oned, reaches its nodes, tells the relay what this device is on and hears an ask left for it;
    /// and while it answers the API's own address, keeps this computer's harnesses and git pointed at it.
    async fn start(self: &Arc<Self>) -> Result<()> {
        if NetworkFile::read(&self.home)?.is_none() {
            return Ok(());
        }
        let device = self.device().await?;
        self.list_runtime(&device).await?;
        if device.address().is_empty() {
            return Ok(());
        }
        one::update::keep_address(&self.home, device.address())?;
        if !self.told.load(std::sync::atomic::Ordering::Relaxed) {
            self.tell_relay_of(&device).await?;
            self.told.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if self.listen == one::api::addr() && matches!(one::mode::here(), one::mode::Mode::Own) && !self.keeping.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::info!("{} {} answering {} from {}", name(), &device.key()[..8], self.listen, self.home.display());
            one::git_wire::wire_here(device.address().to_string());
            let asked = self.clone();
            let apps = move || {
                let onet = asked.clone();
                async move {
                    let device = onet.device.read().await.clone()?;
                    let response = device.fetch("GET", one::protocol::path::APPS, Vec::new(), Vec::new()).await.ok().filter(|r| r.status == 200)?;
                    serde_json::from_slice(&response.body.bytes().await.ok()?).ok()
                }
            };
            tokio::spawn(one::harnesses::keep(apps, self.home.clone(), self.listen.clone()));
        }
        device.connect().await.map_err(|err| anyhow::anyhow!("reaching {}'s devices: {err}", one::config::YOURS))?;
        let asked = device.told().await.map_err(|err| anyhow::anyhow!("telling {}'s relay what this device is on: {err}", one::config::YOURS))?;
        if !asked.is_null() {
            device.keyring().who(|who| one::node::heard(who, &self.key.verifying_key().to_bytes(), &asked));
        }
        Ok(())
    }

    /// Its device, once its home says it's in a network: kept once who's who names the network's address, which it
    /// reaches the network at; till then opened afresh each time, for what needs only who's who and the bucket.
    async fn device(&self) -> Result<Arc<Device>> {
        if let Some(device) = self.device.read().await.clone() {
            return Ok(device);
        }
        let network = NetworkFile::read(&self.home)?.with_context(|| format!("this computer isn't in a network yet: {} join <code> joins one", one::config::CLI))?;
        let mut held = self.device.write().await;
        if let Some(device) = held.clone() {
            return Ok(device);
        }
        let device = Arc::new(self.open_device(&network).await?);
        if !device.address().is_empty() {
            *held = Some(device.clone());
        }
        Ok(device)
    }

    /// Its device in the network its home names: its keyring kept in `<home>/onet`, first pinned to the network's root
    /// there, so it never takes another's log.
    async fn open_device(&self, network: &NetworkFile) -> Result<Device> {
        let kept = self.home.join("onet");
        std::fs::create_dir_all(&kept)?;
        if !kept.join("root").exists() {
            std::fs::write(kept.join("root"), &network.root)?;
        }
        let fields = network.bucket.clone().with_context(|| format!("this computer can't reach {}'s bucket", one::config::YOURS))?;
        // S3 is reached through the URL signer at the network's address, which a computer keeps beside its fields.
        let joined = fields.get("address").cloned();
        anyhow::ensure!(joined.is_some() || !fields.contains_key("endpoint"), "this computer's network file doesn't say where {}'s bucket is asked for: its oned writes it there as it starts", one::config::YOURS);
        let at = joined.clone().map(|a| a.trim_end_matches('/').to_string());
        let address: one::bucket::Address = Arc::new(move || joined.clone().context("a folder is reached without the network's address"));
        let place = one::bucket::Place::network(&fields, address, self.key.clone())?;
        let runtime = self.runtime.as_ref().map(|r| Runtime { at: r.at.clone(), folder: r.folder.clone() });
        Device::on_computer(self.key.to_bytes(), &kept.to_string_lossy(), place, at, runtime).await.map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Lists this device's app runtime, an endpoint of its own key in its folder, as a compute naming this device, which
    /// then runs nothing itself: what a computer that ran its own things does the first time this onet runs there, and a
    /// new one as it joins or makes a network. The relay is told first, where the network has one, so it's never listed
    /// unreachable. Nothing else changes: the device keeps its key, and what's sealed to it stays as it is.
    async fn list_runtime(&self, device: &Device) -> Result<()> {
        use one::database::scopes::Kind;
        let me = self.key.verifying_key().to_bytes();
        let keyring = device.keyring();
        let Some(listed) = keyring.who(|who| -> Result<_> {
            let Some(mine) = who.machine(&me)?.filter(|m| !m.removed) else { return Ok(None) };
            Ok(Some((who.has_runtime(&me)?, who.may(&mine.user, "invite", one::database::scopes::NETWORK), mine)))
        }) else {
            return Ok(());
        };
        let Some((has_runtime, admin, mine)) = listed? else { return Ok(()) };
        if has_runtime && !mine.is(Kind::Compute) {
            return Ok(());
        }
        if !has_runtime {
            let runtime = one::node::runtime_endpoint(&self.home)?;
            self.tell(device, json!({ "add": one::keys::hex(&runtime.key), "admin": false, "role": "compute" })).await?;
            keyring.write_to_bucket(device, &self.key, &mine.user, one::database::content::runtime(&runtime, &mine.user, &mine.name, &me)).await.map_err(|e| anyhow::anyhow!("listing this device's app runtime: {e}"))?;
            tracing::info!("listed this device's app runtime, {}", one::keys::short(&runtime.key));
        }
        if mine.is(Kind::Compute) {
            let me = one::keys::Endpoint { key: me, seal: mine.seal };
            keyring.write_to_bucket(device, &self.key, &mine.user, one::database::content::device(&me, &mine.user, &mine.name, &[Kind::Client], None)).await.map_err(|e| anyhow::anyhow!("leaving this device's things to its app runtime: {e}"))?;
            self.tell(device, json!({ "add": one::keys::hex(&me.key), "admin": admin, "role": "client" })).await?;
            tracing::info!("this device runs nothing itself now: its app runtime does");
        }
        device.refresh().await.map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Tells the network's relay, once it has one, what who's who says of this device and its runtime: the runtime one of
    /// its endpoints, an admin's if its user is one, and this device a client. A network made on this computer had no
    /// relay when they were listed, and the relay changes nothing it already has.
    async fn tell_relay_of(&self, device: &Device) -> Result<()> {
        use one::database::scopes::Kind;
        let me = self.key.verifying_key().to_bytes();
        let Some(said) = device.keyring().who(|who| -> Result<_> {
            let Some(mine) = who.machine(&me)?.filter(|m| !m.removed) else { return Ok(None) };
            let runtime = who.computes()?.into_iter().find(|c| c.runtime_of() == Some(me)).map(|c| c.key);
            Ok(Some((runtime, who.may(&mine.user, "invite", one::database::scopes::NETWORK), mine.is(Kind::Compute))))
        }) else {
            return Ok(());
        };
        let Some((Some(runtime), admin, false)) = said? else { return Ok(()) };
        self.tell(device, json!({ "add": one::keys::hex(&runtime), "admin": admin, "role": "compute" })).await?;
        self.tell(device, json!({ "add": one::keys::hex(&me), "admin": admin, "role": "client" })).await
    }

    /// Tells the network's relay something, signed by this device, where the network has one yet: one it took already is
    /// told.
    async fn tell(&self, device: &Device, fields: Value) -> Result<()> {
        if device.address().is_empty() {
            return Ok(());
        }
        let signed = one::keys::signed_request(&self.key, one::protocol::RELAY, fields, one::keys::next_at(crate::keyring::now()));
        let (status, out) = Address::ask(device, "POST", &format!("{}/relay/tell", device.address()), None, Some(signed)).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        anyhow::ensure!((200..300).contains(&status) || out["code"] == one::protocol::code::ALREADY, "{}'s relay didn't take that: {}", one::config::YOURS, out["error"].as_str().unwrap_or("it didn't say why"));
        Ok(())
    }
}
