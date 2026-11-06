//! Where the network keeps its data: its bucket. It starts as a folder on the device that made the network, which no
//! other machine reaches; an admin moves it to S3-compatible storage, whose credentials only the URL signer
//! holds, with the `storage` setting (an entry only an admin's device writes) that says only where it is, and each
//! device follows: it reaches it through the URL signer, copies there what it reaches of the old one, and keeps its
//! data there from then on. A folder is only ever a machine's own choice, never taken from what others write.

use crate::config::YOURS;
use super::*;

/// The setting that says where the network keeps its data: S3's `endpoint`, `bucket` and `region`, as JSON.
pub const STORAGE: &str = "storage";

/// The network's bucket as its `storage` setting says, if it has one: S3's address alone, never a folder's path.
fn storage_setting(m: &ScopeDb) -> Result<Option<BTreeMap<String, String>>> {
    let Some(value) = m.setting(STORAGE) else { return Ok(None) };
    let fields: BTreeMap<String, String> = serde_json::from_str(&value).context("the storage setting isn't an S3 address")?;
    ensure!(fields.keys().all(|k| ["endpoint", "bucket", "region"].contains(&k.as_str())), "the storage setting names only S3's endpoint, bucket and region");
    ensure!(fields.contains_key("endpoint") && fields.contains_key("bucket"), "the storage setting names S3's endpoint and bucket");
    Ok(Some(fields))
}

impl Node {
    /// Where this device keeps the network's data, as a person sees it.
    pub fn storage(&self) -> Result<Value> {
        self.with(|i| {
            Ok(match &i.bucket {
                Some((Place::Folder(path), _)) => json!({ "kind": "folder", "where": path.display().to_string(), "device": self.device_name() }),
                Some((Place::S3(s3), _)) => json!({ "kind": "s3", "where": s3.bucket, "endpoint": s3.endpoint }),
                None => json!({ "kind": null }),
            })
        })
    }

    /// The network's bucket from the fields this device keeps of it: a folder here, or S3 through the URL signer at the
    /// network's address, signing with this device's key.
    /// A device that joined keeps the address it joined at with them (`address`), until it has read the network's.
    pub(super) fn network_place(&self, fields: &BTreeMap<String, String>) -> Result<Place> {
        let node = Arc::downgrade(&self.0);
        let joined = fields.get("address").cloned();
        let address: crate::bucket::Address = Arc::new(move || {
            let node = Node(node.upgrade().context("this device stopped")?);
            node.with(|i| Ok(i.r.db().setting("address")))?.or_else(|| joined.clone()).with_context(|| format!("{YOURS} has no address yet, where its bucket is reached: an admin sets one with cloudflare use"))
        });
        Place::network(fields, address, ed25519_dalek::SigningKey::from_bytes(self.0.key.as_bytes()))
    }

    /// Moves the network's data, for an admin, to S3-compatible storage (`endpoint`, `bucket`, `region`), whose key the
    /// URL signer holds: reaches it through the signer first, and makes it the network's `storage` setting if
    /// it can.
    pub async fn set_storage(&self, caller: &Caller, body: &Value) -> Result<Value> {
        self.with(|i| {
            ensure!(i.r.db().may(&caller.user, "change-settings", scopes::NETWORK), "only admins move the network's data");
            Ok(())
        })?;
        let fields: BTreeMap<String, String> = ["endpoint", "bucket", "region"]
            .into_iter()
            .filter_map(|k| Some((k.to_string(), body[k].as_str().map(str::trim).filter(|v| !v.is_empty())?.to_string())))
            .collect();
        let place = self.network_place(&fields)?;
        let Place::S3(_) = &place else { bail!("the network's data moves to S3-compatible storage, given its endpoint") };
        place.can_hold_a_network()?;
        let reached = {
            let place = place.clone();
            tokio::task::spawn_blocking(move || bucket_root(&place)).await?
        };
        let steps = vec![json!({ "step": "Reached it through the URL signer", "error": reached.as_ref().err().map(|e| format!("{e:#}")) })];
        if reached.is_err() {
            return Ok(json!({ "tried": steps }));
        }
        let setting = serde_json::to_string(&fields)?;
        self.set_setting(caller, STORAGE, Some(&setting)).await?;
        Ok(json!({ "tried": steps, "kept": true }))
    }

    /// Follows the network's `storage` setting, when this device keeps its data anywhere else: in the background.
    pub(super) fn follow_storage(&self) {
        let wanted = self.with(|i| Ok(storage_setting(i.r.db())?.filter(|f| i.bucket.as_ref().is_none_or(|(_, now)| now.iter().filter(|(k, _)| k.as_str() != "address").ne(f.iter())))));
        let fields = match wanted {
            Ok(Some(fields)) => fields,
            Ok(None) => return,
            Err(err) => return tracing::warn!("storage: {err:#}"),
        };
        if std::mem::replace(&mut *self.0.moving.lock().unwrap(), true) {
            return;
        }
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.move_to(fields).await {
                tracing::warn!("moving the network's data: {err:#}");
            }
            *node.0.moving.lock().unwrap() = false;
        });
    }

    /// Moves this device to the bucket `fields` says, once it reaches it: copies there each object of the bucket it had
    /// that the new one lacks, then keeps the network's data there.
    async fn move_to(&self, fields: BTreeMap<String, String>) -> Result<()> {
        let to = self.network_place(&fields)?;
        let from = self.bucket().ok().map(|b| b.place);
        let root = self.with(|i| i.r.root.map(|r| r.to_hex()).context("no network"))?;
        let (place, theirs) = (to.clone(), root.clone());
        tokio::task::spawn_blocking(move || -> Result<()> {
            place.can_hold_a_network()?;
            if let Some(other) = bucket_root(&place)?.filter(|r| r != theirs.as_bytes()) {
                bail!("that bucket holds another network's data ({})", String::from_utf8_lossy(&other));
            }
            Ok(())
        })
        .await??;
        // Writes wait while it copies, then go to the new bucket, so none is left only in the old one.
        let mut moved = self.0.moved.write().await;
        let (place, theirs) = (to.clone(), root.clone());
        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(from) = from.filter(|f| f.present()) {
                for name in to_move(&from.names()?) {
                    if place.get(&name)?.is_none() {
                        if let Some(bytes) = from.get(&name)? {
                            place.create(&name, &bytes)?;
                        }
                    }
                }
            }
            place.create(NETWORK_OBJECT, theirs.as_bytes())?;
            Ok(())
        })
        .await??;
        let dir = self.network_dir();
        let mut file: NetworkFile = serde_json::from_slice(&std::fs::read(dir.join(NETWORK_FILE))?)?;
        file.bucket = Some(fields.clone());
        self.save_network(&file)?;
        self.with(|i| {
            i.bucket = Some((to.clone(), fields));
            Ok(())
        })?;
        *moved = Some(to);
        drop(moved);
        // What it read of the old bucket stays true, but is read again from the new one.
        self.close_dbs(|_, _| true);
        tracing::info!("the network's data is kept in {} now", self.storage()?["where"].as_str().unwrap_or(""));
        self.emit(Event::Changed { apps: Vec::new() });
        Ok(())
    }

    pub async fn storage_route(&self, me: &Caller, method: &str, path: &str, body: &Value) -> Result<Option<Value>> {
        Ok(Some(match (method, path) {
            ("GET", "/v1/storage") => self.storage()?,
            ("POST", "/v1/storage") => self.set_storage(me, body).await?,
            _ => return Ok(None),
        }))
    }
}

/// Of `names`, every object of the bucket being moved from, sorted, those moving copies, in that order: each database's
/// and log's steps only up to the first position with nothing there, with the signature beside each and the copies
/// after them; everything else as it is. The URL signer creates a step only once the one before it is there (or the
/// copy up to it), and a copy only once its step is, and no reader reads past a position with nothing: what an older
/// writer left past one stays behind, and the next step is written there. A database whose older steps were deleted
/// (`stored::tidied_name`) moves whole, that first: its readers go on past a gap from a copy that holds, and the URL signer
/// creates any step of it.
fn to_move(names: &[String]) -> Vec<String> {
    // A database's prefix and position, of its copy (true), or of its step or the signature beside one: a copy's name is
    // a step's of a prefix ending in `copy`, never taken for one.
    fn of(name: &str) -> Option<(&str, u64, bool)> {
        if let Some((folder, last)) = name.rsplit_once('/')
            && let Some(prefix) = folder.strip_suffix("/copy")
            && let Ok(n) = last.parse()
            && crate::database::stored::copy_name(prefix, n) == name
        {
            return Some((prefix, n, true));
        }
        crate::database::stored::position(name).map(|(prefix, n)| (prefix, n, false))
    }
    let steps: BTreeSet<(&str, u64)> = names.iter().filter_map(|n| of(n).filter(|(p, at, copy)| !copy && crate::database::stored::step_name(p, *at) == *n)).map(|(p, at, _)| (p, at)).collect();
    let tidied: BTreeSet<&str> = names.iter().filter_map(|n| n.strip_suffix("/tidied")).collect();
    let mut ends: BTreeMap<&str, u64> = BTreeMap::new();
    for (prefix, _) in steps.iter().filter(|(p, _)| !tidied.contains(p)) {
        let end = ends.entry(prefix).or_insert(0);
        while steps.contains(&(prefix, *end + 1)) {
            *end += 1;
        }
    }
    let moving = names.iter().filter(|name| of(name).is_none_or(|(prefix, at, _)| tidied.contains(prefix) || ends.get(prefix).is_some_and(|end| at <= *end)));
    let (first, rest): (Vec<&String>, Vec<&String>) = moving.partition(|n| n.ends_with("/tidied"));
    first.into_iter().chain(rest).cloned().collect()
}

#[cfg(test)]
mod tests {
    /// Moving copies what every reader reads, in an order the URL signer takes: no step or copy past a position with
    /// nothing, which an older writer left, and which no reader reads past; and all of a database whose older steps were
    /// deleted, which its readers read on past from a copy.
    #[test]
    fn moving_copies_each_database_s_steps_up_to_a_gap() {
        let (log, db, old) = ("global/one/log", "user:a/notes/notes", "user:a/notes/old");
        let mut names: Vec<String> = [
            crate::database::stored::step_name(log, 1),
            crate::database::stored::step_name(log, 2),
            crate::database::stored::step_name(log, 4),
            crate::database::stored::copy_name(log, 2),
            crate::database::stored::copy_name(log, 4),
            crate::database::stored::newest_name(log),
            crate::database::stored::step_name(db, 1),
            crate::database::stored::sig_name(db, 1),
            crate::database::stored::sig_name(db, 3),
            crate::database::stored::step_name(db, 3),
            crate::database::stored::step_name(db, 1_000_000),
            crate::database::stored::tidied_name(old),
            crate::database::stored::step_name(old, 200),
            crate::database::stored::sig_name(old, 200),
            crate::database::stored::step_name(old, 201),
            crate::database::stored::copy_name(old, 200),
            crate::database::stored::newest_name(old),
            "user:a/blobs/abc".to_string(),
            "network".to_string(),
        ]
        .into();
        names.sort();
        let moved = super::to_move(&names);
        let want = [crate::database::stored::step_name(log, 1), crate::database::stored::step_name(log, 2), crate::database::stored::copy_name(log, 2), crate::database::stored::newest_name(log), crate::database::stored::step_name(db, 1), crate::database::stored::sig_name(db, 1), "user:a/blobs/abc".to_string(), "network".to_string()];
        let whole = names.iter().filter(|n| n.starts_with(&format!("{old}/")));
        assert_eq!(moved.iter().collect::<std::collections::BTreeSet<_>>(), want.iter().chain(whole).collect(), "and a database whose older steps were deleted, whole");
        // As the URL signer creates them (`cloudflare/signer/worker.js`'s `after`), one after another in that order.
        let mut there = std::collections::BTreeSet::new();
        for name in &moved {
            let due = match (name.rsplit_once("/copy/"), crate::database::stored::position(name).filter(|(p, at)| crate::database::stored::step_name(p, *at) == *name)) {
                (Some((prefix, n)), _) => there.contains(&crate::database::stored::step_name(prefix, n.parse().unwrap())),
                (None, Some((prefix, at))) => at == 1 || [crate::database::stored::step_name(prefix, at - 1), crate::database::stored::copy_name(prefix, at - 1), crate::database::stored::tidied_name(prefix)].iter().any(|n| there.contains(n)),
                (None, None) => true,
            };
            assert!(due, "the URL signer refuses {name} then");
            there.insert(name.clone());
        }
    }
}
