//! The kernel's services, built with the pinned Bun and run in workerd here, or deployed to Cloudflare.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

// Building and running.

/// The kernel's services: each one's name, its module, and its Durable Object's class, if it keeps anything.
pub const SERVICES: &[(&str, &str, Option<&str>)] = &[
    ("auth", "auth/worker.js", Some("Auth")),
    ("vault", "vault/worker.js", Some("Vault")),
    ("schedules", "schedules/worker.js", Some("Schedules")),
    ("gateway", "gateway/worker.js", None),
];

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
    /// The local ports each service listens on, in `SERVICES`' order, when this machine runs them.
    pub ports: [u16; 4],
    /// The secret only the kernel's services share, for asking each other.
    #[serde(default, skip_serializing)]
    pub kernel_secret: String,
    /// Better Auth's own secret, for its cookies and the keys it keeps.
    #[serde(default, skip_serializing)]
    pub auth_secret: String,
    /// The schedules service's own X25519 key pair.
    #[serde(default, skip_serializing)]
    pub schedules_key: Value,
    /// The recovery key's X25519 twin's public half.
    pub recovery_key: String,
    /// Nodes' own addresses MCP clients sign in for (each a resource, in OAuth's words), besides the install's.
    #[serde(default)]
    pub resources: Vec<String>,
    /// The install's bucket: `{ endpoint, bucket, region, key_id, secret }`.
    #[serde(default, skip_serializing)]
    pub bucket: Option<Value>,
    /// The SHA-256 fingerprints of the certificates the Android app is signed with: Android opens links to the address
    /// in the app (App Links) only if the address lists the app's package and one of them.
    #[serde(default)]
    pub android: Vec<String>,
    /// For an install whose people sign in with Google (`config::SIGN_IN`): the vault's own key, which opens their
    /// keys, and the Google OAuth client they sign in with, `(id, secret)`.
    #[serde(default, skip_serializing)]
    pub vault_key: String,
    #[serde(default, skip_serializing)]
    pub google: Option<(String, String)>,
    /// The API token `kernel deploy` and `serverless deploy` edit the Cloudflare account's Workers with, where
    /// CLOUDFLARE_API_TOKEN isn't set.
    #[serde(default, skip_serializing)]
    pub cloudflare_token: Option<String>,
}

/// Whether people sign in with Google, which the vault then opens their keys for itself.
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
KERNEL_SECRET=
AUTH_SECRET=
SCHEDULES_KEY=
# @optional
BUCKET=
# For an install whose people sign in with Google: the vault's own key, and their Google OAuth client.
# @optional
VAULT_KEY=
# @optional
GOOGLE_CLIENT_ID=
# @optional
GOOGLE_CLIENT_SECRET=
# The API token kernel deploy edits the Cloudflare account's Workers with, where CLOUDFLARE_API_TOKEN isn't set.
# @optional
CLOUDFLARE_API_TOKEN=
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
        // Settings written before Varlock kept the secrets hold them: they move to .env.local.
        if !settings.kernel_secret.is_empty() {
            ensure!(!local().exists(), "{} still holds secrets, and {} is there too: keep one", path.display(), local().display());
            settings.save()?;
            return Ok(settings);
        }
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
        settings.kernel_secret = get("KERNEL_SECRET").context("no KERNEL_SECRET")?;
        settings.auth_secret = get("AUTH_SECRET").context("no AUTH_SECRET")?;
        settings.schedules_key = serde_json::from_str(&get("SCHEDULES_KEY").context("no SCHEDULES_KEY")?).context("SCHEDULES_KEY isn't JSON")?;
        settings.bucket = get("BUCKET").map(|b| serde_json::from_str(&b)).transpose().context("BUCKET isn't JSON")?;
        settings.cloudflare_token = get("CLOUDFLARE_API_TOKEN");
        if google() {
            settings.vault_key = get("VAULT_KEY").context("no VAULT_KEY, which people signing in with Google need")?;
            settings.google = get("GOOGLE_CLIENT_ID").zip(get("GOOGLE_CLIENT_SECRET"));
            ensure!(settings.google.is_some(), "no GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET: the Google OAuth client people sign in with, whose redirect URI is {}/api/auth/callback/google", settings.address);
        }
        Ok(settings)
    }

    /// New settings, with new secrets: → them, and the new recovery key's secret, to be shown once.
    pub fn new(name: &str, address: &str) -> (Settings, String) {
        let port = crate::config::PORT;
        let ports = [port + 10, port + 11, port + 12, port + 13];
        let secret = || B64U.encode(&rand::random::<[u8; 32]>());
        let own: [u8; 32] = rand::random();
        let seed: [u8; 32] = rand::random();
        let settings = Settings {
            name: name.into(),
            address: address.trim_end_matches('/').into(),
            auth_url: format!("http://127.0.0.1:{}", ports[0]),
            vault_url: format!("http://127.0.0.1:{}", ports[1]),
            schedules_url: format!("http://127.0.0.1:{}", ports[2]),
            node_url: None,
            serverless: false,
            ports,
            kernel_secret: secret(),
            auth_secret: secret(),
            schedules_key: json!({ "secret": B64U.encode(&own), "public": B64U.encode(&public_of(&own)) }),
            recovery_key: B64U.encode(&twin(&seed).1),
            resources: vec![],
            bucket: None,
            android: vec![],
            vault_key: if google() { secret() } else { String::new() },
            google: None,
            cloudflare_token: None,
        };
        (settings, data_encoding::HEXLOWER.encode(&seed))
    }

    /// Writes the settings, and, the first time, the secrets' values to `.env.local`; after that a secret changes
    /// only by `set_secret`, so a reference to 1Password there stays one.
    pub fn save(&self) -> Result<()> {
        let path = Settings::path();
        if !local().exists() {
            let mut text = String::new();
            for (name, value) in [("KERNEL_SECRET", self.kernel_secret.clone()), ("AUTH_SECRET", self.auth_secret.clone()), ("SCHEDULES_KEY", self.schedules_key.to_string())] {
                text += &format!("{name}={}\n", quoted(&value));
            }
            if let Some(bucket) = &self.bucket {
                text += &format!("BUCKET={}\n", quoted(&bucket.to_string()));
            }
            if !self.vault_key.is_empty() {
                text += &format!("VAULT_KEY={}\n", quoted(&self.vault_key));
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

    /// Each service's bindings: (name, value, whether it's secret). A value `service:<name>` is another service, and
    /// `vpc:<id>` a Cloudflare Workers VPC service.
    pub fn bindings(&self, service: &str) -> Vec<(String, String, bool)> {
        // The gateway holds no secrets.
        if service == "gateway" {
            let node = match self.serverless {
                true => ("NODE".into(), format!("service:{}", crate::serverless::SERVICE), false),
                false => self.machine("NODE"),
            };
            return vec![("AUTH".into(), "service:auth".into(), false), node];
        }
        let issuer = format!("{}/api/auth", self.address);
        let mut out = vec![("KERNEL_SECRET".into(), self.kernel_secret.clone(), true)];
        let mut plain = |name: &str, value: String| out.push((name.into(), value, false));
        match service {
            "auth" => {
                plain("VAULT", "service:vault".into());
                plain("ADDRESS", self.address.clone());
                plain("NAME", self.name.clone());
                if google() {
                    plain("SIGN_IN", "google".into());
                    plain("JOIN_DOMAIN", crate::config::JOIN_DOMAIN.into());
                }
                // Tokens are for the install's address unless a client asks for a node's own.
                let resources: Vec<&String> = std::iter::once(&self.address).chain(&self.resources).collect();
                plain("RESOURCES", json!(resources).to_string());
            }
            "vault" => {
                plain("AUTH", "service:auth".into());
                plain("ISSUER", issuer);
                plain("RECOVERY_KEY", self.recovery_key.clone());
                if google() {
                    plain("SIGN_IN", "google".into());
                }
            }
            _ => {
                plain("VAULT", "service:vault".into());
                plain("AUTH", "service:auth".into());
                plain("ISSUER", issuer);
            }
        }
        match service {
            "auth" => {
                out.push(("AUTH_SECRET".into(), self.auth_secret.clone(), true));
                if let Some((id, secret)) = self.google.clone().filter(|_| google()) {
                    out.push(("GOOGLE_CLIENT_ID".into(), id, false));
                    out.push(("GOOGLE_CLIENT_SECRET".into(), secret, true));
                }
            }
            "vault" => {
                if let Some(bucket) = &self.bucket {
                    out.push(("BUCKET".into(), bucket.to_string(), true));
                }
                if google() {
                    out.push(("VAULT_KEY".into(), self.vault_key.clone(), true));
                }
            }
            _ => out.push(("SCHEDULES_KEY".into(), self.schedules_key.to_string(), true)),
        }
        out
    }
}

/// The kernel's services as one workerd runs them, with `modules` in `SERVICES`' order, and the gateway serving the
/// web client's files from `web`.
pub fn served(settings: &Settings, modules: Vec<Vec<u8>>, web: &Path) -> Vec<Served> {
    SERVICES
        .iter()
        .zip(modules)
        .enumerate()
        .map(|(i, ((name, _, class), module))| Served {
            name: name.to_string(),
            module,
            class: class.map(str::to_string),
            listen: format!("127.0.0.1:{}", settings.ports[i]),
            bindings: settings.bindings(name).into_iter().map(|(n, v, _)| (n, v)).collect(),
            assets: (*name == "gateway").then(|| web.to_path_buf()),
            // The gateway's node may be on this machine or its private network, as a node behind a tunnel is.
            outbound: (*name == "gateway").then(|| NEARBY.to_string()),
        })
        .collect()
}

/// The Android app's package, as android/app/build.gradle.kts names it.
pub fn android_package() -> String {
    format!("app.{}.android", crate::config::CLI)
}

impl Settings {
    /// The files the address serves itself: the web client's, as this release has them, and Android's App Links file
    /// (`.well-known/assetlinks.json`), which lets the app open links to the address.
    pub fn web(&self) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> =
            crate::config::KERNEL.iter().filter_map(|(path, bytes)| Some((path.strip_prefix("web/")?.to_string(), bytes.to_vec()))).collect();
        if !self.android.is_empty() {
            let links = json!([{
                "relation": ["delegate_permission/common.handle_all_urls"],
                "target": { "namespace": "android_app", "package_name": android_package(), "sha256_cert_fingerprints": self.android },
            }]);
            files.push((".well-known/assetlinks.json".into(), links.to_string().into_bytes()));
        }
        files
    }
}

/// Runs the kernel's services on this machine, in the pinned workerd, until it stops: their modules built from this
/// release, the web client's files in `<home>/kernel/web`, and their storage in `<home>/kernel/data`.
pub fn serve(settings: &Settings) -> Result<()> {
    let dir = Settings::path().parent().unwrap().to_path_buf();
    let modules = SERVICES.iter().map(|(_, entry, _)| module(entry)).collect::<Result<Vec<_>>>()?;
    let web = dir.join("web");
    let _ = std::fs::remove_dir_all(&web);
    for (file, bytes) in settings.web() {
        let path = web.join(file);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes)?;
    }
    let config = write_config(&dir, &served(settings, modules, &web))?;
    let mut child = start(&config, Vec::new())?;
    let status = child.wait()?;
    bail!("the kernel's workerd stopped: {status}")
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
    run(&["build", entry, "--target=browser", "--format=esm", "--external=cloudflare:*", "--external=node:*", &format!("--outfile={}", out.display())])?;
    Ok(std::fs::read(out)?)
}

/// A kernel service as one workerd runs it: its name, module, Durable Object class, the address it listens on, and its
/// bindings: (name, a text, or `service:<name>` for another server).
pub struct Served {
    pub name: String,
    pub module: Vec<u8>,
    pub class: Option<String>,
    pub listen: String,
    pub bindings: Vec<(String, String)>,
    /// A folder it reads as `ASSETS`, as a Worker reads its static assets on Cloudflare.
    pub assets: Option<PathBuf>,
    /// The service its own requests go to, if not the internet: `NEARBY`, or a test's stand-in sites.
    pub outbound: Option<String>,
}

/// The internet, this machine and its private networks, as an outbound.
pub const NEARBY: &str = "nearby";

/// Writes the workerd config that runs `servers` into `dir` (their modules beside it, their storage in `dir/data`),
/// readable only by its owner, since bindings hold the kernel's secrets.
pub fn write_config(dir: &Path, servers: &[Served]) -> Result<PathBuf> {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"));
    let mut services = Vec::new();
    if servers.iter().any(|s| s.outbound.as_deref() == Some(NEARBY)) {
        services.push(format!("(name = {}, network = (allow = [\"public\", \"private\", \"local\"]))", quote(NEARBY)));
    }
    let mut sockets = Vec::new();
    let mut workers = Vec::new();
    for (i, s) in servers.iter().enumerate() {
        ensure!(s.name.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "a kernel service's name is lowercase");
        std::fs::write(dir.join(format!("{}.js", s.name)), &s.module)?;
        let data = dir.join("data").join(&s.name);
        std::fs::create_dir_all(&data)?;
        services.push(format!("(name = {}, worker = .w{i})", quote(&s.name)));
        services.push(format!("(name = {}, disk = (path = {}, writable = true))", quote(&format!("{}-data", s.name)), quote(&data.to_string_lossy())));
        sockets.push(format!("(name = {}, address = {}, http = (), service = {})", quote(&s.name), quote(&s.listen), quote(&s.name)));
        let mut bindings: Vec<String> = s.class.iter().map(|c| format!("(name = {}, durableObjectNamespace = {})", quote(&c.to_uppercase()), quote(c))).collect();
        if let Some(assets) = &s.assets {
            services.push(format!("(name = {}, disk = (path = {}, writable = false))", quote(&format!("{}-assets", s.name)), quote(&assets.to_string_lossy())));
            bindings.push(format!("(name = \"ASSETS\", service = {})", quote(&format!("{}-assets", s.name))));
        }
        for (name, value) in &s.bindings {
            ensure!(!value.starts_with("vpc:"), "a node behind a Cloudflare Tunnel is reached only by a kernel on Cloudflare: give kernel set node-url a node's address");
            bindings.push(match value.strip_prefix("service:") {
                Some(service) => format!("(name = {}, service = {})", quote(name), quote(service)),
                None => format!("(name = {}, text = {})", quote(name), quote(value)),
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
            "const w{i} :Workerd.Worker = (\n  modules = [(name = \"worker.js\", esModule = embed {})],\n  compatibilityDate = \"{}\",\n  bindings = [{}],\n{objects}{});",
            quote(&format!("{}.js", s.name)),
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

    /// The account's Workers, by name, with the Durable Object migration each has had.
    pub fn scripts(&self) -> Result<std::collections::BTreeMap<String, Option<String>>> {
        let list = self.call("GET", &format!("/accounts/{}/workers/scripts", self.account), None)?;
        Ok(list.as_array().into_iter().flatten().filter_map(|s| Some((s["id"].as_str()?.to_string(), s["migration_tag"].as_str().map(str::to_string)))).collect())
    }

    /// Uploads a Worker: its modules (name, type, bytes), the first its main one, and its bindings and Durable Object
    /// class.
    pub(crate) fn upload(&self, name: &str, modules: &[(&str, &str, &[u8])], metadata: &Value) -> Result<()> {
        let boundary = format!("kernel-{}", data_encoding::HEXLOWER.encode(&rand::random::<[u8; 12]>()));
        let mut body = Vec::new();
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{metadata}\r\n").as_bytes());
        for (module, kind, bytes) in modules {
            body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{module}\"; filename=\"{module}\"\r\nContent-Type: {kind}\r\n\r\n").as_bytes());
            body.extend(*bytes);
            body.extend(b"\r\n");
        }
        body.extend(format!("--{boundary}--\r\n").as_bytes());
        self.call("PUT", &format!("/accounts/{}/workers/scripts/{name}", self.account), Some((format!("multipart/form-data; boundary={boundary}"), body)))?;
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

/// Asks a kernel service at `url`, as only the kernel's own tools may.
fn internal(url: &str, path: &str, kernel_secret: &str, body: Value) -> Result<Value> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
    let mut resp = agent.post(format!("{}/internal/{path}", url.trim_end_matches('/'))).header("x-kernel-secret", kernel_secret).send_json(body)?;
    let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    ensure!(resp.status() == 200, "{}", out["error"].as_str().unwrap_or("the kernel failed"));
    Ok(out)
}

/// Gives the vault the install's scope keys its nodes made before it had a kernel (`{ scope, key, people }` each, from
/// a node that holds them), each sealed there to those of `people` who have keys: → who got each.
pub fn give_keys(settings: &Settings, keys: &[Value]) -> Result<Vec<(String, Vec<String>)>> {
    keys.iter()
        .map(|k| {
            let out = internal(&settings.vault_url, "scope-key", &settings.kernel_secret, json!({ "scope": k["scope"], "key": k["key"], "people": k["people"] }))?;
            let granted = out["granted"].as_array().into_iter().flatten().filter_map(|u| u.as_str().map(str::to_string)).collect();
            Ok((k["scope"].as_str().unwrap_or("").to_string(), granted))
        })
        .collect()
}

/// Everyone with keys in `settings`' install: `{ user, sealed }`, their key pair's secret half sealed to the recovery key.
pub fn people(settings: &Settings) -> Result<Vec<Value>> {
    let out = internal(&settings.vault_url, "recovery", &settings.kernel_secret, json!({}))?;
    Ok(out["people"].as_array().cloned().unwrap_or_default())
}

/// An invite link: opening it lets its person make their first passkey and join.
pub struct Invite {
    pub link: String,
    /// Until when it works, in milliseconds since 1970.
    pub until: u64,
}

/// Invites someone to join `settings`' install, `name` only a label, as `user` (who they are in the install, so nodes
/// know them; a new id if not given). Nobody joins uninvited, the install's first person too. Someone who has keys
/// already lost every passkey: their link carries their keys, opened with the recovery key's secret (`recovery` gives
/// it, asked only then), for their new passkey.
pub fn invite(settings: &Settings, name: &str, user: Option<&str>, recovery: impl FnOnce() -> Option<String>) -> Result<Invite> {
    ensure!(!google(), "people here sign in with Google, so they need no invite: {} kernel invite --email says who someone is in the install", crate::config::CLI);
    let held = match user {
        Some(user) => people(settings)?.into_iter().find(|p| p["user"] == user),
        None => None,
    };
    let key = match held {
        None => None,
        Some(held) => {
            let recovery = recovery().with_context(|| format!("{name} has keys already, so their link must carry them: pipe the recovery key"))?;
            let seed: [u8; 32] = data_encoding::HEXLOWER.decode(recovery.trim().as_bytes()).ok().and_then(|s| s.try_into().ok()).context("that isn't a recovery key")?;
            let sealed = B64U.decode(held["sealed"].as_str().unwrap_or("").as_bytes())?;
            Some(open_sealed(&twin(&seed).0, &sealed).context("that isn't this install's recovery key")?)
        }
    };
    let out = internal(&settings.auth_url, "invite", &settings.kernel_secret, json!({ "name": name, "user": user }))?;
    let (token, until) = (out["token"].as_str().context("the auth service gave no invite")?, out["until"].as_u64().unwrap_or(0));
    if let Some(key) = key {
        internal(&settings.vault_url, "recover", &settings.kernel_secret, json!({ "user": user, "key": B64U.encode(&key), "secret": token, "until": until }))?;
    }
    Ok(Invite { link: format!("{}/invite#{token}", settings.address), until })
}

/// On an install whose people sign in with Google, says who `email` is in the install (`user`, who `name` is) when they
/// first sign in, for someone who was in it before it had a kernel: → until when it holds.
pub fn known_as(settings: &Settings, name: &str, user: &str, email: &str) -> Result<u64> {
    ensure!(google(), "people here sign in with a passkey: an invite link lets someone in");
    let out = internal(&settings.auth_url, "invite", &settings.kernel_secret, json!({ "name": name, "user": user, "email": email }))?;
    Ok(out["until"].as_u64().unwrap_or(0))
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
        match value.strip_prefix("service:") {
            Some(other) if exists(&worker_name(other)) => bindings.push(json!({ "type": "service", "name": name, "service": worker_name(other) })),
            Some(_) => complete = false,
            None => match value.strip_prefix("vpc:") {
                Some(id) => bindings.push(json!({ "type": "vpc_service", "name": name, "service_id": id })),
                None => bindings.push(json!({ "type": if secret { "secret_text" } else { "plain_text" }, "name": name, "text": value })),
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
    let mut left: Vec<usize> = (0..SERVICES.len()).filter(|i| services.contains(&SERVICES[*i].0)).collect();
    for _ in 0..2 {
        let mut again = Vec::new();
        for i in left {
            let (service, _, class) = SERVICES[i];
            let scripts = cf.scripts()?;
            let migrated = scripts.get(&worker_name(service)).is_some_and(|tag| tag.as_deref() == Some("v1"));
            let (mut meta, complete) = metadata(settings, service, class, migrated, |name| scripts.contains_key(name) || name == worker_name(service));
            if service == "gateway" {
                let jwt = cf.assets(&worker_name(service), web)?;
                meta["assets"] = json!({ "jwt": jwt, "config": { "html_handling": "none", "not_found_handling": "none", "run_worker_first": true } });
                meta["bindings"].as_array_mut().unwrap().push(json!({ "type": "assets", "name": "ASSETS" }));
            }
            cf.upload(&worker_name(service), &[("worker.js", "application/javascript+module", &modules[i])], &meta)?;
            cf.reach(&worker_name(service), [&settings.auth_url, &settings.vault_url, &settings.schedules_url, &settings.address][i])?;
            if !complete {
                again.push(i);
            }
        }
        left = again;
        if left.is_empty() {
            return Ok(());
        }
    }
    bail!("some services' bindings never resolved")
}
