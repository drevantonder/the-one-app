//! onet on a computer, `<cli>t`: the computer's way into its network, holding its device's key (`<home>/device.json`),
//! and answering this computer's own programs (the CLI, the local MCP server, the desktop app, harnesses) on its
//! loopback, with the token in its home, as the node does today (`one::api::LOCAL_TOKEN`). What a client answers itself
//! it answers (`one::settings::answered`); joining with a pairing code, and what the recovery key does here, it does; every
//! other request goes, as this device, to the best machine for it: its own app runtime by its loopback, which it asks
//! as this computer's own program, or another over iroh, with the keys it opens. Started by hand, or by a test, until
//! the computer's oned starts it (ARCHITECTURE.md, Designed): `--listen=<addr>` where it answers (the API's own address,
//! `one::api::addr`, unless something else answers there), `--runtime=<addr>` where this device's own app runtime does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use one::config::{CLI, NAME};
use one::node::NetworkFile;

use crate::Device;
use crate::keyring::Address;

mod serve;

/// What `<cli>t` is, for `--help` and for an argument it doesn't take.
fn usage() -> String {
    format!("{CLI}t is {NAME}'s way into the network on this computer: {CLI}t [--listen=<address>] [--runtime=<address of this device's own app runtime>]")
}

/// The name it runs by.
pub fn name() -> String {
    format!("{CLI}t")
}

pub fn invoked() -> bool {
    one::os::invoked_as().is_some_and(|me| me == name())
}

/// Where this device's own app runtime answers on this computer, and what it's asked with: a runtime of its own key, by
/// the token in its folder (`one::node::runtime_folder`), read as it's asked, as the runtime makes it as it starts; while
/// the device's node is its own runtime, listed as compute under the device's key, by the home's token, as its own
/// programs ask it.
pub struct Runtime {
    pub at: String,
    pub folder: PathBuf,
    pub home_token: String,
}

impl Runtime {
    /// The runtime `key` listed as this device's, the device `me`'s, as it's asked.
    pub(crate) fn local(&self, key: [u8; 32], me: &[u8; 32]) -> crate::Local {
        let token: Arc<dyn Fn() -> Option<String> + Send + Sync> = match key == *me {
            true => {
                let home = self.home_token.clone();
                Arc::new(move || Some(home.clone()))
            }
            false => {
                let folder = self.folder.clone();
                Arc::new(move || one::api::local_token(&folder))
            }
        };
        crate::Local { key, addr: self.at.clone(), token }
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
    runtime: Option<String>,
    device: tokio::sync::RwLock<Option<Arc<Device>>>,
    /// Whether it keeps this computer's harnesses and git, as it does once from its first start in a network.
    keeping: std::sync::atomic::AtomicBool,
    /// Held while it runs, so no other onet signs as this device at once.
    _lock: std::fs::File,
}

pub fn run(args: &[String]) -> Result<()> {
    let flag = |prefix: &str| args.iter().find_map(|a| a.strip_prefix(prefix)).map(str::to_string);
    if args.iter().any(|a| !a.starts_with("--listen=") && !a.starts_with("--runtime=")) {
        bail!("{}", usage());
    }
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(async {
        tracing_subscriber::fmt().with_writer(std::io::stderr).with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "onet=info,one=info,warn".into())).init();
        let onet = Arc::new(Onet::open(&one::home(), flag("--listen=").unwrap_or_else(one::api::addr), flag("--runtime="))?);
        let starting = onet.clone();
        tokio::spawn(async move {
            if let Err(err) = starting.start().await {
                tracing::warn!("{err:#}");
            }
        });
        serve::serve(onet).await
    })
}

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
        Ok(Onet { home: home.to_path_buf(), key, name, token, listen, runtime, device: Default::default(), keeping: Default::default(), _lock: lock })
    }

    /// Opens its device, if its home says it's in a network, and reaches its nodes; and while it answers the API's own
    /// address, keeps this computer's harnesses and git pointed at it.
    async fn start(self: &Arc<Self>) -> Result<()> {
        if NetworkFile::read(&self.home)?.is_none() {
            tracing::info!("{} answering {} from {}, in no network yet", name(), self.listen, self.home.display());
            return Ok(());
        }
        let device = self.device().await?;
        tracing::info!("{} {} answering {} from {}", name(), &device.key()[..8], self.listen, self.home.display());
        if self.listen == one::api::addr() && matches!(one::mode::here(), one::mode::Mode::Own) && !self.keeping.swap(true, std::sync::atomic::Ordering::Relaxed) {
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
        Ok(())
    }

    /// Its device, once its home says it's in a network: opened the first time it's asked for after its keys reach it.
    async fn device(&self) -> Result<Arc<Device>> {
        if let Some(device) = self.device.read().await.clone() {
            return Ok(device);
        }
        let network = NetworkFile::read(&self.home)?.with_context(|| format!("this computer isn't in a network yet: {} join <code> joins one", one::config::CLI))?;
        let mut held = self.device.write().await;
        if let Some(device) = held.clone() {
            return Ok(device);
        }
        let device = self.open_device(&network).await?;
        *held = Some(device.clone());
        Ok(device)
    }

    /// Its device in the network its home names: its keyring kept in `<home>/onet`, first pinned to the network's root
    /// there, so it never takes another's log.
    async fn open_device(&self, network: &NetworkFile) -> Result<Arc<Device>> {
        let kept = self.home.join("onet");
        std::fs::create_dir_all(&kept)?;
        if !kept.join("root").exists() {
            std::fs::write(kept.join("root"), &network.root)?;
        }
        let fields = network.bucket.clone().with_context(|| format!("this computer can't reach {}'s bucket", one::config::YOURS))?;
        // S3 is reached through the URL signer at the network's address, which a computer that joined keeps beside it,
        // and this device's own runtime knows otherwise.
        let joined = match (fields.get("address"), fields.contains_key("endpoint")) {
            (Some(address), _) => Some(address.clone()),
            (None, true) => Some(self.runtime_says_address().await?),
            (None, false) => None,
        };
        let address: one::bucket::Address = Arc::new(move || joined.clone().context("a folder is reached without the network's address"));
        let place = one::bucket::Place::network(&fields, address, self.key.clone())?;
        let runtime = self.runtime.clone().map(|at| Runtime { at, folder: one::node::runtime_folder(&self.home), home_token: self.token.clone() });
        Ok(Arc::new(Device::on_computer(self.key.to_bytes(), &kept.to_string_lossy(), place, runtime).await.map_err(|e| anyhow::anyhow!("{e}"))?))
    }

    /// The network's address, as this device's own app runtime says it, asked as either runtime might be (`Runtime`).
    async fn runtime_says_address(&self) -> Result<String> {
        let runtime = self.runtime.clone().with_context(|| format!("this computer's network file doesn't say where {}'s bucket is asked for, and no runtime of its own was given to ask", one::config::YOURS))?;
        let tokens: Vec<String> = one::api::local_token(&one::node::runtime_folder(&self.home)).into_iter().chain([self.token.clone()]).collect();
        tokio::task::spawn_blocking(move || -> Result<String> {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
            for token in tokens {
                let mut answer = agent.get(format!("http://{runtime}/api/state")).header(one::protocol::header::LOCAL_TOKEN, token).call()?;
                if answer.status() == 200 {
                    let state: serde_json::Value = answer.body_mut().read_json()?;
                    return state["network"]["settings"]["address"].as_str().map(str::to_string).with_context(|| format!("this computer's own app runtime doesn't know {}'s address yet", one::config::YOURS));
                }
            }
            bail!("this computer's own app runtime at {runtime} didn't take this computer's tokens")
        })
        .await?
    }
}
