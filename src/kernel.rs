//! The kernel servers (`kernel/`: auth, vault, schedules) as compute servers meet them: the formats they agree on
//! (`kernel/shared/crypto.js` writes the same ones, and `tests/contracts.rs` checks they do), checking access tokens,
//! asking the vault for keys and bucket URLs, and building and running the servers in workerd.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Context, Result, anyhow, bail, ensure};
use curve25519_dalek::edwards::CompressedEdwardsY;
use curve25519_dalek::montgomery::MontgomeryPoint;
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};

// Keys.

fn hkdf(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    hkdf::Hkdf::<Sha256>::new(Some(salt), ikm).expand(info, &mut out).expect("32 bytes is a valid length");
    out
}

/// An app's key in a scope, from the scope's key.
pub fn app_key(scope_key: &[u8; 32], app: &str) -> [u8; 32] {
    hkdf(scope_key, &[], format!("one/app\0{app}").as_bytes())
}

/// A database's key in an app, from the app's key.
pub fn database_key(app_key: &[u8; 32], database: &str) -> [u8; 32] {
    hkdf(app_key, &[], format!("one/database\0{database}").as_bytes())
}

fn cipher(key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new(key.into())
}

/// The public half of an X25519 secret.
pub fn public_of(secret: &[u8; 32]) -> [u8; 32] {
    MontgomeryPoint::mul_base_clamped(*secret).0
}

/// The X25519 twin of an Ed25519 key (the recovery key), as age and backups use it: → (secret, public).
pub fn twin(seed: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let secret: [u8; 32] = Sha512::digest(seed)[..32].try_into().expect("64 bytes");
    (secret, public_of(&secret))
}

/// The X25519 twin of an Ed25519 public key.
pub fn twin_public(public: &[u8; 32]) -> Result<[u8; 32]> {
    Ok(CompressedEdwardsY(*public).decompress().context("not an Ed25519 key")?.to_montgomery().0)
}

/// Seals `plain` to an X25519 public key: share ‖ AES-256-GCM(HKDF(shared, share ‖ to), zero nonce, plain).
pub fn seal_to(to: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>> {
    let ephemeral: [u8; 32] = rand::random();
    let share = public_of(&ephemeral);
    let shared = MontgomeryPoint(*to).mul_clamped(ephemeral).0;
    ensure!(shared != [0; 32], "not a public key");
    let key = hkdf(&shared, &[&share[..], to].concat(), b"one/sealed-to");
    let mut out = share.to_vec();
    out.extend(cipher(&key).encrypt(&Default::default(), plain).map_err(|_| anyhow!("sealing failed"))?);
    Ok(out)
}

/// Opens what was sealed to the public half of `secret`.
pub fn open_sealed(secret: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>> {
    ensure!(sealed.len() >= 48, "not sealed");
    let share: [u8; 32] = sealed[..32].try_into()?;
    let shared = MontgomeryPoint(share).mul_clamped(*secret).0;
    let key = hkdf(&shared, &[&share[..], &public_of(secret)].concat(), b"one/sealed-to");
    cipher(&key).decrypt(&Default::default(), &sealed[32..]).map_err(|_| anyhow!("that key doesn't open it"))
}

/// The id a copy sealed under `secret` is kept by.
pub fn id_of(secret: &[u8]) -> String {
    B64U.encode(&hkdf(secret, &[], b"one/id"))
}

/// Seals `plain` under a secret (text, as the vault is given it): nonce ‖ AES-256-GCM.
pub fn seal_under(secret: &[u8], plain: &[u8]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = nonce.to_vec();
    out.extend(cipher(&hkdf(secret, &[], b"one/sealed-under")).encrypt(&nonce.into(), plain).expect("encrypting in memory"));
    out
}

/// Opens what was sealed under `secret`.
pub fn open_under(secret: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    ensure!(sealed.len() >= 28, "not sealed");
    cipher(&hkdf(secret, &[], b"one/sealed-under")).decrypt(sealed[..12].into(), &sealed[12..]).map_err(|_| anyhow!("that secret doesn't open it"))
}

pub fn decode_key(text: &str) -> Result<[u8; 32]> {
    B64U.decode(text.as_bytes())?.try_into().map_err(|_| anyhow!("a key is 32 bytes"))
}

// Access tokens.

/// What an access token says.
#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    pub iss: String,
    /// The person's user id.
    pub sub: String,
    /// The client it was issued to.
    pub azp: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub aud: Value,
    pub exp: u64,
    #[serde(default)]
    pub jti: String,
}

impl Claims {
    /// Whether it was issued for `resource`.
    pub fn for_resource(&self, resource: &str) -> bool {
        match &self.aud {
            Value::String(a) => a == resource,
            Value::Array(list) => list.iter().any(|a| a == resource),
            _ => false,
        }
    }
}

/// An access token's claims, if a key in `jwks` (the auth server's) signed it, `iss` issued it, and it hasn't expired
/// at `now` (milliseconds).
pub fn verify(token: &str, jwks: &Value, iss: &str, now: u64) -> Result<Claims> {
    let parts: Vec<&str> = token.split('.').collect();
    ensure!(parts.len() == 3, "not a token");
    let header: Value = serde_json::from_slice(&B64U.decode(parts[0].as_bytes())?)?;
    ensure!(header["alg"] == "EdDSA", "not a token signed with Ed25519");
    let jwk = jwks["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|k| k["kid"] == header["kid"] && k["kty"] == "OKP" && k["crv"] == "Ed25519")
        .context("signed by a key the auth server doesn't publish")?;
    let public: [u8; 32] = B64U.decode(jwk["x"].as_str().unwrap_or("").as_bytes())?.try_into().map_err(|_| anyhow!("not an Ed25519 key"))?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&public)?;
    let signature = ed25519_dalek::Signature::from_slice(&B64U.decode(parts[2].as_bytes())?)?;
    key.verify_strict(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature).map_err(|_| anyhow!("its signature is wrong"))?;
    let claims: Claims = serde_json::from_slice(&B64U.decode(parts[1].as_bytes())?).context("its claims aren't an access token's")?;
    ensure!(claims.iss == iss, "issued by {}, not {iss}", claims.iss);
    ensure!(claims.exp * 1000 > now, "it has expired");
    Ok(claims)
}

// The vault, as compute servers ask it.

/// A key a call names, and (from the vault) the key itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub scope: String,
    pub app: String,
    pub database: Option<String>,
    pub key: [u8; 32],
}

/// Presigned URLs, by object name, and until when (milliseconds) they work.
#[derive(Debug, Default, Deserialize)]
pub struct Urls {
    pub read: std::collections::BTreeMap<String, String>,
    /// Each works only with `If-None-Match: *`, so only while nothing has that name.
    pub create: std::collections::BTreeMap<String, String>,
    /// Each works only with `If-Match: <the ETag it was asked with>`.
    pub replace: std::collections::BTreeMap<String, String>,
    pub until: u64,
}

/// The vault server at `url`, asked with a caller's access token.
pub struct Vault {
    pub url: String,
}

impl Vault {
    fn post(&self, token: &str, path: &str, body: Value) -> Result<Value> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
        let mut resp = agent.post(format!("{}{path}", self.url.trim_end_matches('/'))).header("authorization", format!("Bearer {token}")).send_json(body)?;
        let status = resp.status().as_u16();
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        if status != 200 {
            bail!("the vault said: {}", out["error"].as_str().unwrap_or("nothing"));
        }
        Ok(out)
    }

    /// The keys a call names: (scope, app, database?) each.
    pub fn keys(&self, token: &str, want: &[(&str, &str, Option<&str>)]) -> Result<Vec<Key>> {
        let keys: Vec<Value> = want.iter().map(|(s, a, d)| if let Some(d) = d { json!({ "scope": s, "app": a, "database": d }) } else { json!({ "scope": s, "app": a }) }).collect();
        let out = self.post(token, "/keys", json!({ "keys": keys }))?;
        out["keys"]
            .as_array()
            .context("the vault gave no keys")?
            .iter()
            .map(|k| {
                Ok(Key {
                    scope: k["scope"].as_str().context("a key's scope")?.into(),
                    app: k["app"].as_str().context("a key's app")?.into(),
                    database: k["database"].as_str().map(str::to_string),
                    key: decode_key(k["key"].as_str().context("a key")?)?,
                })
            })
            .collect()
    }

    /// URLs to read and create objects named `<scope's folder>/<app>/…` (or `log/…`, the install's log), and to
    /// replace a database's `…/newest` while it's the version an ETag names: (name, etag) each.
    pub fn urls(&self, token: &str, read: &[&str], create: &[&str], replace: &[(&str, &str)]) -> Result<Urls> {
        let replace: Vec<Value> = replace.iter().map(|(name, etag)| json!({ "name": name, "etag": etag })).collect();
        Ok(serde_json::from_value(self.post(token, "/urls", json!({ "read": read, "create": create, "replace": replace }))?)?)
    }
}

/// A scope's folder in the bucket: `user:<id>` is written `user-<id>`.
pub fn folder_of(scope: &str) -> String {
    scope.replacen(':', "-", 1)
}

// Building and running.

/// The kernel's servers: each one's name, its module, and its Durable Object's class.
pub const SERVERS: &[(&str, &str, Option<&str>)] = &[("vault", "vault/worker.js", Some("Vault"))];

/// A kernel server's module, built from the sources this release was signed with by the pinned Bun, once per
/// version of them.
pub fn module(entry: &str) -> Result<Vec<u8>> {
    module_from(crate::config::KERNEL.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect(), entry)
}

/// A kernel server's module, built from `files` (paths in `kernel/`).
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

/// A kernel server as one workerd runs it: its name, module, Durable Object class, the address it listens on, and its
/// bindings: (name, a text, or `service:<name>` for another server).
pub struct Served {
    pub name: String,
    pub module: Vec<u8>,
    pub class: Option<String>,
    pub listen: String,
    pub bindings: Vec<(String, String)>,
    /// The service its own requests go to, if not the internet (a test's stand-in sites).
    pub outbound: Option<String>,
}

/// Writes the workerd config that runs `servers` into `dir` (their modules beside it, their storage in `dir/data`),
/// readable only by its owner, since bindings hold the kernel's secrets.
pub fn write_config(dir: &Path, servers: &[Served]) -> Result<PathBuf> {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"));
    let mut services = Vec::new();
    let mut sockets = Vec::new();
    let mut workers = Vec::new();
    for (i, s) in servers.iter().enumerate() {
        ensure!(s.name.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "a kernel server's name is lowercase");
        std::fs::write(dir.join(format!("{}.js", s.name)), &s.module)?;
        let data = dir.join("data").join(&s.name);
        std::fs::create_dir_all(&data)?;
        services.push(format!("(name = {}, worker = .w{i})", quote(&s.name)));
        services.push(format!("(name = {}, disk = (path = {}, writable = true))", quote(&format!("{}-data", s.name)), quote(&data.to_string_lossy())));
        sockets.push(format!("(name = {}, address = {}, http = (), service = {})", quote(&s.name), quote(&s.listen), quote(&s.name)));
        let mut bindings: Vec<String> = s.class.iter().map(|c| format!("(name = {}, durableObjectNamespace = {})", quote(&c.to_uppercase()), quote(c))).collect();
        for (name, value) in &s.bindings {
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

/// Starts workerd on a config: → the running process.
pub fn start(config: &Path) -> Result<std::process::Child> {
    Ok(std::process::Command::new(crate::apps::workerd()?).arg("serve").arg(config).arg("--experimental").stdin(std::process::Stdio::null()).spawn()?)
}
