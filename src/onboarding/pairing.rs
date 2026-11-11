//! Pairing: a code that lets a new device join the network, and admitting it.

use ::custom_build::allowed;
use ::custom_build::config::YOURS;
use ::custom_build::protocol::{code, Coded, PAIRING_LIFE};
use super::*;

fn used() -> anyhow::Error {
    Coded::error(code::PAIRING_USED, "this pairing code was already used or has expired")
}

#[derive(Serialize, Deserialize)]
pub(super) struct Pairing {
    pub(super) user: String,
    /// The client that asked for it, if one did.
    #[serde(default)]
    pub(super) by: Option<String>,
    /// The user the machine joins as, when it isn't the asker's: an admin's code for someone else's machine.
    #[serde(default)]
    pub(super) admits: Option<String>,
    /// Milliseconds since 1970 after which it no longer works.
    pub(super) until: u64,
    /// The machine whose join with it didn't finish (hex), which alone may use it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tried: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PairingCode {
    /// The network's address, whose relay reaches the device that made it.
    address: String,
    from: Key,
    secret: [u8; 16],
}

#[derive(Serialize, Deserialize)]
pub(super) struct JoinRequest {
    secret: [u8; 16],
    name: String,
    #[serde(rename = "as")]
    as_: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct JoinReply {
    root: String,
    user: String,
    /// For a device: the network's bucket, as a place's fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bucket: Option<BTreeMap<String, String>>,
}

/// A computer that joined with a pairing code: the network's address, and the network as its home keeps it.
pub struct Joined {
    pub address: String,
    pub network: NetworkFile,
}

/// Asks the device that made a pairing code, `code`, to let in the computer whose key is `key`, called `name`, over the
/// relay at the network's address the code names, showing it the code's ticket: → the network, once it's let in.
pub async fn join_with(key: &SigningKey, name: &str, code: &str) -> Result<Joined> {
    let bytes = data_encoding::BASE32_NOPAD.decode(code.trim().to_uppercase().as_bytes()).map_err(|_| Coded::error(code::BAD_PAIRING_CODE, "that isn't a pairing code: copy it again, or ask for a new one"))?;
    let code: PairingCode = ciborium::from_reader(bytes.as_slice()).map_err(|_| Coded::error(code::BAD_PAIRING_CODE, "that isn't a pairing code: copy it again, or ask for a new one"))?;
    let request = JoinRequest { secret: code.secret, name: name.to_string(), as_: "device".into() };
    let reply = net::ask_to_join(key, &code.address, &net::pairing_ticket(&code.secret), code.from, serde_json::to_value(&request)?).await.map_err(|err| Coded::error(code::UNREACHABLE, format!("couldn't reach the device that made the pairing code: {err:#}")))?;
    if let Some(err) = reply.get("error") {
        return Err(Coded::or_plain(reply["code"].as_str().map(str::to_string), err.as_str().unwrap_or("the pairing code was refused")));
    }
    let reply: JoinReply = serde_json::from_value(reply)?;
    // A folder on another machine isn't here: it's this network's bucket only if it says so. S3 is reached through
    // the URL signer, which this device can ask once it knows the network's address.
    let here = |f: &BTreeMap<String, String>| Place::from_fields(f).ok().and_then(|p| bucket_root(&p).ok().flatten()).is_some_and(|r| r == reply.root.as_bytes());
    let mut bucket = reply.bucket.filter(|f| f.contains_key("endpoint") || here(f));
    // Its address is in the log, read from the bucket through it: until then, it's the one the code names.
    if let Some(f) = bucket.as_mut().filter(|f| f.contains_key("endpoint")) {
        f.insert("address".into(), code.address.clone());
    }
    if bucket.is_none() {
        tracing::warn!("this device can't reach the network's bucket: it keeps no app's data until the network moves to S3-compatible storage");
    }
    Ok(Joined { address: code.address, network: NetworkFile { root: reply.root, user: reply.user, bucket, folders: BTreeMap::new() } })
}

pub(super) fn load(dir: &Path) -> HashMap<[u8; 16], Pairing> {
    let list: Vec<(String, Pairing)> = platform::files::read_json(&dir.join("pairings.json"));
    list.into_iter()
        .filter_map(|(k, v)| Some((data_encoding::HEXLOWER.decode(k.as_bytes()).ok()?.try_into().ok()?, v)))
        .collect()
}

impl Node {
    /// Makes a one-time pairing code for another of the local user's machines, which joins as a device.
    pub fn pair(&self) -> Result<String> {
        Ok(self.pair_as(&Caller::local(self)?, None)?.0)
    }

    /// Like `pair`, for `asker`, or for an admin, a code for another machine of `user` (an id or a name). With the id
    /// of the user the code admits.
    pub fn pair_as(&self, asker: &Caller, user: Option<&str>) -> Result<(String, String)> {
        let user = user.map(|u| self.find_user(u)).transpose()?;
        self.with(|i| {
            let m = i.r.db();
            ensure!(m.compute(&self.0.id)?.is_some(), "this device isn't in {} yet", ::custom_build::config::YOURS);
            let kinds = if user.as_ref().is_none_or(|u| *u == asker.user) { MACHINE } else { &[scopes::Kind::Compute][..] };
            allowed!(devices::may_list(m, &asker.user, user.as_deref().unwrap_or(&asker.user), kinds)?, "only admins pair other users' machines");
            ensure!(m.serves(&self.0.id, &asker.user)?, "{} doesn't serve you", self.device_name());
            Ok(())
        })?;
        let address = self.with(|i| Ok(i.r.db().setting("address")?))?.with_context(|| format!("a computer joins through {YOURS}'s address, and it has none yet: an admin sets it with cloudflare use"))?;
        let joins_as = user.clone().unwrap_or_else(|| asker.user.clone());
        let secret: [u8; 16] = rand::random();
        let by = (asker.key != self.0.id).then(|| hex(&asker.key));
        self.add_pairing(secret, Pairing { user: asker.user.clone(), by, admits: user, until: now_ms() + PAIRING_LIFE, tried: None })?;
        // Told with the key the code names, which answers the computer joining with it: the relay lets that computer
        // reach only the key that told it of the code.
        let (node, fields) = (self.clone(), json!({ "pairing": { "hash": net::ticket_hash(&net::pairing_ticket(&secret)), "until": now_ms() + PAIRING_LIFE } }));
        tokio::spawn(async move {
            if let Err(err) = scope::settings::tell(&node, &node.0.key, fields).await {
                tracing::warn!("telling {YOURS}'s relay of a pairing code: {err:#}");
            }
        });
        let code = PairingCode { address, from: self.0.id, secret };
        let mut bytes = Vec::new();
        ciborium::into_writer(&code, &mut bytes)?;
        Ok((data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase(), joins_as))
    }

    /// Pairing codes are kept in a file, so one still works after the device that made it restarts.
    pub(super) fn add_pairing(&self, secret: [u8; 16], pairing: Pairing) -> Result<()> {
        let mut pairings = self.0.pairings.lock().unwrap();
        pairings.retain(|_, p| p.until > now_ms());
        pairings.insert(secret, pairing);
        self.save_pairings(&pairings)
    }

    pub(super) fn take_pairing(&self, secret: &[u8; 16]) -> Result<Option<Pairing>> {
        let mut pairings = self.0.pairings.lock().unwrap();
        let pairing = pairings.remove(secret);
        if pairing.is_some() {
            self.save_pairings(&pairings)?;
        }
        Ok(pairing)
    }

    pub(super) fn save_pairings(&self, pairings: &HashMap<[u8; 16], Pairing>) -> Result<()> {
        let list: Vec<(String, &Pairing)> = pairings.iter().map(|(k, v)| (data_encoding::HEXLOWER.encode(k), v)).collect();
        write_private(&self.0.dir.join("pairings.json"), &serde_json::to_vec(&list)?)?;
        Ok(())
    }

    /// Joins the network a pairing code is from → that network's address.
    pub async fn join(&self, code: &str) -> Result<String> {
        self.ensure_no_network()?;
        let joined = join_with(&self.0.key, &self.device_name(), code).await?;
        self.save_network(&joined.network)?;
        self.joined()?;
        Ok(joined.address)
    }

    /// Opens the network its home names now that it's joined it: by itself (`join`), or on a computer, where its onet
    /// joins as the device, as the app runtime asked by it (`/api/joined`). Open already, it's nothing.
    pub fn joined(&self) -> Result<()> {
        if self.has_network() {
            return Ok(());
        }
        self.open_network()?;
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.read_scopes(None).await {
                tracing::debug!("reading the bucket after joining: {err:#}");
            }
        });
        self.emit(Event::Network);
        Ok(())
    }

    /// Joins a machine with a pairing code: used up once it's in, and kept for that machine alone when its join didn't
    /// finish, so trying again with the same code finishes it rather than finding it listed already.
    pub(super) async fn admit(&self, key: Key, request: JoinRequest) -> Result<JoinReply> {
        let mut pairing = self.take_pairing(&request.secret)?.filter(|p| p.until > now_ms()).ok_or_else(used)?;
        if pairing.tried.as_ref().is_some_and(|t| *t != hex(&key)) {
            self.keep_pairing(request.secret, pairing);
            return Err(used());
        }
        let admitted = self.admit_with(key, &request, &pairing).await;
        if admitted.is_err() {
            // Once it's listed, the code is that machine's to finish with; before, anyone's who has it.
            if self.with(|i| Ok(i.r.db().machine(&key)?.is_some()))? {
                pairing.tried = Some(hex(&key));
            }
            self.keep_pairing(request.secret, pairing);
        }
        admitted
    }

    fn keep_pairing(&self, secret: [u8; 16], pairing: Pairing) {
        let mut pairings = self.0.pairings.lock().unwrap();
        pairings.insert(secret, pairing);
        if let Err(err) = self.save_pairings(&pairings) {
            tracing::warn!("keeping a half-finished pairing code: {err:#}");
        }
    }

    async fn admit_with(&self, key: Key, request: &JoinRequest, pairing: &Pairing) -> Result<JoinReply> {
        ensure!(request.as_ == "device", "only computers join with a pairing code: sign in at {YOURS}'s address instead");
        let user = pairing.admits.clone().unwrap_or_else(|| pairing.user.clone());
        let (root, bucket) = self.with(|i| Ok((i.r.root().map(|r| r.to_hex()).context("no network")?, i.bucket.as_ref().map(|(_, f)| f.clone()))))?;
        let by = pairing.by.as_deref().map(unhex).transpose()?;
        let endpoint = key::Endpoint { key, seal: key::twin_public(&key)? };
        // A machine of the user's own is their device too. An admin lists another user's as compute, which runs
        // what that admin's listing may (`global`, groups) but never that user's own scope, until a device of theirs
        // lists it again (`scope::derive`'s runners).
        let roles = if user == pairing.user { MACHINE } else { &[scopes::Kind::Compute][..] };
        // Listed already by a join with this code that didn't finish: only the relay's left.
        if !self.with(|i| Ok(i.r.db().machine(&key)?.is_some_and(|m| m.user == user)))? {
            let listed = content::device(&endpoint, &user, &request.name, roles, None);
            self.member_content(&Caller::own(by.unwrap_or(self.0.id), pairing.user.clone()), listed).await.context("this device can no longer admit that machine")?;
        }
        // Waited for: until the relay lets it in, the machine that joined reaches nothing.
        if self.address().is_ok() {
            let admin = self.with(|i| Ok(i.r.db().may(&user, "invite", scopes::NETWORK)))?;
            scope::settings::tell(self, self, json!({ "add": hex(&key), "admin": admin, "role": "compute" })).await.with_context(|| format!("{YOURS}'s network didn't take this machine: try again with the same code"))?;
            // Auth learns whose it is only here, as it never signs in to ask: without this it just can't answer that
            // user's requests yet, so a refusal (a pairing device auth doesn't know either) doesn't stop the join.
            if user == pairing.user
                && let Err(err) = scope::settings::ask_auth(self, "POST", "/api/auth/devices/theirs", json!({ "key": hex(&key), "user": user, "name": request.name })).await
            {
                tracing::warn!("telling auth whose computer this is: {err:#}");
            }
        }
        Ok(JoinReply { root, user, bucket })
    }
}
