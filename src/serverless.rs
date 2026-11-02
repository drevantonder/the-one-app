//! The serverless node (`serverless/`) on Cloudflare: a node of the install as a Worker in its kernel's account, with
//! no address of its own, which the gateway sends `/v1/` and `/mcp` to by service binding. A release carries its built
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
    let shim = include_str!("runtime/shim.js");
    let built_js = format!("export const shim = {};\nexport const compatibilityDate = {};\n", json!(shim), json!(crate::apps::COMPATIBILITY_DATE));
    Ok(vec![
        ("worker.js".into(), "application/javascript+module", include_bytes!("../serverless/worker.js").to_vec()),
        ("one_serverless.js".into(), "application/javascript+module", built("one_serverless.js")?),
        ("one_serverless_bg.wasm".into(), "application/wasm", built("one_serverless_bg.wasm")?),
        ("built.js".into(), "application/javascript+module", built_js.into_bytes()),
        ("icons.json".into(), "text/plain", include_bytes!("../vendor/phosphor/icons.json").to_vec()),
    ])
}

/// Its bindings: the kernel's auth and vault, Worker Loader, the install's address and name, the gateway's relay, and
/// the node on a machine that what needs one goes on to.
pub fn metadata(settings: &Settings) -> Value {
    let mut bindings = vec![
        json!({ "type": "service", "name": "AUTH", "service": worker_name("auth") }),
        json!({ "type": "service", "name": "VAULT", "service": worker_name("vault") }),
        json!({ "type": "worker_loader", "name": "LOADER" }),
        json!({ "type": "plain_text", "name": "ADDRESS", "text": settings.address }),
        json!({ "type": "plain_text", "name": "NAME", "text": settings.name }),
        // The gateway's relay, which tells servers what this node wrote.
        json!({ "type": "durable_object_namespace", "name": "RELAY", "class_name": "Relay", "script_name": worker_name("gateway") }),
    ];
    if settings.node_url.is_some() {
        bindings.push(match settings.machine("MACHINE") {
            (name, value, _) if value.starts_with("vpc:") => json!({ "type": "vpc_service", "name": name, "service_id": &value[4..] }),
            (name, value, _) => json!({ "type": "plain_text", "name": name, "text": value }),
        });
    }
    // Its logs say which calls ran here, for whoever owns the account.
    json!({ "main_module": "worker.js", "compatibility_date": crate::apps::COMPATIBILITY_DATE, "bindings": bindings, "observability": { "enabled": true } })
}

/// Puts the serverless node in the kernel's Cloudflare account, with no address but the gateway's binding to it.
pub fn deploy(cf: &Cloudflare, settings: &Settings) -> Result<()> {
    anyhow::ensure!(settings.try_person.is_none(), "a try install's kernel runs only beside its try install");
    let modules = modules()?;
    let name = worker_name(SERVICE);
    let meta = metadata(settings);
    let mut what = vec![meta.to_string().into_bytes()];
    what.extend(modules.iter().flat_map(|(n, k, b)| [n.as_bytes().to_vec(), k.as_bytes().to_vec(), b.clone()]));
    let what = Uploaded::hash(&what);
    let uploaded = Uploaded::load();
    if uploaded.same(&cf.scripts()?, &name, &what) {
        return Ok(());
    }
    let modules: Vec<(&str, &str, &[u8])> = modules.iter().map(|(n, k, b)| (n.as_str(), *k, b.as_slice())).collect();
    cf.upload(&name, &modules, &meta)?;
    cf.hide(&name)?;
    uploaded.record(cf, vec![(name, what)])
}
