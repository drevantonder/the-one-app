//! Pairing: a code that lets a new device join the install, and admitting it.

use super::*;

impl Node {
    /// Makes a one-time pairing code for another of the local user's machines, which joins as a device.
    pub fn pair(&self) -> Result<String> {
        Ok(self.pair_as(&Caller { key: self.0.id, user: self.user()?, agent: None, token: None }, None)?.0)
    }

    /// Like `pair`, for `asker`, or for an admin, a code for another machine of `user` (an id or a name). With the id
    /// of the user the code admits.
    pub fn pair_as(&self, asker: &Caller, user: Option<&str>) -> Result<(String, String)> {
        let user = user.map(|u| self.find_user(u)).transpose()?;
        self.with(|i| {
            let m = &i.r.members;
            ensure!(m.compute(&self.0.id).is_some(), "this device isn't in {} yet", crate::config::YOURS);
            ensure!(user.as_ref().is_none_or(|u| *u == asker.user) || m.is_admin(&asker.user), "only admins pair other people's machines");
            ensure!(m.serves(&self.0.id, &asker.user), "{} doesn't serve you", self.device_name());
            Ok(())
        })?;
        let address = self.with(|i| Ok(i.r.members.settings.get("address").cloned()))?.context("a computer joins through your One App's address, and it has none yet: an admin sets it with kernel use")?;
        let joins_as = user.clone().unwrap_or_else(|| asker.user.clone());
        let secret: [u8; 16] = rand::random();
        let by = (asker.key != self.0.id).then(|| hex(&asker.key));
        self.add_pairing(secret, Pairing { user: asker.user.clone(), by, admits: user, until: now_ms() + PAIRING_LIFE })?;
        self.tell_relay(json!({ "pairing": { "hash": net::ticket_hash(&net::pairing_ticket(&secret)), "until": now_ms() + PAIRING_LIFE } }));
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

    /// Joins the install a pairing code is for, as a device.
    pub async fn join(&self, code: &str) -> Result<()> {
        let bytes = data_encoding::BASE32_NOPAD.decode(code.trim().to_uppercase().as_bytes()).context("that isn't a pairing code")?;
        let code: PairingCode = ciborium::from_reader(bytes.as_slice()).context("that isn't a pairing code")?;
        self.ensure_no_install()?;
        let request = JoinRequest { secret: code.secret, name: self.device_name(), as_: "device".into() };
        let reply = self.ask_to_join(&code.address, &net::pairing_ticket(&code.secret), code.from, serde_json::to_value(&request)?).await.context("couldn't reach the device that made the pairing code")?;
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("the pairing code was refused"));
        }
        let reply: JoinReply = serde_json::from_value(reply)?;
        // A folder on another machine isn't here: it's this install's bucket only if it says so. S3 is reached through
        // the URL signer, which this device can ask once it knows the install's address.
        let here = |f: &BTreeMap<String, String>| Place::from_fields(f).ok().and_then(|p| p.get(INSTALL_OBJECT).ok().flatten()).is_some_and(|r| r == reply.root.as_bytes());
        let bucket = reply.bucket.filter(|f| f.contains_key("endpoint") || here(f));
        if bucket.is_none() {
            tracing::warn!("this device can't reach the install's bucket: it keeps no app's data until the install moves to S3-compatible storage");
        }
        self.save_install(&InstallFile { root: reply.root, user: reply.user, bucket, folders: BTreeMap::new() })?;
        self.open_install()?;
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.read_logs().await {
                tracing::debug!("reading the bucket after joining: {err:#}");
            }
        });
        self.emit(Event::Install);
        Ok(())
    }

    pub(super) async fn admit(&self, key: Key, request: JoinRequest) -> Result<JoinReply> {
        let pairing = self.take_pairing(&request.secret)?.filter(|p| p.until > now_ms()).context("this pairing code was already used or has expired")?;
        // Phones joined as clients before they signed in at your One App's address: only devices join now.
        ensure!(request.as_ == "device", "only computers join with a pairing code: sign in at your One App's address instead");
        let user = pairing.admits.clone().unwrap_or_else(|| pairing.user.clone());
        let (root, bucket) = self.with(|i| Ok((i.r.root.map(|r| r.to_hex()).context("no install")?, i.bucket.as_ref().map(|(_, f)| f.clone()))))?;
        let by = pairing.by.as_deref().map(unhex).transpose()?;
        let endpoint = crate::keys::Endpoint { key, seal: crate::kernel::twin_public(&key)? };
        // A machine of the person's own is their device too. An admin lists another person's as compute, which runs
        // what that admin's listing may (`global`, groups) but never that person's own scope, until a device of theirs
        // lists it again (`Members::runs_for`).
        let roles = if user == pairing.user { MACHINE } else { &[members::Kind::Compute][..] };
        let listed = members::device(&endpoint, &user, &request.name, roles, None);
        self.member_content(&Caller { key: by.unwrap_or(self.0.id), user: pairing.user.clone(), agent: None, token: None }, listed).await.context("this device can no longer admit that machine")?;
        self.tell_relay(json!({ "add": hex(&key), "role": "compute" }));
        Ok(JoinReply { root, user, bucket })
    }
}
