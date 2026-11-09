//! The network's Workers on Cloudflare, from this machine: `cloudflare`, and `cloudflare <what>`.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::cli::{Command, Failed, Given};
use crate::config::{CLI, NAME, YOURS};
use crate::database::scopes::Role;
use crate::api::Client;

/// `settings` with the network's devices (each key, its roles and whether it's an admin's) and its recovery key, as the
/// node on this machine knows them, for the relay to take only them; none when no node answers here, which `needed`
/// refuses, since that relay would take no one.
fn devices_here(node: &Client, settings: crate::cloudflare::Settings, needed: bool) -> Result<crate::cloudflare::Settings> {
    let network = node.state().ok().and_then(|s| s.network);
    let admin = |user: &str| network.iter().flat_map(|n| &n.users).any(|u| u.id == user && u.role == Role::Admin);
    let mut devices: Vec<Value> = network
        .iter()
        .flat_map(|n| n.devices.iter().map(|d| (&d.key, &d.roles, &d.user)).chain(n.clients.iter().map(|c| (&c.key, &c.roles, &c.user))))
        .filter(|(key, ..)| key.len() == 64)
        .map(|(key, roles, user)| {
            let roles: Vec<&str> = roles.iter().map(|r| r.as_str()).collect();
            json!({ "key": key, "role": roles.join(" "), "admin": admin(user) })
        })
        .collect();
    anyhow::ensure!(!needed || !devices.is_empty(), "no node of {YOURS} answers on this machine, so its relay would take none of its devices: run this where one does, or with --new for a new network, whose relay takes devices once this runs again beside one");
    // The network's recovery key too, as a client, not an admin's and not a node: what it lets in, the relay takes, as
    // an admin's if its user is one.
    devices.extend(network.as_ref().and_then(|n| n.recovery.as_ref()).map(|k| json!({ "key": k, "role": "client", "admin": false, "recovery": true })));
    let root = network.and_then(|n| n.root).filter(|r| r.len() == 64);
    Ok(crate::cloudflare::Settings { devices, root, ..settings })
}


/// Makes this device the Workers' home, or refuses (exit 3, which `one-dev update workers` passes over) where it isn't.
fn at_home(settings: &mut crate::cloudflare::Settings) -> Result<()> {
    settings.at_home().map_err(|err| match err.downcast::<crate::cloudflare::NotHome>() {
        Ok(not) => Failed::Refused(not.0).into(),
        Err(err) => err,
    })
}

/// Cloudflare's API with the token in CLOUDFLARE_API_TOKEN, or else the Workers' own (their .env.local's), for the account
/// `--account` names or the token's only one.
fn cloudflare(g: &Given, settings: Option<&crate::cloudflare::Settings>) -> Result<crate::cloudflare::Cloudflare> {
    let token = std::env::var("CLOUDFLARE_API_TOKEN").ok().or_else(|| settings.and_then(|s| s.cloudflare_token.clone()));
    let token = token.context("give a Cloudflare API token that may edit Workers in CLOUDFLARE_API_TOKEN, or as the Workers' CLOUDFLARE_API_TOKEN in their .env.local")?;
    let api = "https://api.cloudflare.com/client/v4".to_string();
    let account = match g.flag("account") {
        Some(account) => account.to_string(),
        None => crate::cloudflare::Cloudflare { api: api.clone(), token: token.clone(), account: String::new() }.accounts()?,
    };
    Ok(crate::cloudflare::Cloudflare { api, token, account })
}

/// Where a network's Workers are, and what they're set to, without their secrets.
fn cloudflare_said(s: &crate::cloudflare::Settings) -> String {
    let mut out = format!("{NAME} on Cloudflare\n  address   {}\n", s.address);
    if s.auth_url.starts_with("http://127.0.0.1") {
        out += &format!("  here on   127.0.0.1:{} (the address), :{} (auth)\n", s.ports[1], s.ports[0]);
    }
    if !s.resources.is_empty() {
        out += &format!("  nodes     {}\n", s.resources.join(", "));
    }
    out += &format!("  bucket    {}\n", s.bucket.as_ref().map(|b| format!("{} in {}", b["bucket"].as_str().unwrap_or("?"), b["endpoint"].as_str().unwrap_or("?"))).unwrap_or("none yet: cloudflare set bucket -".into()));
    if !s.android.is_empty() {
        out += &format!("  android   {} signed by {}\n", crate::cloudflare::android_package(), s.android.join(", "));
    }
    out
}

/// The network's Workers on Cloudflare, from the device that's their home.
pub fn commands() -> Command {
    Command {
        help: vec![
            ("cloudflare".into(), "the network's Workers on Cloudflare, as this machine has them: their addresses and settings".into()),
            ("cloudflare serve [--address <url>]".into(), "run the network's Workers in workerd (auth, the relay, the URL signer, and the gateway users reach at the network's address) on this machine; the first time, with that address".into()),
            ("cloudflare deploy [--domain <domain>] [--address <url>] [--account <id>] [--new]".into(), format!("deploy the network's Workers to Cloudflare, with an API token that may edit them in CLOUDFLARE_API_TOKEN; the first time, at one of the account's domains ({CLI}-<service>.<domain>, the network's address {CLI}.<domain> unless --address says, for good: passkeys belong to it), or at its workers.dev address; and the Cloudflare node with it, once cloudflare node deploy put it there; beside a node of the network, whose devices its relay takes, unless --new")),
            ("cloudflare node deploy [--account <id>]".into(), "put the network's Cloudflare node in its Workers' Cloudflare account (a node as a Worker, with the API token in CLOUDFLARE_API_TOKEN), which devices ask over iroh when none of your machines answers".into()),
            ("cloudflare set <setting> <value>".into(), "change one: auth-url, resources (nodes' own addresses, comma-separated), bucket (its JSON object piped, with - as its value) or android (the SHA-256 fingerprints of the certificates the Android app is signed with, comma-separated, or -: links to the address open the app)".into()),
            ("cloudflare use <address>".into(), "point this network at its Workers: its nodes take sign-ins made at the address".into()),
        ],
        run: |g| {
            let node = g.node;
            match g.words().as_slice() {
                ["cloudflare"] => print!("{}", cloudflare_said(&crate::cloudflare::Settings::load()?)),
                ["cloudflare", "serve"] => {
                    let settings = match crate::cloudflare::Settings::load() {
                        Ok(settings) => settings,
                        Err(err) => {
                            crate::cloudflare::Settings::none_here().map_err(|here| err.context(format!("{here:#}")))?;
                            let url = g.flag("address").context("say where users will reach the network, with --address https://…")?;
                            let settings = crate::cloudflare::Settings::new(url);
                            settings.save()?;
                            settings
                        }
                    };
                    print!("{}", cloudflare_said(&settings));
                    let settings = devices_here(g.node, settings, false)?;
                    crate::cloudflare::serve(&settings)?
                }
                ["cloudflare", "deploy"] => {
                    let loaded = crate::cloudflare::Settings::load();
                    let first = loaded.is_err();
                    let cf = cloudflare(g, loaded.as_ref().ok())?;
                    let settings = match loaded {
                        Ok(settings) => settings,
                        Err(err) => {
                            crate::cloudflare::Settings::none_here().map_err(|here| err.context(format!("{here:#}")))?;
                            // Workers already in the account were made elsewhere: new settings here would replace their secrets.
                            let made = cf.scripts()?.contains_key(&crate::cloudflare::worker_name("auth"));
                            anyhow::ensure!(!made || g.has("new"), "the Cloudflare account has this network's Workers already, made on another machine: deploy them there, where their settings are, or bring them here ({} and .env.local). --new replaces them, and everything they hold can't be read again", crate::cloudflare::Settings::path().display());
                            let domain = g.flag("domain").map(|d| d.trim_matches('.').to_string());
                            let (domain, address) = match domain {
                                Some(domain) => (domain.clone(), format!("https://{CLI}.{domain}")),
                                None => {
                                    let domain = format!("{}.workers.dev", cf.subdomain()?);
                                    (domain.clone(), format!("https://{}.{domain}", crate::cloudflare::worker_name("gateway")))
                                }
                            };
                            let at = |service: &str| format!("https://{}.{domain}", crate::cloudflare::worker_name(service));
                            let address = g.flag("address").map(|a| a.trim_end_matches('/').to_string()).unwrap_or(address);
                            let mut settings = crate::cloudflare::Settings::new(&address);
                            settings.auth_url = at("auth");
                            settings.save()?;
                            println!("Keep {} too: the Workers' secrets are in it, and a deploy without them can't read what's there. Better, move each to 1Password and leave a 1Password reference (op://…) in its place.\n", crate::cloudflare::Settings::path().with_file_name(".env.local").display());
                            // A network with a domain needs a provider's client, which is put beside the secrets first.
                            if crate::config::JOIN_DOMAIN.is_empty() { settings } else { crate::cloudflare::Settings::load()? }
                        }
                    };
                    let mut settings = settings;
                    at_home(&mut settings)?;
                    let settings = devices_here(g.node, settings, !g.has("new"))?;
                    // A first deploy has nothing at its address yet to read.
                    let newest = if first { crate::cloudflare::Newest::built() } else { crate::cloudflare::Newest::read(&settings.address)? };
                    println!("{}", newest.said);
                    let mut web = settings.web_with(newest.web);
                    web.extend(crate::cloudflare::precompressed(&web)?);
                    crate::cloudflare::deploy(&cf, &settings, &newest.workers, &web)?;
                    if settings.cloudflare_node {
                        crate::cloudflare::node::deploy(&cf, &settings, &newest.workers)?;
                    }
                    // The clients its users sign in with, signed by this admin's device: a new device's sign-in counts
                    // only for one of them (`sign_in_proof::CLIENTS`), whatever auth says.
                    if let Some(clients) = settings.sign_in_clients() {
                        let had = node.state().ok().and_then(|s| s.network?.settings.get(crate::onboarding::sign_in_proof::CLIENTS).cloned());
                        if had.as_deref() != Some(clients.as_str())
                            && let Err(err) = node.post("/v1/settings", json!({ "name": crate::onboarding::sign_in_proof::CLIENTS, "value": clients }))
                        {
                            eprintln!("A new device's sign-in can't be checked until an admin's device runs {CLI} settings {} {clients}: {err:#}", crate::onboarding::sign_in_proof::CLIENTS);
                        }
                    }
                    print!("Deployed.\n{}", cloudflare_said(&settings));
                }
                ["cloudflare", "node", "deploy"] => {
                    let mut settings = crate::cloudflare::Settings::load()?;
                    at_home(&mut settings)?;
                    let mut settings = devices_here(g.node, settings, true)?;
                    let cf = cloudflare(g, Some(&settings))?;
                    if settings.cloudflare_node_endpoint().is_none() {
                        settings.cloudflare_node_key = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>());
                        settings.set_secret("CLOUDFLARE_NODE_KEY", &settings.cloudflare_node_key)?;
                    }
                    // The relay it deploys again binds the Cloudflare node's endpoint only for a network that has one.
                    let was = std::mem::replace(&mut settings.cloudflare_node, true);
                    crate::cloudflare::node::deploy(&cf, &settings, &crate::cloudflare::Newest::read(&settings.address)?.workers)?;
                    if !was {
                        settings.save()?;
                    }
                    // So devices answer its endpoint, and Devices shows it, for its admins to say where it comes in the order
                    // things run in.
                    node.post("/v1/settings", json!({ "name": crate::database::scopes::CLOUDFLARE_NODE, "value": settings.cloudflare_node_endpoint() }))?;
                    println!("The Cloudflare node is in your Cloudflare account: devices ask it over iroh when none of your machines answers.");
                }
                ["cloudflare", "set", name, value] => {
                    let mut settings = crate::cloudflare::Settings::load()?;
                    let piped = || -> Result<Value> { serde_json::from_reader(std::io::stdin()).context("pipe its JSON object on stdin") };
                    match *name {
                        "auth-url" => settings.auth_url = value.trim_end_matches('/').to_string(),
                        "resources" => settings.resources = value.split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect(),
                        "bucket" if *value == "-" => {
                            let bucket = piped()?;
                            settings.set_secret("BUCKET", &bucket.to_string())?;
                            settings.bucket = Some(bucket);
                        }
                        "bucket" => bail!("pipe its JSON object on stdin and give - as its value, so it's in no shell's history"),
                        "android" => {
                            // As assetlinks.json writes them: 32 pairs of hex digits, in capitals, between colons.
                            let mut prints = Vec::new();
                            for given in value.split(',').map(str::trim).filter(|p| !p.is_empty() && *p != "-") {
                                let hex = given.replace(':', "").to_uppercase();
                                anyhow::ensure!(hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()), "{given} isn't a certificate's SHA-256 fingerprint (apksigner verify --print-certs <apk> says it)");
                                prints.push(hex.as_bytes().chunks(2).map(|p| std::str::from_utf8(p).unwrap()).collect::<Vec<_>>().join(":"));
                            }
                            settings.android = prints;
                        }
                        _ => bail!("no Cloudflare setting {name}: auth-url, resources, bucket or android"),
                    }
                    settings.save()?;
                    println!("Set {name}. {CLI} cloudflare serve takes it at its next start, and {CLI} cloudflare deploy when it next runs.");
                }
                ["cloudflare", "use", address] => {
                    node.post("/v1/settings", json!({ "name": "address", "value": address.trim_end_matches('/') }))?;
                    println!("This network is at {address}: its nodes take sign-ins made there.");
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}
