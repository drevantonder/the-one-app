//! Services, as an app runtime runs them: switching one on or off on a device, what this computer's oned keeps its
//! parts by (`parts_wanted`), what an app declares its service may reach, listing a part as this device's, and naming
//! the scope of each service switched on before services did (`routes`). Carrying asks to its parts and back is the
//! runtime's own, over what devices speak to each other.

mod routes;

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use api::Caller;
use api::runtime::Runs;
use api::runtime::prelude::*;
use custom_build::config::YOURS;
use key::hex;
use scope::content;
use scope::scopes::{self, Scope};
use services::sandbox::Declared;
use services::{Network, Wanted, added_scope};

/// How often it looks for a service switched on here before services named their app's scope.
const NAMING_EVERY: Duration = Duration::from_secs(10);

pub fn runs() -> Runs {
    Runs { routes: routes::routes(), every: Some((NAMING_EVERY, |rt| Box::pin(name_services_scopes(rt)))), ..Runs::named("services") }
}

/// Switches a part of a device's oned (a service, or `apps`, the app runtime) on or off there (this device if none is
/// named), for the device's user: that device's oned starts or stops it.
pub async fn set_service(rt: &Rt, caller: &Caller, device: Option<&str>, name: &str, on: bool) -> Result<()> {
    ensure!(device.is_some() || caller.key == rt.id(), "say which device");
    let device = device.map(str::to_string).unwrap_or_else(|| rt.device_name());
    let content = rt.read(|r| {
        let scope = if on { added_scope(r, &caller.user, name)? } else { None };
        scope::settings::service(r.db(), &caller.user, &device, name, on, scope.as_ref())
    })?;
    rt.append(&Scope::Global, &caller.user, content).await
}

/// What this device's oned keeps its parts by (`oned::keep_parts`): the network they're in, which are switched on here
/// (each with when it was: switched on again while it's on, it starts afresh) and off, and whether to erase what the
/// device keeps.
pub fn parts_wanted(rt: &Rt) -> Result<Wanted> {
    let address = rt.address()?;
    let user = rt.caller()?.user;
    let device = rt.device();
    let erase = devices_runtime::erasing(rt);
    let name = rt.device_name();
    rt.read(|r| {
        let root = r.replica.root().with_context(|| format!("this device isn't in {YOURS} yet"))?.to_hex();
        let switches = r.db().switches()?;
        let here = switches.into_iter().filter(|((k, _), _)| *k == device);
        let (on, off): (Vec<_>, Vec<_>) = here.partition(|(_, s)| s.on);
        Ok(Wanted {
            network: Network { address, root, user },
            device: name,
            on: on.into_iter().map(|((_, name), s)| (name, s.time)).collect(),
            off: off.into_iter().map(|((_, name), _)| name).collect(),
            erase,
        })
    })
}

/// Each service switched on here before a service named the scope of the app it runs for, switched on again naming
/// it, once: so removing the app, or moving its service, reaches it. Each goes on running throughout.
async fn name_services_scopes(rt: Rt) -> Result<()> {
    let caller = rt.caller()?;
    let device = rt.device();
    let named = rt.read(|r| {
        let m = r.db();
        let switches = m.switches()?;
        let unnamed = switches.iter().filter(|((k, name), s)| *k == device && s.on && s.scope.is_none() && name != scopes::APPS_PART).map(|((_, name), _)| name.clone());
        let mut entries = Vec::new();
        for name in unnamed {
            if let Some(scope) = added_scope(r, &caller.user, &name)?
                && let Ok(entry) = scope::settings::service(m, &caller.user, &hex(&device), &name, true, Some(&scope))
            {
                entries.push(entry);
            }
        }
        Ok(entries)
    });
    for content in named.unwrap_or_default() {
        if let Err(err) = rt.append(&Scope::Global, &caller.user, content).await {
            tracing::info!("naming a service's scope: {err:#}");
        }
    }
    Ok(())
}

/// What the app `name` this device runs says of its service's wrapper (`service: { network, user }` in its app.js), or
/// why it can't be read: the app isn't running here, or its deploy says nothing of it.
pub async fn service_declared(rt: &Rt, name: &str) -> Result<Declared> {
    let d = rt.describe(name).await.context("its app isn't running on this device")?;
    ensure!(d["service"].is_object(), "its app doesn't say what its service may reach");
    Ok(serde_json::from_value(d["service"].clone())?)
}

/// Lists one of this device's parts, the service `service`, as its user's, a part of this device (oned turned it on,
/// and it asked with its key and seal key). It holds no keys: it reads who's who from this device's runtime, which says
/// who may ask it. A part listed before it named its device is listed again, naming it.
pub async fn list_part(rt: &Rt, endpoint: &key::Endpoint, service: &str) -> Result<()> {
    let user = rt.caller()?.user;
    let device = rt.device();
    let listed = rt.read(|r| {
        let db = r.db();
        ensure!(db.service(&endpoint.key)?.is_some() || db.machine_ever(&endpoint.key)?.is_none(), "that key is listed already, and not as a service");
        Ok(db.service(&endpoint.key)?.map(|s| s.device == Some(device)))
    })?;
    if listed != Some(true) {
        rt.append(&Scope::Global, &user, content::listed_service(endpoint, &user, &rt.device_name(), service, &device)).await?;
    }
    Ok(())
}
