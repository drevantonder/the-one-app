//! The Cloudflare node (`oned/cloudflare/node/`) on Cloudflare: a node of the network as a Worker in the account of its other Workers, with
//! no address of its own, and an iroh endpoint of its own key, which the relay wakes. A release carries its built
//! WebAssembly (`update program` builds it, where Rust builds for WebAssembly); the apps it runs it reads from the network's
//! bucket, each at its latest deploy.

use anyhow::{Context, Result};
use serde_json::{json};

use crate::{Cloudflare, Files, NODE, Settings, Worker, upload_hash};

/// The Cloudflare node's Worker's modules: (name, type, bytes), `worker.js` first. `built.js` is the shim each app runs
/// in (the apps themselves it reads from the bucket, each at its latest deploy).
pub fn modules() -> Result<Vec<(String, &'static str, Vec<u8>)>> {
    let files = ::ui::ui_folder::wasm(::ui::carried::CLOUDFLARE_NODE_FILES, ::ui::carried::CLOUDFLARE_NODE);
    let built = |name: &str| {
        let found = files.iter().find(|(n, _)| n == name).map(|(_, b)| b.clone());
        found.with_context(|| format!("this build of {} has no oned for Cloudflare: {}-dev update program builds one, on a Linux device with Rust", custom_build::config::NAME, custom_build::config::CLI))
    };
    Ok(modules_from(built("one_cloudflare_node.js")?, built("one_cloudflare_node_bg.wasm")?))
}

/// Its modules, from its WebAssembly's glue and the WebAssembly, as wasm-bindgen makes them.
pub fn modules_from(glue: Vec<u8>, wasm: Vec<u8>) -> Vec<(String, &'static str, Vec<u8>)> {
    let shim = ::api::call::shim();
    let built_js = format!("export const shim = {};\nexport const compatibilityDate = {};\n", json!(shim), json!(::deploy::build::COMPATIBILITY_DATE));
    vec![
        ("worker.js".into(), "application/javascript+module", include_bytes!("../node/worker.js").to_vec()),
        ("one_cloudflare_node.js".into(), "application/javascript+module", glue),
        ("one_cloudflare_node_bg.wasm".into(), "application/wasm", wasm),
        ("built.js".into(), "application/javascript+module", built_js.into_bytes()),
        ("protocol.js".into(), "application/javascript+module", ::api::protocol_js::javascript().into_bytes()),
        ("crypto.js".into(), "application/javascript+module", include_bytes!("../../../key/crypto.js").to_vec()),
        ("http.js".into(), "application/javascript+module", include_bytes!("../../../gateway/http.js").to_vec()),
    ]
}

/// It as the workerd that runs the network's Workers here runs it, from `modules`, beside them.
pub fn served(settings: &Settings, mut modules: Vec<(String, &'static str, Vec<u8>)>) -> crate::Served {
    let (_, _, main) = modules.remove(0);
    crate::Served {
        name: NODE.name.into(),
        module: main,
        modules: modules.into_iter().map(|(name, _, bytes)| (name, bytes)).collect(),
        classes: NODE.classes.iter().map(|c| c.to_string()).collect(),
        listen: None,
        bindings: NODE.bindings(settings).into_iter().map(|(n, v, _)| (n, v)).collect(),
        assets: None,
        // A throwaway network's bucket is on this machine.
        outbound: Some(crate::NEARBY.to_string()),
    }
}

/// Puts the Cloudflare node in the Cloudflare account of the network's Workers, with no address, and
/// the relay again, from `sources`, which hands it the datagrams sent to its endpoint once it's there.
pub fn deploy(cf: &Cloudflare, settings: &Settings, sources: &Files) -> Result<()> {
    anyhow::ensure!(settings.throwaway_user.is_none(), "a throwaway network's Workers run only beside its throwaway network");
    anyhow::ensure!(settings.cloudflare_node_endpoint().is_some(), "oned on Cloudflare has no key: cloudflare oned deploy makes one");
    let modules = modules()?;
    let name = NODE.worker_name();
    let scripts = cf.scripts()?;
    let (meta, _) = crate::metadata(settings, &NODE, scripts.get(&name).and_then(|s| s.migration.as_deref()), |other| scripts.contains_key(other));
    let mut what = vec![meta.to_string().into_bytes()];
    what.extend(modules.iter().flat_map(|(n, k, b)| [n.as_bytes().to_vec(), k.as_bytes().to_vec(), b.clone()]));
    let what = upload_hash(&what);
    if !(scripts.contains_key(&name) && cf.runs(&name, &what)?) {
        let modules: Vec<(&str, &str, &[u8])> = modules.iter().map(|(n, k, b)| (n.as_str(), *k, b.as_slice())).collect();
        cf.upload(&name, &modules, &meta, &what)?;
    }
    // Its address may have been turned on by hand.
    cf.hide(&name)?;
    crate::deploy_some(cf, settings, sources, &[], &[Worker::named("relay")])?;
    crate::remove_renamed(cf, std::iter::once(&NODE))
}
