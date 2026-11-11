//! What the device's own key opens and signs, which an app runtime asks of its holder rather than taking for its own: the
//! epochs of the scopes the device opens, the Vault keys and app schedules' keys and anything else sealed to it; its
//! steps in `global`'s scope database, which only a user's own devices sign; and its requests to the relay and auth,
//! which say what it lets in and who it is. A computer's app runtime is an endpoint of its own key, and asks its
//! device's holder, the computer's onet, on this machine's loopback (`Holder`), with the token the device's own programs
//! ask with; an app runtime that is its own device (the tests', and oned on Cloudflare's kind) holds the key itself, and
//! answers `/api/holds` for one asking it in turn.

use axum::extract::State;
use axum::routing::{get, post};
use data_encoding::BASE64URL_NOPAD as B64U;

use super::*;
use crate::api::Routes;
use ::api::Body;
use ::api::Reply;
use ::api::Who;
use ::api::holds::{HeldShare, HeldSign, HeldWrite, sign_request};
use ::database::stored::Keys;

pub(super) enum Holds {
    /// The device's key itself.
    Key(SigningKey),
    /// The device's holder on this machine, as its app runtime asks it, and the device's key and seal key it said.
    Asks { holder: Holder, device: Key, seal: [u8; 32] },
}

/// Where an app runtime asks its device's holder: its loopback address, and the token its device's own programs ask
/// with (`api::LOCAL_TOKEN`, in the device's home).
#[derive(Clone)]
pub struct Holder {
    pub at: String,
    pub token: String,
}

impl Holder {
    pub(super) async fn ask(&self, method: &'static str, path: &str, body: Option<Value>) -> Result<Value> {
        let (url, token) = (format!("http://{}{path}", self.at), self.token.clone());
        let (status, said) = tokio::task::spawn_blocking(move || -> Result<(u16, Value)> {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
            let header = custom_build::protocol::header::LOCAL_TOKEN;
            let mut resp = match body {
                Some(body) => agent.post(&url).header(header, &token).send_json(body)?,
                None => agent.run(ureq::http::Request::builder().method(method).uri(&url).header(header, &token).body(())?)?,
            };
            Ok((resp.status().as_u16(), resp.body_mut().read_json().unwrap_or(Value::Null)))
        })
        .await?
        .with_context(|| format!("this app runtime's device holds its key at {}, and it didn't answer: is it running?", self.at))?;
        ensure!(status == 200, "this app runtime's device wouldn't {path}: {}", said["error"].as_str().unwrap_or("it didn't say why"));
        Ok(said)
    }
}

impl Holds {
    /// What the holder at `holder` holds: the device's key and seal key, as it says them.
    pub(super) async fn asking(holder: Holder) -> Result<Holds> {
        let said = holder.ask("GET", "/api/holds", None).await?;
        let hexed = |k: &str| -> Result<[u8; 32]> { unhex(said[k].as_str().with_context(|| format!("the holder said no {k}"))?) };
        Ok(Holds::Asks { device: hexed("key")?, seal: hexed("seal")?, holder })
    }

    /// The device whose key this is.
    pub(super) fn device(&self) -> Key {
        match self {
            Holds::Key(key) => key.verifying_key().to_bytes(),
            Holds::Asks { device, .. } => *device,
        }
    }

    /// Writes each of `contents` as the device's next step in `global`, in order, as `user`, its user: signed by its key,
    /// by `host` or by its holder, which writes them there in one ask and from which `host` then reads them.
    pub(super) async fn write_global(&self, host: &Host, user: &str, contents: Vec<Content>) -> Result<()> {
        match self {
            Holds::Key(key) => {
                for content in contents {
                    host.write_step_as(key, user, content).await?;
                }
                Ok(())
            }
            Holds::Asks { holder, .. } => {
                // Its blobs go first, as `put_step`'s do, so a device that reads the steps finds them: the holder signs
                // and writes the steps, and has none of this runtime's blobs.
                for blob in contents.iter().flat_map(|c| &c.blobs) {
                    database_runtime::share(&host.rt(), &Scope::Global, blob, false).await.with_context(|| format!("blob {} isn't here: put it first", &blob[..12.min(blob.len())]))?;
                }
                holder.ask("POST", "/api/holds/write", Some(json!({ "user": user, "contents": contents }))).await?;
                host.read_steps_now().await?;
                host.ping(&Scope::Global, None);
                Ok(())
            }
        }
    }

    /// Its holder reads and writes the network's data in the bucket `fields` name from now on, as its app runtime is about to.
    pub(super) async fn moved(&self, fields: &BTreeMap<String, String>) -> Result<()> {
        match self {
            Holds::Key(_) => Ok(()),
            Holds::Asks { holder, .. } => holder.ask("POST", "/api/holds/moved", Some(json!(::api::holds::HeldMoved { bucket: fields.clone() }))).await.map(drop),
        }
    }

    /// The epochs of `scope` sealed to the device in the bucket, of those `hashes` names by number, each kept only if
    /// it's the key its entry made: one that isn't is someone's junk, passed over.
    pub(super) async fn epochs(&self, bucket: &Copied, scope: &Scope, hashes: &BTreeMap<u32, String>) -> Result<Keys> {
        let mut out = Keys::new();
        for (n, hash) in hashes {
            let name = ::database::keys::epoch_name(&scope.to_string(), *n, &hex(&self.device()));
            let Some(o) = bucket::Bucket::get(bucket, &name).await? else { continue };
            match key::open_sealed_by(self, &o.bytes).await.ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| key::epoch_hash(k) == *hash) {
                Some(key) => drop(out.insert(*n, key)),
                None => tracing::warn!("{name} isn't the key {scope}'s epoch {n} entry made: passed over"),
            }
        }
        Ok(out)
    }

    /// The Vault secret keys of `scope` sealed to the device in the bucket, of those `publics` names by number, each
    /// kept only if its public half is the one its `vault` step names.
    pub(super) async fn vault_keys(&self, bucket: &Copied, scope: &Scope, publics: &[(u32, [u8; 32])]) -> Result<BTreeMap<u32, [u8; 32]>> {
        let mut out = BTreeMap::new();
        for (n, public) in publics {
            let name = ::database::keys::vault_key_name(&scope.to_string(), *n, &hex(&self.device()));
            let Some(o) = bucket::Bucket::get(bucket, &name).await? else { continue };
            match key::open_sealed_by(self, &o.bytes).await.ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| key::public_of(k) == *public) {
                Some(key) => drop(out.insert(*n, key)),
                None => tracing::warn!("{name} isn't the Vault key its step names: passed over"),
            }
        }
        Ok(out)
    }

    /// `app`'s keys in `scope` that app schedules sealed to the device, of those `hashes` names by epoch, each kept only
    /// if its hash is the one they name.
    pub(super) async fn app_schedules_keys(&self, bucket: &Copied, scope: &Scope, app: &str, hashes: &BTreeMap<u32, String>) -> Result<Keys> {
        let (scope, me) = (scope.to_string(), hex(&self.device()));
        let mut out = Keys::new();
        for (n, hash) in hashes {
            let name = ::database::keys::app_schedules_key_name(&scope, app, &me, *n);
            let found = match bucket::Bucket::get(bucket, &name).await? {
                Some(o) => Some(o),
                None => bucket::Bucket::get(bucket, &::database::keys::app_schedules_key_name_before(&scope, app, &me, *n)).await?,
            };
            let Some(o) = found else {
                tracing::warn!("{name} isn't in the bucket yet");
                continue;
            };
            match key::open_sealed_by(self, &o.bytes).await.ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| key::epoch_hash(k) == *hash) {
                Some(key) => drop(out.insert(*n, key)),
                None => tracing::warn!("{name} isn't the key its app schedules name: passed over"),
            }
        }
        Ok(out)
    }
}

impl key::Opens for Holds {
    fn public(&self) -> [u8; 32] {
        match self {
            Holds::Key(key) => key::public_of(&key::seal_secret(key)),
            Holds::Asks { seal, .. } => *seal,
        }
    }

    async fn shared(&self, share: &[u8; 32]) -> Result<[u8; 32]> {
        match self {
            Holds::Key(key) => Ok(key::agree(&key::seal_secret(key), share)),
            Holds::Asks { holder, .. } => {
                let said = holder.ask("POST", "/api/holds/shared", Some(json!({ "share": B64U.encode(share) }))).await?;
                let shared = B64U.decode(said["shared"].as_str().context("the holder said nothing it shares")?.as_bytes())?;
                <[u8; 32]>::try_from(shared).map_err(|_| anyhow!("the holder's shared secret isn't 32 bytes"))
            }
        }
    }
}

impl key::Signs for Holds {
    fn public(&self) -> [u8; 32] {
        self.device()
    }

    async fn sign_bytes(&self, bytes: &[u8]) -> Result<[u8; 64]> {
        match self {
            Holds::Key(key) => key.sign_bytes(bytes).await,
            Holds::Asks { holder, .. } => {
                let said = holder.ask("POST", "/api/holds/sign", Some(json!({ "bytes": B64U.encode(bytes) }))).await?;
                let signature = B64U.decode(said["signature"].as_str().context("the holder signed nothing")?.as_bytes())?;
                <[u8; 64]>::try_from(signature).map_err(|_| anyhow!("the holder's signature isn't 64 bytes"))
            }
        }
    }
}

pub(crate) fn routes() -> Routes {
    Routes::default()
        .route("/api/holds", Who::ThisMachine, get(held))
        .route("/api/holds/shared", Who::ThisMachine, post(shared))
        .route("/api/holds/write", Who::ThisMachine, post(write))
        .route("/api/holds/sign", Who::ThisMachine, post(sign))
}

/// The device's key and seal key.
async fn held(State(host): State<Host>) -> Reply {
    let holds = &host.0.holds;
    Ok(axum::Json(json!({ "key": hex(&holds.device()), "seal": hex(&key::Opens::public(holds)) })))
}

/// The X25519 secret the device's seal key shares with `share`: what opens something sealed to the device.
async fn shared(State(host): State<Host>, Body(body): Body<HeldShare>) -> Reply {
    let share = <[u8; 32]>::try_from(B64U.decode(body.share.as_bytes()).map_err(anyhow::Error::from)?).map_err(|_| anyhow!("a share is 32 bytes"))?;
    let shared = key::Opens::shared(&host.0.holds, &share).await?;
    Ok(axum::Json(json!({ "shared": B64U.encode(&shared) })))
}

/// Writes a step of `global` as the device, for its user alone.
async fn write(State(host): State<Host>, Body(body): Body<HeldWrite>) -> Reply {
    if body.user != host.user()? {
        return Err(anyhow!("the device writes in global only as its own user").into());
    }
    host.write_steps(&body.user, body.contents).await?;
    Ok(axum::Json(json!({})))
}

/// Signs a request to the relay or auth as the device (`sign_request`).
async fn sign(State(host): State<Host>, Body(body): Body<HeldSign>) -> Reply {
    let Holds::Key(key) = &host.0.holds else { return Err(anyhow!("this app runtime doesn't hold its device's key").into()) };
    let signature = sign_request(key, &B64U.decode(body.bytes.as_bytes()).map_err(anyhow::Error::from)?)?;
    Ok(axum::Json(json!({ "signature": B64U.encode(&signature) })))
}
