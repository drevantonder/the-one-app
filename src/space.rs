//! Membership: who is in a space, folded from its `space` entries. See SPEC.md, "Membership".

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
    pub person: String,
    pub name: String,
    pub role: Role,
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

impl Members {
    /// Folds a space's entries (any order is fine; only `space` entries count).
    pub fn fold(space: &Key, entries: &[Entry]) -> Members {
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
                let person = text("person")?;
                let device_name = text("device_name")?;
                self.devices.insert(author, Member { device: author, person, name: device_name, role: Role::Admin, since: entry.order(), until: None });
            }
            "admit" => {
                let by = self.active(&author, entry.order()).context("admitter is not a member")?.clone();
                let device = unhex(&text("device")?)?;
                let person = text("person")?;
                let role: Role = serde_json::from_value(data["role"].clone())?;
                ensure!(by.role == Role::Admin || (person == by.person && role == Role::Member), "not allowed to admit");
                if self.active(&device, entry.order()).is_some() {
                    return Ok(());
                }
                let name = text("name")?;
                self.devices.insert(device, Member { device, person, name, role, since: entry.order(), until: None });
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
}

pub fn root_message(device: &Key, name: &str) -> Vec<u8> {
    [device.as_slice(), name.as_bytes()].concat()
}

pub fn genesis(root: &SigningKey, device: &Key, name: &str, person: &str, device_name: &str) -> Content {
    let sig = root.sign(&root_message(device, name));
    Content {
        app: APP.into(),
        kind: "genesis".into(),
        data: json!({ "name": name, "root_sig": data_encoding::HEXLOWER.encode(&sig.to_bytes()), "person": person, "device_name": device_name }),
    }
}

pub fn admit(device: &Key, person: &str, name: &str, role: Role) -> Content {
    Content { app: APP.into(), kind: "admit".into(), data: json!({ "device": hex(device), "person": person, "name": name, "role": role }) }
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
