//! Devices, as an app runtime runs them: serving their tools (`tools`), erasing what this device keeps, and who holds
//! the network's recovery key (`routes`). Letting devices in and taking them out, pairing and the recovery key's own
//! changes are what its device signs (the runtime's `settings`), so they're asked of it through
//! `Holds::answer_as_device`.

mod routes;
mod tools;

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, ensure};
use api::Caller;
use api::runtime::Runs;
use api::runtime::prelude::*;
use custom_build::allowed;
use key::hex;
use scope::scopes;
use serde_json::{Value, json};

pub fn runs() -> Runs {
    Runs { tools: tools::served(), routes: routes::routes(), state: Box::new(State::default()), ..Runs::named("devices") }
}

/// What it keeps while its runtime runs.
#[derive(Default)]
pub struct State {
    /// Asked to erase what this device keeps (`erase`): its oned does, then stops.
    erasing: AtomicBool,
}

/// Whether this device was asked to erase what it keeps, which its oned does next.
pub fn erasing(rt: &Rt) -> bool {
    rt.state::<State>().erasing.load(Ordering::SeqCst)
}

/// Erases what this device keeps (by name or key), for its user or an admin asking from one of their own devices,
/// directly: asked of another device, it says to ask that one, once it's reached it. Once it has answered, its oned
/// stops its parts and deletes its home (its copies of the network's data, the pages it kept, and each service's
/// folder and data, WhatsApp's link among them), then stops. Removing it next takes it out. → what's erasing.
pub async fn erase(rt: &Rt, caller: &Caller, device: &str) -> Result<Value> {
    let key = rt.read(|r| r.db().find_machine(device))?;
    let name = rt.read(|r| r.db().machine(&key)?.map(|d| r.db().device_name(&d)).transpose())?.unwrap_or_default();
    let me = rt.id();
    if key != me {
        ensure!(rt.reaches(&key).await, "{}, so it can't erase what it keeps. Turn it on and try again.", rt.away(&key));
        return Err(api::Elsewhere { why: format!("only {name} erases what it keeps"), runtimes: vec![hex(&key)] }.into());
    }
    rt.read(|r| {
        let db = r.db();
        allowed!(caller.key != me && db.client(&caller.key)?.is_some() && db.may(&caller.user, "remove", &scopes::device(&key)), "only {name}'s user or an admin erases it, from another of their devices");
        Ok(())
    })?;
    tracing::warn!("asked to erase what {name} keeps: its oned does, then stops");
    rt.state::<State>().erasing.store(true, Ordering::SeqCst);
    Ok(json!({ "erasing": name }))
}

/// Who holds the recovery key, for any user of the network, as who's who says it does and what the bucket has for
/// them: `{ recovery, holders: [{ user, name, devices: [{ key, seal, name, holds }] }] }`. A device with `holds` false
/// waits for someone with the key to seal it (`devices::cli`).
pub async fn recovery_holders(rt: &Rt, caller: &Caller) -> Result<Value> {
    ensure!(rt.read_or(false, |r| Ok(r.db().user(&caller.user)?.is_some())), "only the network's users read who holds its recovery key");
    let (recovery, holders) = rt.read(|r| {
        let db = r.db();
        let mut holders = Vec::new();
        for user in db.recovery_holders()? {
            let name = db.user(&user)?.map(|u| u.name).unwrap_or_default();
            holders.push((user.clone(), name, db.recovery_devices(&user)?));
        }
        Ok((db.recovery()?.as_ref().map(hex), holders))
    })?;
    let mut out = Vec::new();
    for (user, name, devices) in holders {
        let mut listed = Vec::new();
        for d in devices {
            let holds = match &recovery {
                Some(r) => rt.object(&database::keys::recovery_held_name(r, &hex(&d.key))).await?.is_some_and(|o| key::unhex(r).is_ok_and(|key| key::held_sealed(&key, &hex(&d.key), &o).is_some())),
                None => false,
            };
            listed.push(json!({ "key": hex(&d.key), "seal": hex(&d.seal), "name": d.name, "holds": holds }));
        }
        out.push(json!({ "user": user, "name": name, "devices": listed }));
    }
    Ok(json!({ "recovery": recovery, "holders": out }))
}
