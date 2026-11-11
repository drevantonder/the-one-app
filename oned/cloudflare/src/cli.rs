//! The network's Workers on Cloudflare, from this machine: `cloudflare`, and `cloudflare <what>`.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use ::api::cli::{Command, Given};
use ::custom_build::config::{CLI, NAME, YOURS};
use ::scope::scopes::Role;
use ::api::Client;

/// `settings` with the network's devices (each key, its roles and whether it's an admin's) and its recovery key, as the
/// app runtime on this machine knows them, for the relay to take only them; none when no device answers here, which `needed`
/// refuses, since that relay would take no one.
fn devices_here(api: &Client, settings: crate::Settings, needed: bool) -> Result<crate::Settings> {
    let network = api.state().ok().and_then(|s| s.network);
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
    anyhow::ensure!(!needed || !devices.is_empty(), "no app runtime of {YOURS} answers on this machine, so its relay would take none of its devices: run this where one does, or with --new for a new network, whose relay takes devices once this runs again beside one");
    // The network's recovery key too, as a client, not an admin's and not an app runtime: what it lets in, the relay takes, as
    // an admin's if its user is one.
    devices.extend(network.as_ref().and_then(|n| n.recovery.as_ref()).map(|k| json!({ "key": k, "role": "client", "admin": false, "recovery": true })));
    let root = network.and_then(|n| n.root).filter(|r| r.len() == 64);
    Ok(crate::Settings { devices, root, ..settings })
}

/// Puts `newest` on Cloudflare: each Worker, and oned on Cloudflare if the network has one. The relay says what they're on
/// (`Newest::deployed`), for every client to show.
fn put_up(cf: &crate::Cloudflare, settings: &mut crate::Settings, newest: crate::Newest) -> Result<()> {
    // Setting up Basin needs more of the Cloudflare account than a deploy token has, and is done once: a network whose token
    // can't yet still gets its Workers.
    if settings.telemetry.as_ref().is_none_or(|t| t.pipelines.is_empty()) {
        if let Err(err) = crate::basin::provision(cf, settings) {
            eprintln!("Insights won't have data until this is done, by a deploy with a token that may: {err:#}");
        }
    }
    if let Err(err) = crate::basin::exports(cf, settings) {
        eprintln!("The Workers' own traces and logs won't reach Insights until this is done, by a deploy with a token that may: {err:#}");
    }
    let usage = |settings: &mut crate::Settings| {
        if let Err(err) = crate::basin::usage(cf, settings) {
            eprintln!("Insights won't show what the network uses of Cloudflare until this is done, by a deploy with a token that may: {err:#}");
        }
    };
    usage(settings);
    settings.deployed = Some(newest.deployed(settings));
    let mut web = settings.web_with(newest.web);
    web.extend(crate::precompressed(&web)?);
    crate::deploy(cf, settings, &newest.workers, &web)?;
    if settings.cloudflare_node {
        crate::worker::deploy(cf, settings, &newest.workers)?;
    }
    crate::remove_renamed(cf)?;
    // A Durable Object class deployed for the first time just now has made its namespace.
    if let Some(before) = settings.usage.clone() {
        usage(settings);
        if settings.usage.as_ref() != Some(&before) {
            crate::deploy_some(cf, settings, &newest.workers, &web, &[crate::Worker::named("telemetry")])?;
        }
    }
    Ok(())
}

/// Cloudflare's permission groups a Workers AI key has: running its models over the REST API.
const WORKERS_AI_PERMISSIONS: [&str; 2] = ["Workers AI Read", "Workers AI Write"];

/// Everyone's `AI key: workers-ai`, so every app's `ctx.ai.run` answers with Workers AI from the network's first deploy
/// (AI Gateway, ai-gateway/): a token that runs the account's models and nothing else, kept in Everyone's Vault through
/// this computer's app runtime, never anywhere else. Made once, by the first deploy whose token may make tokens.
fn workers_ai_key(cf: &crate::Cloudflare, api: &Client) -> Result<()> {
    let kind = ::ai_gateway::Company::WorkersAi.key_kind();
    let items = api.get("/api/vault?scope=global")?;
    if items.as_array().into_iter().flatten().any(|it| it["type"] == kind.as_str()) {
        return Ok(());
    }
    let groups = cf.list(&format!("/accounts/{}/tokens/permission_groups", cf.account))?;
    let ids: Vec<Value> = WORKERS_AI_PERMISSIONS.iter().map(|name| groups.iter().find(|g| g["name"] == *name).and_then(|g| g["id"].as_str()).map(|id| json!({ "id": id })).with_context(|| format!("Cloudflare has no permission group {name}"))).collect::<Result<_>>()?;
    let policies = json!([{ "effect": "allow", "resources": { format!("com.cloudflare.api.account.{}", cf.account): "*" }, "permission_groups": ids }]);
    let made = cf.post(&format!("/accounts/{}/tokens", cf.account), json!({ "name": format!("{CLI}-workers-ai"), "policies": policies })).context("making the token apps run Workers AI's models with, which needs a Cloudflare API token with Account API Tokens Write")?;
    let token = made["value"].as_str().context("Cloudflare made the token and didn't give its value")?;
    let host = ::ai_gateway::Company::WorkersAi.host();
    api.post("/api/vault", json!({ "scope": "global", "title": "Workers AI Key", "type": kind, "domain": "cloudflare.com", "fields": { "account_id": { "value": cf.account, "secret": false }, "api_key": { "value": token, "secret": true } }, "hosts": [host], "apps": [::key::APP] }))?;
    Ok(())
}

/// Cloudflare's API with the token in CLOUDFLARE_API_TOKEN, or else the Workers' own (their .env.local's), for the account
/// `--account` names or the token's only one.
fn cloudflare(g: &Given, settings: Option<&crate::Settings>) -> Result<crate::Cloudflare> {
    let token = std::env::var("CLOUDFLARE_API_TOKEN").ok().or_else(|| settings.and_then(|s| s.cloudflare_token.clone()));
    let token = token.context("give a Cloudflare API token that may edit Workers in CLOUDFLARE_API_TOKEN, or as the Workers' CLOUDFLARE_API_TOKEN in their .env.local")?;
    let api = "https://api.cloudflare.com/client/v4".to_string();
    let account = match g.flag("account") {
        Some(account) => account.to_string(),
        None => crate::Cloudflare { api: api.clone(), token: token.clone(), account: String::new() }.accounts()?,
    };
    Ok(crate::Cloudflare { api, token, account })
}

/// Where a network's Workers are, and what they're set to, without their secrets.
fn cloudflare_said(s: &crate::Settings) -> String {
    let mut out = format!("{NAME} on Cloudflare\n  address   {}\n", s.address);
    if s.auth_url.starts_with("http://127.0.0.1") {
        out += &format!("  here on   127.0.0.1:{} (the address), :{} (auth)\n", s.ports[1], s.ports[0]);
    }
    if !s.resources.is_empty() {
        out += &format!("  resources {}\n", s.resources.join(", "));
    }
    out += &format!("  bucket    {}\n", s.bucket.as_ref().map(|b| format!("{} in {}", b["bucket"].as_str().unwrap_or("?"), b["endpoint"].as_str().unwrap_or("?"))).unwrap_or("none yet: cloudflare set bucket -".into()));
    if !s.android.is_empty() {
        out += &format!("  android   {} signed by {}\n", crate::android_package(), s.android.join(", "));
    }
    out
}

/// The network's Workers on Cloudflare, from any admin's device.
pub fn commands() -> Command {
    Command {
        help: vec![
            ("cloudflare".into(), "the network's Workers on Cloudflare, as this machine has them: their addresses and settings".into()),
            ("cloudflare serve [--address <url>]".into(), "run the network's Workers in workerd (auth, the relay, the URL signer, and the gateway users reach at the network's address) on this machine; the first time, with that address".into()),
            ("cloudflare deploy [--domain <domain>] [--address <url>] [--account <id>] [--new]".into(), format!("deploy the network's Workers to Cloudflare, with their settings from 1Password where they're kept there, and an API token that may edit them in CLOUDFLARE_API_TOKEN; the first time, at one of the account's domains ({CLI}-<service>.<domain>, the network's address {CLI}.<domain> unless --address says, for good: passkeys belong to it), or at its workers.dev address; and oned on Cloudflare with it, once cloudflare oned deploy put it there; beside an app runtime of the network, whose devices its relay takes, unless --new")),
            ("cloudflare oned deploy [--account <id>]".into(), "put oned on Cloudflare in the network's Workers' Cloudflare account (an app runtime as a Worker, with the API token in CLOUDFLARE_API_TOKEN), which devices ask over iroh when none of your computers answers".into()),
            ("cloudflare set <setting> <value>".into(), "change one: auth-url, resources (computers' own addresses, comma-separated), bucket (its JSON object piped, with - as its value) or android (the SHA-256 fingerprints of the certificates the Android app is signed with, comma-separated, or -: links to the address open the app)".into()),
            ("cloudflare use <address>".into(), "point this network at its Workers: its devices take sign-ins made at the address".into()),
        ],
        run: |g| {
            let api = g.api;
            match g.words().as_slice() {
                ["cloudflare"] => print!("{}", cloudflare_said(&crate::Settings::load()?)),
                ["cloudflare", "serve"] => {
                    let settings = match crate::Settings::load() {
                        Ok(settings) => settings,
                        Err(err) => {
                            crate::Settings::none_here().map_err(|here| err.context(format!("{here:#}")))?;
                            let url = g.flag("address").context("say where users will reach the network, with --address https://…")?;
                            let settings = crate::Settings::new(url);
                            settings.save()?;
                            settings
                        }
                    };
                    print!("{}", cloudflare_said(&settings));
                    let settings = devices_here(g.api, settings, false)?;
                    crate::serve(&settings)?
                }
                ["cloudflare", "deploy"] => {
                    let kept = crate::Settings::from_1password(g.has("new"));
                    if let Err(err) = &kept
                        && crate::Settings::path().exists()
                    {
                        eprintln!("Deploying with the Cloudflare settings on this device, which a deploy then keeps in 1Password: {err:#}");
                    }
                    let loaded = crate::Settings::load();
                    // A throwaway network's Workers run here, not on Cloudflare: they start again on the newest.
                    #[cfg(unix)]
                    if let Some(settings) = loaded.as_ref().ok().filter(|s| s.throwaway_user.is_some()) {
                        crate::redeploy(settings)?;
                        println!("Asked the Workers to start again on the newest.");
                        return Ok(true);
                    }
                    let first = loaded.is_err();
                    // With no settings here, why 1Password gave none says more than a missing token.
                    let cf = match (cloudflare(g, loaded.as_ref().ok()), &kept) {
                        (Err(err), Err(why)) if loaded.is_err() => return Err(err.context(format!("{why:#}"))),
                        (cf, _) => cf?,
                    };
                    let settings = match loaded {
                        Ok(settings) => settings,
                        Err(err) => {
                            if let Err(here) = crate::Settings::none_here() {
                                return Err(err.context(format!("{here:#}")));
                            }
                            // Workers already in the account were made elsewhere: new settings here would replace their secrets.
                            let made = cf.scripts()?.contains_key(&crate::worker_name("auth"));
                            if made && !g.has("new") {
                                return Err(kept.err().unwrap_or(err).context("the Cloudflare account has this network's Workers already, and their settings aren't on this device. --new replaces them, and everything they hold can't be read again"));
                            }
                            let domain = g.flag("domain").map(|d| d.trim_matches('.').to_string());
                            let (domain, address) = match domain {
                                Some(domain) => (domain.clone(), format!("https://{CLI}.{domain}")),
                                None => {
                                    let domain = format!("{}.workers.dev", cf.subdomain()?);
                                    (domain.clone(), format!("https://{}.{domain}", crate::worker_name("gateway")))
                                }
                            };
                            let at = |service: &str| format!("https://{}.{domain}", crate::worker_name(service));
                            let address = g.flag("address").map(|a| a.trim_end_matches('/').to_string()).unwrap_or(address);
                            let mut settings = crate::Settings::new(&address);
                            settings.auth_url = at("auth");
                            settings.save()?;
                            println!("Keep {} too: the Workers' secrets are in it, and a deploy without them can't read what's there. Better, move each to 1Password and leave a 1Password reference (op://…) in its place.\n", crate::Settings::path().with_file_name(".env.local").display());
                            // A network with a domain needs a provider's client, which is put beside the secrets first.
                            if ::custom_build::config::JOIN_DOMAIN.is_empty() { settings } else { crate::Settings::load()? }
                        }
                    };
                    let settings = devices_here(g.api, settings, !g.has("new"))?;
                    // A first deploy has nothing at its address yet to read.
                    let newest = if first { crate::Newest::built() } else { crate::Newest::read(&settings.address)? };
                    println!("{}", newest.said);
                    let mut settings = settings;
                    put_up(&cf, &mut settings, newest)?;
                    settings.save()?;
                    if let Err(err) = workers_ai_key(&cf, api) {
                        eprintln!("Apps' AI won't answer with Workers AI until a deploy makes its key, with a token that may make tokens, beside a computer of the network: {err:#}");
                    }
                    // The clients its users sign in with, signed by this admin's device: a new device's sign-in counts
                    // only for one of them (`sign_in_proof::CLIENTS`), whatever auth says.
                    if let Some(clients) = settings.sign_in_clients() {
                        let had = api.state().ok().and_then(|s| s.network?.settings.get(::scope::sign_in_proof::CLIENTS).cloned());
                        if had.as_deref() != Some(clients.as_str())
                            && let Err(err) = api.post("/api/settings", json!({ "name": ::scope::sign_in_proof::CLIENTS, "value": clients }))
                        {
                            eprintln!("A new device's sign-in can't be checked until an admin's device runs {CLI} settings {} {clients}: {err:#}", ::scope::sign_in_proof::CLIENTS);
                        }
                    }
                    print!("Deployed.\n{}", cloudflare_said(&settings));
                }
                ["cloudflare", "oned", "deploy"] => {
                    if let Err(err) = crate::Settings::from_1password(false)
                        && !crate::Settings::path().exists()
                    {
                        return Err(err);
                    }
                    let mut settings = devices_here(g.api, crate::Settings::load()?, true)?;
                    let cf = cloudflare(g, Some(&settings))?;
                    if settings.cloudflare_node_endpoint().is_none() {
                        settings.cloudflare_node_key = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>());
                        settings.set_secret("CLOUDFLARE_NODE_KEY", &settings.cloudflare_node_key)?;
                    }
                    // The relay it deploys again binds oned on Cloudflare's endpoint only for a network that has one.
                    let was = std::mem::replace(&mut settings.cloudflare_node, true);
                    let newest = crate::Newest::read(&settings.address)?;
                    put_up(&cf, &mut settings, newest)?;
                    if !was {
                        settings.save()?;
                    }
                    // So devices answer its endpoint, and Devices shows it, for its admins to say where it comes in the order
                    // things run in.
                    api.post("/api/settings", json!({ "name": ::scope::scopes::CLOUDFLARE_NODE, "value": settings.cloudflare_node_endpoint() }))?;
                    println!("oned is on your Cloudflare account: devices ask it over iroh when none of your computers answers.");
                }
                ["cloudflare", "set", name, value] => {
                    let mut settings = crate::Settings::load()?;
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
                    api.post("/api/settings", json!({ "name": "address", "value": address.trim_end_matches('/') }))?;
                    println!("This network is at {address}: its devices take sign-ins made there.");
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
    }
}
