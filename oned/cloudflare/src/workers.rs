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
/// AI, `r2:<bucket>` an R2 bucket of the account's, and `pipeline:<id>` a stream of Basin's.
pub type Binding = (String, String, bool);

/// One of the network's Workers: everything said about it is here.
pub struct Worker {
    pub name: &'static str,
    /// Its module, among the Workers' files (`ui_files::list`). Its WebAssembly is the `.wasm` files in its module's folder, modules of its own
    /// (Workers can't compile it).
    pub entry: &'static str,
    /// Its Durable Objects' classes, if it keeps anything, in the order they were added: each one's migration is its place.
    pub classes: &'static [&'static str],
    /// Where it's reached and the local port it listens on, if anywhere but by other Workers' bindings.
    place: Option<fn(&Settings) -> (&str, u16)>,
    /// Whether it's also reached at its workers.dev address: by what mustn't come through the gateway.
    own_address: bool,
    /// Whether it runs the Cloudflare node's build, which carries its devices' endpoints.
    node_build: bool,
    /// Whether it serves the web client's files, as its static assets.
    pub serves_web: bool,
    /// Whether it reads the network's bucket itself.
    pub reads_bucket: bool,
    bindings: fn(&Settings) -> Vec<Binding>,
    /// When Cloudflare runs it besides, as a cron expression (its `scheduled`): only there, never on one machine.
    cron: Option<&'static str>,
}

/// The network's Workers, in the order they're deployed.
///
/// Auth's Durable Object `Auth` holds every user's passkeys and accounts. The paths under `/api/auth` name the
/// passkeys, Google's redirect URI, OAuth clients and access tokens' issuer, so they never change.
pub const WORKERS: &[Worker] = &[
    Worker { name: "auth", entry: "auth/worker.js", classes: &["Auth"], place: Some(|s| (s.auth_url.as_str(), s.ports[0])), own_address: false, node_build: false, serves_web: false, reads_bucket: false, bindings: auth_bindings, cron: None },
    Worker { name: "relay", entry: "relay/worker.js", classes: &["Relay", "List"], place: None, own_address: false, node_build: false, serves_web: false, reads_bucket: false, bindings: relay_bindings, cron: None },
    Worker { name: "signer", entry: "signer/worker.js", classes: &[], place: None, own_address: false, node_build: false, serves_web: false, reads_bucket: true, bindings: signer_bindings, cron: None },
    Worker { name: "mcp", entry: "mcp/worker.js", classes: &["Client"], place: None, own_address: false, node_build: true, serves_web: false, reads_bucket: true, bindings: mcp_bindings, cron: None },
    Worker { name: "telemetry", entry: "telemetry/worker.js", classes: &[], place: None, own_address: true, node_build: false, serves_web: false, reads_bucket: false, bindings: telemetry_bindings, cron: Some("10 * * * *") },
    Worker { name: "gateway", entry: "gateway/worker.js", classes: &[], place: Some(|s| (s.address.as_str(), s.ports[1])), own_address: false, node_build: false, serves_web: true, reads_bucket: false, bindings: gateway_bindings, cron: None },
    Worker { name: "push", entry: "push/worker.js", classes: &["Push"], place: None, own_address: false, node_build: false, serves_web: false, reads_bucket: false, bindings: notifications_bindings, cron: None },
];

/// The Cloudflare node's Worker (`oned/cloudflare/node/`), `<cli>-node`: no address, and an iroh endpoint of its own key, which
/// the relay wakes. It's built with WebAssembly, so not one of `WORKERS`' (`node::modules`).
pub const NODE: Worker = Worker { name: "node", entry: "node/worker.js", classes: &["Node"], place: None, own_address: false, node_build: false, serves_web: false, reads_bucket: true, bindings: node_bindings, cron: None };

/// The Workers traced at 100% (`metadata`), their logs exported too: all but the telemetry Worker and the Cloudflare node.
/// The web client is the gateway's assets.
const TRACED: [&str; 6] = ::insights::TRACED;

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
    /// The notifications Worker's secrets, by their names in `SCHEMA` (`NOTIFICATIONS_SECRETS`): the network's VAPID key, which is made
    /// with the Workers, and the custom build's APNs, FCM and WNS credentials, as its admin adds them.
    #[serde(default, skip)]
    pub notifications: std::collections::BTreeMap<String, String>,
    /// A throwaway network's one user, `(id, name)`, whom its auth lets in with no passkey and no Google, so clients
    /// are tested with nobody there. Only `one-dev throwaway` sets it, on the Workers it runs beside its throwaway network, listening
    /// only on this machine; `deploy` refuses settings with it, so no Workers users use ever have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throwaway_user: Option<(String, String)>,
    /// Where its telemetry is in Basin, once a deploy has set that up (`basin::provision`). Workers that run on one machine
    /// have none, and keep it in a stand-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<super::basin::Telemetry>,
    /// The token only for telemetry's bucket that `telemetry` was set up with, which the telemetry Worker reads with.
    #[serde(default, skip_serializing)]
    pub telemetry_token: Option<String>,
    /// Where the Workers' own traces go, once a deploy has pointed Cloudflare's destination there (`basin::exports`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traces: Option<String>,
    /// Where their logs go, the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs: Option<String>,
    /// What Cloudflare sends those with, which the telemetry Worker checks: its export can't sign.
    #[serde(default, skip_serializing)]
    pub otlp_token: Option<String>,
    /// The token only for reading the account's analytics, made by a deploy (`basin::usage`), which the telemetry Worker
    /// reads what the network uses of Cloudflare with.
    #[serde(default, skip_serializing)]
    pub analytics_token: Option<String>,
    /// What of the Cloudflare account is the network's, as a deploy found it (`basin::usage`), for the telemetry Worker to
    /// read the usage of.
    #[serde(skip)]
    pub usage: Option<Value>,
    /// What the relay says of the Workers (`Newest::deployed`), set by whatever deploys or serves them, so Devices can show
    /// what they're on.
    #[serde(skip)]
    pub deployed: Option<String>,
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
# The token only for telemetry's bucket, made by cloudflare deploy, which the telemetry Worker reads Basin with.
# @optional
TELEMETRY_TOKEN=
# What Cloudflare's export of the Workers' traces and logs is let in by, made by cloudflare deploy.
# @optional
OTLP_TOKEN=
# The token only for reading the account's analytics, made by cloudflare deploy, which the telemetry Worker reads the
# network's usage of Cloudflare with.
# @optional
ANALYTICS_TOKEN=
# The Cloudflare node's endpoint key, made by cloudflare node deploy.
# @optional
CLOUDFLARE_NODE_KEY=
# The notifications Worker's: the network's VAPID key (base64url PKCS#8, P-256), made with the Workers, which browsers' notifications
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
# The custom build's Firebase project's service account key (its JSON), which Android's notifications are sent with.
# @optional
FCM_ACCOUNT=
# The Windows app's Microsoft Entra app registration, which Windows' notifications are sent with.
# @optional
WNS_TENANT_ID=
# @optional
WNS_CLIENT_ID=
# @optional
WNS_CLIENT_SECRET=
";

const NOTIFICATIONS_SECRETS: &[&str] = &["VAPID_KEY", "APNS_KEY", "APNS_KEY_ID", "APNS_TEAM_ID", "FCM_ACCOUNT", "WNS_TENANT_ID", "WNS_CLIENT_ID", "WNS_CLIENT_SECRET"];

/// A new VAPID key: base64url PKCS#8 of a P-256 key, as the notifications Worker imports it.
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
        .find(|(target, _, _)| *target == custom_build::config::TARGET)
        .ok_or_else(|| anyhow!("this machine can't run the Workers: Varlock has no build for {}", custom_build::config::TARGET))?;
    let dir = platform::cache().join(format!("varlock-{VARLOCK}"));
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

/// The vault the `op(op://…)` references in `.env.local`'s `text` are in, if it has any.
fn vault_of(text: &str) -> Option<&str> {
    text.split("op(op://").nth(1).and_then(|rest| rest.split('/').next()).filter(|v| !v.is_empty())
}

/// The 1Password item a network's Workers' settings are kept in, beside their secrets, for any admin's device to deploy from.
fn item_title() -> String {
    format!("{} Cloudflare settings", custom_build::config::NAME)
}

/// `op` with `args`, and `input` on its standard input.
fn op(args: &[&str], input: Option<&str>) -> Result<std::process::Output> {
    use std::io::Write;
    let mut child = std::process::Command::new("op")
        .args(args)
        .stdin(if input.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("op (1Password's CLI) isn't on this device: install it, and sign it in with the service account that holds the Workers' secrets")?;
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input.as_bytes())?;
    }
    Ok(child.wait_with_output()?)
}

/// The item's two files, `(settings.json, .env.local)`, as 1Password's `op item get --format json` says them.
fn files_of(item: &Value) -> Option<(String, String)> {
    let field = |id: &str| item["fields"].as_array()?.iter().find(|f| f["id"] == id)?["value"].as_str().map(str::to_string);
    Some((field("settings")?, field("env")?))
}

/// `.env.local`'s `text` with no value in it: its comments and what Varlock reads, each secret's 1Password reference, and
/// the names it reads from where it runs (the service account's token) with nothing after them.
fn references_only(text: &str) -> String {
    text.lines()
        .filter(|l| l.starts_with('#') || l.trim().is_empty() || l.split_once('=').is_some_and(|(_, v)| v.is_empty() || v.starts_with("op(op://")))
        .map(|l| format!("{l}\n"))
        .collect()
}

impl Settings {
    pub fn path() -> PathBuf {
        Settings::path_in(&platform::home())
    }

    fn path_in(home: &Path) -> PathBuf {
        home.join("cloudflare").join("settings.json")
    }

    pub fn load() -> Result<Settings> {
        Settings::load_in(&platform::home())
    }

    /// The settings kept in `home`'s folder: a throwaway network's, read from `one-dev throwaway` with its own home passed
    /// rather than this process told it's there.
    pub fn load_in(home: &Path) -> Result<Settings> {
        let path = Settings::path_in(home);
        let text = std::fs::read_to_string(&path).with_context(|| format!("this machine has no Cloudflare settings yet: {} cloudflare deploy takes a real network's from 1Password, and {} cloudflare serve --address <url> runs Workers here", custom_build::config::CLI, custom_build::config::CLI))?;
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
        settings.telemetry_token = get("TELEMETRY_TOKEN");
        settings.otlp_token = get("OTLP_TOKEN");
        settings.analytics_token = get("ANALYTICS_TOKEN");
        settings.cloudflare_node_key = get("CLOUDFLARE_NODE_KEY").unwrap_or_default();
        settings.notifications = NOTIFICATIONS_SECRETS.iter().filter_map(|name| Some((name.to_string(), get(name)?))).collect();
        if settings.throwaway_user.is_none() {
            for provider in &mut settings.providers {
                provider.client = get(&provider.secret("CLIENT_ID")).zip(get(&provider.secret("CLIENT_SECRET")));
            }
            // A network with a domain has no other way in for its users, so it's never deployed without one.
            ensure!(custom_build::config::JOIN_DOMAIN.is_empty() || settings.providers.iter().any(|p| p.client.is_some()), "this network's users sign in with a provider's account, and no provider has its client here: set {} (its redirect URI is {}/api/auth/callback/{})", settings.providers.first().map_or(String::new(), |p| format!("{} and {}", p.secret("CLIENT_ID"), p.secret("CLIENT_SECRET"))), settings.address, settings.providers.first().map_or("", |p| &p.id));
        }
        Ok(settings)
    }

    /// Where the Workers' `signal` (one of `basin::EXPORTED`) goes.
    pub fn exported(&self, signal: &str) -> Option<&str> {
        match signal {
            "traces" => self.traces.as_deref(),
            _ => self.logs.as_deref(),
        }
    }

    pub fn exported_mut(&mut self, signal: &str) -> &mut Option<String> {
        match signal {
            "traces" => &mut self.traces,
            _ => &mut self.logs,
        }
    }

    /// What `sign_in_proof::CLIENTS` says of the clients this network registered, if it has any.
    pub fn sign_in_clients(&self) -> Option<String> {
        let clients: Vec<String> = self.providers.iter().filter_map(|p| Some(format!("{} {}", p.issuer, p.client.as_ref()?.0))).collect();
        (!clients.is_empty()).then(|| clients.join("\n"))
    }

    /// Writes this machine's `settings.json` and `.env.local` from the 1Password item they're kept in, where a real
    /// network's are: unless the settings here are ones that aren't (a throwaway network's, or Workers tried here), or
    /// `new` ones are to be made. A failure says why, and leaves the files here as they were.
    pub fn from_1password(new: bool) -> Result<()> {
        let path = Settings::path();
        let env = std::fs::read_to_string(local(&platform::home())).unwrap_or_default();
        if new || path.exists() && vault_of(&env).is_none() {
            return Ok(());
        }
        let title = item_title();
        // A service account finds an item only in a vault it's told, so each it may read is asked.
        let vaults = op(&["vault", "list", "--format", "json"], None)?;
        ensure!(vaults.status.success(), "op can't list 1Password's vaults ({}): sign this device in to 1Password with the service account that holds {title} (OP_SERVICE_ACCOUNT_TOKEN)", String::from_utf8_lossy(&vaults.stderr).trim());
        let vaults: Vec<Value> = serde_json::from_slice(&vaults.stdout).context("op said something other than its vaults")?;
        let found = vaults.iter().filter_map(|v| v["id"].as_str()).find_map(|vault| op(&["item", "get", &title, "--vault", vault, "--format", "json"], None).ok().filter(|o| o.status.success()));
        let out = found.with_context(|| format!("1Password has no item {title} in a vault this device's sign-in reads: sign it in with the service account that holds it (OP_SERVICE_ACCOUNT_TOKEN), or deploy once from a device whose Cloudflare settings are here, which keeps them there"))?;
        let item: Value = serde_json::from_slice(&out.stdout).context("op said something other than the item")?;
        let (settings, env) = files_of(&item).with_context(|| format!("{title} in 1Password isn't the Workers' settings: it needs its fields settings and env, as a deploy keeps them"))?;
        write_private(&path, &settings)?;
        write_private(&local(&platform::home()), &env)
    }

    /// Keeps `settings.json` and `.env.local`'s references in their 1Password item, in the vault the references are in,
    /// so any admin's device signed in to it deploys the Workers (`from_1password`). Settings with no references aren't
    /// a real network's, and stay here.
    fn keep_in_1password(&self) -> Result<()> {
        let env = std::fs::read_to_string(local(&self.dir)).unwrap_or_default();
        let Some(vault) = vault_of(&env) else { return Ok(()) };
        let (settings, env) = (std::fs::read_to_string(Settings::path_in(&self.dir))?, references_only(&env));
        let title = item_title();
        let there = op(&["item", "get", &title, "--vault", vault, "--format", "json"], None)?;
        let item: Option<Value> = there.status.success().then(|| serde_json::from_slice(&there.stdout)).transpose().context("op said something other than the item")?;
        if item.as_ref().and_then(files_of).is_some_and(|f| f == (settings.clone(), env.clone())) {
            return Ok(());
        }
        let field = |id: &str, label: &str, value: &str| json!({ "id": id, "type": "STRING", "label": label, "value": value });
        let template = json!({ "title": title, "category": "SECURE_NOTE", "fields": [field("settings", "settings.json", &settings), field("env", ".env.local", &env)] }).to_string();
        let out = match item.as_ref().and_then(|i| i["id"].as_str()) {
            // op edits an item only from a template in a file; this one holds no secret.
            Some(id) => {
                let file = local(&self.dir).with_file_name("1password-item.json");
                std::fs::write(&file, &template)?;
                let out = op(&["item", "edit", id, "--vault", vault, &format!("--template={}", file.display())], None);
                let _ = std::fs::remove_file(&file);
                out?
            }
            None => op(&["item", "create", "--vault", vault, "--format", "json", "-"], Some(&template))?,
        };
        ensure!(out.status.success(), "1Password didn't keep the Workers' settings in {title}, so other devices deploy with what it had: {}", String::from_utf8_lossy(&out.stderr).trim());
        Ok(())
    }

    /// Refuses unless this machine has no Workers' settings or secrets, even ones that don't load, for new ones to be
    /// made: new settings would keep earlier Workers' secrets, or replace settings that only failed to load.
    pub fn none_here() -> Result<()> {
        none_in(Settings::path().parent().unwrap())
    }

    /// New settings, with new secrets.
    pub fn new(address: &str) -> Settings {
        let port = custom_build::config::PORT;
        let ports = [port + 10, port + 13];
        let secret = || B64U.encode(&rand::random::<[u8; 32]>());
        Settings {
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
            notifications: [("VAPID_KEY".to_string(), vapid_key())].into(),
            root: None,
            throwaway_user: None,
            telemetry: None,
            telemetry_token: None,
            traces: None,
            logs: None,
            otlp_token: None,
            analytics_token: None,
            usage: None,
            deployed: None,
            dir: platform::home(),
        }
    }

    /// A throwaway network's Workers (`one-dev throwaway`): on free ports here, its address its gateway's at 127.0.0.1, which `one-dev throwaway`
    /// lets each device it borrows reach at the same address; its bucket where its node serves its data (`bucket`), and
    /// `user` signed in with nothing to show.
    pub fn for_throwaway(home: &Path, bucket: &str, user: (String, String)) -> Result<Settings> {
        let free = || -> Result<u16> { Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port()) };
        let ports = [free()?, free()?];
        let mut settings = Settings::new(&format!("http://127.0.0.1:{}", ports[1]));
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
            for (name, value) in &self.notifications {
                text += &format!("{name}={}\n", quoted(value));
            }
            write_private(&local(&self.dir), &text)?;
        }
        std::fs::write(path.with_file_name(".env.schema"), SCHEMA)?;
        write_private(&path, &String::from_utf8(serde_json::to_vec_pretty(self)?)?)?;
        self.keep_in_1password()
    }

    /// Sets a secret's value in `.env.local`, unless it's a reference to 1Password, where it changes instead. A new
    /// one, where the others are references, is kept in 1Password too, in their vault, and referenced as they are.
    pub fn set_secret(&self, name: &str, value: &str) -> Result<()> {
        let text = std::fs::read_to_string(local(&self.dir)).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with(&format!("{name}="))).map(str::to_string).collect();
        if let Some(line) = text.lines().find(|l| l.starts_with(&format!("{name}="))) {
            ensure!(!line.contains("op("), "{name} is kept in 1Password: change it there ({})", line.trim_start_matches(&format!("{name}=")));
        }
        let kept = match vault_of(&text) {
            Some(vault) => format!("op(op://{vault}/{}/password)", in_1password(vault, &format!("{} Cloudflare {name}", custom_build::config::NAME), value)?),
            None => quoted(value),
        };
        lines.push(format!("{name}={kept}"));
        write_private(&local(&self.dir), &(lines.join("\n") + "\n"))?;
        self.keep_in_1password()
    }

    /// The Cloudflare node's endpoint (its public key, hex), once it has a key.
    pub fn cloudflare_node_endpoint(&self) -> Option<String> {
        let seed: [u8; 32] = data_encoding::HEXLOWER.decode(self.cloudflare_node_key.as_bytes()).ok()?.try_into().ok()?;
        Some(data_encoding::HEXLOWER.encode(ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().as_bytes()))
    }
}

/// The location Workers' objects are made in, where the config sets one.
fn location() -> Option<Binding> {
    (!custom_build::config::RELAY_LOCATION.is_empty()).then(|| ("LOCATION".into(), custom_build::config::RELAY_LOCATION.into(), false))
}

fn plain(name: &str, value: impl Into<String>) -> Binding {
    (name.into(), value.into(), false)
}

fn secret(name: &str, value: impl Into<String>) -> Binding {
    (name.into(), value.into(), true)
}

/// The gateway holds no secrets.
fn gateway_bindings(_: &Settings) -> Vec<Binding> {
    let named = [("NAME", custom_build::config::NAME), ("YOURS", custom_build::config::YOURS), ("CLI", custom_build::config::CLI)].map(|(n, v)| plain(n, v));
    let bound = [("AUTH", "auth"), ("IROH", "relay"), ("SIGNER", "signer"), ("MCP", "mcp"), ("PUSH", "push"), ("TELEMETRY", "telemetry")].map(|(n, s)| plain(n, format!("service:{s}")));
    bound.into_iter().chain(named).collect()
}

/// The notifications Worker: the network's credentials with the push services, and the relay, which says which devices are the network's.
fn notifications_bindings(settings: &Settings) -> Vec<Binding> {
    let cli = custom_build::config::CLI;
    let mut out = vec![
        plain("RELAY", "service:relay"),
        plain("ADDRESS", settings.address.clone()),
        plain("NAME", custom_build::config::NAME),
        // The Apple apps' bundle ids (the iPhone's is config.toml's `iphone`, the Mac's is native-apps/mac/build.sh's), which their notifications go to.
        plain("APNS_TOPICS", json!([custom_build::config::IPHONE, format!("app.{cli}.mac")]).to_string()),
    ];
    out.extend(settings.notifications.iter().map(|(name, value)| secret(name, value.clone())));
    out
}

/// The relay holds no secrets: only the keys of the network's first devices, which it starts its list with, and the
/// Cloudflare node's, whose datagrams it hands its Durable Object, and the remote MCP server's devices, each bound to its
/// object while it runs; and what the Workers are on and the device they're deployed from (`Newest::deployed`), which it
/// says to every device.
fn relay_bindings(settings: &Settings) -> Vec<Binding> {
    let devices = (!settings.devices.is_empty()).then(|| plain("DEVICES", json!(settings.devices).to_string()));
    let node = settings.cloudflare_node_endpoint().filter(|_| settings.cloudflare_node).map(|key| [plain("BOUND", key), plain("NODE", format!("object:{}:{}", NODE.name, NODE.classes[0]))]);
    let mcp = plain("MCP", format!("object:mcp:{}", Worker::named("mcp").classes[0]));
    let deployed = settings.deployed.clone().map(|d| plain("DEPLOYED", d));
    devices.into_iter().chain(node.into_iter().flatten()).chain([mcp, plain("YOURS", custom_build::config::YOURS)]).chain(deployed).chain(location()).collect()
}

/// The remote MCP server: remote MCP clients' devices, whose secrets open only with their clients' tokens. It reaches the
/// relay only by its entrypoint for them, and reads the bucket through the URL signer as any device does.
fn mcp_bindings(settings: &Settings) -> Vec<Binding> {
    let mut out = vec![
        plain("AUTH", "service:auth"),
        plain("RELAY", "service:relay:McpDevices"),
        plain("SIGNER", "service:signer"),
        plain("TELEMETRY", "service:telemetry"),
        plain("ADDRESS", settings.address.clone()),
        plain("NAME", custom_build::config::NAME),
        plain("YOURS", custom_build::config::YOURS),
    ];
    out.extend(location());
    out
}

/// The Worker that holds nothing but who may reach it and where telemetry is in Basin; on one machine, the stand-in's
/// (`served_with`) in Basin's place.
fn telemetry_bindings(settings: &Settings) -> Vec<Binding> {
    let store = match &settings.telemetry {
        Some(telemetry) => {
            let mut bound: Vec<Binding> = [("SPANS", "spans"), ("LOGS", "logs"), ("METRICS", "metrics")].iter().map(|(binding, table)| plain(binding, format!("pipeline:{}", telemetry.streams.get(*table).map_or("", String::as_str)))).collect();
            bound.push(plain("SQL", telemetry.sql()));
            bound.push(secret("SQL_TOKEN", settings.telemetry_token.clone().unwrap_or_default()));
            if let (Some(token), Some(usage)) = (&settings.analytics_token, &settings.usage) {
                bound.push(secret("ANALYTICS_TOKEN", token.clone()));
                bound.push(plain("USAGE", usage.to_string()));
            }
            bound
        }
        None => vec![plain("SPANS", format!("service:{STAND_IN}:Spans")), plain("LOGS", format!("service:{STAND_IN}:Logs")), plain("METRICS", format!("service:{STAND_IN}:Metrics")), plain("SQL", format!("http://{STAND_IN}/")), plain("SQL_TOKEN", "stand-in")],
    };
    let mut out = vec![plain("RELAY", "service:relay"), plain("YOURS", custom_build::config::YOURS), plain("CLI", custom_build::config::CLI)];
    out.extend(store);
    out.extend(settings.otlp_token.clone().map(|token| secret("OTLP_TOKEN", token)));
    out
}

/// The Cloudflare node: its endpoint's key, and the relay it's reached through.
fn node_bindings(settings: &Settings) -> Vec<Binding> {
    let mut out = vec![
        plain("AUTH", "service:auth"),
        plain("SIGNER", "service:signer"),
        plain("TELEMETRY", "service:telemetry"),
        plain("LOADER", "loader:"),
        // Workers AI, for AI Gateway's calls of the apps it runs, paid by the account (oned/cloudflare/node: `ai_run`).
        plain("AI", "ai:"),
        plain("ADDRESS", settings.address.clone()),
        plain("NAME", custom_build::config::NAME),
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
    let mut out = vec![plain("RELAY", "service:relay"), plain("YOURS", custom_build::config::YOURS)];
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
        // Notifies a user's devices, by the notifications Worker's entrypoint for the network's own Workers.
        plain("PUSH", "service:push:Sender"),
        plain("ADDRESS", settings.address.clone()),
        plain("NAME", custom_build::config::NAME),
        plain("YOURS", custom_build::config::YOURS),
        // Its own apps' sign-ins come back to `<cli>://signed-in/`.
        plain("CLI", custom_build::config::CLI),
    ];
    if let Some((id, name)) = &settings.throwaway_user {
        out.extend([plain("SIGN_IN", "throwaway"), plain("THROWAWAY_USER", id.clone()), plain("THROWAWAY_NAME", name.clone())]);
    } else {
        if !custom_build::config::JOIN_DOMAIN.is_empty() {
            out.push(plain("JOIN_DOMAIN", custom_build::config::JOIN_DOMAIN));
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
pub fn served(settings: &Settings, web: &Path, sources: &Files) -> Result<Vec<Served>> {
    served_with(settings, web, sources, crate::node::modules().ok())
}

/// `served`, with the Cloudflare node's build `cloudflare_node` (this build's, or a test's).
pub fn served_with(settings: &Settings, web: &Path, sources: &Files, cloudflare_node: Option<Modules>) -> Result<Vec<Served>> {
    let node = cloudflare_node.clone().filter(|_| settings.cloudflare_node).map(|modules| crate::node::served(settings, modules));
    // A build without the Cloudflare node's WebAssembly has no remote MCP server, and nothing binds one.
    let mut served = Vec::new();
    for worker in WORKERS {
        let Some(modules) = worker.modules(sources, cloudflare_node.as_ref()) else { continue };
        served.push(Served {
            name: worker.name.to_string(),
            module: module_from(sources.clone(), worker.entry)?,
            modules,
            classes: worker.classes.iter().map(|c| c.to_string()).collect(),
            listen: worker.place(settings).map(|(_, port)| format!("127.0.0.1:{port}")),
            bindings: worker.bindings(settings).into_iter().map(|(n, v, _)| (n, v)).collect(),
            assets: worker.serves_web.then(|| web.to_path_buf()),
            // A throwaway network's URL signer, and the remote MCP server's devices reading by its URLs, reach its bucket on this machine.
            outbound: (worker.reads_bucket && settings.throwaway_user.is_some()).then(|| NEARBY.to_string()),
        });
    }
    served.extend(node);
    if settings.telemetry.is_none() {
        served.push(stand_in(sources)?);
        served.iter_mut().find(|s| s.name == "telemetry").context("no telemetry Worker")?.outbound = Some(STAND_IN.to_string());
    }
    let names: Vec<String> = served.iter().map(|s| s.name.clone()).collect();
    for s in &mut served {
        s.bindings.retain(|(_, v)| reaches(v).is_none_or(|other| names.iter().any(|n| n == other)));
    }
    Ok(served)
}

/// Where telemetry goes in Workers that run on one machine, in Basin's place: Basin is only on Cloudflare
/// (`insights/worker/stand-in.js`).
const STAND_IN: &str = "telemetry-store";

fn stand_in(sources: &Files) -> Result<Served> {
    Ok(Served { name: STAND_IN.into(), module: module_from(sources.clone(), "telemetry/stand-in.js")?, modules: vec![], classes: vec!["Store".into()], listen: None, bindings: vec![], assets: None, outbound: None })
}

/// The Android app's package, as native-apps/android/app/build.gradle.kts names it.
pub fn android_package() -> String {
    custom_build::config::ANDROID.to_string()
}

impl Settings {
    /// The files the address serves itself: the web client's, as this release has them, Android's App Links file
    /// (`.well-known/assetlinks.json`) and Windows' web links file (`.well-known/windows-app-web-link`), which let the
    /// apps open links to the address, and the guide to connecting an AI (`llms.txt`).
    pub fn web(&self) -> Vec<(String, Vec<u8>)> {
        self.web_with(::ui::web::with_client())
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
        files.push(("llms.txt".into(), api_features::skills::guide(Some(&self.address)).into_bytes()));
        files
    }
}

/// The Windows app's package family name, as Windows names the package native-apps/windows/build.sh makes: the config's name in
/// letters and digits, then its publisher's (`CN=` and the name in letters, digits and spaces) hash, the first 8 bytes
/// of the SHA-256 of it in UTF-16LE, in Crockford's base32.
pub fn windows_package() -> String {
    let name = custom_build::config::NAME;
    let publisher = format!("CN={}", name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect::<String>());
    let hash = sha2::Sha256::digest(publisher.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
    let bits = u128::from(u64::from_be_bytes(hash[..8].try_into().unwrap())) << 1;
    let id: String = (0..13).map(|i| b"0123456789abcdefghjkmnpqrstvwxyz"[(bits >> ((12 - i) * 5)) as usize & 31] as char).collect();
    format!("{}_{id}", name.chars().filter(char::is_ascii_alphanumeric).collect::<String>())
}

/// `/.well-known/windows-app-web-link`: the pages at the address the Windows app opens, when a link to one is opened
/// from another app, as the web client has them (ui/web/routes.js): You, a page, its Accounts, an app's screen,
/// Settings, a Vault request and an invite.
fn windows_links() -> Vec<u8> {
    let paths = ["/", "/page/*", "/accounts", "/accounts/*", "/app/*", "/settings", "/settings/*", "/vault", "/invite"];
    json!([{ "packageFamilyName": windows_package(), "paths": paths }]).to_string().into_bytes()
}

/// Runs the network's Workers on this machine, in the pinned workerd, until it stops: their modules built from this
/// release, the web client's files in `<home>/ui/web`, and their storage in `<home>/cloudflare/data`. Where
/// they're a throwaway network's, which has no Cloudflare to deploy to, a deploy (`redeploy`) starts them again on the
/// newest the network's record names.
pub fn serve(settings: &Settings) -> Result<()> {
    let dir = Settings::path_in(&settings.dir).parent().unwrap().to_path_buf();
    let web = web_dir(settings);
    let here = |a: &str| a.strip_prefix("127.0.0.1:").or_else(|| a.strip_prefix("http://127.0.0.1:")).is_some_and(|port| port.parse::<u16>().is_ok());
    let mut settings = settings.clone();
    let mut newest = crate::Newest::built();
    #[cfg(unix)]
    let redeploys = redeploys::listen(&dir, &settings.address)?;
    loop {
        settings.deployed = Some(newest.deployed(&settings));
        let served = served(&settings, &web, &newest.workers)?;
        ensure!(settings.throwaway_user.is_none() || here(&settings.address) && served.iter().all(|s| s.listen.as_deref().is_none_or(here)), "a throwaway network's auth lets in anyone who reaches it, so its Workers run only beside its throwaway network, at 127.0.0.1");
        let _ = std::fs::remove_dir_all(&web);
        write_web_of(&settings, newest.web.clone())?;
        let config = write_config(&dir, &served)?;
        let mut child = start(&config, Vec::new())?;
        #[cfg(unix)]
        {
            redeploys.running(child.id());
            let status = child.wait()?;
            match redeploys.next() {
                Some(next) => newest = next,
                None => bail!("the Workers' workerd stopped: {status}"),
            }
        }
        #[cfg(not(unix))]
        bail!("the Workers' workerd stopped: {}", child.wait()?);
    }
}

/// Asks the Workers `serve` runs here to start again on the newest: for a throwaway network, a deploy. → once `serve` has
/// read the newest, or why it couldn't.
#[cfg(unix)]
pub fn redeploy(settings: &Settings) -> Result<()> {
    use std::io::Read;
    let mut asked = std::os::unix::net::UnixStream::connect(Settings::path_in(&settings.dir).with_file_name(redeploys::SOCKET)).context("the Workers aren't running here: the throwaway network starts them")?;
    let mut said = String::new();
    asked.read_to_string(&mut said)?;
    ensure!(said.is_empty(), "{said}");
    Ok(())
}

/// The newest, read while the Workers still answer, and their workerd, stopped for `serve` to start another.
#[cfg(unix)]
mod redeploys {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::{Arc, Mutex};

    /// Where `serve` listens for a deploy, in the Workers' folder.
    pub const SOCKET: &str = "redeploy.sock";

    pub struct Redeploys {
        workerd: Arc<AtomicI32>,
        newest: Arc<Mutex<Option<crate::Newest>>>,
    }

    /// Listens for `redeploy`'s asks on a thread of its own.
    pub fn listen(dir: &Path, address: &str) -> Result<Redeploys> {
        let path = dir.join(SOCKET);
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).with_context(|| format!("listen at {}", path.display()))?;
        let (workerd, newest) = (Arc::new(AtomicI32::new(0)), Arc::new(Mutex::new(None)));
        let (pid, slot, address) = (workerd.clone(), newest.clone(), address.to_string());
        std::thread::spawn(move || {
            for mut asked in listener.incoming().flatten() {
                let said = match crate::Newest::read(&address) {
                    Ok(read) => {
                        *slot.lock().unwrap() = Some(read);
                        // Not asked to stop: workerd told to waits for its open connections, and the relay's never close.
                        let pid = pid.load(Ordering::SeqCst);
                        if pid > 0 {
                            unsafe { libc::kill(pid, libc::SIGKILL) };
                        }
                        String::new()
                    }
                    Err(err) => format!("the newest can't be read, so the Workers stay as they are: {err:#}"),
                };
                let _ = asked.write_all(said.as_bytes());
            }
        });
        Ok(Redeploys { workerd, newest })
    }

    impl Redeploys {
        /// `pid` is the workerd now running. One asked for while it started is stopped now, as it wasn't then.
        pub fn running(&self, pid: u32) {
            self.workerd.store(pid as i32, Ordering::SeqCst);
            if self.newest.lock().unwrap().is_some() {
                unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            }
        }

        /// What was asked for, once the workerd it stopped has gone.
        pub fn next(&self) -> Option<crate::Newest> {
            self.workerd.store(0, Ordering::SeqCst);
            self.newest.lock().unwrap().take()
        }
    }
}

/// Where the network's Workers read the web client's files from on this machine.
fn web_dir(settings: &Settings) -> PathBuf {
    Settings::path_in(&settings.dir).parent().unwrap().join("web")
}

/// Writes the web client's files as this program has them (`Settings::web`) where the Workers read them (`serve`'s
/// workerd reads each file as it's asked for, so a file written while they run is what the next request gets): each one
/// whole or not at all, so a request never gets half of one.
pub fn write_web(settings: &Settings) -> Result<()> {
    write_web_of(settings, ::ui::web::with_client())
}

fn write_web_of(settings: &Settings, client: Files) -> Result<()> {
    let web = web_dir(settings);
    let mut files = settings.web_with(client);
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
    let kept = platform::cache().join("brotli");
    let names: Vec<PathBuf> = wanted.iter().map(|(_, bytes)| kept.join(data_encoding::HEXLOWER.encode(&Sha256::digest(bytes)))).collect();
    let missing: Vec<usize> = (0..wanted.len()).filter(|&i| !names[i].is_file()).collect();
    if !missing.is_empty() {
        let dir = std::env::temp_dir().join(format!("{}-brotli-{}", custom_build::config::CLI, rand::random::<u64>()));
        let made = (|| {
            std::fs::create_dir_all(&kept)?;
            std::fs::create_dir_all(dir.join("in"))?;
            std::fs::create_dir_all(dir.join("out"))?;
            for &i in &missing {
                std::fs::write(dir.join("in").join(i.to_string()), &wanted[i].1)?;
            }
            std::fs::write(dir.join("brotli.mjs"), BROTLI)?;
            let out = std::process::Command::new(::deploy::build::bun()?).args(["brotli.mjs"]).current_dir(&dir).env("DO_NOT_TRACK", "1").output()?;
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
    let dir = std::env::temp_dir().join(format!("{}-scripts-{}", custom_build::config::CLI, rand::random::<u64>()));
    let checked = (|| {
        for (path, text) in &scripts {
            let file = dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap())?;
            std::fs::write(file, text)?;
        }
        let out = std::process::Command::new(::deploy::build::bun()?)
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
    module_from(::ui::ui_folder::cloudflare(), entry)
}

/// The module of what the Workers and the web client share with the Rust (`protocol::javascript`), in `cloudflare/`.
pub const PROTOCOL: &str = "shared/protocol.js";

/// A Worker's module, built from `files` (paths in `cloudflare/`).
pub fn module_from(mut files: Vec<(String, Vec<u8>)>, entry: &str) -> Result<Vec<u8>> {
    // What they say that Rust says too, as this build says it (`protocol`): never a file of the repo.
    files.retain(|(path, _)| path != PROTOCOL);
    files.push((PROTOCOL.into(), ::api::protocol_js::javascript().into_bytes()));
    let mut hash = Sha256::new();
    for (path, bytes) in &files {
        hash.update(format!("{path}\0{}\0", bytes.len()));
        hash.update(bytes);
    }
    let hash = data_encoding::HEXLOWER.encode(&hash.finalize());
    let kept = platform::cache().join("cloudflare").join(format!("{}-{}.js", &hash[..32], entry.replace('/', "-")));
    if let Ok(module) = std::fs::read(&kept) {
        return Ok(module);
    }
    let dir = std::env::temp_dir().join(format!("{}-cloudflare-{}", custom_build::config::CLI, rand::random::<u64>()));
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
    let bun = ::deploy::build::bun()?;
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
    pub classes: Vec<String>,
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
        let mut bindings: Vec<String> = s.classes.iter().map(|c| format!("(name = {}, durableObjectNamespace = {})", quote(&c.to_uppercase()), quote(c))).collect();
        if let Some(assets) = &s.assets {
            // With `.well-known/`, which Windows and Android ask the address for.
            services.push(format!("(name = {}, disk = (path = {}, writable = false, allowDotfiles = true))", quote(&format!("{}-assets", s.name)), quote(&assets.to_string_lossy())));
            bindings.push(format!("(name = \"ASSETS\", service = {})", quote(&format!("{}-assets", s.name))));
        }
        for (name, value) in &s.bindings {
            // An R2 bucket is bound only on Cloudflare, where the URL signer here reaches it by URL; Workers AI and Basin's streams are only there.
            if value.starts_with("r2:") || value.starts_with("pipeline:") || value == "ai:" {
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
        let objects = match s.classes.as_slice() {
            [] => String::new(),
            classes => format!(
                "  durableObjectNamespaces = [{}],\n  durableObjectStorage = (localDisk = {}),\n",
                // The first class's objects keep the Worker's own key, as they had before it had others.
                classes.iter().enumerate().map(|(i, c)| format!("(className = {}, uniqueKey = {}, enableSql = true)", quote(c), quote(&if i == 0 { s.name.clone() } else { format!("{}-{c}", s.name) }))).collect::<Vec<_>>().join(", "),
                quote(&format!("{}-data", s.name))
            ),
        };
        workers.push(format!(
            "const w{i} :Workerd.Worker = (\n  modules = [{}],\n  compatibilityDate = \"{}\",\n  bindings = [{}],\n{objects}{});",
            modules.join(", "),
            ::deploy::build::COMPATIBILITY_DATE,
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
    let mut command = std::process::Command::new(::deploy::build::workerd()?);
    command.arg("serve").arg(config).arg("--experimental").stdin(std::process::Stdio::null());
    #[cfg(not(unix))]
    ensure!(listening.is_empty(), "workerd takes listening sockets only on Unix");
    platform::os::child(&mut command, &listening.iter().map(|(n, l)| (n.as_str(), l)).collect::<Vec<_>>());
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
            ("DELETE", _) => agent.delete(&url).header("authorization", &auth).call()?,
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

    /// What's at `path`, or none if nothing is.
    pub(crate) fn found(&self, path: &str) -> Result<Option<Value>> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(60))).build().into();
        let mut resp = agent.get(format!("{}{path}", self.api)).header("authorization", format!("Bearer {}", self.token)).header("user-agent", custom_build::config::CLI).call()?;
        if resp.status() == 404 {
            return Ok(None);
        }
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        ensure!(out["success"] == true, "Cloudflare said {} to GET {path}: {}", resp.status(), out["errors"]);
        Ok(Some(out["result"].clone()))
    }

    /// Every page of a list.
    pub(crate) fn list(&self, path: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for page in 1.. {
            let some = self.call("GET", &format!("{path}?per_page=100&page={page}"), None)?;
            let got = some.as_array().map_or(0, Vec::len);
            out.extend(some.as_array().into_iter().flatten().cloned());
            if got < 100 {
                break;
            }
        }
        Ok(out)
    }

    pub(crate) fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.send("POST", path, body)
    }

    pub(crate) fn delete(&self, path: &str) -> Result<Value> {
        self.send("DELETE", path, json!({}))
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

    /// The id of the one version the Worker `name` serves, if it's the one `what` names (`upload_hash`): a Worker that would
    /// get the same isn't uploaded again, which would restart its Durable Objects, and whatever else put a version there
    /// (an edit, a rollback) counts as what it is.
    fn serving(&self, name: &str, what: &str) -> Result<Option<String>> {
        let list = self.call("GET", &format!("/accounts/{}/workers/scripts/{name}/deployments", self.account), None)?;
        let versions = list["deployments"][0]["versions"].as_array().cloned().unwrap_or_default();
        let [only] = versions.as_slice() else { return Ok(None) };
        let id = only["version_id"].as_str().context("Cloudflare named no version")?;
        let version = self.call("GET", &format!("/accounts/{}/workers/scripts/{name}/versions/{id}", self.account), None)?;
        Ok((version["annotations"]["workers/message"] == what).then(|| id.to_string()))
    }

    /// Whether the Worker `name` runs what `what` names.
    pub(crate) fn runs(&self, name: &str, what: &str) -> Result<bool> {
        Ok(self.serving(name, what)?.is_some())
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
        let version = self.serving(name, what)?.with_context(|| format!("Cloudflare took the upload of {name}, but serves another version"))?;
        // Cloudflare's default lets a Durable Object that's running keep the old code for up to five minutes (a deferred
        // update), so auth and the relay would answer on the old build, or the old secrets, after "Deployed". A deploy
        // is the moment to take the new one.
        self.send("POST", &format!("/accounts/{}/workers/scripts/{name}/deployments", self.account), json!({ "strategy": "percentage", "versions": [{ "percentage": 100, "version_id": version }], "annotations": { "workers/message": what }, "code_update_strategy": { "mode": "immediate" } }))
            .with_context(|| format!("{name} is uploaded, but its Durable Objects weren't told to take it at once"))?;
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

    /// Runs a Worker at `cron` too, or on no schedule. Changed only if it differs: a change takes Cloudflare up to 15
    /// minutes to take up, in which a run can be missed.
    pub(crate) fn schedule(&self, name: &str, cron: Option<&str>) -> Result<()> {
        let path = format!("/accounts/{}/workers/scripts/{name}/schedules", self.account);
        let has: Vec<String> = self.call("GET", &path, None)?["schedules"].as_array().into_iter().flatten().filter_map(|s| s["cron"].as_str().map(str::to_string)).collect();
        if has.iter().map(String::as_str).eq(cron.iter().copied()) {
            return Ok(());
        }
        self.send("PUT", &path, json!(cron.iter().map(|c| json!({ "cron": c })).collect::<Vec<_>>()))?;
        Ok(())
    }

    /// Gives a Worker its workers.dev address alone.
    pub(crate) fn expose(&self, name: &str) -> Result<()> {
        self.send("POST", &format!("/accounts/{}/workers/scripts/{name}/subdomain", self.account), json!({ "enabled": true, "previews_enabled": false }))?;
        Ok(())
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
    format!("{}-{name}", custom_build::config::CLI)
}

/// What a Worker's upload says besides its module: its bindings (another service's by its Worker's name), its Durable
/// Objects' classes, and the migration that adds those it hasn't had (`migration`: the tag of the one it has had, `v<n>`
/// for its first n classes).
pub fn metadata(settings: &Settings, worker: &Worker, migration: Option<&str>, exists: impl Fn(&str) -> bool) -> (Value, bool) {
    let mut complete = true;
    let mut bindings: Vec<Value> = worker.classes.iter().map(|class| json!({ "type": "durable_object_namespace", "name": class.to_uppercase(), "class_name": class })).collect();
    for (name, value, secret) in worker.bindings(settings) {
        // The stand-in exists only on one machine: Cloudflare refuses a binding to a Worker it doesn't have, and a network
        // whose telemetry isn't set up yet still gets its Workers.
        if reaches(&value) == Some(STAND_IN) {
            continue;
        }
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
                None if value.starts_with("pipeline:") => bindings.push(json!({ "type": "pipelines", "name": name, "pipeline": value.trim_start_matches("pipeline:") })),
                None => bindings.push(json!({ "type": if secret { "secret_text" } else { "plain_text" }, "name": name, "text": value })),
            },
        }
    }
    let mut meta = json!({ "main_module": "worker.js", "compatibility_date": ::deploy::build::COMPATIBILITY_DATE, "bindings": bindings, "observability": { "enabled": true } });
    // Kept in Cloudflare, sent to the telemetry Worker once a deploy has made where to (`basin::exports`), and never for the
    // Cloudflare node, which reports its calls as every node does, or the telemetry Worker.
    if TRACED.contains(&worker.name) {
        meta["observability"]["traces"] = json!({ "enabled": true, "head_sampling_rate": 1 });
        for (signal, _) in crate::basin::EXPORTED {
            if settings.exported(signal).is_some() {
                meta["observability"][signal]["enabled"] = json!(true);
                meta["observability"][signal]["destinations"] = json!([crate::basin::destination(signal)]);
                meta["observability"][signal]["persist"] = json!(true);
            }
        }
    }
    let had = migration.and_then(|tag| tag.strip_prefix('v')?.parse::<usize>().ok()).unwrap_or(0);
    if had < worker.classes.len() {
        meta["migrations"] = json!({ "new_tag": format!("v{}", worker.classes.len()), "new_sqlite_classes": worker.classes[had..] });
        if had > 0 {
            meta["migrations"]["old_tag"] = json!(format!("v{had}"));
        }
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
            let Some(wasm) = worker.modules(sources, crate::node::modules().ok().as_ref()) else { bail!("this build has no Cloudflare node, which the {} Worker runs: {}-dev update node builds one", worker.name, custom_build::config::CLI) };
            let module = module_from(sources.clone(), worker.entry)?;
            let mut what = vec![meta.to_string().into_bytes(), module.clone(), url.unwrap_or("").as_bytes().to_vec()];
            what.extend(wasm.iter().flat_map(|(name, bytes)| [name.as_bytes().to_vec(), bytes.clone()]));
            if worker.serves_web {
                what.extend(web.iter().flat_map(|(path, bytes)| [path.as_bytes().to_vec(), bytes.clone()]));
            }
            let what = upload_hash(&what);
            // Its address may have been changed by hand; one with none is reached only by binding. So may its schedule.
            let reach = || -> Result<()> {
                match url {
                    Some(url) => cf.reach(&name, url)?,
                    None if worker.own_address => cf.expose(&name)?,
                    None => cf.hide(&name)?,
                }
                cf.schedule(&name, worker.cron)
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

#[cfg(test)]
mod observability_tests {
    use super::*;

    /// Every Worker's traces and logs are exported but the telemetry Worker's, which would be handed its own export, and
    /// the Cloudflare node's, which reports its calls as every node does.
    #[test]
    fn every_worker_but_telemetry_and_the_node_is_exported() {
        let mut settings = Settings::new("https://example.org");
        settings.traces = Some("https://t.example/telemetry/otlp".into());
        settings.logs = settings.traces.clone();
        let observed = |worker: &Worker| metadata(&settings, worker, None, |_| true).0["observability"].clone();
        for name in TRACED {
            let o = observed(Worker::named(name));
            assert_eq!((&o["traces"]["enabled"], &o["traces"]["destinations"][0], &o["logs"]["destinations"][0]), (&json!(true), &json!(crate::basin::destination("traces")), &json!(crate::basin::destination("logs"))), "{name}");
        }
        assert!(TRACED.contains(&"mcp"));
        for worker in [Worker::named("telemetry"), &NODE] {
            let o = observed(worker);
            assert!(o["traces"].is_null() && o["logs"].is_null(), "{}", worker.name);
        }
    }
}
