//! The Cloudflare node (`cloudflare/node/`) on Cloudflare: a node of the network as a Worker in the account of its other Workers, with
//! no address of its own, and an iroh endpoint of its own key, which the relay wakes. A release carries its built
//! WebAssembly (`update` builds it, where Rust builds for WebAssembly); the apps it runs it reads from the network's
//! bucket, each at its latest deploy.

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::cloudflare::{Cloudflare, NODE, Settings, Uploaded, Worker};

/// The Cloudflare node's Worker's modules: (name, type, bytes), `worker.js` first. `built.js` is the shim each app runs
/// in (the apps themselves it reads from the bucket, each at its latest deploy).
pub fn modules() -> Result<Vec<(String, &'static str, Vec<u8>)>> {
    let built = |name: &str| {
        let found = crate::config::CLOUDFLARE_NODE.iter().find(|(n, _)| *n == name).map(|(_, b)| b.to_vec());
        found.with_context(|| format!("this build of {} has no Cloudflare node: {} update builds one, on a Linux device with Rust", crate::config::NAME, crate::config::CLI))
    };
    Ok(modules_from(built("one_cloudflare_node.js")?, built("one_cloudflare_node_bg.wasm")?))
}

/// Its modules, from its WebAssembly's glue and the WebAssembly, as wasm-bindgen makes them.
pub fn modules_from(glue: Vec<u8>, wasm: Vec<u8>) -> Vec<(String, &'static str, Vec<u8>)> {
    let shim = include_str!("../app/runtime/shim.js");
    // And the services built for Cloudflare it carries, each `service-<app>.js`, by their app's name.
    let services = crate::config::CLOUDFLARE_SERVICES;
    let imports: String = services.iter().enumerate().map(|(i, (name, _))| format!("import s{i} from {};\n", json!(format!("./service-{name}.js")))).collect();
    let named: Vec<String> = services.iter().enumerate().map(|(i, (name, _))| format!("{}: s{i}", json!(name))).collect();
    let built_js = format!("{imports}export const shim = {};\nexport const compatibilityDate = {};\nexport const services = {{ {} }};\n", json!(shim), json!(crate::app::runtime::COMPATIBILITY_DATE), named.join(", "));
    let mut out = vec![
        ("worker.js".into(), "application/javascript+module", include_bytes!("../../cloudflare/node/worker.js").to_vec()),
        ("one_cloudflare_node.js".into(), "application/javascript+module", glue),
        ("one_cloudflare_node_bg.wasm".into(), "application/wasm", wasm),
        ("built.js".into(), "application/javascript+module", built_js.into_bytes()),
        ("protocol.js".into(), "application/javascript+module", crate::protocol::javascript().into_bytes()),
    ];
    out.extend(services.iter().map(|(name, bytes)| (format!("service-{name}.js"), "application/javascript+module", bytes.to_vec())));
    out
}

/// It as the workerd that runs the network's Workers here runs it, from `modules`, beside them.
pub fn served(settings: &Settings, mut modules: Vec<(String, &'static str, Vec<u8>)>) -> crate::cloudflare::Served {
    let (_, _, main) = modules.remove(0);
    crate::cloudflare::Served {
        name: NODE.name.into(),
        module: main,
        modules: modules.into_iter().map(|(name, _, bytes)| (name, bytes)).collect(),
        class: NODE.class.map(str::to_string),
        listen: None,
        bindings: NODE.bindings(settings).into_iter().map(|(n, v, _)| (n, v)).collect(),
        assets: None,
        // A throwaway network's bucket is on this machine.
        outbound: Some(crate::cloudflare::NEARBY.to_string()),
    }
}

/// What its upload says besides its modules: its bindings, its endpoint's Durable Object class
/// `Node` and that class's migration the first time, and its logs on.
pub fn metadata(settings: &Settings, migration: Option<&str>, exists: impl Fn(&str) -> bool) -> Value {
    let (mut meta, _) = crate::cloudflare::metadata(settings, &NODE, migration, exists);
    // Its logs say which calls ran here, for whoever owns the account.
    meta["observability"] = json!({ "enabled": true });
    meta
}

/// Puts the Cloudflare node in the Cloudflare account of the network's Workers, with no address, and
/// the relay again, which hands it the datagrams sent to its endpoint once it's there.
pub fn deploy(cf: &Cloudflare, settings: &Settings) -> Result<()> {
    anyhow::ensure!(settings.throwaway_person.is_none(), "a throwaway network's Workers run only beside its throwaway network");
    anyhow::ensure!(settings.cloudflare_node_endpoint().is_some(), "the Cloudflare node has no key: cloudflare node deploy makes one");
    let modules = modules()?;
    let name = NODE.worker_name();
    let scripts = cf.scripts()?;
    let meta = metadata(settings, scripts.get(&name).and_then(|s| s.migration.as_deref()), |other| scripts.contains_key(other));
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
    crate::cloudflare::deploy_some(cf, settings, &[], &[Worker::named("relay")])
}
