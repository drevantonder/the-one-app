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
    ("vault", "vault/worker.js", Some("Vault")),
    ("schedules", "schedules/worker.js", Some("Schedules")),
    ("relay", "relay/worker.js", Some("Relay")),
    ("signer", "signer/worker.js", None),
    ("gateway", "gateway/worker.js", Some("Relay")),
];

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

/// How an install's kernel is set up, wherever it runs: kept in `<home>/kernel/settings.json`, with its secrets in
/// `.env.local` beside it, which Varlock reads (see `SCHEMA`): their values, for a kernel tried here, or
/// `op(op://…)` references to 1Password, for one people use, so no file here holds them.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct Settings {
    /// The install's name, as sign-in screens say it.
    pub name: String,
    /// The install's address, the one people use: the gateway's, which sends sign-in to the auth service, `/v1/` to a
    /// node and the rest to the web client. Passkeys belong to its host name, so it doesn't change.
    pub address: String,
    /// Where the kernel's own tools reach the auth service; people reach it at the address.
    pub auth_url: String,
    /// Where nodes reach the vault and the schedules service.
    pub vault_url: String,
    pub schedules_url: String,
    /// Where the gateway sends `/v1/` and `/mcp`: a node's address, or `cloudflare:<id>` for a node behind a Cloudflare
    /// Tunnel with no public address, which the gateway on Cloudflare reaches by that Workers VPC service.
    #[serde(default)]
    pub node_url: Option<String>,
    /// Whether the gateway sends `/v1/` and `/mcp` to the install's serverless node instead (`serverless deploy`),
    /// which sends on to `node_url` what needs a machine.
    #[serde(default)]
    pub serverless: bool,
    /// The local ports the auth service, the vault, the schedules service and the gateway listen on, when this machine
    /// runs them. The relay and the URL signer have none: other Workers reach them by binding.
    pub ports: [u16; 4],
    /// The install's devices' public keys (hex), from the node on this machine when the kernel is deployed or served:
    /// the relay starts its list with them, as its admins', when it has none yet.
    #[serde(skip)]
    pub devices: Vec<String>,
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
        let ports = [port + 10, port + 11, port + 12, port + 13];
        let secret = || B64U.encode(&rand::random::<[u8; 32]>());
        Settings {
            name: name.into(),
            address: address.trim_end_matches('/').into(),
            auth_url: format!("http://127.0.0.1:{}", ports[0]),
            vault_url: format!("http://127.0.0.1:{}", ports[1]),
            schedules_url: format!("http://127.0.0.1:{}", ports[2]),
            node_url: None,
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
    /// lets each device it borrows reach at the same address; its node `node`, its bucket where that node serves its
    /// data (`bucket`), and `person` signed in with nothing to show.
    pub fn for_try(node: &str, bucket: &str, person: (String, String)) -> Result<Settings> {
        let free = || -> Result<u16> { Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port()) };
        let ports = [free()?, free()?, free()?, free()?];
        let mut settings = Settings::new("Try", &format!("http://127.0.0.1:{}", ports[3]));
        (settings.auth_url, settings.vault_url, settings.schedules_url) = (format!("http://127.0.0.1:{}", ports[0]), format!("http://127.0.0.1:{}", ports[1]), format!("http://127.0.0.1:{}", ports[2]));
        settings.ports = ports;
        settings.node_url = Some(format!("http://{node}"));
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

    /// Sets a secret's value in `.env.local`, unless it's a reference to 1Password, where it changes instead.
    pub fn set_secret(&self, name: &str, value: &str) -> Result<()> {
        let text = std::fs::read_to_string(local()).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with(&format!("{name}="))).map(str::to_string).collect();
        if let Some(line) = text.lines().find(|l| l.starts_with(&format!("{name}="))) {
            ensure!(!line.contains("op("), "{name} is kept in 1Password: change it there ({})", line.trim_start_matches(&format!("{name}=")));
        }
        lines.push(format!("{name}={}", quoted(value)));
        write_private(&local(), &(lines.join("\n") + "\n"))
    }

    /// How a Worker reaches the node on a machine, `node_url`, as the binding `name`: a Workers VPC service, or else
    /// a URL (`<name>_URL`).
    pub fn machine(&self, name: &str) -> (String, String, bool) {
        let node = self.node_url.clone().unwrap_or_default();
        match node.strip_prefix("cloudflare:") {
            Some(id) => (name.into(), format!("vpc:{id}"), false),
            None => (format!("{name}_URL"), node, false),
        }
    }

    /// The serverless node's endpoint (its public key, hex), once it has a key.
    pub fn serverless_endpoint(&self) -> Option<String> {
        let seed: [u8; 32] = data_encoding::HEXLOWER.decode(self.serverless_key.as_bytes()).ok()?.try_into().ok()?;
        Some(data_encoding::HEXLOWER.encode(ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().as_bytes()))
    }

    /// Where a service is reached, and the local port it listens on: none for the relay and the URL signer, which only
    /// other Workers' bindings reach.
    pub fn place(&self, service: &str) -> Option<(&str, u16)> {
        let i = ["auth", "vault", "schedules", "gateway"].iter().position(|s| *s == service)?;
        Some(([&self.auth_url, &self.vault_url, &self.schedules_url, &self.address][i].as_str(), self.ports[i]))
    }

    /// Each service's bindings: (name, value, whether it's secret). A value `service:<name>` is another service,
    /// `object:<name>:<class>` another service's Durable Object class, `loader:` Worker Loader, `vpc:<id>` a Cloudflare
    /// Workers VPC service, and `r2:<bucket>` an R2 bucket of the account's.
    pub fn bindings(&self, service: &str) -> Vec<(String, String, bool)> {
        // The gateway holds no secrets.
        if service == "gateway" {
            let node = match self.serverless {
                true => ("NODE".into(), format!("service:{}", crate::serverless::SERVICE), false),
                false => self.machine("NODE"),
            };
            return vec![("AUTH".into(), "service:auth".into(), false), ("IROH".into(), "service:relay".into(), false), ("SIGNER".into(), "service:signer".into(), false), node];
        }
        let location = (!crate::config::RELAY_LOCATION.is_empty()).then(|| ("LOCATION".into(), crate::config::RELAY_LOCATION.into(), false));
        // Nor does the relay: only the keys of the install's first devices, which it starts its list with, and the
        // serverless node's, whose datagrams it hands its Durable Object.
        if service == "relay" {
            let devices = (!self.devices.is_empty()).then(|| ("DEVICES".into(), json!(self.devices).to_string(), false));
            let bound = self.serverless_endpoint().filter(|_| self.serverless).map(|key| [("BOUND".into(), key, false), ("NODE".into(), format!("object:{}:Node", crate::serverless::SERVICE), false)]);
            return devices.into_iter().chain(bound.into_iter().flatten()).chain(location).collect();
        }
        // The serverless node: its endpoint's key, the relay it's reached through, and the node on a machine the rest
        // of what needs one goes on to.
        if service == crate::serverless::SERVICE {
            let mut out = vec![
                ("AUTH".into(), "service:auth".into(), false),
                ("VAULT".into(), "service:vault".into(), false),
                ("LOADER".into(), "loader:".into(), false),
                ("ADDRESS".into(), self.address.clone(), false),
                ("NAME".into(), self.name.clone(), false),
                ("NODE_KEY".into(), self.serverless_key.clone(), true),
                ("RELAY".into(), "object:relay:Relay".into(), false),
            ];
            out.extend(location);
            if self.node_url.is_some() {
                out.push(self.machine("MACHINE"));
            }
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
                plain("ACCENT", crate::config::ACCENT.into());
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
    let node = crate::serverless::modules().ok().filter(|_| settings.serverless).map(|modules| crate::serverless::served(settings, modules));
    SERVICES
        .iter()
        .zip(modules)
        .map(|((name, entry, class), module)| Served {
            name: name.to_string(),
            module,
            modules: wasm(entry),
            class: class.map(str::to_string),
            listen: settings.place(name).map(|(_, port)| format!("127.0.0.1:{port}")),
            bindings: settings.bindings(name).into_iter().map(|(n, v, _)| (n, v)).collect(),
            assets: (*name == "gateway").then(|| web.to_path_buf()),
            // The gateway's node may be on this machine or its private network, as a node behind a tunnel is; a try
            // install's URL signer reaches its bucket on this machine.
            outbound: (*name == "gateway" || *name == "signer" && settings.try_person.is_some()).then(|| NEARBY.to_string()),
        })
        .chain(node)
        .collect()
}

/// The Android app's package, as android/app/build.gradle.kts names it.
pub fn android_package() -> String {
    format!("app.{}.android", crate::config::CLI)
}

impl Settings {
    /// The files the address serves itself: the web client's, as this release has them, and Android's App Links file
    /// (`.well-known/assetlinks.json`) and Windows' web links file (`.well-known/windows-app-web-link`), which let the
    /// apps open links to the address.
    pub fn web(&self) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> =
            crate::config::KERNEL.iter().filter_map(|(path, bytes)| Some((path.strip_prefix("web/")?.to_string(), bytes.to_vec()))).collect();
        // The page says whose it is before anyone signs in: the white label's name and accent.
        let attr = |v: &str| v.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
        if let Some((_, page)) = files.iter_mut().find(|(path, _)| path == "index.html") {
            let label = format!("<title>{}</title>\n<meta name=\"application-name\" content=\"{}\">\n<meta name=\"theme-color\" content=\"{}\">", attr(crate::config::NAME), attr(crate::config::NAME), attr(crate::config::ACCENT));
            *page = String::from_utf8_lossy(page).replacen("<title></title>", &label, 1).into_bytes();
        }
        if !self.android.is_empty() {
            let links = json!([{
                "relation": ["delegate_permission/common.handle_all_urls"],
                "target": { "namespace": "android_app", "package_name": android_package(), "sha256_cert_fingerprints": self.android },
            }]);
            files.push((".well-known/assetlinks.json".into(), links.to_string().into_bytes()));
        }
        files.push((".well-known/windows-app-web-link".into(), windows_links()));
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
    let files = settings.web();
    parses(&files)?;
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
    run(&["build", entry, "--target=browser", "--format=esm", "--external=cloudflare:*", "--external=node:*", "--external=*.wasm", &format!("--outfile={}", out.display())])?;
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
            ensure!(!value.starts_with("vpc:"), "a node behind a Cloudflare Tunnel is reached only by a kernel on Cloudflare: give kernel set node-url a node's address");
            // An R2 bucket is bound only on Cloudflare; here the URL signer reaches it by URL.
            if value.starts_with("r2:") {
                continue;
            }
            let object = value.strip_prefix("object:").and_then(|o| o.split_once(':'));
            bindings.push(match (value.strip_prefix("service:"), object) {
                (Some(service), _) => format!("(name = {}, service = {})", quote(name), quote(service)),
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
/// Object class, and that class's migration the first time.
pub fn metadata(settings: &Settings, service: &str, class: Option<&str>, migrated: bool, exists: impl Fn(&str) -> bool) -> (Value, bool) {
    let mut complete = true;
    let mut bindings: Vec<Value> = class.iter().map(|class| json!({ "type": "durable_object_namespace", "name": class.to_uppercase(), "class_name": class })).collect();
    for (name, value, secret) in settings.bindings(service) {
        let object = value.strip_prefix("object:").and_then(|o| o.split_once(':'));
        match value.strip_prefix("service:") {
            Some(other) if exists(&worker_name(other)) => bindings.push(json!({ "type": "service", "name": name, "service": worker_name(other) })),
            Some(_) => complete = false,
            // Another Worker's class, once it's there: the relay and the serverless node each bind the other's, so the
            // first of them deployed goes without, and has it when it's deployed again.
            None if object.is_some() => {
                if let Some((other, class)) = object.filter(|(other, _)| exists(&worker_name(other))) {
                    bindings.push(json!({ "type": "durable_object_namespace", "name": name, "class_name": class, "script_name": worker_name(other) }));
                }
            }
            None if value == "loader:" => bindings.push(json!({ "type": "worker_loader", "name": name })),
            None => match (value.strip_prefix("vpc:"), value.strip_prefix("r2:")) {
                (Some(id), _) => bindings.push(json!({ "type": "vpc_service", "name": name, "service_id": id })),
                (_, Some(bucket)) => bindings.push(json!({ "type": "r2_bucket", "name": name, "bucket_name": bucket })),
                _ => bindings.push(json!({ "type": if secret { "secret_text" } else { "plain_text" }, "name": name, "text": value })),
            },
        }
    }
    let mut meta = json!({ "main_module": "worker.js", "compatibility_date": crate::apps::COMPATIBILITY_DATE, "bindings": bindings });
    if let Some(class) = class.filter(|_| !migrated) {
        meta["migrations"] = json!({ "new_tag": "v1", "new_sqlite_classes": [class] });
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
            let migrated = scripts.get(&name).is_some_and(|s| s.migration.as_deref() == Some("v1"));
            let (mut meta, complete) = metadata(settings, service, class, migrated, |other| scripts.contains_key(other) || other == name);
            let url = settings.place(service).map(|(url, _)| url);
            let wasm = wasm(SERVICES[i].1);
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
            parts.extend(wasm.iter().map(|(name, bytes)| (name.as_str(), "application/wasm", bytes.as_slice())));
            let version = cf.upload(&name, &parts, &meta)?;
            reach()?;
            // Later services' bindings to it resolve in this pass, and its class has had its migration.
            scripts.insert(name.clone(), Script { migration: class.map(|_| "v1".to_string()) });
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
}
