//! Membership: who is in a space, rebuilt by replaying its `space` entries. See SPEC.md, "Membership".

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use p2panda_core::{Signature, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::entry::{Content, Entry, Key, Order, hex, unhex, verifying};

pub const APP: &str = "space";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
}

#[derive(Clone, Debug, Serialize)]
pub struct Member {
    #[serde(serialize_with = "ser_hex")]
    pub device: Key,
    pub user: String,
    pub name: String,
    pub role: Role,
    /// A device that keeps no log (a phone): others ask it nothing and don't sync with it.
    pub client: bool,
    /// Every name it had, from when: apps see the name a device had when it wrote.
    #[serde(skip)]
    pub names: Vec<(Order, String)>,
    #[serde(skip)]
    pub since: Order,
    #[serde(skip)]
    pub until: Option<Order>,
}

fn ser_hex<S: serde::Serializer>(k: &Key, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex(k))
}

#[derive(Clone, Debug, Default)]
pub struct Members {
    pub name: String,
    pub devices: BTreeMap<Key, Member>,
}

impl Member {
    /// The name this device had at `order`.
    pub fn name_at(&self, order: Order) -> &str {
        self.names.iter().rev().find(|(since, _)| *since <= order).map_or(&self.name, |(_, n)| n)
    }
}

impl Members {
    /// Replays a space's entries (any order is fine; only `space` entries count).
    pub fn replay(space: &Key, entries: &[Entry]) -> Members {
        let mut members = Members::default();
        let mut sorted: Vec<&Entry> = entries.iter().filter(|e| e.content.app == APP).collect();
        sorted.sort_by_key(|e| e.order());
        for entry in sorted {
            if let Err(err) = members.apply(space, entry) {
                tracing::debug!("ignored {} entry: {err:#}", entry.content.kind);
            }
        }
        members
    }

    fn apply(&mut self, space: &Key, entry: &Entry) -> Result<()> {
        let author = entry.author();
        let data = &entry.content.data;
        let text = |key: &str| data[key].as_str().map(str::to_string).with_context(|| format!("no {key}"));
        match entry.content.kind.as_str() {
            "genesis" => {
                ensure!(self.devices.is_empty(), "space already has a genesis");
                let name = text("name")?;
                let sig = data_encoding::HEXLOWER.decode(text("root_sig")?.as_bytes())?;
                let sig = Signature::from_bytes(&sig.try_into().map_err(|_| anyhow::anyhow!("bad signature length"))?);
                ensure!(verifying(space)?.verify(&root_message(&author, &name), &sig), "bad root signature");
                self.name = name;
                let user = text("user")?;
                let device_name = text("device_name")?;
                let names = vec![(entry.order(), device_name.clone())];
                self.devices.insert(author, Member { device: author, user, name: device_name, role: Role::Admin, client: false, names, since: entry.order(), until: None });
            }
            "admit" => {
                let by = self.active(&author, entry.order()).context("admitter is not a member")?.clone();
                let device = unhex(&text("device")?)?;
                let user = text("user")?;
                let role: Role = serde_json::from_value(data["role"].clone())?;
                ensure!(by.role == Role::Admin || (user == by.user && role == Role::Member), "not allowed to admit");
                if self.active(&device, entry.order()).is_some() {
                    return Ok(());
                }
                let name = text("name")?;
                let client = data["client"].as_bool().unwrap_or(false);
                ensure!(!self.devices.contains_key(&device), "a removed device can't come back; it joins again with a new key");
                let names = vec![(entry.order(), name.clone())];
                self.devices.insert(device, Member { device, user, name, role, client, names, since: entry.order(), until: None });
            }
            "remove" => {
                let by = self.active(&author, entry.order()).context("remover is not a member")?;
                ensure!(by.role == Role::Admin, "only admins remove");
                let device = unhex(&text("device")?)?;
                if let Some(m) = self.devices.get_mut(&device) {
                    m.until.get_or_insert(entry.order());
                }
            }
            "rename" => {
                let by = self.active(&author, entry.order()).context("renamer is not a member")?.role.clone();
                let device = unhex(&text("device")?)?;
                ensure!(device == author || by == Role::Admin, "not allowed to rename");
                let name = text("name")?;
                if let Some(m) = self.devices.get_mut(&device) {
                    m.names.push((entry.order(), name.clone()));
                    m.name = name;
                }
            }
            other => bail!("unknown kind {other}"),
        }
        Ok(())
    }

    /// The member `device` was at `order`, if it was one.
    pub fn active(&self, device: &Key, order: Order) -> Option<&Member> {
        self.devices.get(device).filter(|m| m.since <= order && m.until.is_none_or(|u| order < u))
    }

    /// Whether `device` is a member now.
    pub fn current(&self, device: &Key) -> Option<&Member> {
        self.devices.get(device).filter(|m| m.until.is_none())
    }

    pub fn current_devices(&self) -> impl Iterator<Item = &Member> {
        self.devices.values().filter(|m| m.until.is_none())
    }

    /// Whether `device` was, at `order`, one of an admin's own devices: a member whose user had an admin device then.
    pub fn admins(&self, device: &Key, order: Order) -> bool {
        let Some(m) = self.active(device, order) else { return false };
        self.devices.keys().filter_map(|d| self.active(d, order)).any(|a| a.role == Role::Admin && a.user == m.user)
    }
}

pub fn root_message(device: &Key, name: &str) -> Vec<u8> {
    [device.as_slice(), name.as_bytes()].concat()
}

pub fn genesis(root: &SigningKey, device: &Key, name: &str, user: &str, device_name: &str) -> Content {
    let sig = root.sign(&root_message(device, name));
    Content {
        app: APP.into(),
        kind: "genesis".into(),
        data: json!({ "name": name, "root_sig": data_encoding::HEXLOWER.encode(&sig.to_bytes()), "user": user, "device_name": device_name }),
    }
}

pub fn admit(device: &Key, user: &str, name: &str, role: Role) -> Content {
    Content { app: APP.into(), kind: "admit".into(), data: json!({ "device": hex(device), "user": user, "name": name, "role": role }) }
}

/// Admits a device that keeps no log, such as a phone.
pub fn admit_client(device: &Key, user: &str, name: &str) -> Content {
    let mut c = admit(device, user, name, Role::Member);
    c.data["client"] = json!(true);
    c
}

pub fn remove(device: &Key) -> Content {
    Content { app: APP.into(), kind: "remove".into(), data: json!({ "device": hex(device) }) }
}

pub fn rename(device: &Key, name: &str) -> Content {
    Content { app: APP.into(), kind: "rename".into(), data: json!({ "device": hex(device), "name": name }) }
}

pub fn as_json(members: &Members) -> Value {
    json!({ "name": members.name, "devices": members.current_devices().collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Entry;

    struct Dev {
        key: SigningKey,
        last: Option<Entry>,
        time: u64,
    }

    impl Dev {
        fn new(seed: u8) -> Self {
            Dev { key: SigningKey::from([seed; 32]), last: None, time: 0 }
        }
        fn id(&self) -> Key {
            *self.key.verifying_key().as_bytes()
        }
        fn write(&mut self, space: Key, at: u64, content: Content) -> Entry {
            self.time = self.time.max(at);
            let e = Entry::create(&self.key, space, self.time, self.last.as_ref(), content);
            self.last = Some(e.clone());
            e
        }
    }

    fn setup() -> (SigningKey, Key, Dev, Vec<Entry>) {
        let root = SigningKey::from([9; 32]);
        let space = *root.verifying_key().as_bytes();
        let mut admin = Dev::new(1);
        let id = admin.id();
        let g = admin.write(space, 1, genesis(&root, &id, "Home", "andre", "mac"));
        (root, space, admin, vec![g])
    }

    #[test]
    fn genesis_needs_the_space_key() {
        let (_, space, _, _) = setup();
        let mut impostor = Dev::new(2);
        let id = impostor.id();
        let fake = impostor.write(space, 1, genesis(&SigningKey::from([7; 32]), &id, "Home", "x", "y"));
        assert!(Members::replay(&space, &[fake]).devices.is_empty());
    }

    #[test]
    fn admins_admit_people_and_members_admit_only_their_own_devices() {
        let (_, space, mut admin, mut entries) = setup();
        let (mut phone, mut stranger, laptop) = (Dev::new(3), Dev::new(4), Dev::new(5));
        entries.push(admin.write(space, 2, admit(&phone.id(), "andre", "phone", Role::Member)));
        entries.push(admin.write(space, 3, admit(&stranger.id(), "sam", "sam's", Role::Member)));
        // The phone may add another of Andre's devices, but not someone else's, and not an admin.
        entries.push(phone.write(space, 4, admit(&laptop.id(), "andre", "laptop", Role::Member)));
        entries.push(phone.write(space, 5, admit(&Dev::new(6).id(), "eve", "eve's", Role::Member)));
        entries.push(stranger.write(space, 6, admit(&Dev::new(7).id(), "sam", "sam's admin", Role::Admin)));
        let m = Members::replay(&space, &entries);
        let names: Vec<&str> = m.current_devices().map(|d| d.name.as_str()).collect();
        assert_eq!(m.current_devices().count(), 4, "{names:?}");
        assert!(m.current(&laptop.id()).is_some());
        assert!(m.current(&Dev::new(6).id()).is_none());
        assert!(m.current(&Dev::new(7).id()).is_none());
    }

    #[test]
    fn an_admins_other_devices_are_theirs_too() {
        let (_, space, mut admin, mut entries) = setup();
        let (laptop, stranger) = (Dev::new(3), Dev::new(4));
        entries.push(admin.write(space, 2, admit(&laptop.id(), "andre", "laptop", Role::Member)));
        entries.push(admin.write(space, 3, admit(&stranger.id(), "sam", "sam's", Role::Member)));
        entries.push(admin.write(space, 10, remove(&admin.id())));
        let m = Members::replay(&space, &entries);
        assert!(m.admins(&laptop.id(), (5, laptop.id(), 0)));
        assert!(!m.admins(&stranger.id(), (5, stranger.id(), 0)));
        assert!(!m.admins(&laptop.id(), (11, laptop.id(), 0)), "not once the user has no admin device");
    }

    #[test]
    fn a_removed_device_is_cut_off_from_then_on() {
        let (_, space, mut admin, mut entries) = setup();
        let phone = Dev::new(3);
        entries.push(admin.write(space, 2, admit(&phone.id(), "andre", "phone", Role::Member)));
        entries.push(admin.write(space, 10, remove(&phone.id())));
        let m = Members::replay(&space, &entries);
        assert!(m.current(&phone.id()).is_none());
        assert!(m.active(&phone.id(), (5, phone.id(), 0)).is_some(), "what it wrote before removal still counts");
        assert!(m.active(&phone.id(), (11, phone.id(), 0)).is_none(), "nothing it writes after does");
    }

    #[test]
    fn a_removed_device_cant_be_admitted_again() {
        let (_, space, mut admin, mut entries) = setup();
        let phone = Dev::new(3);
        entries.push(admin.write(space, 2, admit(&phone.id(), "andre", "phone", Role::Member)));
        entries.push(admin.write(space, 3, remove(&phone.id())));
        entries.push(admin.write(space, 4, admit(&phone.id(), "andre", "phone", Role::Member)));
        let m = Members::replay(&space, &entries);
        assert!(m.current(&phone.id()).is_none());
        assert!(m.active(&phone.id(), (2, phone.id(), 9)).is_some(), "what it wrote while a member still counts");
    }

    #[test]
    fn apps_see_the_name_a_device_had_when_it_wrote() {
        let (_, space, mut admin, mut entries) = setup();
        entries.push(admin.write(space, 5, rename(&admin.id(), "new name")));
        let m = Members::replay(&space, &entries);
        let me = m.current(&admin.id()).unwrap();
        assert_eq!(me.name_at((3, admin.id(), 0)), "mac");
        assert_eq!(me.name_at((6, admin.id(), 0)), "new name");
        assert_eq!(me.name, "new name");
    }

    #[test]
    fn replay_order_doesnt_depend_on_arrival_order() {
        let (_, space, mut admin, mut entries) = setup();
        entries.push(admin.write(space, 2, admit(&Dev::new(3).id(), "andre", "phone", Role::Member)));
        entries.push(admin.write(space, 3, rename(&Dev::new(3).id(), "Andre's phone")));
        let forwards = Members::replay(&space, &entries);
        entries.reverse();
        let backwards = Members::replay(&space, &entries);
        assert_eq!(format!("{:?}", forwards.devices), format!("{:?}", backwards.devices));
    }
}
