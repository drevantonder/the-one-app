//! The kernel services (`kernel/`: auth, vault, schedules) as nodes meet them: the formats they agree on
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

/// An app's key in a scope is random, kept sealed under the scope's key: nonce ‖ AES-256-GCM(HKDF(scope key,
/// "one/app\0<app>"), nonce, app key). Erasing the app deletes it, which destroys the key and so the app's data.
pub fn seal_app_key(scope_key: &[u8; 32], app: &str, app_key: &[u8; 32]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = nonce.to_vec();
    out.extend(cipher(&hkdf(scope_key, &[], format!("one/app\0{app}").as_bytes())).encrypt(&nonce.into(), &app_key[..]).expect("encrypting in memory"));
    out
}

/// The key an app's data in a scope was sealed with before app keys were random: derived from the scope's key. Only
/// for data written then (`db::app_key`).
pub fn derived_app_key(scope_key: &[u8; 32], app: &str) -> [u8; 32] {
    hkdf(scope_key, &[], format!("one/app\0{app}").as_bytes())
}

/// Where an app's sealed key is kept in the bucket, beside its databases: `_key` in its folder, which no database
/// can be named.
pub fn app_key_name(scope: &str, app: &str) -> String {
    format!("{}/{app}/_key", crate::backups::folder(scope))
}

/// An app's key, from its sealed copy and its scope's key.
pub fn open_app_key(scope_key: &[u8; 32], app: &str, sealed: &[u8]) -> Result<[u8; 32]> {
    ensure!(sealed.len() == 60, "not a sealed app key");
    let plain = cipher(&hkdf(scope_key, &[], format!("one/app\0{app}").as_bytes())).decrypt(sealed[..12].into(), &sealed[12..]).map_err(|_| anyhow!("that scope's key doesn't open {app}'s"))?;
    plain.try_into().map_err(|_| anyhow!("an app's key is 32 bytes"))
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

/// An access token's claims, if a key in `jwks` (the auth service's) signed it, `iss` issued it, and it hasn't expired
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
        .context("signed by a key the auth service doesn't publish")?;
    let public: [u8; 32] = B64U.decode(jwk["x"].as_str().unwrap_or("").as_bytes())?.try_into().map_err(|_| anyhow!("not an Ed25519 key"))?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&public)?;
    let signature = ed25519_dalek::Signature::from_slice(&B64U.decode(parts[2].as_bytes())?)?;
    key.verify_strict(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature).map_err(|_| anyhow!("its signature is wrong"))?;
    let claims: Claims = serde_json::from_slice(&B64U.decode(parts[1].as_bytes())?).context("its claims aren't an access token's")?;
    ensure!(claims.iss == iss, "issued by {}, not {iss}", claims.iss);
    ensure!(claims.exp * 1000 > now, "it has expired");
    Ok(claims)
}

/// The auth service at an install's address, as nodes check its tokens: its keys fetched and kept five minutes, or
/// until a token names a key that isn't among them (as `kernel/shared/token.js`'s `Issuer` does).
pub struct Issuer {
    pub iss: String,
    keys: std::sync::Mutex<Option<(Value, std::time::Instant)>>,
}

impl Issuer {
    pub fn new(address: &str) -> Issuer {
        Issuer { iss: format!("{}/api/auth", address.trim_end_matches('/')), keys: Default::default() }
    }

    /// A token's claims, as `verify` checks them, at `now` (milliseconds).
    pub fn verify(&self, token: &str, now: u64) -> Result<Claims> {
        for fresh in [false, true] {
            let kept = self.keys.lock().unwrap().clone().filter(|(_, at)| !fresh && at.elapsed() < Duration::from_secs(300));
            let jwks = match kept {
                Some((jwks, _)) => jwks,
                None => {
                    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into();
                    let jwks: Value = agent.get(format!("{}/jwks", self.iss)).call().context("the auth service didn't give its keys")?.body_mut().read_json()?;
                    *self.keys.lock().unwrap() = Some((jwks.clone(), std::time::Instant::now()));
                    jwks
                }
            };
            match verify(token, &jwks, &self.iss, now) {
                Err(err) if !fresh && err.to_string().contains("key the auth service") => continue,
                other => return other,
            }
        }
        unreachable!("the second try returns")
    }
}

// The vault, as nodes ask it.

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

/// What a token's session at the vault opens.
#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub user: String,
    /// The client it's for: an OAuth client's id, or `schedules` for a scheduled run.
    pub client: Option<String>,
    pub scopes: Vec<String>,
    /// The apps it may use, if it's limited to some (as a scheduled run is).
    pub apps: Option<Vec<String>>,
    /// Whether it was let only read: it gets keys and read URLs, and changes and sends nothing.
    #[serde(default)]
    pub read_only: bool,
    pub until: u64,
}

/// A run the schedules service hands a node: `POST <node>/kernel/run`, its token as the bearer. `run` names
/// it, the same each time it's sent, so a node that leases by it runs it once.
#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    pub run: String,
    pub app: String,
    pub schedule: String,
    pub op: String,
    pub input: Value,
    pub scope: String,
}

/// The vault at `url`, asked with a caller's access token.
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

    /// What a token opens: a node asks before it runs anything as its person.
    pub fn session(&self, token: &str) -> Result<Session> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
        let mut resp = agent.get(format!("{}/session", self.url.trim_end_matches('/'))).header("authorization", format!("Bearer {token}")).call()?;
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        ensure!(resp.status() == 200, "the vault said: {}", out["error"].as_str().unwrap_or("nothing"));
        Ok(serde_json::from_value(out)?)
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


// Building and running.

/// The kernel's services: each one's name, its module, and its Durable Object's class, if it keeps anything.
pub const SERVICES: &[(&str, &str, Option<&str>)] = &[
    ("auth", "auth/worker.js", Some("Auth")),
    ("vault", "vault/worker.js", Some("Vault")),
    ("schedules", "schedules/worker.js", Some("Schedules")),
    ("gateway", "gateway/worker.js", None),
];

/// How an install's kernel is set up, wherever it runs: kept in `<home>/kernel/settings.json`, readable only by its
/// owner, since it holds the kernel's secrets.
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
    /// Where the gateway sends `/v1/` and `/mcp`: a node's address.
    #[serde(default)]
    pub node_url: Option<String>,
    /// The local ports each service listens on, in `SERVICES`' order, when this machine runs them.
    pub ports: [u16; 4],
    /// The secret only the kernel's services share, for asking each other.
    pub kernel_secret: String,
    /// Better Auth's own secret, for its cookies and the keys it keeps.
    pub auth_secret: String,
    /// The schedules service's own X25519 key pair.
    pub schedules_key: Value,
    /// The recovery key's X25519 twin's public half.
    pub recovery_key: String,
    /// Nodes' own addresses MCP clients sign in for (each a resource, in OAuth's words), besides the install's.
    #[serde(default)]
    pub resources: Vec<String>,
    /// The install's bucket: `{ endpoint, bucket, region, key_id, secret }`.
    #[serde(default)]
    pub bucket: Option<Value>,
    /// Google sign-in: `{ clientId, clientSecret }`.
    #[serde(default)]
    pub google: Option<Value>,
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::home().join("kernel").join("settings.json")
    }

    pub fn load() -> Result<Settings> {
        let path = Settings::path();
        let text = std::fs::read_to_string(&path).with_context(|| format!("this machine runs no kernel yet: {} kernel serve --address <url> starts one", crate::config::CLI))?;
        serde_json::from_str(&text).with_context(|| format!("{} isn't a kernel's settings", path.display()))
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
            ports,
            kernel_secret: secret(),
            auth_secret: secret(),
            schedules_key: json!({ "secret": B64U.encode(&own), "public": B64U.encode(&public_of(&own)) }),
            recovery_key: B64U.encode(&twin(&seed).1),
            resources: vec![],
            bucket: None,
            google: None,
        };
        (settings, data_encoding::HEXLOWER.encode(&seed))
    }

    pub fn save(&self) -> Result<()> {
        let path = Settings::path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let part = path.with_extension("part");
        std::fs::write(&part, serde_json::to_vec_pretty(self)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(std::fs::rename(part, path)?)
    }

    /// Each service's bindings: (name, value, whether it's secret). A value `service:<name>` is another service.
    pub fn bindings(&self, service: &str) -> Vec<(String, String, bool)> {
        // The gateway holds no secrets.
        if service == "gateway" {
            return vec![("AUTH".into(), "service:auth".into(), false), ("NODE_URL".into(), self.node_url.clone().unwrap_or_default(), false)];
        }
        let issuer = format!("{}/api/auth", self.address);
        let mut out = vec![("KERNEL_SECRET".into(), self.kernel_secret.clone(), true)];
        let mut plain = |name: &str, value: String| out.push((name.into(), value, false));
        match service {
            "auth" => {
                plain("VAULT", "service:vault".into());
                plain("ADDRESS", self.address.clone());
                plain("NAME", self.name.clone());
                // Tokens are for the install's address unless a client asks for a node's own.
                let resources: Vec<&String> = std::iter::once(&self.address).chain(&self.resources).collect();
                plain("RESOURCES", json!(resources).to_string());
            }
            "vault" => {
                plain("AUTH", "service:auth".into());
                plain("ISSUER", issuer);
                plain("RECOVERY_KEY", self.recovery_key.clone());
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
                if let Some(google) = &self.google {
                    out.push(("GOOGLE".into(), google.to_string(), true));
                }
            }
            "vault" => {
                if let Some(bucket) = &self.bucket {
                    out.push(("BUCKET".into(), bucket.to_string(), true));
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

/// The web client's files, as this release has them: (path in `kernel/web/`, bytes).
pub fn web_files() -> impl Iterator<Item = (&'static str, &'static [u8])> {
    crate::config::KERNEL.iter().filter_map(|(path, bytes)| Some((path.strip_prefix("web/")?, *bytes)))
}

/// Runs the kernel's services on this machine, in the pinned workerd, until it stops: their modules built from this
/// release, the web client's files in `<home>/kernel/web`, and their storage in `<home>/kernel/data`.
pub fn serve(settings: &Settings) -> Result<()> {
    let dir = Settings::path().parent().unwrap().to_path_buf();
    let modules = SERVICES.iter().map(|(_, entry, _)| module(entry)).collect::<Result<Vec<_>>>()?;
    let web = dir.join("web");
    let _ = std::fs::remove_dir_all(&web);
    for (file, bytes) in web_files() {
        let path = web.join(file);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes)?;
    }
    let config = write_config(&dir, &served(settings, modules, &web))?;
    let mut child = start(&config)?;
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
    fn scripts(&self) -> Result<std::collections::BTreeMap<String, Option<String>>> {
        let list = self.call("GET", &format!("/accounts/{}/workers/scripts", self.account), None)?;
        Ok(list.as_array().into_iter().flatten().filter_map(|s| Some((s["id"].as_str()?.to_string(), s["migration_tag"].as_str().map(str::to_string)))).collect())
    }

    /// Uploads a Worker: its module, bindings and Durable Object class, and turns its workers.dev address on.
    fn upload(&self, name: &str, module: &[u8], metadata: &Value) -> Result<()> {
        let boundary = format!("kernel-{}", data_encoding::HEXLOWER.encode(&rand::random::<[u8; 12]>()));
        let mut body = Vec::new();
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{metadata}\r\n").as_bytes());
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"worker.js\"; filename=\"worker.js\"\r\nContent-Type: application/javascript+module\r\n\r\n").as_bytes());
        body.extend(module);
        body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
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

    /// Puts a Worker at `url`: its workers.dev address, or a host in one of the account's domains, with its
    /// workers.dev address off, so each service has one address.
    fn reach(&self, name: &str, url: &str) -> Result<()> {
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

/// Lets someone sign up at the auth service at `auth_url`, with the kernel's secret, as `user` (who they are in the
/// install, so nodes know them) if it's given: nobody signs up uninvited, the install's first person too.
pub fn invite(auth_url: &str, kernel_secret: &str, email: &str, user: Option<&str>) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
    let mut resp = agent.post(format!("{}/internal/invite", auth_url.trim_end_matches('/'))).header("x-kernel-secret", kernel_secret).send_json(json!({ "email": email, "user": user }))?;
    let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    ensure!(resp.status() == 200, "the auth service said: {}", out["error"].as_str().unwrap_or("nothing"));
    Ok(())
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
            None => bindings.push(json!({ "type": if secret { "secret_text" } else { "plain_text" }, "name": name, "text": value })),
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
    let mut left: Vec<usize> = (0..SERVICES.len()).collect();
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
            cf.upload(&worker_name(service), &modules[i], &meta)?;
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
