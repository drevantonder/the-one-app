//! The serverless node (`serverless/`) on Cloudflare: a node of the install as a Worker in its kernel's account, with
//! no address of its own, which the gateway sends `/v1/` to by service binding, and an iroh endpoint of its
//! own key, which the relay wakes. A release carries its built
//! WebAssembly (`update` builds it, where Rust builds for WebAssembly); the apps it runs it reads from the install's
//! bucket, each at its latest deploy.

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::kernel::{Cloudflare, Settings, Uploaded, worker_name};

/// The serverless node's Worker is `<cli>-node`.
pub const SERVICE: &str = "node";

/// The serverless node's Worker's modules: (name, type, bytes), `worker.js` first. `built.js` is the shim each app runs
/// in (the apps themselves it reads from the bucket, each at its latest deploy); `icons.json` the icons every device
/// draws.
pub fn modules() -> Result<Vec<(String, &'static str, Vec<u8>)>> {
    let built = |name: &str| {
        let found = crate::config::SERVERLESS.iter().find(|(n, _)| *n == name).map(|(_, b)| b.to_vec());
        found.with_context(|| format!("this build of {} has no serverless node: {} update builds one, on a Linux device with Rust", crate::config::NAME, crate::config::CLI))
    };
    Ok(modules_from(built("one_serverless.js")?, built("one_serverless_bg.wasm")?))
}

/// Its modules, from its WebAssembly's glue and the WebAssembly, as wasm-bindgen makes them.
pub fn modules_from(glue: Vec<u8>, wasm: Vec<u8>) -> Vec<(String, &'static str, Vec<u8>)> {
    let shim = include_str!("runtime/shim.js");
    let built_js = format!("export const shim = {};\nexport const compatibilityDate = {};\n", json!(shim), json!(crate::apps::COMPATIBILITY_DATE));
    vec![
        ("worker.js".into(), "application/javascript+module", include_bytes!("../serverless/worker.js").to_vec()),
        ("one_serverless.js".into(), "application/javascript+module", glue),
        ("one_serverless_bg.wasm".into(), "application/wasm", wasm),
        ("built.js".into(), "application/javascript+module", built_js.into_bytes()),
        ("icons.json".into(), "text/plain", include_bytes!("../vendor/phosphor/icons.json").to_vec()),
    ]
}

/// It as the kernel's workerd runs it here, from `modules`, beside the kernel's services.
pub fn served(settings: &Settings, mut modules: Vec<(String, &'static str, Vec<u8>)>) -> crate::kernel::Served {
    let (_, _, main) = modules.remove(0);
    crate::kernel::Served {
        name: SERVICE.into(),
        module: main,
        modules: modules.into_iter().map(|(name, _, bytes)| (name, bytes)).collect(),
        class: Some("Node".into()),
        listen: None,
        bindings: settings.bindings(SERVICE).into_iter().map(|(n, v, _)| (n, v)).collect(),
        assets: None,
        // Its node on a machine, and a try install's bucket, are on this machine.
        outbound: Some(crate::kernel::NEARBY.to_string()),
    }
}

/// What its upload says besides its modules: its bindings (`Settings::bindings`), its endpoint's Durable Object class
/// `Node` and that class's migration the first time, and its logs on.
pub fn metadata(settings: &Settings, migrated: bool, exists: impl Fn(&str) -> bool) -> Value {
    let (mut meta, _) = crate::kernel::metadata(settings, SERVICE, Some("Node"), migrated, exists);
    // Its logs say which calls ran here, for whoever owns the account.
    meta["observability"] = json!({ "enabled": true });
    meta
}

/// Puts the serverless node in the kernel's Cloudflare account, with no address but the gateway's binding to it, and
/// the relay again, which hands it the datagrams sent to its endpoint once it's there.
pub fn deploy(cf: &Cloudflare, settings: &Settings) -> Result<()> {
    anyhow::ensure!(settings.try_person.is_none(), "a try install's kernel runs only beside its try install");
    anyhow::ensure!(settings.serverless_endpoint().is_some(), "the serverless node has no key: serverless deploy makes one");
    let modules = modules()?;
    let name = worker_name(SERVICE);
    let scripts = cf.scripts()?;
    let migrated = scripts.get(&name).is_some_and(|s| s.migration.as_deref() == Some("v1"));
    let meta = metadata(settings, migrated, |other| scripts.contains_key(other));
    let mut what = vec![meta.to_string().into_bytes()];
    what.extend(modules.iter().flat_map(|(n, k, b)| [n.as_bytes().to_vec(), k.as_bytes().to_vec(), b.clone()]));
    let what = Uploaded::hash(&what);
    let uploaded = Uploaded::load();
    if !uploaded.same(cf, &name, &what) {
        let modules: Vec<(&str, &str, &[u8])> = modules.iter().map(|(n, k, b)| (n.as_str(), *k, b.as_slice())).collect();
        let version = cf.upload(&name, &modules, &meta)?;
        uploaded.record(vec![(name.clone(), what, version)])?;
    }
    // Its address may have been turned on by hand.
    cf.hide(&name)?;
    let relay = crate::kernel::SERVICES.iter().position(|(s, _, _)| *s == "relay").unwrap();
    let mut kernel = vec![Vec::new(); crate::kernel::SERVICES.len()];
    kernel[relay] = crate::kernel::module(crate::kernel::SERVICES[relay].1)?;
    crate::kernel::deploy_some(cf, settings, &kernel, &[], &["relay"])
}
