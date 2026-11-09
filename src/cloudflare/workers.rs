//! The network's Workers, built with the pinned Bun and run in workerd here, or deployed to Cloudflare.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// A binding: (name, value, whether it's secret). A value `service:<name>` is another Worker, or `service:<name>:<entrypoint>`
/// one of its entrypoints, `object:<name>:<class>` another Worker's Durable Object class, `loader:` Worker Loader, `ai:` Workers
/// AI, and `r2:<bucket>` an R2 bucket of the account's.
pub type Binding = (String, String, bool);

/// One of the network's Workers: everything said about it is here.
pub struct Worker {
    pub name: &'static str,
    /// Its module, in `cloudflare/`. Its WebAssembly is the `.wasm` files in its module's folder, modules of its own
    /// (Workers can't compile it).
    pub entry: &'static str,
    /// Its Durable Object's class, if it keeps anything.
    pub class: Option<&'static str>,
    /// Where it's reached and the local port it listens on, if anywhere but by other Workers' bindings.
    place: Option<fn(&Settings) -> (&str, u16)>,
    /// Whether it runs the Cloudflare node's build, which carries its devices' endpoints.
    node_build: bool,
    /// Whether it serves the web client's files, as its static assets.
    pub serves_web: bool,
    /// Whether it reads the network's bucket itself.
    pub reads_bucket: bool,
    bindings: fn(&Settings) -> Vec<Binding>,
}

/// The network's Workers, in the order they're deployed.
///
/// Auth's Durable Object `Auth` holds every user's passkeys and accounts. The paths under `/api/auth` name the
/// passkeys, Google's redirect URI, OAuth clients and access tokens' issuer, so they never change.
pub const WORKERS: &[Worker] = &[
    Worker { name: "auth", entry: "auth/worker.js", class: Some("Auth"), place: Some(|s| (s.auth_url.as_str(), s.ports[0])), node_build: false, serves_web: false, reads_bucket: false, bindings: auth_bindings },
    Worker { name: "relay", entry: "relay/worker.js", class: Some("Relay"), place: None, node_build: false, serves_web: false, reads_bucket: false, bindings: relay_bindings },
    Worker { name: "signer", entry: "signer/worker.js", class: None, place: None, node_build: false, serves_web: false, reads_bucket: true, bindings: signer_bindings },
    Worker { name: "mcp", entry: "mcp/worker.js", class: Some("Client"), place: None, node_build: true, serves_web: false, reads_bucket: true, bindings: mcp_bindings },
    Worker { name: "gateway", entry: "gateway/worker.js", class: None, place: Some(|s| (s.address.as_str(), s.ports[1])), node_build: false, serves_web: true, reads_bucket: false, bindings: gateway_bindings },
    Worker { name: "push", entry: "push/worker.js", class: Some("Push"), place: None, node_build: false, serves_web: false, reads_bucket: false, bindings: push_bindings },
];

/// The Cloudflare node's Worker (`cloudflare/node/`), `<cli>-node`: no address, and an iroh endpoint of its own key, which
/// the relay wakes. It's built with WebAssembly, so not one of `WORKERS`' (`node::modules`).
pub const NODE: Worker = Worker { name: "node", entry: "node/worker.js", class: Some("Node"), place: None, node_build: false, serves_web: false, reads_bucket: true, bindings: node_bindings };

impl Worker {
    pub fn named(name: &str) -> &'static Worker {
        WORKERS.iter().find(|w| w.name == name).expect("one of the network's Workers")
    }

    /// Where it's reached, and the local port it listens on.
    pub fn place<'a>(&self, settings: &'a Settings) -> Option<(&'a str, u16)> {
        self.place.map(|place| place(settings))
    }

    pub fn bindings(&self, settings: &Settings) -> Vec<Binding> {
        (self.bindings)(settings)
    }

    /// Its modules besides its own, (name, bytes): its WebAssembly, and, for the remote MCP server, the Cloudflare node's
    /// build (`node`), which runs its devices' endpoints. None if it needs that and there's none.
    pub fn modules(&self, files: &Files, node: Option<&Modules>) -> Option<Vec<(String, Vec<u8>)>> {
        let mut out = wasm(files, self.entry);
        if self.node_build {
            out.extend(node?.iter().filter(|(name, _, _)| name.starts_with("one_cloudflare_node")).map(|(name, _, bytes)| (name.clone(), bytes.clone())));
        }
        Some(out)
    }

    pub fn worker_name(&self) -> String {
        worker_name(self.name)
    }
}

/// A Worker's WebAssembly modules, (name, bytes): the `.wasm` files in its module's folder.
fn wasm(files: &Files, entry: &str) -> Vec<(String, Vec<u8>)> {
    let dir = entry.rsplit_once('/').map_or("", |(dir, _)| dir);
    files
        .iter()
        .filter_map(|(path, bytes)| {
            let name = path.strip_prefix(dir)?.strip_prefix('/')?;
            (name.ends_with(".wasm") && !name.contains('/')).then(|| (name.to_string(), bytes.clone()))
        })
        .collect()
}

/// Files by their path: the Workers' (in `cloudflare/`), or the web client's.
pub type Files = Vec<(String, Vec<u8>)>;

pub type Modules = Vec<(String, &'static str, Vec<u8>)>;

/// An OIDC provider a network's users can sign in with, and link. Its client is what the network registered with it:
/// `<ID>_CLIENT_ID` and `<ID>_CLIENT_SECRET` beside the Workers' secrets, its redirect URI `<address>/api/auth/callback/<id>`.
#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub issuer: String,
    /// The claim of its signed token that says which domain an account is at. None: the domain of a verified email.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_claim: Option<String>,
    /// The parameter that steers its page to the network's domain, if it has one. Only a hint: nothing is decided by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_hint: Option<String>,
    /// Its client, `(id, secret)`, if the network has registered with it.
    #[serde(skip)]
    pub client: Option<(String, String)>,
}

impl Provider {
    /// The name, beside the Workers' secrets, of one of its client's values.
    fn secret(&self, what: &str) -> String {
        format!("{}_{what}", self.id.to_uppercase().replace('-', "_"))
    }

    /// What auth is told of it, if the network has registered with it.
    fn binding(&self) -> Option<Value> {
        let (id, secret) = self.client.as_ref()?;
        Some(json!({ "id": self.id, "name": self.name, "issuer": self.issuer, "domain_claim": self.domain_claim, "domain_hint": self.domain_hint, "client_id": id, "client_secret": secret }))
    }
}

/// The providers a network starts with: none registered until their client is set.
fn providers() -> Vec<Provider> {
    vec![Provider { id: "google".into(), name: "Google".into(), issuer: "https://accounts.google.com".into(), domain_claim: Some("hd".into()), domain_hint: Some("hd".into()), client: None }]
}

/// The Workers' secrets' schema, with a client for each of the network's providers.
fn schema(settings: &Settings) -> String {
    let mut out = SCHEMA.to_string();
    for p in &settings.providers {
        out += &format!("# The client this network registered with {}, whose redirect URI is {}/api/auth/callback/{}.\n# @optional\n{}=\n# @optional\n{}=\n", p.name, settings.address, p.id, p.secret("CLIENT_ID"), p.secret("CLIENT_SECRET"));
    }
    out
}

/// How a network's Workers are set up, wherever they run: kept in `<home>/cloudflare/settings.json`, with its secrets in
/// `.env.local` beside it, which Varlock reads (see `SCHEMA`): their values, for Workers tried here, or
/// `op(op://…)` references to 1Password, for a real network, so no file here holds them.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct Settings {
    /// The network's name, as sign-in screens say it.
    pub name: String,
    /// The network's address, the one users use: the gateway's, which sends signing in to auth, MCP to the MCP
    /// Worker, the relay's paths to the relay and the rest to the web client. Passkeys belong to its host name, so it
    /// doesn't change.
    pub address: String,
    /// Where the CLI's Cloudflare commands reach auth; users reach it at the address.
    pub auth_url: String,
    /// Whether the network has a Cloudflare node (`cloudflare node deploy`), which the relay wakes.
    #[serde(default)]
    pub cloudflare_node: bool,
    /// The local ports auth and the gateway listen on, when this machine runs them. The other services
    /// have none: other Workers reach them by binding.
    pub ports: [u16; 2],
    /// The network's devices (`{ key, role, admin }`: key in hex, roles joined by spaces), and its recovery key as a
    /// client, from the node on this machine when the Workers are deployed or served: the relay starts its list with
    /// them when it has none yet.
    #[serde(skip)]
    pub devices: Vec<serde_json::Value>,
    /// The network's root (hex), from the node on this machine as `devices` is: what a sign-in for a
    /// device names beside it (`sign_in_proof`). None for Workers made before their network.
    #[serde(skip)]
    pub root: Option<String>,
    /// The Cloudflare node's endpoint key (hex), which only its Worker holds: its public half is on the relay's list.
    #[serde(default, skip_serializing)]
    pub cloudflare_node_key: String,
    /// Better Auth's own secret, for its cookies and the keys it keeps.
    #[serde(default, skip_serializing)]
    pub auth_secret: String,
    /// Nodes' own addresses MCP clients sign in for (each a resource, in OAuth's words), besides the network's.
    #[serde(default)]
    pub resources: Vec<String>,
    /// The network's bucket: `{ endpoint, bucket, region, key_id, secret }`, the URL signer's only secret: nothing else
    /// holds its credentials.
    #[serde(default, skip_serializing)]
    pub bucket: Option<Value>,
    /// The SHA-256 fingerprints of the certificates the Android app is signed with: Android opens links to the address
    /// in the app (App Links) only if the address lists the app's package and one of them.
    #[serde(default)]
    pub android: Vec<String>,
    /// The OIDC providers its users can sign in with: a button shows only for one whose client is registered.
    #[serde(default = "providers")]
    pub providers: Vec<Provider>,
    /// The API token `cloudflare deploy` and `cloudflare node deploy` edit the Cloudflare account's Workers with, where
    /// CLOUDFLARE_API_TOKEN isn't set.
    #[serde(default, skip_serializing)]
    pub cloudflare_token: Option<String>,
    /// The push Worker's secrets, by their names in `SCHEMA` (`PUSH_SECRETS`): the network's VAPID key, which is made
    /// with the Workers, and the custom build's APNs, FCM and WNS credentials, as its admin adds them.
    #[serde(default, skip)]
    pub push: std::collections::BTreeMap<String, String>,
    /// A throwaway network's one user, `(id, name)`, whom its auth lets in with no passkey and no Google, so clients
    /// are tested with nobody there. Only `one-dev throwaway` sets it, on the Workers it runs beside its throwaway network, listening
    /// only on this machine; `deploy` refuses settings with it, so no Workers users use ever have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throwaway_user: Option<(String, String)>,
    /// The device the Workers are deployed from, `(key in hex, name as it was)`: the first to deploy them, where their
    /// secrets are. Another device's deploy would upload over its, and make the secrets they make for themselves
    /// again (`at_home`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<(String, String)>,
    /// The node home these are kept in (`load_in`): this machine's, or a throwaway network's.
    #[serde(skip)]
    pub(super) dir: PathBuf,
}

/// What Varlock reads the Workers' secrets by, each named as the Workers' binding is; `.env.local` gives their values.
const SCHEMA: &str = "\
# The Workers' secrets, read with Varlock: their values are in .env.local, as written for Workers tried on this
# machine, or op(op://…) references to 1Password for a real network. Never print them.
# @defaultSensitive=true
# @defaultRequired=true
# ---
AUTH_SECRET=
# The network's bucket's credentials, which only the URL signer holds.
# @optional
BUCKET=
# The API token cloudflare deploy edits the Cloudflare account's Workers with, where CLOUDFLARE_API_TOKEN isn't set.
# @optional
CLOUDFLARE_API_TOKEN=
# The Cloudflare node's endpoint key, made by cloudflare node deploy.
# @optional
CLOUDFLARE_NODE_KEY=
# The push Worker's: the network's VAPID key (base64url PKCS#8, P-256), made with the Workers, which browsers' pushes
# are signed with.
# @optional
VAPID_KEY=
# The custom build's APNs key: its .p8 file's text, its key id and the Apple developer team's id.
# @optional
APNS_KEY=
# @optional
APNS_KEY_ID=
# @optional
APNS_TEAM_ID=
# The custom build's Firebase project's service account key (its JSON), which Android's pushes are sent with.
# @optional
FCM_ACCOUNT=
# The Windows app's Microsoft Entra app registration, which Windows' pushes are sent with.
# @optional
WNS_TENANT_ID=
# @optional
WNS_CLIENT_ID=
# @optional
WNS_CLIENT_SECRET=
";

const PUSH_SECRETS: &[&str] = &["VAPID_KEY", "APNS_KEY", "APNS_KEY_ID", "APNS_TEAM_ID", "FCM_ACCOUNT", "WNS_TENANT_ID", "WNS_CLIENT_ID", "WNS_CLIENT_SECRET"];

/// A new VAPID key: base64url PKCS#8 of a P-256 key, as the push Worker imports it.
fn vapid_key() -> String {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng).expect("making a P-256 key");
    B64U.encode(pkcs8.as_ref())
}

/// Keeps `value` in a new item of 1Password's `vault`, titled `title`, handed to `op` on its standard input so no
/// command line shows it: → the item's id.
fn in_1password(vault: &str, title: &str, value: &str) -> Result<String> {
    use std::io::Write;
    let template = json!({ "title": title, "category": "PASSWORD", "fields": [{ "id": "password", "type": "CONCEALED", "purpose": "PASSWORD", "label": "password", "value": value }] });
    let mut op = std::process::Command::new("op")
        .args(["item", "create", "--vault", vault, "--format", "json", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("the Workers' other secrets are in 1Password, and op (1Password's CLI) isn't here to keep this one there")?;
    op.stdin.take().unwrap().write_all(template.to_string().as_bytes())?;
    let out = op.wait_with_output()?;
    ensure!(out.status.success(), "1Password didn't keep it: {}", String::from_utf8_lossy(&out.stderr).trim());
    let item: Value = serde_json::from_slice(&out.stdout).context("op said something other than the item it made")?;
    Ok(item["id"].as_str().context("op didn't say the item's id")?.to_string())
}

/// The Varlock `cloudflare` reads secrets with, and each platform's download of it with the download's sha256, as its
/// release's checksums.txt says.
const VARLOCK: &str = "1.24.0";
const VARLOCK_DOWNLOADS: &[(&str, &str, &str)] = &[
    ("x86_64-unknown-linux-gnu", "varlock-linux-x64.tar.gz", "254ed7df6959852296919d885494f851cdeab98c544c584316f42125c56a0519"),
    ("aarch64-unknown-linux-gnu", "varlock-linux-arm64.tar.gz", "8da290816331b51dc0d21b3d574c2003b88d33195da48e5c193d04e54d85b81d"),
    ("aarch64-apple-darwin", "varlock-macos-arm64.tar.gz", "4ea453c1d93cde3bfc761dbe7a8b29bf309d451a34a7c6661c06f1a131ffcea4"),
    ("x86_64-apple-darwin", "varlock-macos-x64.tar.gz", "745842687c0c0bf06bc598084e664384e78ee9e6025a28a7fa30f7a3f3c04fae"),
];

/// The pinned Varlock, downloaded once into the cache and checked against its pinned hash.
fn varlock() -> Result<PathBuf> {
    let (_, file, sha256) = VARLOCK_DOWNLOADS
        .iter()
        .find(|(target, _, _)| *target == crate::config::TARGET)
        .ok_or_else(|| anyhow!("this machine can't run the Workers: Varlock has no build for {}", crate::config::TARGET))?;
    let dir = crate::cache().join(format!("varlock-{VARLOCK}"));
    let path = dir.join("varlock");
    if path.exists() {
        return Ok(path);
    }
    let url = format!("https://github.com/dmno-dev/varlock/releases/download/varlock@{VARLOCK}/{file}");
    let download = ureq::get(&url).call()?.body_mut().with_config().limit(200 << 20).read_to_vec()?;
    ensure!(data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(&download)) == *sha256, "the download of Varlock doesn't match its pinned hash");
    // Unpacked beside where it goes and moved into place in one step, so another run at once never runs half of it.
    let tmp = dir.with_extension(format!("{}.part", rand::random::<u64>()));
    std::fs::create_dir_all(&tmp)?;
    std::fs::write(tmp.join("download.tgz"), &download)?;
    let unpacked = std::process::Command::new("tar").args(["-xzf", "download.tgz", "./varlock"]).current_dir(&tmp).status();
    std::fs::create_dir_all(&dir)?;
    let moved = std::fs::rename(tmp.join("varlock"), &path);
    let _ = std::fs::remove_dir_all(&tmp);
    ensure!(unpacked?.success(), "couldn't unpack Varlock");
    moved?;
    Ok(path)
}

/// Where the Workers' secrets' values are: `KEY=value` lines, Varlock's `.env.local`.
fn local(home: &Path) -> PathBuf {
    Settings::path_in(home).with_file_name(".env.local")
}

/// Writes a file only its owner may read, in one step.
fn write_private(path: &Path, text: &str) -> Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    let part = path.with_extension("part");
    std::fs::write(&part, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(std::fs::rename(part, path)?)
}

fn none_in(dir: &Path) -> Result<()> {
    let there: Vec<&str> = ["settings.json", ".env.local"].into_iter().filter(|f| dir.join(f).exists()).collect();
    ensure!(there.is_empty(), "{} has Workers' {} already, which didn't load: move them away to make new ones", dir.display(), there.join(" and "));
    Ok(())
}

/// A value as `.env.local` writes it: in single quotes, which Varlock takes as they are.
fn quoted(value: &str) -> String {
    format!("'{value}'")
}

/// This device isn't where the Workers are deployed from (`Settings::at_home`).
#[derive(Debug)]
pub struct NotHome(pub String);

impl std::fmt::Display for NotHome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotHome {}

impl Settings {
    pub fn path() -> PathBuf {
        Settings::path_in(&crate::home())
    }

    fn path_in(home: &Path) -> PathBuf {
        home.join("cloudflare").join("settings.json")
    }

    pub fn load() -> Result<Settings> {
        Settings::load_in(&crate::home())
    }

    /// The settings kept in `home`'s folder: a throwaway network's, read from `one-dev throwaway` with its own home passed
    /// rather than this process told it's there.
    pub fn load_in(home: &Path) -> Result<Settings> {
        let path = Settings::path_in(home);
        let text = std::fs::read_to_string(&path).with_context(|| format!("this machine has no Cloudflare settings yet: {} cloudflare serve --address <url> runs the Workers here, and their settings are on the machine that deployed them", crate::config::CLI))?;
        let mut settings: Settings = serde_json::from_str(&text).with_context(|| format!("{} isn't the Workers' settings", path.display()))?;
        settings.dir = home.to_path_buf();
        let dir = path.parent().unwrap();
        std::fs::write(dir.join(".env.schema"), schema(&settings))?;
        let out = std::process::Command::new(varlock()?)
            .args(["load", "--format", "json"])
            .current_dir(dir)
            .env("VARLOCK_TELEMETRY_DISABLED", "true")
            .env("DO_NOT_TRACK", "1")
            .output()?;
        // Varlock masks secrets in what it says when it can't load them.
        ensure!(out.status.success(), "Varlock couldn't read the Workers' secrets ({}):\n{}{}", local(home).display(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        let secrets: std::collections::BTreeMap<String, String> = serde_json::from_slice(&out.stdout).context("Varlock said something other than the Workers' secrets")?;
        let get = |name: &str| secrets.get(name).filter(|v| !v.is_empty()).cloned();
        settings.auth_secret = get("AUTH_SECRET").context("no AUTH_SECRET")?;
        settings.bucket = get("BUCKET").map(|b| serde_json::from_str(&b)).transpose().context("BUCKET isn't JSON")?;
        settings.cloudflare_token = get("CLOUDFLARE_API_TOKEN");
        settings.cloudflare_node_key = get("CLOUDFLARE_NODE_KEY").unwrap_or_default();
        settings.push = PUSH_SECRETS.iter().filter_map(|name| Some((name.to_string(), get(name)?))).collect();
        if settings.throwaway_user.is_none() {
            for provider in &mut settings.providers {
                provider.client = get(&provider.secret("CLIENT_ID")).zip(get(&provider.secret("CLIENT_SECRET")));
            }
            // A network with a domain has no other way in for its users, so it's never deployed without one.
            ensure!(crate::config::JOIN_DOMAIN.is_empty() || settings.providers.iter().any(|p| p.client.is_some()), "this network's users sign in with a provider's account, and no provider has its client here: set {} (its redirect URI is {}/api/auth/callback/{})", settings.providers.first().map_or(String::new(), |p| format!("{} and {}", p.secret("CLIENT_ID"), p.secret("CLIENT_SECRET"))), settings.address, settings.providers.first().map_or("", |p| &p.id));
        }
        Ok(settings)
    }

    /// What `sign_in_proof::CLIENTS` says of the clients this network registered, if it has any.
    pub fn sign_in_clients(&self) -> Option<String> {
        let clients: Vec<String> = self.providers.iter().filter_map(|p| Some(format!("{} {}", p.issuer, p.client.as_ref()?.0))).collect();
        (!clients.is_empty()).then(|| clients.join("\n"))
    }

    /// Makes this device the Workers' home if they have none yet, and refuses on any other: they're deployed from one
    /// device only, the one with their secrets.
    pub fn at_home(&mut self) -> Result<()> {
        let (key, name) = crate::node::device_here(&crate::home())?;
        match &self.home {
            Some((home, _)) if *home != key => {
                let name = &self.home.as_ref().unwrap().1;
                return Err(NotHome(format!("this network's Cloudflare Workers are deployed from {name}, where their secrets are: run this there")).into());
            }
            Some((_, was)) if *was == name => return Ok(()),
            _ => {}
        }
        self.home = Some((key, name));
        self.save()
    }

    /// Refuses unless this machine has no Workers' settings or secrets, even ones that don't load, for new ones to be
    /// made: new settings would keep earlier Workers' secrets, or replace settings that only failed to load.
    pub fn none_here() -> Result<()> {
        none_in(Settings::path().parent().unwrap())
    }

    /// New settings, with new secrets.
    pub fn new(name: &str, address: &str) -> Settings {
        let port = crate::config::PORT;
        let ports = [port + 10, port + 13];
        let secret = || B64U.encode(&rand::random::<[u8; 32]>());
        Settings {
            name: name.into(),
            address: address.trim_end_matches('/').into(),
            auth_url: format!("http://127.0.0.1:{}", ports[0]),
            cloudflare_node: false,
            cloudflare_node_key: data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>()),
            ports,
            auth_secret: secret(),
            resources: vec![],
            bucket: None,
            android: vec![],
            providers: providers(),
            cloudflare_token: None,
            devices: vec![],
            push: [("VAPID_KEY".to_string(), vapid_key())].into(),
            root: None,
            throwaway_user: None,
            home: None,
            dir: crate::home(),
        }
    }

    /// A throwaway network's Workers (`one-dev throwaway`): on free ports here, its address its gateway's at 127.0.0.1, which `one-dev throwaway`
    /// lets each device it borrows reach at the same address; its bucket where its node serves its data (`bucket`), and
    /// `user` signed in with nothing to show.
    pub fn for_throwaway(home: &Path, bucket: &str, user: (String, String)) -> Result<Settings> {
        let free = || -> Result<u16> { Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port()) };
        let ports = [free()?, free()?];
        let mut settings = Settings::new("Throwaway", &format!("http://127.0.0.1:{}", ports[1]));
        settings.auth_url = format!("http://127.0.0.1:{}", ports[0]);
        settings.ports = ports;
        settings.throwaway_user = Some(user);
        settings.dir = home.to_path_buf();
        // Its node serves its data there (throwaway::bucket), signatures unchecked: the URL signer holds these.
        settings.bucket = Some(json!({ "endpoint": format!("http://{bucket}"), "bucket": "throwaway", "region": "auto", "key_id": "throwaway", "secret": "throwaway" }));
        Ok(settings)
    }

    /// Writes the settings, and, the first time, the secrets' values to `.env.local`; after that a secret changes
    /// only by `set_secret`, so a reference to 1Password there stays one.
    pub fn save(&self) -> Result<()> {
        let path = Settings::path_in(&self.dir);
        if !local(&self.dir).exists() {
            let mut text = String::new();
            text += &format!("AUTH_SECRET={}\n", quoted(&self.auth_secret));
            if let Some(bucket) = &self.bucket {
                text += &format!("BUCKET={}\n", quoted(&bucket.to_string()));
            }
            if !self.cloudflare_node_key.is_empty() {
                text += &format!("CLOUDFLARE_NODE_KEY={}\n", quoted(&self.cloudflare_node_key));
            }
            for (name, value) in &self.push {
                text += &format!("{name}={}\n", quoted(value));
            }
            write_private(&local(&self.dir), &text)?;
        }
        std::fs::write(path.with_file_name(".env.schema"), SCHEMA)?;
        write_private(&path, &String::from_utf8(serde_json::to_vec_pretty(self)?)?)
    }

    /// Sets a secret's value in `.env.local`, unless it's a reference to 1Password, where it changes instead. A new
    /// one, where the others are references, is kept in 1Password too, in their vault, and referenced as they are.
    pub fn set_secret(&self, name: &str, value: &str) -> Result<()> {
        let text = std::fs::read_to_string(local(&self.dir)).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with(&format!("{name}="))).map(str::to_string).collect();
        if let Some(line) = text.lines().find(|l| l.starts_with(&format!("{name}="))) {
            ensure!(!line.contains("op("), "{name} is kept in 1Password: change it there ({})", line.trim_start_matches(&format!("{name}=")));
        }
        let vault = text.split("op(op://").nth(1).and_then(|rest| rest.split('/').next()).filter(|v| !v.is_empty());
        let kept = match vault {
            Some(vault) => format!("op(op://{vault}/{}/password)", in_1password(vault, &format!("{} Cloudflare {name}", self.name), value)?),
            None => quoted(value),
        };
        lines.push(format!("{name}={kept}"));
        write_private(&local(&self.dir), &(lines.join("\n") + "\n"))
    }

    /// The Cloudflare node's endpoint (its public key, hex), once it has a key.
    pub fn cloudflare_node_endpoint(&self) -> Option<String> {
        let seed: [u8; 32] = data_encoding::HEXLOWER.decode(self.cloudflare_node_key.as_bytes()).ok()?.try_into().ok()?;
        Some(data_encoding::HEXLOWER.encode(ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().as_bytes()))
    }
}

/// The location Workers' objects are made in, where the config sets one.
fn location() -> Option<Binding> {
    (!crate::config::RELAY_LOCATION.is_empty()).then(|| ("LOCATION".into(), crate::config::RELAY_LOCATION.into(), false))
}

fn plain(name: &str, value: impl Into<String>) -> Binding {
    (name.into(), value.into(), false)
}

fn secret(name: &str, value: impl Into<String>) -> Binding {
    (name.into(), value.into(), true)
}

/// The gateway holds no secrets.
fn gateway_bindings(_: &Settings) -> Vec<Binding> {
    let named = [("NAME", crate::config::NAME), ("YOURS", crate::config::YOURS), ("CLI", crate::config::CLI)].map(|(n, v)| plain(n, v));
    let bound = [("AUTH", "auth"), ("IROH", "relay"), ("SIGNER", "signer"), ("MCP", "mcp"), ("PUSH", "push")].map(|(n, s)| plain(n, format!("service:{s}")));
    bound.into_iter().chain(named).collect()
}

/// The push Worker: the network's push credentials, and the relay, which says which devices are the network's.
fn push_bindings(settings: &Settings) -> Vec<Binding> {
    let cli = crate::config::CLI;
    let mut out = vec![
        plain("RELAY", "service:relay"),
        plain("ADDRESS", settings.address.clone()),
        plain("NAME", settings.name.clone()),
        // The Apple apps' bundle ids (client/native/iphone/project.yml, mac/build.sh), which their pushes go to.
        plain("APNS_TOPICS", json!([format!("app.{cli}.phone"), format!("app.{cli}.mac")]).to_string()),
    ];
    out.extend(settings.push.iter().map(|(name, value)| secret(name, value.clone())));
    out
}

/// The relay holds no secrets: only the keys of the network's first devices, which it starts its list with, and the
/// Cloudflare node's, whose datagrams it hands its Durable Object, and the remote MCP server's devices, each bound to its
/// object while it runs.
fn relay_bindings(settings: &Settings) -> Vec<Binding> {
    let devices = (!settings.devices.is_empty()).then(|| plain("DEVICES", json!(settings.devices).to_string()));
    let node = settings.cloudflare_node_endpoint().filter(|_| settings.cloudflare_node).map(|key| [plain("BOUND", key), plain("NODE", format!("object:{}:{}", NODE.name, NODE.class.unwrap()))]);
    let mcp = plain("MCP", format!("object:mcp:{}", Worker::named("mcp").class.unwrap()));
    devices.into_iter().chain(node.into_iter().flatten()).chain([mcp, plain("YOURS", crate::config::YOURS)]).chain(location()).collect()
}

/// The remote MCP server: remote MCP clients' devices, whose secrets open only with their clients' tokens. It reaches the
/// relay only by its entrypoint for them, and reads the bucket through the URL signer as any device does.
fn mcp_bindings(settings: &Settings) -> Vec<Binding> {
    let mut out = vec![
        plain("AUTH", "service:auth"),
        plain("RELAY", "service:relay:McpDevices"),
        plain("SIGNER", "service:signer"),
        plain("ADDRESS", settings.address.clone()),
        plain("NAME", settings.name.clone()),
        plain("YOURS", crate::config::YOURS),
    ];
    out.extend(location());
    out
}

/// The Cloudflare node: its endpoint's key, and the relay it's reached through.
fn node_bindings(settings: &Settings) -> Vec<Binding> {
    let mut out = vec![
        plain("AUTH", "service:auth"),
        plain("SIGNER", "service:signer"),
        plain("LOADER", "loader:"),
        // Workers AI, for the services built for Cloudflare that use it.
        plain("AI", "ai:"),
        plain("ADDRESS", settings.address.clone()),
        plain("NAME", settings.name.clone()),
        secret("NODE_KEY", settings.cloudflare_node_key.clone()),
        plain("RELAY", "service:relay:CloudflareNode"),
    ];
    // The network's root, as auth's: its members are replayed from no other.
    out.extend(settings.root.clone().map(|root| plain("ROOT", root)));
    out.extend(location());
    out
}

/// The URL signer holds only the bucket's credentials, and asks the relay who's allowed.
fn signer_bindings(settings: &Settings) -> Vec<Binding> {
    let mut out = vec![plain("RELAY", "service:relay"), plain("YOURS", crate::config::YOURS)];
    if let Some(bucket) = &settings.bucket {
        out.push(secret("BUCKET", bucket.to_string()));
        let endpoint = bucket["endpoint"].as_str().unwrap_or("");
        if endpoint.split("://").nth(1).unwrap_or(endpoint).trim_end_matches('/').ends_with(".r2.cloudflarestorage.com") {
            out.push(plain("R2", format!("r2:{}", bucket["bucket"].as_str().unwrap_or(""))));
        }
    }
    out
}

fn auth_bindings(settings: &Settings) -> Vec<Binding> {
    let mut out = vec![
        // Whether a request's signer is one of the network's endpoints, and an admin's.
        plain("RELAY", "service:relay"),
        // Pushes to a user's devices, by the push Worker's entrypoint for the network's own Workers.
        plain("PUSH", "service:push:Sender"),
        plain("ADDRESS", settings.address.clone()),
        plain("NAME", settings.name.clone()),
        plain("YOURS", crate::config::YOURS),
        // Its own apps' sign-ins come back to `<cli>://signed-in/`.
        plain("CLI", crate::config::CLI),
    ];
    if let Some((id, name)) = &settings.throwaway_user {
        out.extend([plain("SIGN_IN", "throwaway"), plain("THROWAWAY_USER", id.clone()), plain("THROWAWAY_NAME", name.clone())]);
    } else {
        if !crate::config::JOIN_DOMAIN.is_empty() {
            out.push(plain("JOIN_DOMAIN", crate::config::JOIN_DOMAIN));
        }
        let registered: Vec<Value> = settings.providers.iter().filter_map(Provider::binding).collect();
        if !registered.is_empty() {
            out.push(secret("PROVIDERS", json!(registered).to_string()));
        }
    }
    out.extend(settings.root.clone().map(|root| plain("ROOT", root)));
    // Tokens are for the network's address unless a client asks for a node's own.
    let resources: Vec<&String> = std::iter::once(&settings.address).chain(&settings.resources).collect();
    out.push(plain("RESOURCES", json!(resources).to_string()));
    out.push(secret("AUTH_SECRET", settings.auth_secret.clone()));
    out
}

/// The Worker, if any, that a binding's value reaches: `service:<name>` or `object:<name>:<class>`.
fn reaches(value: &str) -> Option<&str> {
    value.strip_prefix("service:").or_else(|| value.strip_prefix("object:")).map(|rest| rest.split(':').next().unwrap())
}

/// The network's Workers as one workerd runs them, the gateway serving the web client's files from `web`; and the
/// Cloudflare node, if the network has one and this build carries it.
pub fn served(settings: &Settings, web: &Path) -> Result<Vec<Served>> {
    served_with(settings, web, crate::cloudflare::node::modules().ok())
}

/// `served`, with the Cloudflare node's build `cloudflare_node` (this build's, or a test's).
pub fn served_with(settings: &Settings, web: &Path, cloudflare_node: Option<Modules>) -> Result<Vec<Served>> {
    let node = cloudflare_node.clone().filter(|_| settings.cloudflare_node).map(|modules| crate::cloudflare::node::served(settings, modules));
    // A build without the Cloudflare node's WebAssembly has no remote MCP server, and nothing binds one.
    let mut served = Vec::new();
    for worker in WORKERS {
        let Some(modules) = worker.modules(&crate::ui_folder::cloudflare(), cloudflare_node.as_ref()) else { continue };
        served.push(Served {
            name: worker.name.to_string(),
            module: module(worker.entry)?,
            modules,
            class: worker.class.map(str::to_string),
            listen: worker.place(settings).map(|(_, port)| format!("127.0.0.1:{port}")),
            bindings: worker.bindings(settings).into_iter().map(|(n, v, _)| (n, v)).collect(),
            assets: worker.serves_web.then(|| web.to_path_buf()),
            // A throwaway network's URL signer, and the remote MCP server's devices reading by its URLs, reach its bucket on this machine.
            outbound: (worker.reads_bucket && settings.throwaway_user.is_some()).then(|| NEARBY.to_string()),
        });
    }
    served.extend(node);
    let names: Vec<String> = served.iter().map(|s| s.name.clone()).collect();
    for s in &mut served {
        s.bindings.retain(|(_, v)| reaches(v).is_none_or(|other| names.iter().any(|n| n == other)));
    }
    Ok(served)
}

/// The Android app's package, as client/native/android/app/build.gradle.kts names it.
pub fn android_package() -> String {
    format!("app.{}.android", crate::config::CLI)
}

impl Settings {
    /// The files the address serves itself: the web client's, as this release has them, Android's App Links file
    /// (`.well-known/assetlinks.json`) and Windows' web links file (`.well-known/windows-app-web-link`), which let the
    /// apps open links to the address, and the guide to connecting an AI (`llms.txt`).
    pub fn web(&self) -> Vec<(String, Vec<u8>)> {
        self.web_with(crate::web::with_client())
    }

    /// `web`, with the web client's files as `files` has them.
    pub fn web_with(&self, mut files: Files) -> Files {
        if !self.android.is_empty() {
            let links = json!([{
                "relation": ["delegate_permission/common.handle_all_urls"],
                "target": { "namespace": "android_app", "package_name": android_package(), "sha256_cert_fingerprints": self.android },
            }]);
            files.push((".well-known/assetlinks.json".into(), links.to_string().into_bytes()));
        }
        files.push((".well-known/windows-app-web-link".into(), windows_links()));
        // How an AI connects itself, for anyone who has the address: never what apps the network has.
        files.push(("llms.txt".into(), crate::skills::guide(Some(&self.address)).into_bytes()));
        files
    }
}

/// The Windows app's package family name, as Windows names the package client/native/windows/build.sh makes: the config's name in
/// letters and digits, then its publisher's (`CN=` and the name in letters, digits and spaces) hash, the first 8 bytes
/// of the SHA-256 of it in UTF-16LE, in Crockford's base32.
pub fn windows_package() -> String {
    let name = crate::config::NAME;
    let publisher = format!("CN={}", name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect::<String>());
    let hash = sha2::Sha256::digest(publisher.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
    let bits = u128::from(u64::from_be_bytes(hash[..8].try_into().unwrap())) << 1;
    let id: String = (0..13).map(|i| b"0123456789abcdefghjkmnpqrstvwxyz"[(bits >> ((12 - i) * 5)) as usize & 31] as char).collect();
    format!("{}_{id}", name.chars().filter(char::is_ascii_alphanumeric).collect::<String>())
}

/// `/.well-known/windows-app-web-link`: the pages at the address the Windows app opens, when a link to one is opened
/// from another app, as the web client has them (cloudflare/web/routes.js): You, a page, its Accounts, an app's screen,
/// Settings, a Vault request and an invite.
fn windows_links() -> Vec<u8> {
    let paths = ["/", "/page/*", "/accounts", "/accounts/*", "/app/*", "/settings", "/settings/*", "/vault", "/invite"];
    json!([{ "packageFamilyName": windows_package(), "paths": paths }]).to_string().into_bytes()
}

/// Runs the network's Workers on this machine, in the pinned workerd, until it stops: their modules built from this
/// release, the web client's files in `<home>/cloudflare/web`, and their storage in `<home>/cloudflare/data`.
pub fn serve(settings: &Settings) -> Result<()> {
    let dir = Settings::path_in(&settings.dir).parent().unwrap().to_path_buf();
    let web = web_dir(settings);
    let served = served(settings, &web)?;
    let here = |a: &str| a.strip_prefix("127.0.0.1:").or_else(|| a.strip_prefix("http://127.0.0.1:")).is_some_and(|port| port.parse::<u16>().is_ok());
    ensure!(settings.throwaway_user.is_none() || here(&settings.address) && served.iter().all(|s| s.listen.as_deref().is_none_or(here)), "a throwaway network's auth lets in anyone who reaches it, so its Workers run only beside its throwaway network, at 127.0.0.1");
    let _ = std::fs::remove_dir_all(&web);
    write_web(settings)?;
    let config = write_config(&dir, &served)?;
    let mut child = start(&config, Vec::new())?;
    let status = child.wait()?;
    bail!("the Workers' workerd stopped: {status}")
}

/// Where the network's Workers read the web client's files from on this machine.
fn web_dir(settings: &Settings) -> PathBuf {
    Settings::path_in(&settings.dir).parent().unwrap().join("web")
}

/// Writes the web client's files as this program has them (`Settings::web`) where the Workers read them (`serve`'s
/// workerd reads each file as it's asked for, so a file written while they run is what the next request gets): each one
/// whole or not at all, so a request never gets half of one.
pub fn write_web(settings: &Settings) -> Result<()> {
    let web = web_dir(settings);
    let mut files = settings.web();
    parses(&files)?;
    files.extend(precompressed(&files)?);
    for (file, bytes) in files {
        let path = web.join(&file);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let part = path.with_extension(format!("{}.part", path.extension().map(|e| e.to_string_lossy()).unwrap_or_default()));
        std::fs::write(&part, bytes)?;
        std::fs::rename(part, path)?;
    }
    Ok(())
}

/// What makes each file in `in/` brotli at its best in `out/`: quality 11 with the largest window.
const BROTLI: &str = r#"import { brotliCompressSync, constants as c } from "node:zlib";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
for (const name of readdirSync("in")) {
  const data = readFileSync(`in/${name}`);
  writeFileSync(`out/${name}`, brotliCompressSync(data, { params: { [c.BROTLI_PARAM_QUALITY]: 11, [c.BROTLI_PARAM_LGWIN]: 24, [c.BROTLI_PARAM_SIZE_HINT]: data.length } }));
}
"#;

/// Brotli at its best of each large file the address serves that a browser reads as text or code (`<file>.br`, which the
/// gateway answers with `Content-Encoding: br` to a browser that takes it, so the client's WebAssembly is a fifth
/// smaller than the edge's own quicker brotli makes it, the first thing a first visit waits on). The pinned Bun makes
/// them, which is why it isn't a dependency of ours; one copy of each, by what the file is, is kept in the cache.
pub fn precompressed(web: &[(String, Vec<u8>)]) -> Result<Vec<(String, Vec<u8>)>> {
    let wanted: Vec<&(String, Vec<u8>)> = web
        .iter()
        .filter(|(path, bytes)| bytes.len() >= 1024 && path != "index.html" && ["js", "css", "json", "html", "wasm", "svg"].contains(&path.rsplit('.').next().unwrap_or("")))
        .collect();
    let kept = crate::cache().join("brotli");
    let names: Vec<PathBuf> = wanted.iter().map(|(_, bytes)| kept.join(data_encoding::HEXLOWER.encode(&Sha256::digest(bytes)))).collect();
    let missing: Vec<usize> = (0..wanted.len()).filter(|&i| !names[i].is_file()).collect();
    if !missing.is_empty() {
        let dir = std::env::temp_dir().join(format!("{}-brotli-{}", crate::config::CLI, rand::random::<u64>()));
        let made = (|| {
            std::fs::create_dir_all(&kept)?;
            std::fs::create_dir_all(dir.join("in"))?;
            std::fs::create_dir_all(dir.join("out"))?;
            for &i in &missing {
                std::fs::write(dir.join("in").join(i.to_string()), &wanted[i].1)?;
            }
            std::fs::write(dir.join("brotli.mjs"), BROTLI)?;
            let out = std::process::Command::new(crate::build::bun()?).args(["brotli.mjs"]).current_dir(&dir).env("DO_NOT_TRACK", "1").output()?;
            ensure!(out.status.success(), "couldn't compress the files the address serves:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            for &i in &missing {
                let part = names[i].with_extension(format!("{}.part", rand::random::<u64>()));
                std::fs::copy(dir.join("out").join(i.to_string()), &part)?;
                std::fs::rename(part, &names[i])?;
            }
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&dir);
        made?;
    }
    wanted.iter().zip(&names).map(|((path, _), name)| Ok((format!("{path}.br"), std::fs::read(name)?))).collect()
}

/// Fails, naming what's wrong, unless every script among `web` parses: its `.js` files and the HTML's inline scripts,
/// each read by the pinned Bun as it reads the services' sources. A browser given one that doesn't runs none of it.
pub fn parses(web: &[(String, Vec<u8>)]) -> Result<()> {
    let mut scripts: Vec<(String, String)> = Vec::new();
    for (path, bytes) in web {
        let text = String::from_utf8_lossy(bytes);
        if path.ends_with(".js") {
            scripts.push((path.clone(), text.into_owned()));
        } else if path.ends_with(".html") {
            let mut rest = text.as_ref();
            while let Some(start) = rest.find("<script") {
                let Some(open) = rest[start..].find('>') else { break };
                let tag = &rest[start..start + open];
                let body = &rest[start + open + 1..];
                let end = body.find("</script>").unwrap_or(body.len());
                // An import map is JSON, not a script.
                if !tag.contains("src=") && !tag.contains("importmap") {
                    scripts.push((format!("{path}.{}.js", scripts.len()), body[..end].to_string()));
                }
                rest = &body[end..];
            }
        }
    }
    if scripts.is_empty() {
        return Ok(());
    }
    let dir = std::env::temp_dir().join(format!("{}-scripts-{}", crate::config::CLI, rand::random::<u64>()));
    let checked = (|| {
        for (path, text) in &scripts {
            let file = dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap())?;
            std::fs::write(file, text)?;
        }
        let out = std::process::Command::new(crate::build::bun()?)
            .args(["build", "--no-bundle", "--outdir=out"])
            .args(scripts.iter().map(|(path, _)| path))
            .current_dir(&dir)
            .env("DO_NOT_TRACK", "1")
            .output()?;
        ensure!(out.status.success(), "a script the address serves doesn't parse, so it isn't deployed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    checked
}

/// A Worker's module, built from the sources this release was signed with by the pinned Bun, once per
/// version of them.
pub fn module(entry: &str) -> Result<Vec<u8>> {
    module_from(crate::ui_folder::cloudflare(), entry)
}

/// The module of what the Workers and the web client share with the Rust (`protocol::javascript`), in `cloudflare/`.
pub const PROTOCOL: &str = "shared/protocol.js";

/// A Worker's module, built from `files` (paths in `cloudflare/`).
pub fn module_from(mut files: Vec<(String, Vec<u8>)>, entry: &str) -> Result<Vec<u8>> {
    // What they say that Rust says too, as this build says it (`protocol`): never a file of the repo.
    files.retain(|(path, _)| path != PROTOCOL);
    files.push((PROTOCOL.into(), crate::protocol::javascript().into_bytes()));
    let mut hash = Sha256::new();
    for (path, bytes) in &files {
        hash.update(format!("{path}\0{}\0", bytes.len()));
        hash.update(bytes);
    }
    let hash = data_encoding::HEXLOWER.encode(&hash.finalize());
    let kept = crate::cache().join("cloudflare").join(format!("{}-{}.js", &hash[..32], entry.replace('/', "-")));
    if let Ok(module) = std::fs::read(&kept) {
        return Ok(module);
    }
    let dir = std::env::temp_dir().join(format!("{}-cloudflare-{}", crate::config::CLI, rand::random::<u64>()));
    let built = (|| {
        for (path, bytes) in &files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, bytes)?;
        }
        build(&dir, entry)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    let module = built?;
    std::fs::create_dir_all(kept.parent().unwrap())?;
    let part = kept.with_extension(format!("{}.part", rand::random::<u64>()));
    std::fs::write(&part, &module)?;
    std::fs::rename(&part, &kept)?;
    Ok(module)
}

/// Builds `entry` in the Workers' folder `dir` into one ES module.
pub fn build(dir: &Path, entry: &str) -> Result<Vec<u8>> {
    let bun = crate::build::bun()?;
    let cache = bun.parent().unwrap().join("cache");
    let run = |args: &[&str]| -> Result<()> {
        let out = std::process::Command::new(&bun).args(args).current_dir(dir).env("BUN_INSTALL_CACHE_DIR", &cache).env("DO_NOT_TRACK", "1").output()?;
        ensure!(out.status.success(), "bun {} failed:\n{}{}", args[0], String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Ok(())
    };
    if dir.join("package.json").is_file() {
        run(&["install", "--frozen-lockfile"])?;
    }
    let out = dir.join("out.js");
    run(&["build", entry, "--target=browser", "--format=esm", "--external=cloudflare:*", "--external=node:*", "--external=*.wasm", "--external=*/one_cloudflare_node.js", &format!("--outfile={}", out.display())])?;
    Ok(std::fs::read(out)?)
}

/// A Worker as one workerd runs it: its name, module, Durable Object class, the address it listens on, and its
/// bindings: (name, a text, or `service:<name>` for another service).
pub struct Served {
    pub name: String,
    pub module: Vec<u8>,
    pub modules: Vec<(String, Vec<u8>)>,
    pub class: Option<String>,
    /// Where it listens, if anywhere but its binding.
    pub listen: Option<String>,
    pub bindings: Vec<(String, String)>,
    /// A folder it reads as `ASSETS`, as a Worker reads its static assets on Cloudflare.
    pub assets: Option<PathBuf>,
    /// The service its own requests go to, if not the internet: `NEARBY`, or a test's stand-in sites.
    pub outbound: Option<String>,
}

/// The internet, this machine and its private networks, as an outbound.
pub const NEARBY: &str = "nearby";

/// Writes the workerd config that runs `served` into `dir` (their modules beside it, their storage in `dir/data`),
/// readable only by its owner, since bindings hold the Workers' secrets.
pub fn write_config(dir: &Path, served: &[Served]) -> Result<PathBuf> {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"));
    let mut services = Vec::new();
    if served.iter().any(|s| s.outbound.as_deref() == Some(NEARBY)) {
        services.push(format!("(name = {}, network = (allow = [\"public\", \"private\", \"local\"]))", quote(NEARBY)));
    }
    let mut sockets = Vec::new();
    let mut workers = Vec::new();
    for (i, s) in served.iter().enumerate() {
        ensure!(s.name.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "a Worker's name is lowercase");
        std::fs::write(dir.join(format!("{}.js", s.name)), &s.module)?;
        let mut modules = vec![format!("(name = \"worker.js\", esModule = embed {})", quote(&format!("{}.js", s.name)))];
        for (name, bytes) in &s.modules {
            let file = format!("{}-{name}", s.name);
            std::fs::write(dir.join(&file), bytes)?;
            let kind = match name.rsplit('.').next() {
                Some("wasm") => "wasm",
                Some("js") => "esModule",
                _ => "text",
            };
            modules.push(format!("(name = {}, {kind} = embed {})", quote(name), quote(&file)));
        }
        let data = dir.join("data").join(&s.name);
        std::fs::create_dir_all(&data)?;
        services.push(format!("(name = {}, worker = .w{i})", quote(&s.name)));
        services.push(format!("(name = {}, disk = (path = {}, writable = true))", quote(&format!("{}-data", s.name)), quote(&data.to_string_lossy())));
        if let Some(listen) = &s.listen {
            sockets.push(format!("(name = {}, address = {}, http = (), service = {})", quote(&s.name), quote(listen), quote(&s.name)));
        }
        let mut bindings: Vec<String> = s.class.iter().map(|c| format!("(name = {}, durableObjectNamespace = {})", quote(&c.to_uppercase()), quote(c))).collect();
        if let Some(assets) = &s.assets {
            // With `.well-known/`, which Windows and Android ask the address for.
            services.push(format!("(name = {}, disk = (path = {}, writable = false, allowDotfiles = true))", quote(&format!("{}-assets", s.name)), quote(&assets.to_string_lossy())));
            bindings.push(format!("(name = \"ASSETS\", service = {})", quote(&format!("{}-assets", s.name))));
        }
        for (name, value) in &s.bindings {
            // An R2 bucket is bound only on Cloudflare, where the URL signer here reaches it by URL; Workers AI is only there.
            if value.starts_with("r2:") || value == "ai:" {
                continue;
            }
            let object = value.strip_prefix("object:").and_then(|o| o.split_once(':'));
            bindings.push(match (value.strip_prefix("service:"), object) {
                (Some(service), _) => match service.split_once(':') {
                    Some((service, entrypoint)) => format!("(name = {}, service = (name = {}, entrypoint = {}))", quote(name), quote(service), quote(entrypoint)),
                    None => format!("(name = {}, service = {})", quote(name), quote(service)),
                },
                (_, Some((service, class))) => format!("(name = {}, durableObjectNamespace = (className = {}, serviceName = {}))", quote(name), quote(class), quote(service)),
                _ if value == "loader:" => format!("(name = {}, workerLoader = ())", quote(name)),
                _ => format!("(name = {}, text = {})", quote(name), quote(value)),
            });
        }
        let objects = match &s.class {
            Some(class) => format!(
                "  durableObjectNamespaces = [(className = {}, uniqueKey = {}, enableSql = true)],\n  durableObjectStorage = (localDisk = {}),\n",
                quote(class),
                quote(&s.name),
                quote(&format!("{}-data", s.name))
            ),
            None => String::new(),
        };
        workers.push(format!(
            "const w{i} :Workerd.Worker = (\n  modules = [{}],\n  compatibilityDate = \"{}\",\n  bindings = [{}],\n{objects}{});",
            modules.join(", "),
            crate::app::runtime::COMPATIBILITY_DATE,
            bindings.join(", "),
            s.outbound.as_ref().map(|o| format!("  globalOutbound = {},\n", quote(o))).unwrap_or_default(),
        ));
    }
    let config = format!(
        "using Workerd = import \"/workerd/workerd.capnp\";\n\nconst config :Workerd.Config = (\n  services = [\n    {}\n  ],\n  sockets = [\n    {}\n  ],\n);\n\n{}\n",
        services.join(",\n    "),
        sockets.join(",\n    "),
        workers.join("\n\n"),
    );
    let path = dir.join("config.capnp");
    std::fs::write(&path, config)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

/// Starts workerd on a config: → the running process. Each of `listening` is a socket already listening for the
/// service of its name, there in place of the config's address, so no one else can take the port before workerd does,
/// and what reaches it waits for workerd rather than being refused. Only the process holds them once it's started.
pub fn start(config: &Path, listening: Vec<(String, std::net::TcpListener)>) -> Result<std::process::Child> {
    let mut command = std::process::Command::new(crate::app::runtime::workerd()?);
    command.arg("serve").arg(config).arg("--experimental").stdin(std::process::Stdio::null());
    #[cfg(not(unix))]
    ensure!(listening.is_empty(), "workerd takes listening sockets only on Unix");
    crate::app::runtime::child(&mut command, &listening.iter().map(|(n, l)| (n.as_str(), l)).collect::<Vec<_>>());
    Ok(command.spawn()?)
}

/// Cloudflare's API, with an API token that may edit Workers.
pub struct Cloudflare {
    pub api: String,
    pub token: String,
    pub account: String,
}

impl Cloudflare {
    fn call(&self, method: &str, path: &str, body: Option<(String, Vec<u8>)>) -> Result<Value> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(120))).build().into();
        let url = format!("{}{path}", self.api);
        let auth = format!("Bearer {}", self.token);
        let mut resp = match (method, body) {
            ("GET", _) | (_, None) => agent.get(&url).header("authorization", &auth).call()?,
            ("PUT", Some((kind, bytes))) => agent.put(&url).header("authorization", &auth).header("content-type", &kind).send(&bytes[..])?,
            (_, Some((kind, bytes))) => agent.post(&url).header("authorization", &auth).header("content-type", &kind).send(&bytes[..])?,
        };
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        if out["success"] != true {
            let errors: Vec<String> = out["errors"].as_array().into_iter().flatten().map(|e| e["message"].as_str().unwrap_or("").to_string()).collect();
            bail!("Cloudflare said {} to {method} {path}: {}", resp.status(), errors.join("; "));
        }
        Ok(out["result"].clone())
    }

    fn send(&self, method: &str, path: &str, body: Value) -> Result<Value> {
        self.call(method, path, Some(("application/json".into(), body.to_string().into_bytes())))
    }

    /// The token's account, when it has just one.
    pub fn accounts(&self) -> Result<String> {
        let list = self.call("GET", "/accounts", None)?;
        let ids: Vec<&str> = list.as_array().into_iter().flatten().filter_map(|a| a["id"].as_str()).collect();
        match ids.as_slice() {
            [one] => Ok(one.to_string()),
            [] => bail!("that token reaches no Cloudflare account"),
            _ => bail!("that token reaches several accounts: say which with --account <id>"),
        }
    }

    /// The account's workers.dev subdomain: services are at `https://<name>.<subdomain>.workers.dev`.
    pub fn subdomain(&self) -> Result<String> {
        Ok(self.call("GET", &format!("/accounts/{}/workers/subdomain", self.account), None)?["subdomain"].as_str().context("the account has no workers.dev subdomain yet: open Workers in Cloudflare's dashboard once")?.to_string())
    }

    /// The account's Workers, by name.
    pub fn scripts(&self) -> Result<std::collections::BTreeMap<String, Script>> {
        let list = self.call("GET", &format!("/accounts/{}/workers/scripts", self.account), None)?;
        let script = |s: &Value| Script { migration: s["migration_tag"].as_str().map(str::to_string) };
        Ok(list.as_array().into_iter().flatten().filter_map(|s| Some((s["id"].as_str()?.to_string(), script(s)))).collect())
    }

    /// Whether the Worker `name` runs what `what` names (`upload_hash`), as Cloudflare says of the one version it serves:
    /// a Worker that would get the same isn't uploaded again, which would restart its Durable Objects, and whatever
    /// else put a version there (an edit, a rollback) counts as what it is.
    pub(crate) fn runs(&self, name: &str, what: &str) -> Result<bool> {
        let list = self.call("GET", &format!("/accounts/{}/workers/scripts/{name}/deployments", self.account), None)?;
        let versions = list["deployments"][0]["versions"].as_array().cloned().unwrap_or_default();
        let [only] = versions.as_slice() else { return Ok(false) };
        let version = self.call("GET", &format!("/accounts/{}/workers/scripts/{name}/versions/{}", self.account, only["version_id"].as_str().context("Cloudflare named no version")?), None)?;
        Ok(version["annotations"]["workers/message"] == what)
    }

    /// Uploads a Worker: its modules (name, type, bytes), the first its main one, and its bindings and Durable Object
    /// class, with what names them (`what`) kept on the version, and checks Cloudflare then serves it.
    pub(crate) fn upload(&self, name: &str, modules: &[(&str, &str, &[u8])], metadata: &Value, what: &str) -> Result<()> {
        let mut metadata = metadata.clone();
        metadata["annotations"] = json!({ "workers/message": what });
        let boundary = format!("cloudflare-{}", data_encoding::HEXLOWER.encode(&rand::random::<[u8; 12]>()));
        let mut body = Vec::new();
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{metadata}\r\n").as_bytes());
        for (module, kind, bytes) in modules {
            body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{module}\"; filename=\"{module}\"\r\nContent-Type: {kind}\r\n\r\n").as_bytes());
            body.extend(*bytes);
            body.extend(b"\r\n");
        }
        body.extend(format!("--{boundary}--\r\n").as_bytes());
        self.call("PUT", &format!("/accounts/{}/workers/scripts/{name}", self.account), Some((format!("multipart/form-data; boundary={boundary}"), body)))?;
        ensure!(self.runs(name, what)?, "Cloudflare took the upload of {name}, but serves another version");
        Ok(())
    }

    /// Uploads `files` (path, bytes) as a Worker's static assets, those Cloudflare hasn't already: → the token its
    /// upload takes them with.
    fn assets(&self, name: &str, files: &[(String, Vec<u8>)]) -> Result<String> {
        let hash = |path: &str, bytes: &[u8]| data_encoding::HEXLOWER.encode(&Sha256::new().chain_update(bytes).chain_update(path.as_bytes()).finalize())[..32].to_string();
        let manifest: serde_json::Map<String, Value> = files.iter().map(|(path, bytes)| (format!("/{path}"), json!({ "hash": hash(path, bytes), "size": bytes.len() }))).collect();
        let session = self.send("POST", &format!("/accounts/{}/workers/scripts/{name}/assets-upload-session", self.account), json!({ "manifest": manifest }))?;
        let mut token = session["jwt"].as_str().context("Cloudflare gave no upload token")?.to_string();
        let upload = Cloudflare { api: self.api.clone(), token: token.clone(), account: self.account.clone() };
        for bucket in session["buckets"].as_array().into_iter().flatten() {
            let boundary = format!("assets-{}", data_encoding::HEXLOWER.encode(&rand::random::<[u8; 12]>()));
            let mut body = Vec::new();
            for wanted in bucket.as_array().into_iter().flatten().filter_map(Value::as_str) {
                let (path, bytes) = files.iter().find(|(path, bytes)| hash(path, bytes) == wanted).context("Cloudflare asked for a file it wasn't offered")?;
                let kind = mime(path);
                body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{wanted}\"; filename=\"{wanted}\"\r\nContent-Type: {kind}\r\n\r\n").as_bytes());
                body.extend(data_encoding::BASE64.encode(bytes).as_bytes());
                body.extend(b"\r\n");
            }
            body.extend(format!("--{boundary}--\r\n").as_bytes());
            let done = upload.call("POST", &format!("/accounts/{}/workers/assets/upload?base64=true", self.account), Some((format!("multipart/form-data; boundary={boundary}"), body)))?;
            if let Some(jwt) = done["jwt"].as_str() {
                token = jwt.to_string();
            }
        }
        Ok(token)
    }

    /// Gives a Worker no address: only other Workers' bindings reach it.
    pub(crate) fn hide(&self, name: &str) -> Result<()> {
        self.send("POST", &format!("/accounts/{}/workers/scripts/{name}/subdomain", self.account), json!({ "enabled": false, "previews_enabled": false }))?;
        Ok(())
    }

    /// Puts a Worker at `url`: its workers.dev address, or a host in one of the account's domains, with its
    /// workers.dev address off, so each service has one address.
    pub(crate) fn reach(&self, name: &str, url: &str) -> Result<()> {
        let host = url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or("");
        let subdomain = format!("/accounts/{}/workers/scripts/{name}/subdomain", self.account);
        if host.ends_with(".workers.dev") {
            self.send("POST", &subdomain, json!({ "enabled": true, "previews_enabled": false }))?;
            return Ok(());
        }
        let labels: Vec<&str> = host.split('.').collect();
        let zone = (1..labels.len().saturating_sub(1))
            .map(|i| labels[i..].join("."))
            .find_map(|domain| self.call("GET", &format!("/zones?name={domain}"), None).ok()?.as_array()?.first()?["id"].as_str().map(str::to_string))
            .with_context(|| format!("{host} is in none of the account's domains"))?;
        self.send("PUT", &format!("/accounts/{}/workers/domains", self.account), json!({ "environment": "production", "hostname": host, "service": name, "zone_id": zone }))?;
        self.send("POST", &subdomain, json!({ "enabled": false, "previews_enabled": false }))?;
        Ok(())
    }
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// A Worker's name on Cloudflare: `<cli>-<name>`, so custom builds sharing an account don't clash.
pub fn worker_name(name: &str) -> String {
    format!("{}-{name}", crate::config::CLI)
}

/// What a Worker's upload says besides its module: its bindings (another service's by its Worker's name), its Durable
/// Object class, and that class's migration the first time (`migration`: the tag of the one the Worker has had).
pub fn metadata(settings: &Settings, worker: &Worker, migration: Option<&str>, exists: impl Fn(&str) -> bool) -> (Value, bool) {
    let mut complete = true;
    let class = worker.class;
    let mut bindings: Vec<Value> = class.iter().map(|class| json!({ "type": "durable_object_namespace", "name": class.to_uppercase(), "class_name": class })).collect();
    for (name, value, secret) in worker.bindings(settings) {
        let object = value.strip_prefix("object:").and_then(|o| o.split_once(':'));
        // `service:<name>` or `service:<name>:<entrypoint>`, one of its named entrypoints.
        let service = value.strip_prefix("service:").map(|s| s.split_once(':').map_or((s, None), |(s, e)| (s, Some(e))));
        match service {
            Some((other, entrypoint)) if exists(&worker_name(other)) => {
                let mut binding = json!({ "type": "service", "name": name, "service": worker_name(other) });
                if let Some(entrypoint) = entrypoint {
                    binding["entrypoint"] = json!(entrypoint);
                }
                bindings.push(binding)
            }
            Some(_) => complete = false,
            // Another Worker's class, once it's there: the relay and the Cloudflare node each bind the other's, so the
            // first of them deployed goes without, and has it when it's deployed again.
            None if object.is_some() => {
                if let Some((other, class)) = object.filter(|(other, _)| exists(&worker_name(other))) {
                    bindings.push(json!({ "type": "durable_object_namespace", "name": name, "class_name": class, "script_name": worker_name(other) }));
                }
            }
            None if value == "loader:" => bindings.push(json!({ "type": "worker_loader", "name": name })),
            None if value == "ai:" => bindings.push(json!({ "type": "ai", "name": name })),
            None => match value.strip_prefix("r2:") {
                Some(bucket) => bindings.push(json!({ "type": "r2_bucket", "name": name, "bucket_name": bucket })),
                None => bindings.push(json!({ "type": if secret { "secret_text" } else { "plain_text" }, "name": name, "text": value })),
            },
        }
    }
    let mut meta = json!({ "main_module": "worker.js", "compatibility_date": crate::app::runtime::COMPATIBILITY_DATE, "bindings": bindings });
    if let Some(class) = class.filter(|_| migration != Some("v1")) {
        meta["migrations"] = json!({ "new_tag": "v1", "new_sqlite_classes": [class] });
    }
    (meta, complete)
}

/// Deploys the network's Workers to Cloudflare, each with its Durable Object, built from `sources`, the gateway with
/// the web client's files (`web`) as its static assets, which it serves itself. A service whose binding names one not
/// deployed yet goes up without it, and again once that one is.
pub fn deploy(cf: &Cloudflare, settings: &Settings, sources: &Files, web: &[(String, Vec<u8>)]) -> Result<()> {
    deploy_some(cf, settings, sources, web, &WORKERS.iter().collect::<Vec<_>>())
}

pub fn deploy_some(cf: &Cloudflare, settings: &Settings, sources: &Files, web: &[(String, Vec<u8>)], workers: &[&Worker]) -> Result<()> {
    ensure!(settings.throwaway_user.is_none(), "a throwaway network's auth lets in anyone who reaches it, so its Workers run only beside its throwaway network");
    if workers.iter().any(|w| w.serves_web) {
        parses(web)?;
    }
    let mut left = workers.to_vec();
    let mut scripts = cf.scripts()?;
    for _ in 0..2 {
        let mut again = Vec::new();
        for worker in left {
            let name = worker.worker_name();
            let migration = scripts.get(&name).and_then(|s| s.migration.clone());
            let (mut meta, complete) = metadata(settings, worker, migration.as_deref(), |other| scripts.contains_key(other) || other == name);
            let url = worker.place(settings).map(|(url, _)| url);
            let Some(wasm) = worker.modules(sources, crate::cloudflare::node::modules().ok().as_ref()) else { bail!("this build has no Cloudflare node, which the {} Worker runs: {}-dev update node builds one", worker.name, crate::config::CLI) };
            let module = module_from(sources.clone(), worker.entry)?;
            let mut what = vec![meta.to_string().into_bytes(), module.clone(), url.unwrap_or("").as_bytes().to_vec()];
            what.extend(wasm.iter().flat_map(|(name, bytes)| [name.as_bytes().to_vec(), bytes.clone()]));
            if worker.serves_web {
                what.extend(web.iter().flat_map(|(path, bytes)| [path.as_bytes().to_vec(), bytes.clone()]));
            }
            let what = upload_hash(&what);
            // Its address may have been changed by hand; one with none is reached only by binding.
            let reach = || match url {
                Some(url) => cf.reach(&name, url),
                None => cf.hide(&name),
            };
            if complete && scripts.contains_key(&name) && cf.runs(&name, &what)? {
                reach()?;
                continue;
            }
            if worker.serves_web {
                let jwt = cf.assets(&name, web)?;
                meta["assets"] = json!({ "jwt": jwt, "config": { "html_handling": "none", "not_found_handling": "none", "run_worker_first": true } });
                meta["bindings"].as_array_mut().unwrap().push(json!({ "type": "assets", "name": "ASSETS" }));
            }
            let mut parts: Vec<(&str, &str, &[u8])> = vec![("worker.js", "application/javascript+module", &module)];
            parts.extend(wasm.iter().map(|(name, bytes)| (name.as_str(), if name.ends_with(".js") { "application/javascript+module" } else { "application/wasm" }, bytes.as_slice())));
            cf.upload(&name, &parts, &meta, &what)?;
            reach()?;
            // Later services' bindings to it resolve in this pass, and it has had its migration.
            let migration = meta["migrations"]["new_tag"].as_str().map(str::to_string).or(migration);
            scripts.insert(name.clone(), Script { migration });
            if !complete {
                again.push(worker);
            }
        }
        left = again;
        if left.is_empty() {
            return Ok(());
        }
    }
    bail!("some services' bindings never resolved")
}

pub struct Script {
    /// The Durable Object migration it has had.
    pub migration: Option<String>,
}

/// What names everything a Worker would be sent (the secrets in its bindings too), which its version keeps on
/// Cloudflare (`Cloudflare::upload`, `Cloudflare::runs`).
pub(crate) fn upload_hash(parts: &[Vec<u8>]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    data_encoding::HEXLOWER.encode(&hash.finalize())
}
