//! The kernel's services, built with the pinned Bun and run in workerd here, or deployed to Cloudflare.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

// Building and running.

/// The kernel's services: each one's name, its module, and its Durable Object's class, if it keeps anything. Each
/// one's WebAssembly is the `.wasm` files in its module's folder, modules of its own (Workers can't compile it).
pub const SERVICES: &[(&str, &str, Option<&str>)] = &[
    ("auth", "auth/worker.js", Some("Auth")),
    ("relay", "relay/worker.js", Some("Relay")),
    ("signer", "signer/worker.js", None),
    ("mcp", "mcp/worker.js", Some("Client")),
    ("gateway", "gateway/worker.js", None),
];

/// A Durable Object class a service had once, (service, class), which its next upload deletes on Cloudflare.
const DELETED: &[(&str, &str)] = &[("gateway", "Relay")];

/// A service's WebAssembly modules, (name, bytes): the `.wasm` files in its module's folder, as this release has them.
pub fn wasm(entry: &str) -> Vec<(String, Vec<u8>)> {
    let dir = entry.rsplit_once('/').map_or("", |(dir, _)| dir);
    crate::config::KERNEL
        .iter()
        .filter_map(|(path, bytes)| {
            let name = path.strip_prefix(dir)?.strip_prefix('/')?;
            (name.ends_with(".wasm") && !name.contains('/')).then(|| (name.to_string(), bytes.to_vec()))
        })
        .collect()
}

/// The serverless node's modules, as `serverless::modules` gives them.
pub type Modules = Vec<(String, &'static str, Vec<u8>)>;

/// A service's modules besides its own, (name, bytes): its WebAssembly, and for the MCP Worker the serverless node's
/// build (`serverless`), which runs its devices' endpoints. None if there's none.
pub fn modules_of(service: &str, entry: &str, serverless: Option<&Modules>) -> Option<Vec<(String, Vec<u8>)>> {
    let mut out = wasm(entry);
    if service == "mcp" {
        let built = serverless?.clone();
        out.extend(built.into_iter().filter(|(name, _, _)| name.starts_with("one_serverless")).map(|(name, _, bytes)| (name, bytes)));
    }
    Some(out)
}

/// How an install's kernel is set up, wherever it runs: kept in `<home>/kernel/settings.json`, with its secrets in
/// `.env.local` beside it, which Varlock reads (see `SCHEMA`): their values, for a kernel tried here, or
/// `op(op://…)` references to 1Password, for one people use, so no file here holds them.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct Settings {
    /// The install's name, as sign-in screens say it.
    pub name: String,
    /// The install's address, the one people use: the gateway's, which sends sign-in to the auth service, MCP to the MCP
    /// Worker, the relay's paths to the relay and the rest to the web client. Passkeys belong to its host name, so it
    /// doesn't change.
    pub address: String,
    /// Where the kernel's own tools reach the auth service; people reach it at the address.
    pub auth_url: String,
    /// Whether the install has a serverless node (`serverless deploy`), which the relay wakes.
    #[serde(default)]
    pub serverless: bool,
    /// The local ports the auth service and the gateway listen on, when this machine runs them. The other services
    /// have none: other Workers reach them by binding.
    pub ports: [u16; 2],
    /// The install's devices (`{ key, role, admin }`: key in hex, roles joined by spaces), and its recovery key as a
    /// client, from the node on this machine when the kernel is deployed or served: the relay starts its list with
    /// them when it has none yet.
    #[serde(skip)]
    pub devices: Vec<serde_json::Value>,
    /// The serverless node's endpoint key (hex), which only its Worker holds: its public half is on the relay's list.
    #[serde(default, skip_serializing)]
    pub serverless_key: String,
    /// Better Auth's own secret, for its cookies and the keys it keeps.
    #[serde(default, skip_serializing)]
    pub auth_secret: String,
    /// Nodes' own addresses MCP clients sign in for (each a resource, in OAuth's words), besides the install's.
    #[serde(default)]
    pub resources: Vec<String>,
    /// The install's bucket: `{ endpoint, bucket, region, key_id, secret }`, the URL signer's only secret: nothing else
    /// holds its credentials.
    #[serde(default, skip_serializing)]
    pub bucket: Option<Value>,
    /// The SHA-256 fingerprints of the certificates the Android app is signed with: Android opens links to the address
    /// in the app (App Links) only if the address lists the app's package and one of them.
    #[serde(default)]
    pub android: Vec<String>,
    /// For an install whose people sign in with Google (`config::SIGN_IN`): the Google OAuth client they sign in
    /// with, `(id, secret)`.
    #[serde(default, skip_serializing)]
    pub google: Option<(String, String)>,
    /// The API token `kernel deploy` and `serverless deploy` edit the Cloudflare account's Workers with, where
    /// CLOUDFLARE_API_TOKEN isn't set.
    #[serde(default, skip_serializing)]
    pub cloudflare_token: Option<String>,
    /// A try install's one person, `(id, name)`, whom its kernel signs in with no passkey and no Google, so clients
    /// are tested with nobody there. Only `one try` sets it, on the kernel it runs beside its try install, listening
    /// only on this machine; `deploy` refuses settings with it, so no kernel people use ever has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub try_person: Option<(String, String)>,
}

/// Whether people sign in with Google.
fn google() -> bool {
    crate::config::SIGN_IN == "google"
}

/// What Varlock reads the kernel's secrets by, each named as the services' binding is; `.env.local` gives their values.
const SCHEMA: &str = "\
# The kernel's secrets, read with Varlock: their values are in .env.local, as written for a kernel tried on this
# machine, or op(op://…) references to 1Password for one people use. Never print them.
# @defaultSensitive=true
# @defaultRequired=true
# ---
AUTH_SECRET=
# The install's bucket's credentials, which only the URL signer holds.
# @optional
BUCKET=
# For an install whose people sign in with Google: their Google OAuth client.
# @optional
GOOGLE_CLIENT_ID=
# @optional
GOOGLE_CLIENT_SECRET=
# The API token kernel deploy edits the Cloudflare account's Workers with, where CLOUDFLARE_API_TOKEN isn't set.
# @optional
CLOUDFLARE_API_TOKEN=
# The serverless node's endpoint key, made by serverless deploy.
# @optional
SERVERLESS_KEY=
";

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
        .context("the kernel's other secrets are in 1Password, and op (1Password's CLI) isn't here to keep this one there")?;
    op.stdin.take().unwrap().write_all(template.to_string().as_bytes())?;
    let out = op.wait_with_output()?;
    ensure!(out.status.success(), "1Password didn't keep it: {}", String::from_utf8_lossy(&out.stderr).trim());
    let item: Value = serde_json::from_slice(&out.stdout).context("op said something other than the item it made")?;
    Ok(item["id"].as_str().context("op didn't say the item's id")?.to_string())
}

/// The Varlock `kernel` reads secrets with, and each platform's download of it with the download's sha256, as its
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
        .ok_or_else(|| anyhow!("this machine can't run a kernel: Varlock has no build for {}", crate::config::TARGET))?;
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

/// Where the kernel's secrets' values are: `KEY=value` lines, Varlock's `.env.local`.
fn local() -> PathBuf {
    Settings::path().with_file_name(".env.local")
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
    ensure!(there.is_empty(), "{} has a kernel's {} already, which didn't load: move them away to make new ones", dir.display(), there.join(" and "));
    Ok(())
}

/// A value as `.env.local` writes it: in single quotes, which Varlock takes as they are.
fn quoted(value: &str) -> String {
    format!("'{value}'")
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::home().join("kernel").join("settings.json")
    }

    pub fn load() -> Result<Settings> {
        let path = Settings::path();
        let text = std::fs::read_to_string(&path).with_context(|| format!("this machine runs no kernel yet: {} kernel serve --address <url> starts one, and its settings are on the machine that made it", crate::config::CLI))?;
        let mut settings: Settings = serde_json::from_str(&text).with_context(|| format!("{} isn't a kernel's settings", path.display()))?;
        let dir = path.parent().unwrap();
        std::fs::write(dir.join(".env.schema"), SCHEMA)?;
        let out = std::process::Command::new(varlock()?)
            .args(["load", "--format", "json"])
            .current_dir(dir)
            .env("VARLOCK_TELEMETRY_DISABLED", "true")
            .env("DO_NOT_TRACK", "1")
            .output()?;
        // Varlock masks secrets in what it says when it can't load them.
        ensure!(out.status.success(), "Varlock couldn't read the kernel's secrets ({}):\n{}{}", local().display(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        let secrets: std::collections::BTreeMap<String, String> = serde_json::from_slice(&out.stdout).context("Varlock said something other than the kernel's secrets")?;
        let get = |name: &str| secrets.get(name).filter(|v| !v.is_empty()).cloned();
        settings.auth_secret = get("AUTH_SECRET").context("no AUTH_SECRET")?;
        settings.bucket = get("BUCKET").map(|b| serde_json::from_str(&b)).transpose().context("BUCKET isn't JSON")?;
        settings.cloudflare_token = get("CLOUDFLARE_API_TOKEN");
        settings.serverless_key = get("SERVERLESS_KEY").unwrap_or_default();
        if settings.try_person.is_none() && google() {
            settings.google = get("GOOGLE_CLIENT_ID").zip(get("GOOGLE_CLIENT_SECRET"));
            ensure!(settings.google.is_some(), "no GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET: the Google OAuth client people sign in with, whose redirect URI is {}/api/auth/callback/google", settings.address);
        }
        Ok(settings)
    }

    /// Refuses unless this machine has no kernel's settings or secrets, even ones that don't load, for new ones to be
    /// made: new settings would keep an earlier kernel's secrets, or replace settings that only failed to load.
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
            serverless: false,
            serverless_key: data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>()),
            ports,
            auth_secret: secret(),
            resources: vec![],
            bucket: None,
            android: vec![],
            google: None,
            cloudflare_token: None,
            devices: vec![],
            try_person: None,
        }
    }

    /// A try install's kernel (`one try`): on free ports here, its address its gateway's at 127.0.0.1, which `one try`
    /// lets each device it borrows reach at the same address; its bucket where its node serves its data (`bucket`), and
    /// `person` signed in with nothing to show.
    pub fn for_try(bucket: &str, person: (String, String)) -> Result<Settings> {
        let free = || -> Result<u16> { Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port()) };
        let ports = [free()?, free()?];
        let mut settings = Settings::new("Try", &format!("http://127.0.0.1:{}", ports[1]));
        settings.auth_url = format!("http://127.0.0.1:{}", ports[0]);
        settings.ports = ports;
        settings.try_person = Some(person);
        // Its node serves its data there (try_install::bucket), signatures unchecked: the URL signer holds these.
        settings.bucket = Some(json!({ "endpoint": format!("http://{bucket}"), "bucket": "try", "region": "auto", "key_id": "try", "secret": "try" }));
        Ok(settings)
    }

    /// Writes the settings, and, the first time, the secrets' values to `.env.local`; after that a secret changes
    /// only by `set_secret`, so a reference to 1Password there stays one.
    pub fn save(&self) -> Result<()> {
        let path = Settings::path();
        if !local().exists() {
            let mut text = String::new();
            text += &format!("AUTH_SECRET={}\n", quoted(&self.auth_secret));
            if let Some(bucket) = &self.bucket {
                text += &format!("BUCKET={}\n", quoted(&bucket.to_string()));
            }
            if !self.serverless_key.is_empty() {
                text += &format!("SERVERLESS_KEY={}\n", quoted(&self.serverless_key));
            }
            write_private(&local(), &text)?;
        }
        std::fs::write(path.with_file_name(".env.schema"), SCHEMA)?;
        write_private(&path, &String::from_utf8(serde_json::to_vec_pretty(self)?)?)
    }

    /// Sets a secret's value in `.env.local`, unless it's a reference to 1Password, where it changes instead. A new
    /// one, where the others are references, is kept in 1Password too, in their vault, and referenced as they are.
    pub fn set_secret(&self, name: &str, value: &str) -> Result<()> {
        let text = std::fs::read_to_string(local()).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with(&format!("{name}="))).map(str::to_string).collect();
        if let Some(line) = text.lines().find(|l| l.starts_with(&format!("{name}="))) {
            ensure!(!line.contains("op("), "{name} is kept in 1Password: change it there ({})", line.trim_start_matches(&format!("{name}=")));
        }
        let vault = text.split("op(op://").nth(1).and_then(|rest| rest.split('/').next()).filter(|v| !v.is_empty());
        let kept = match vault {
            Some(vault) => format!("op(op://{vault}/{}/password)", in_1password(vault, &format!("{} kernel {name}", self.name), value)?),
            None => quoted(value),
        };
        lines.push(format!("{name}={kept}"));
        write_private(&local(), &(lines.join("\n") + "\n"))
    }

    /// The serverless node's endpoint (its public key, hex), once it has a key.
    pub fn serverless_endpoint(&self) -> Option<String> {
        let seed: [u8; 32] = data_encoding::HEXLOWER.decode(self.serverless_key.as_bytes()).ok()?.try_into().ok()?;
        Some(data_encoding::HEXLOWER.encode(ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().as_bytes()))
    }

    /// Where a service is reached, and the local port it listens on: none for the relay and the URL signer, which only
    /// other Workers' bindings reach.
    pub fn place(&self, service: &str) -> Option<(&str, u16)> {
        let i = ["auth", "gateway"].iter().position(|s| *s == service)?;
        Some(([&self.auth_url, &self.address][i].as_str(), self.ports[i]))
    }

    /// Each service's bindings: (name, value, whether it's secret). A value `service:<name>` is another service,
    /// `object:<name>:<class>` another service's Durable Object class, `loader:` Worker Loader, and `r2:<bucket>` an R2
    /// bucket of the account's.
    pub fn bindings(&self, service: &str) -> Vec<(String, String, bool)> {
        // The gateway holds no secrets.
        if service == "gateway" {
            return vec![("AUTH".into(), "service:auth".into(), false), ("IROH".into(), "service:relay".into(), false), ("SIGNER".into(), "service:signer".into(), false), ("MCP".into(), "service:mcp".into(), false)];
        }
        let location = (!crate::config::RELAY_LOCATION.is_empty()).then(|| ("LOCATION".into(), crate::config::RELAY_LOCATION.into(), false));
        // Nor does the relay: only the keys of the install's first devices, which it starts its list with, and the
        // serverless node's, whose datagrams it hands its Durable Object.
        if service == "relay" {
            let devices = (!self.devices.is_empty()).then(|| ("DEVICES".into(), json!(self.devices).to_string(), false));
            let bound = self.serverless_endpoint().filter(|_| self.serverless).map(|key| [("BOUND".into(), key, false), ("NODE".into(), format!("object:{}:Node", crate::serverless::SERVICE), false)]);
            // And the MCP Worker's devices, each bound to its object while it runs.
            let mcp = ("MCP".into(), "object:mcp:Client".into(), false);
            return devices.into_iter().chain(bound.into_iter().flatten()).chain([mcp]).chain(location).collect();
        }
        // The MCP Worker: remote MCP clients' devices, whose secrets open only with their clients' tokens. It reaches
        // the relay only by its entrypoint for them, and reads the bucket through the URL signer as any device does.
        if service == "mcp" {
            let mut out = vec![
                ("AUTH".into(), "service:auth".into(), false),
                ("RELAY".into(), "service:relay:McpDevices".into(), false),
                ("SIGNER".into(), "service:signer".into(), false),
                ("ADDRESS".into(), self.address.clone(), false),
                ("NAME".into(), self.name.clone(), false),
            ];
            out.extend(location);
            return out;
        }
        // The serverless node: its endpoint's key, and the relay it's reached through.
        if service == crate::serverless::SERVICE {
            let mut out = vec![
                ("AUTH".into(), "service:auth".into(), false),
                ("SIGNER".into(), "service:signer".into(), false),
                ("LOADER".into(), "loader:".into(), false),
                ("ADDRESS".into(), self.address.clone(), false),
                ("NAME".into(), self.name.clone(), false),
                ("NODE_KEY".into(), self.serverless_key.clone(), true),
                ("RELAY".into(), "service:relay:ServerlessNode".into(), false),
            ];
            out.extend(location);
            return out;
        }
        // The URL signer holds only the bucket's credentials, and asks the relay who's allowed.
        if service == "signer" {
            let mut out = vec![("RELAY".into(), "service:relay".into(), false)];
            if let Some(bucket) = &self.bucket {
                out.push(("BUCKET".into(), bucket.to_string(), true));
                let endpoint = bucket["endpoint"].as_str().unwrap_or("");
                if endpoint.split("://").nth(1).unwrap_or(endpoint).trim_end_matches('/').ends_with(".r2.cloudflarestorage.com") {
                    out.push(("R2".into(), format!("r2:{}", bucket["bucket"].as_str().unwrap_or("")), false));
                }
            }
            return out;
        }
        let issuer = format!("{}/api/auth", self.address);
        let mut out = vec![];
        let mut plain = |name: &str, value: String| out.push((name.into(), value, false));
        match service {
            "auth" => {
                // Whether a request's signer is one of the install's endpoints, and an admin's.
                plain("RELAY", "service:relay".into());
                plain("ADDRESS", self.address.clone());
                plain("NAME", self.name.clone());
                plain("YOURS", crate::config::YOURS.into());
                // Its own apps' sign-ins come back to `<cli>://signed-in/`.
                plain("CLI", crate::config::CLI.into());
                if let Some((id, name)) = &self.try_person {
                    plain("SIGN_IN", "try".into());
                    plain("TRY_USER", id.clone());
                    plain("TRY_NAME", name.clone());
                } else if google() {
                    plain("SIGN_IN", "google".into());
                    plain("JOIN_DOMAIN", crate::config::JOIN_DOMAIN.into());
                }
                // Tokens are for the install's address unless a client asks for a node's own.
                let resources: Vec<&String> = std::iter::once(&self.address).chain(&self.resources).collect();
                plain("RESOURCES", json!(resources).to_string());
            }
            _ => {
                plain("AUTH", "service:auth".into());
                plain("ISSUER", issuer);
                if self.try_person.is_some() {
                    plain("SIGN_IN", "try".into());
                }
            }
        }
        if service == "auth" {
            out.push(("AUTH_SECRET".into(), self.auth_secret.clone(), true));
            if let Some((id, secret)) = self.google.clone().filter(|_| google()) {
                out.push(("GOOGLE_CLIENT_ID".into(), id, false));
                out.push(("GOOGLE_CLIENT_SECRET".into(), secret, true));
            }
        }
        out
    }
}

/// The kernel's services as one workerd runs them, with `modules` in `SERVICES`' order, and the gateway serving the
/// web client's files from `web`; and the serverless node, if the install has one and this build carries it.
pub fn served(settings: &Settings, modules: Vec<Vec<u8>>, web: &Path) -> Vec<Served> {
    served_with(settings, modules, web, crate::serverless::modules().ok())
}

/// `served`, with the serverless node's build `serverless` (this build's, or a test's).
pub fn served_with(settings: &Settings, modules: Vec<Vec<u8>>, web: &Path, serverless: Option<Modules>) -> Vec<Served> {
    let node = serverless.clone().filter(|_| settings.serverless).map(|modules| crate::serverless::served(settings, modules));
    // A build without the serverless node's WebAssembly has no MCP Worker, and nothing binds one.
    let mcp = serverless.is_some();
    SERVICES
        .iter()
        .zip(modules)
        .filter_map(|((name, entry, class), module)| Some((name, class, module, modules_of(name, entry, serverless.as_ref())?)))
        .map(|(name, class, module, modules)| Served {
            name: name.to_string(),
            module,
            modules,
            class: class.map(str::to_string),
            listen: settings.place(name).map(|(_, port)| format!("127.0.0.1:{port}")),
            bindings: settings.bindings(name).into_iter().filter(|(_, v, _)| mcp || !v.starts_with("service:mcp") && !v.starts_with("object:mcp:")).map(|(n, v, _)| (n, v)).collect(),
            assets: (*name == "gateway").then(|| web.to_path_buf()),
            // A try install's URL signer, and the MCP Worker's devices reading by its URLs, reach its bucket on this machine.
            outbound: (matches!(*name, "signer" | "mcp") && settings.try_person.is_some()).then(|| NEARBY.to_string()),
        })
        .chain(node)
        .collect()
}

/// The client's build, as the web client fetches it (`/client/<version>/`): what its files hash to, so a browser
/// keeps one for good. None if this build has no client (`update` builds it, where Rust builds for WebAssembly).
pub fn client_version() -> Option<String> {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    for (name, bytes) in crate::config::CLIENT {
        hash.update(name.as_bytes());
        hash.update(bytes);
    }
    (!crate::config::CLIENT.is_empty()).then(|| data_encoding::HEXLOWER.encode(&hash.finalize()[..8]))
}

/// The Android app's package, as android/app/build.gradle.kts names it.
pub fn android_package() -> String {
    format!("app.{}.android", crate::config::CLI)
}

/// A script of the web client, its imports of other scripts of it written as their names by content (`/device.js?v=…`),
/// and the hash it's named by (the start of its SHA-256), which those are in turn: so a change to one script renames
/// each that imports it. Memoised in `done`; `path`s in `stack` are being named, so an import cycle (there is none) keeps its
/// plain path.
fn name_script(path: &str, files: &[(String, Vec<u8>)], scripts: &[String], done: &mut std::collections::HashMap<String, (Vec<u8>, String)>, stack: &mut Vec<String>) -> Option<String> {
    use sha2::Digest;
    if let Some((_, v)) = done.get(path) {
        return Some(v.clone());
    }
    if stack.iter().any(|p| p == path) {
        return None;
    }
    stack.push(path.to_string());
    let text = String::from_utf8_lossy(&files.iter().find(|(p, _)| p == path)?.1).into_owned();
    let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut out = String::new();
    let mut at = 0;
    // Each quoted `./x.js`, `../x.js` or `/x.js` that is another script of the client.
    for (i, _) in text.match_indices('"') {
        if i < at {
            continue;
        }
        let end = text[i + 1..].find(|c: char| !(c.is_ascii_alphanumeric() || "./_-".contains(c))).map(|n| i + 1 + n);
        let Some(end) = end.filter(|&e| text[e..].starts_with('"')) else { continue };
        let spec = &text[i + 1..end];
        if !spec.ends_with(".js") || !(spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/')) {
            continue;
        }
        // Resolved as the browser does, from this script's place: `..` at the top stays there.
        let mut parts: Vec<&str> = if spec.starts_with('/') { vec![] } else { dir.split('/').filter(|p| !p.is_empty()).collect() };
        for part in spec.split('/') {
            match part {
                "" | "." => {}
                ".." => drop(parts.pop()),
                part => parts.push(part),
            }
        }
        let target = parts.join("/");
        if target == path || !scripts.contains(&target) {
            continue;
        }
        if let Some(v) = name_script(&target, files, scripts, done, stack) {
            out.push_str(&text[at..i]);
            out.push_str(&format!("\"/{target}?v={v}\""));
            at = end + 1;
        }
    }
    out.push_str(&text[at..]);
    stack.pop();
    let v = data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(out.as_bytes()))[..16].to_string();
    done.insert(path.to_string(), (out.into_bytes(), v.clone()));
    Some(v)
}

impl Settings {
    /// The files the address serves itself: the web client's, as this release has them, Android's App Links file
    /// (`.well-known/assetlinks.json`) and Windows' web links file (`.well-known/windows-app-web-link`), which let the
    /// apps open links to the address, and the guide to connecting an AI (`llms.txt`).
    pub fn web(&self) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> =
            crate::config::KERNEL.iter().filter_map(|(path, bytes)| Some((path.strip_prefix("web/")?.to_string(), bytes.to_vec()))).collect();
        // The page says whose it is before anyone signs in: the white label's name.
        let attr = |v: &str| v.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
        files.push(("icons.json".into(), crate::screen::ICONS.to_vec()));
        // The client's own screens, which it shows from here and never from a node (`screen::client_template`).
        files.extend(crate::screen::client_screens().map(|name| (format!("screens/{name}.html"), crate::screen::client_template(name).into_bytes())));
        // Each file the page loads is named by its content (`?v=`, which the gateway checks), so a browser keeps it for
        // good and asks again only once it changes: those it names itself, and those web.js asks for (`one-files`).
        let named = |path: &str, bytes: &[u8]| {
            use sha2::Digest;
            format!("/{path}?v={}", &data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(bytes))[..16])
        };
        // A script names the scripts it imports by their content too (each after its own imports), so a reopen fetches
        // none of them: web.js's, and the worker's that reaches nodes. What it's named by is what it is with them.
        let scripts: Vec<String> = files.iter().map(|(path, _)| path.clone()).filter(|path| path.ends_with(".js") && !["sw.js", "start.js"].contains(&path.as_str())).collect();
        let mut done: std::collections::HashMap<String, (Vec<u8>, String)> = Default::default();
        for path in &scripts {
            name_script(path, &files, &scripts, &mut done, &mut vec![]);
        }
        for (path, bytes) in files.iter_mut() {
            if let Some((rewritten, _)) = done.remove(path.as_str()) {
                *bytes = rewritten;
            }
        }
        let names: Vec<(String, String)> = files.iter().filter(|(path, _)| !["index.html", "sw.js"].contains(&path.as_str())).map(|(path, bytes)| (path.clone(), named(path, bytes))).collect();
        let start = files.iter().find(|(path, _)| path == "start.js").map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned());
        if let Some((_, page)) = files.iter_mut().find(|(path, _)| path == "index.html") {
            let mut text = String::from_utf8_lossy(page).into_owned();
            // start.js runs inline, as the page arrives, before its styles and scripts are in: its hash is in the
            // page's policy, so it's the one inline script that may.
            if let Some(start) = start {
                use sha2::Digest;
                let hash = data_encoding::BASE64.encode(&sha2::Sha256::digest(start.as_bytes()));
                text = text.replacen("<script src=\"/start.js\"></script>", &format!("<script>{start}</script>"), 1).replacen("script-src 'self'", &format!("script-src 'self' 'sha256-{hash}'"), 1);
            }
            for (path, name) in &names {
                text = text.replace(&format!("=\"/{path}\""), &format!("=\"{name}\""));
            }
            // The scripts web.js imports are asked for beside it, not a round trip after each, on a first visit.
            let preload: String = names.iter().filter(|(path, _)| scripts.contains(path) && !["web.js", "net-worker.js"].contains(&path.as_str())).map(|(_, name)| format!("<link rel=\"modulepreload\" href=\"{name}\">\n")).collect();
            text = text.replacen("<script type=\"module\"", &format!("{preload}<script type=\"module\""), 1);
            let asked = names.iter().filter(|(path, _)| ["frame.html", "icons.json", "net-worker.js"].contains(&path.as_str()) || path.starts_with("screens/")).map(|(path, name)| format!("{path}={}", &name[name.len() - 16..])).collect::<Vec<_>>().join(" ");
            let label = format!(
                "<title>{}</title>\n<meta name=\"application-name\" content=\"{}\">\n<meta name=\"one-files\" content=\"{asked}\">",
                attr(crate::config::NAME),
                attr(crate::config::NAME)
            );
            *page = text.replacen("<title></title>", &label, 1).into_bytes();
        }
        // The client that reaches nodes over iroh, at a path named by its build, which the page says (net.js).
        if let Some(version) = client_version() {
            for (name, bytes) in crate::config::CLIENT {
                files.push((format!("client/{version}/{name}"), bytes.to_vec()));
            }
            if let Some((_, page)) = files.iter_mut().find(|(path, _)| path == "index.html") {
                *page = String::from_utf8_lossy(page).replacen(r#"<meta name="one-client" content="">"#, &format!(r#"<meta name="one-client" content="{version}">"#), 1).into_bytes();
            }
        }
        if !self.android.is_empty() {
            let links = json!([{
                "relation": ["delegate_permission/common.handle_all_urls"],
                "target": { "namespace": "android_app", "package_name": android_package(), "sha256_cert_fingerprints": self.android },
            }]);
            files.push((".well-known/assetlinks.json".into(), links.to_string().into_bytes()));
        }
        files.push((".well-known/windows-app-web-link".into(), windows_links()));
        // How an AI connects itself, for anyone who has the address: never what apps the install has.
        files.push(("llms.txt".into(), crate::skills::guide(Some(&self.address)).into_bytes()));
        files
    }
}

/// The Windows app's package family name, as Windows names the package windows/build.sh makes: the config's name in
/// letters and digits, then its publisher's (`CN=` and the name in letters, digits and spaces) hash, the first 8 bytes
/// of the SHA-256 of it in UTF-16LE, in Crockford's base32.
pub fn windows_package() -> String {
    use sha2::Digest;
    let name = crate::config::NAME;
    let publisher = format!("CN={}", name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect::<String>());
    let hash = sha2::Sha256::digest(publisher.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>());
    let bits = u128::from(u64::from_be_bytes(hash[..8].try_into().unwrap())) << 1;
    let id: String = (0..13).map(|i| b"0123456789abcdefghjkmnpqrstvwxyz"[(bits >> ((12 - i) * 5)) as usize & 31] as char).collect();
    format!("{}_{id}", name.chars().filter(char::is_ascii_alphanumeric).collect::<String>())
}

/// `/.well-known/windows-app-web-link`: the pages at the address the Windows app opens, when a link to one is opened
/// from another app, as the web client has them (kernel/web/web.js): You, a page, its Services, an app's screen,
/// Settings, a Vault request and an invite.
fn windows_links() -> Vec<u8> {
    let paths = ["/", "/page/*", "/services/*", "/app/*", "/settings", "/settings/*", "/vault", "/invite"];
    json!([{ "packageFamilyName": windows_package(), "paths": paths }]).to_string().into_bytes()
}

/// Runs the kernel's services on this machine, in the pinned workerd, until it stops: their modules built from this
/// release, the web client's files in `<home>/kernel/web`, and their storage in `<home>/kernel/data`.
pub fn serve(settings: &Settings) -> Result<()> {
    let dir = Settings::path().parent().unwrap().to_path_buf();
    let modules = SERVICES.iter().map(|(_, entry, _)| module(entry)).collect::<Result<Vec<_>>>()?;
    let web = dir.join("web");
    let served = served(settings, modules, &web);
    let here = |a: &str| a.strip_prefix("127.0.0.1:").or_else(|| a.strip_prefix("http://127.0.0.1:")).is_some_and(|port| port.parse::<u16>().is_ok());
    ensure!(settings.try_person.is_none() || here(&settings.address) && served.iter().all(|s| s.listen.as_deref().is_none_or(here)), "a try install's kernel signs in anyone who reaches it, so it runs only beside its try install, at 127.0.0.1");
    let _ = std::fs::remove_dir_all(&web);
    let mut files = settings.web();
    parses(&files)?;
    files.extend(precompressed(&files)?);
    for (file, bytes) in files {
        let path = web.join(file);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes)?;
    }
    let config = write_config(&dir, &served)?;
    let mut child = start(&config, Vec::new())?;
    let status = child.wait()?;
    bail!("the kernel's workerd stopped: {status}")
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
    use sha2::Digest;
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
            let out = std::process::Command::new(crate::deploy::bun()?).args(["brotli.mjs"]).current_dir(&dir).env("DO_NOT_TRACK", "1").output()?;
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
                if !tag.contains("src=") {
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
        let out = std::process::Command::new(crate::deploy::bun()?)
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

/// A kernel service's module, built from the sources this release was signed with by the pinned Bun, once per
/// version of them.
pub fn module(entry: &str) -> Result<Vec<u8>> {
    module_from(crate::config::KERNEL.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect(), entry)
}

/// A kernel service's module, built from `files` (paths in `kernel/`).
pub fn module_from(files: Vec<(String, Vec<u8>)>, entry: &str) -> Result<Vec<u8>> {
    let mut hash = Sha256::new();
    for (path, bytes) in &files {
        hash.update(format!("{path}\0{}\0", bytes.len()));
        hash.update(bytes);
    }
    let hash = data_encoding::HEXLOWER.encode(&hash.finalize());
    let kept = crate::cache().join("kernel").join(format!("{}-{}.js", &hash[..32], entry.replace('/', "-")));
    if let Ok(module) = std::fs::read(&kept) {
        return Ok(module);
    }
    let dir = std::env::temp_dir().join(format!("{}-kernel-{}", crate::config::CLI, rand::random::<u64>()));
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

/// Builds `entry` in the kernel folder `dir` into one ES module.
pub fn build(dir: &Path, entry: &str) -> Result<Vec<u8>> {
    let bun = crate::deploy::bun()?;
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
    run(&["build", entry, "--target=browser", "--format=esm", "--external=cloudflare:*", "--external=node:*", "--external=*.wasm", "--external=*/one_serverless.js", &format!("--outfile={}", out.display())])?;
    Ok(std::fs::read(out)?)
}

/// A kernel service as one workerd runs it: its name, module, Durable Object class, the address it listens on, and its
/// bindings: (name, a text, or `service:<name>` for another service).
pub struct Served {
    pub name: String,
    pub module: Vec<u8>,
    /// Its WebAssembly modules, (name, bytes).
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
/// readable only by its owner, since bindings hold the kernel's secrets.
pub fn write_config(dir: &Path, served: &[Served]) -> Result<PathBuf> {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"));
    let mut services = Vec::new();
    if served.iter().any(|s| s.outbound.as_deref() == Some(NEARBY)) {
        services.push(format!("(name = {}, network = (allow = [\"public\", \"private\", \"local\"]))", quote(NEARBY)));
    }
    let mut sockets = Vec::new();
    let mut workers = Vec::new();
    for (i, s) in served.iter().enumerate() {
        ensure!(s.name.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "a kernel service's name is lowercase");
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
            // An R2 bucket is bound only on Cloudflare; here the URL signer reaches it by URL.
            if value.starts_with("r2:") {
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
            crate::apps::COMPATIBILITY_DATE,
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
    let mut command = std::process::Command::new(crate::apps::workerd()?);
    command.arg("serve").arg(config).arg("--experimental").stdin(std::process::Stdio::null());
    #[cfg(not(unix))]
    ensure!(listening.is_empty(), "workerd takes listening sockets only on Unix");
    crate::apps::child(&mut command, &listening.iter().map(|(n, l)| (n.as_str(), l)).collect::<Vec<_>>());
    Ok(command.spawn()?)
}

// Cloudflare.

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

    /// The versions a Worker's active deployment runs, as one string, if it has one: without hyphens, as `upload`
    /// gives them.
    pub(crate) fn active(&self, name: &str) -> Result<Option<String>> {
        let list = self.call("GET", &format!("/accounts/{}/workers/scripts/{name}/deployments", self.account), None)?;
        let versions = list["deployments"][0]["versions"].as_array().into_iter().flatten().filter_map(|v| Some(v["version_id"].as_str()?.replace('-', ""))).collect::<Vec<_>>();
        Ok((!versions.is_empty()).then(|| versions.join(",")))
    }

    /// Uploads a Worker: its modules (name, type, bytes), the first its main one, and its bindings and Durable Object
    /// class: → the version it deployed, if Cloudflare says.
    pub(crate) fn upload(&self, name: &str, modules: &[(&str, &str, &[u8])], metadata: &Value) -> Result<Option<String>> {
        let boundary = format!("kernel-{}", data_encoding::HEXLOWER.encode(&rand::random::<[u8; 12]>()));
        let mut body = Vec::new();
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{metadata}\r\n").as_bytes());
        for (module, kind, bytes) in modules {
            body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{module}\"; filename=\"{module}\"\r\nContent-Type: {kind}\r\n\r\n").as_bytes());
            body.extend(*bytes);
            body.extend(b"\r\n");
        }
        body.extend(format!("--{boundary}--\r\n").as_bytes());
        let done = self.call("PUT", &format!("/accounts/{}/workers/scripts/{name}", self.account), Some((format!("multipart/form-data; boundary={boundary}"), body)))?;
        Ok(done["deployment_id"].as_str().map(str::to_string))
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

/// An invite link: opening it lets its person sign up, once, as who it says.
pub struct Invite {
    pub link: String,
    /// The id they'll have in the install.
    pub user: String,
    /// Until when it works, in milliseconds since 1970.
    pub until: u64,
}

/// How long an invite works.
pub const INVITE_DAYS: u64 = 3;

/// Invites someone to join the install at `address`, `name` only a label, as `user` (who they'll be in the install; a
/// new id if not given), and where people sign in with Google, as the Google account `email`. The link's secret is
/// made here and never leaves but in the link: the auth service is told only its hash, in a request signed with
/// `key`, an admin's device's.
pub fn invite(address: &str, key: &ed25519_dalek::SigningKey, name: &str, user: Option<&str>, email: Option<&str>) -> Result<Invite> {
    ensure!(!name.trim().is_empty(), "give their name");
    ensure!(!google() || email.is_some(), "people here sign in with Google: --email says which account they sign up with");
    let user = user.map(str::to_string).unwrap_or_else(crate::entry::new_id);
    let secret = B64U.encode(&rand::random::<[u8; 32]>());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
    let until = now + INVITE_DAYS * 86_400_000;
    let mut fields = json!({ "route": "POST /api/auth/invites", "id": data_encoding::HEXLOWER.encode(&crate::keys::sha256(secret.as_bytes())), "name": name.trim(), "user": user, "until": until });
    if let Some(email) = email {
        fields["email"] = json!(email);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
    let mut resp = agent.post(format!("{address}/api/auth/invites")).send_json(crate::keys::signed_request(key, "one/auth", fields, now))?;
    let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    ensure!(resp.status() == 200, "{}", out["error"].as_str().unwrap_or("the auth service made no invite"));
    Ok(Invite { link: format!("{address}/invite#{secret}"), user, until })
}

/// A web file's type, by its extension.
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

/// A kernel service's Worker's name on Cloudflare: `<cli>-<service>`, so white labels sharing an account don't clash.
pub fn worker_name(service: &str) -> String {
    format!("{}-{service}", crate::config::CLI)
}

/// What a Worker's upload says besides its module: its bindings (another service's by its Worker's name), its Durable
/// Object class, and that class's migration the first time, or the migration that deletes a class it had (`migration`:
/// the tag of the one the Worker has had).
pub fn metadata(settings: &Settings, service: &str, class: Option<&str>, migration: Option<&str>, exists: impl Fn(&str) -> bool) -> (Value, bool) {
    let mut complete = true;
    let mut bindings: Vec<Value> = class.iter().map(|class| json!({ "type": "durable_object_namespace", "name": class.to_uppercase(), "class_name": class })).collect();
    for (name, value, secret) in settings.bindings(service) {
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
            // Another Worker's class, once it's there: the relay and the serverless node each bind the other's, so the
            // first of them deployed goes without, and has it when it's deployed again.
            None if object.is_some() => {
                if let Some((other, class)) = object.filter(|(other, _)| exists(&worker_name(other))) {
                    bindings.push(json!({ "type": "durable_object_namespace", "name": name, "class_name": class, "script_name": worker_name(other) }));
                }
            }
            None if value == "loader:" => bindings.push(json!({ "type": "worker_loader", "name": name })),
            None => match value.strip_prefix("r2:") {
                Some(bucket) => bindings.push(json!({ "type": "r2_bucket", "name": name, "bucket_name": bucket })),
                None => bindings.push(json!({ "type": if secret { "secret_text" } else { "plain_text" }, "name": name, "text": value })),
            },
        }
    }
    let mut meta = json!({ "main_module": "worker.js", "compatibility_date": crate::apps::COMPATIBILITY_DATE, "bindings": bindings });
    if let Some(class) = class.filter(|_| migration != Some("v1")) {
        meta["migrations"] = json!({ "new_tag": "v1", "new_sqlite_classes": [class] });
    }
    if class.is_none() && migration == Some("v1") {
        let gone: Vec<&str> = DELETED.iter().filter(|(s, _)| *s == service).map(|(_, class)| *class).collect();
        if !gone.is_empty() {
            meta["migrations"] = json!({ "old_tag": "v1", "new_tag": "v2", "deleted_classes": gone });
        }
    }
    (meta, complete)
}

/// Deploys the kernel's services to Cloudflare, each a Worker with its Durable Object, built from this release, the
/// gateway with the web client's files (`web`) as its static assets, which it serves itself. A service whose binding
/// names one not deployed yet goes up without it, and again once that one is.
pub fn deploy(cf: &Cloudflare, settings: &Settings, modules: &[Vec<u8>], web: &[(String, Vec<u8>)]) -> Result<()> {
    deploy_some(cf, settings, modules, web, &SERVICES.iter().map(|(s, _, _)| *s).collect::<Vec<_>>())
}

/// `deploy`, of only the services named.
pub fn deploy_some(cf: &Cloudflare, settings: &Settings, modules: &[Vec<u8>], web: &[(String, Vec<u8>)], services: &[&str]) -> Result<()> {
    ensure!(settings.try_person.is_none(), "a try install's kernel signs in anyone who reaches it, so it runs only beside its try install");
    if services.contains(&"gateway") {
        parses(web)?;
    }
    let mut left: Vec<usize> = (0..SERVICES.len()).filter(|i| services.contains(&SERVICES[*i].0)).collect();
    let mut scripts = cf.scripts()?;
    let uploaded = Uploaded::load();
    let mut sent = Vec::new();
    for _ in 0..2 {
        let mut again = Vec::new();
        for i in left {
            let (service, _, class) = SERVICES[i];
            let name = worker_name(service);
            let migration = scripts.get(&name).and_then(|s| s.migration.clone());
            let (mut meta, complete) = metadata(settings, service, class, migration.as_deref(), |other| scripts.contains_key(other) || other == name);
            let url = settings.place(service).map(|(url, _)| url);
            let Some(wasm) = modules_of(service, SERVICES[i].1, crate::serverless::modules().ok().as_ref()) else { bail!("this build has no serverless node, which the {service} service runs: {} update builds one", crate::config::CLI) };
            let mut what = vec![meta.to_string().into_bytes(), modules[i].clone(), url.unwrap_or("").as_bytes().to_vec()];
            what.extend(wasm.iter().flat_map(|(name, bytes)| [name.as_bytes().to_vec(), bytes.clone()]));
            if service == "gateway" {
                what.extend(web.iter().flat_map(|(path, bytes)| [path.as_bytes().to_vec(), bytes.clone()]));
            }
            let what = Uploaded::hash(&what);
            // Its address may have been changed by hand; one with none is reached only by binding.
            let reach = || match url {
                Some(url) => cf.reach(&name, url),
                None => cf.hide(&name),
            };
            if complete && uploaded.same(cf, &name, &what) {
                reach()?;
                continue;
            }
            if service == "gateway" {
                let jwt = cf.assets(&name, web)?;
                meta["assets"] = json!({ "jwt": jwt, "config": { "html_handling": "none", "not_found_handling": "none", "run_worker_first": true } });
                meta["bindings"].as_array_mut().unwrap().push(json!({ "type": "assets", "name": "ASSETS" }));
            }
            let mut parts: Vec<(&str, &str, &[u8])> = vec![("worker.js", "application/javascript+module", &modules[i])];
            parts.extend(wasm.iter().map(|(name, bytes)| (name.as_str(), if name.ends_with(".js") { "application/javascript+module" } else { "application/wasm" }, bytes.as_slice())));
            let version = cf.upload(&name, &parts, &meta)?;
            reach()?;
            // Later services' bindings to it resolve in this pass, and it has had its migration.
            let migration = meta["migrations"]["new_tag"].as_str().map(str::to_string).or(migration);
            scripts.insert(name.clone(), Script { migration });
            if complete {
                sent.push((name, what, version));
            } else {
                again.push(i);
            }
        }
        left = again;
        if left.is_empty() {
            return uploaded.record(sent);
        }
    }
    bail!("some services' bindings never resolved")
}

/// A Worker of the account's, as its list says.
pub struct Script {
    /// The Durable Object migration it has had.
    pub migration: Option<String>,
}

/// What this machine last uploaded to each of the kernel's Workers: a hash of everything it sent (the secrets in its
/// bindings too), and the version Cloudflare made of it. A Worker that would get the same, and still runs that
/// version (no other upload, edit or rollback since), isn't uploaded again: an update deploys the kernel each time the node changes, and most of
/// its changes leave every Worker as it was.
pub(crate) struct Uploaded(serde_json::Map<String, Value>);

impl Uploaded {
    fn path() -> PathBuf {
        Settings::path().with_file_name("uploaded.json")
    }

    pub(crate) fn load() -> Uploaded {
        let read = std::fs::read(Uploaded::path()).ok().and_then(|b| serde_json::from_slice(&b).ok());
        Uploaded(read.unwrap_or_default())
    }

    pub(crate) fn hash(parts: &[Vec<u8>]) -> String {
        let mut hash = Sha256::new();
        for part in parts {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part);
        }
        data_encoding::HEXLOWER.encode(&hash.finalize())
    }

    /// Whether `name` runs `what`, as this machine uploaded it, and nothing has deployed it since.
    pub(crate) fn same(&self, cf: &Cloudflare, name: &str, what: &str) -> bool {
        let Some(was) = self.0.get(name).filter(|was| was["hash"] == what) else { return false };
        cf.active(name).ok().flatten().is_some_and(|active| was["version"].as_str() == Some(active.as_str()))
    }

    /// Keeps what was uploaded to each of `sent` (name, hash, the version Cloudflare made of it).
    pub(crate) fn record(mut self, sent: Vec<(String, String, Option<String>)>) -> Result<()> {
        if sent.is_empty() {
            return Ok(());
        }
        for (name, what, version) in sent {
            match version {
                Some(version) => self.0.insert(name, json!({ "hash": what, "version": version })),
                None => self.0.remove(&name),
            };
        }
        std::fs::write(Uploaded::path(), Value::Object(self.0).to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_settings_are_made_only_where_no_kernels_secrets_are() {
        let dir = std::env::temp_dir().join(format!("kernel-settings-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(none_in(&dir).is_ok());
        std::fs::write(dir.join(".env.local"), "AUTH_SECRET='an earlier kernel's secret'\n").unwrap();
        let refused = format!("{:#}", none_in(&dir).unwrap_err());
        assert!(refused.contains(".env.local") && !refused.contains("an earlier kernel's secret"), "{refused}");
        std::fs::remove_file(dir.join(".env.local")).unwrap();
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        assert!(none_in(&dir).is_err(), "settings that didn't load aren't replaced");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_gateways_old_relay_class_is_deleted_once_and_no_other_service_loses_one() {
        let settings = Settings::new("Test", "https://one.example");
        let migrations = |service: &str, class: Option<&str>, had: Option<&str>| metadata(&settings, service, class, had, |_| true).0["migrations"].clone();
        assert_eq!(migrations("gateway", None, Some("v1")), json!({ "old_tag": "v1", "new_tag": "v2", "deleted_classes": ["Relay"] }));
        assert!(migrations("gateway", None, Some("v2")).is_null() && migrations("gateway", None, None).is_null(), "it's deleted once");
        assert!(migrations("signer", None, Some("v1")).is_null(), "no other service's classes are touched");
        assert_eq!(migrations("auth", Some("Auth"), None), json!({ "new_tag": "v1", "new_sqlite_classes": ["Auth"] }));
        assert!(migrations("auth", Some("Auth"), Some("v1")).is_null());
    }
}
