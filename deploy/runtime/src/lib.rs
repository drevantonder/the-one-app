//! Deploy, as an app runtime runs it: deploying an app (`deploy`, `deploy_making`), forking one, its code (`source`),
//! adding it to a page and removing it (`add`), and the logos of the sites apps connect to (`logo`). The catalog of
//! apps it runs (settling, bundles, built-in apps, what each says it is) is the runtime's, reached through `Apps`.

mod logos;
mod routes;
mod tools;

use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use api::Caller;
use api::call::Event;
use api::runtime::Runs;
use api::runtime::prelude::*;
use custom_build::allowed;
use key::{Key, hex, unhex};
use scope::content::{self, Content};
use scope::scopes::{self, Scope, new_id};
use serde_json::{Value, json};

/// The deploy's routes, and the logos it keeps.
pub fn runs() -> Runs {
    Runs { routes: routes::routes(), state: Box::new(State::default()), ..Runs::named("deploy") }
}

/// What serves `add`.
pub fn add_runs() -> Runs {
    Runs { tools: tools::add(), ..Runs::named("add") }
}

/// What serves `remove`.
pub fn remove_runs() -> Runs {
    Runs { tools: tools::remove(), ..Runs::named("remove") }
}

/// What it keeps while its runtime runs.
#[derive(Default)]
pub struct State {
    /// The logos of the sites apps and services connect to, once read from the runtime's home.
    logos: std::sync::OnceLock<logos::Logos>,
}

/// The logo of `domain`, the site `app` connects to, as a data URL and its kind, once this device has fetched it.
/// Fetches it in the background when it hasn't, or did a week ago, then tells shells `app` changed.
pub fn logo(rt: &Rt, domain: &str, app: &str) -> Option<(String, String)> {
    let logos = rt.state::<State>().logos.get_or_init(|| logos::Logos::open(rt.home()));
    let (url, due) = logos.get(rt.home(), domain, custom_build::now_ms());
    if due && logos.begin(domain) {
        let (rt, domain, app) = (rt.clone(), domain.to_string(), app.to_string());
        tokio::spawn(async move {
            let d = domain.clone();
            let logo = match tokio::task::spawn_blocking(move || logos::fetch(&d)).await {
                Ok(Ok(logo)) => Some(logo),
                Ok(Err(err)) => {
                    tracing::warn!("{app}'s logo: {err:#}");
                    None
                }
                Err(_) => None,
            };
            let logos = rt.state::<State>().logos.get_or_init(|| logos::Logos::open(rt.home()));
            if logos.finish(rt.home(), &domain, logo, custom_build::now_ms()) {
                rt.emit(Event::Changed { apps: vec![app] });
            }
        });
    }
    url
}

/// Adds `app` and the apps it uses in `scope`, or removes it (`added`), its services on `computer` (one of the caller's by name).
pub async fn add(rt: &Rt, caller: &Caller, app: &str, scope: &Scope, added: bool, computer: Option<&str>) -> Result<()> {
    rt.read_callers_scopes(caller).await?;
    if added {
        rt.read(|r| {
            let a = r.db().app(app)?;
            ensure!(a.as_ref().is_none_or(|a| !a.erased), "no app named {app}: its name is free again");
            ensure!(a.is_none_or(|a| a.deleted.is_none()), "{app} is in Recently Deleted: apps_restore {app} brings it back");
            Ok(())
        })?;
    }
    for (scope, content) in add_entries(rt, caller, app, scope, added, computer).await? {
        rt.append(&scope, &caller.user, content).await?;
    }
    Ok(())
}

/// The entries that add (or remove) `app` in `scope` for `caller`, with their own rights there. Adding it
/// also adds each app it uses, and each they use, that isn't added there yet. Each of their services runs for
/// the scope on one of the caller's computers it can run on: `computer` (by
/// name) for `app`'s, which is asked for when there's a choice, and added again with another computer, they move
/// there; and for an app it uses, `computer` if it can run there, else the one computer it can, if there's one, and it stays where
/// it runs already. Removing `app` stops its services there.
pub async fn add_entries(rt: &Rt, caller: &Caller, app: &str, scope: &Scope, added: bool, computer: Option<&str>) -> Result<Vec<(Scope, Content)>> {
    let mut wanted = vec![app.to_string()];
    let mut i = 0;
    while added && i < wanted.len() {
        rt.bring_in(caller, &wanted[i]).await?;
        for used in rt.uses(&wanted[i]).await?.0 {
            if !wanted.contains(&used) {
                wanted.push(used);
            }
        }
        i += 1;
    }
    // Each one's title, services, and the caller's computers they can run on.
    let mut services = Vec::new();
    let mut widget = None;
    for (n, name) in wanted.iter().enumerate() {
        let computers = match rt.describe_any(name).await {
            Ok(described) => {
                if n == 0 {
                    widget = described["widgets"][0]["name"].as_str().map(str::to_string);
                }
                rt.computers_for(caller, &described)
            }
            Err(_) => None,
        };
        services.push((rt.title(name).await?, rt.uses(name).await?.1, computers));
    }
    rt.read(|r| {
        let m = r.db();
        let named = r.scope_name(scope)?;
        allowed!(m.may(&caller.user, "add-apps", &scopes::object(scope)), "You can't add apps to {named}.");
        ensure!(r.holds(scope), "This device can't open {named} yet.");
        let here = r.state(scope).map(|s| s.added.clone()).unwrap_or_default();
        let mut out = Vec::new();
        for (n, name) in wanted.iter().enumerate().rev() {
            ensure!(m.app(name)?.is_some(), "no app named {name}{}: apps_list lists them", if n > 0 { format!(", which {app} uses") } else { String::new() });
            if n == 0 || !here.contains(name) {
                out.push((scope.clone(), content::added(name, added)));
            }
        }
        // Where each service runs for this scope now: stopped where it no longer should, started where it should.
        let ours = |s: &scopes::Switch| s.on && s.scope.as_deref() == Some(scope.to_string().as_str());
        let switches = m.switches()?;
        for (n, (title, names, computers)) in services.iter().enumerate() {
            let listed = |name: &str| computers.iter().flatten().find(|l| l.eq_ignore_ascii_case(name)).cloned();
            let chosen = match (n, computers) {
                (_, _) if !added => None,
                (0, Some(computers)) => match computer {
                    Some(computer) => Some(listed(computer).with_context(|| format!("{title} can't run on {computer}: it can run on {}", if computers.is_empty() { "none of your computers".to_string() } else { computers.join(", ") }))?),
                    None if computers.len() > 1 => bail!("Where should {title} run? Give computer: {}.", computers.join(" or ")),
                    None => computers.first().cloned(),
                },
                (_, Some(computers)) => computer.and_then(listed).or_else(|| (computers.len() == 1).then(|| computers[0].clone())),
                _ => None,
            };
            for service in names {
                let running: Vec<Key> = switches.iter().filter(|((_, name), s)| name == service && ours(s)).map(|((k, _), _)| *k).collect();
                let to = match chosen.as_deref().filter(|_| n == 0 || running.is_empty()) {
                    Some(computer) => Some(scope::settings::service(m, &caller.user, computer, service, true, Some(scope))?),
                    None => None,
                };
                let to_key = to.as_ref().and_then(|c| c.data["device"].as_str()).and_then(|k| unhex(k).ok());
                for k in running.iter().filter(|k| !added || (to_key.is_some() && Some(**k) != to_key)) {
                    out.push((Scope::Global, scope::settings::service(m, &caller.user, &hex(k), service, false, None)?));
                }
                if let Some(to) = to.filter(|_| !running.contains(&to_key.unwrap_or_default())) {
                    out.push((Scope::Global, to));
                }
            }
        }
        // Newly added, its first widget goes on that page for whoever added it, unless they have one of it already; and
        // added by someone who'd taken it off that page, it's back on it.
        let own = Scope::User(caller.user.clone());
        if added && r.holds(&own) {
            let page = if *scope == own { "user".to_string() } else { scope.to_string() };
            let was = r.arrangements(&caller.user).get(&page).cloned().unwrap_or(Value::Null);
            let mut items = was["items"].as_array().cloned().unwrap_or_default();
            let hidden = was["hidden"].as_array().cloned().unwrap_or_default();
            let shown: Vec<Value> = hidden.iter().filter(|n| n.as_str() != Some(app)).cloned().collect();
            let widget = widget.as_ref().filter(|_| !here.iter().any(|n| n == app) && !items.iter().any(|item| item["app"] == app && item["widget"].is_string()));
            if let Some(widget) = widget {
                items.push(json!({ "app": app, "widget": widget, "id": new_id() }));
            }
            if widget.is_some() || shown.len() != hidden.len() {
                let hidden = if shown.len() != hidden.len() { json!(shown) } else { was["hidden"].clone() };
                out.push((own, content::arrange(&page, json!(items), hidden, was["pages"].clone())));
            }
        }
        Ok(out)
    })
}

/// Forks `app` as a new app called `name`, owned by `owner` (by default the caller), with data of its own; it
/// runs `app`'s latest deploy until it's deployed itself.
pub async fn fork(rt: &Rt, caller: &Caller, app: &str, name: &str, owner: Option<&str>) -> Result<()> {
    let bundle = rt.read(|r| {
        let m = r.db();
        ensure!(m.app(name)?.is_none(), "there's already an app called {name}");
        m.app(app)?.with_context(|| format!("no app named {app}"))?.bundle.with_context(|| format!("{app} has no deploy to fork"))
    })?;
    let owner = owner.map(str::to_string).unwrap_or_else(|| format!("user:{}", caller.user));
    let builds = deploy::bundle::services(&database_runtime::blob(&rt, &bundle).await?).into_values();
    rt.as_member(caller, content::fork(name, &owner, app)).await?;
    rt.as_member(caller, content::deploy(name, &bundle, builds)).await
}

/// Deploys `bundle` as `app`'s code, for someone acting for its owner. Deploying an older bundle again rolls back.
/// `bundle` may be the start of an earlier deploy's. With `wait`, → whether this device runs it within a few
/// seconds, as its next call here would.
pub async fn deploy(rt: &Rt, caller: &Caller, app: &str, bundle: &str, wait: bool) -> Result<bool> {
    let bundle = rt.read(|r| {
        let m = r.db();
        let a = m.app(app)?.with_context(|| format!("no app named {app}"))?;
        allowed!(m.may(&caller.user, "deploy", &scopes::app(app)), "only someone acting for {app}'s owner ({}) deploys it", a.owner);
        ensure!(a.deleted.is_none(), "{app} is in Recently Deleted: apps_restore {app} brings it back to deploy to, apps_erase {app} frees its name for good, or give this app another name");
        Ok(match a.history.iter().find(|(h, ..)| bundle.len() >= 8 && h.starts_with(bundle)) {
            Some((h, ..)) => h.clone(),
            None => bundle.to_string(),
        })
    })?;
    // From elsewhere, only a bundle they may read: their own, or one an entry names for them; so too its service's
    // builds, which the deploy names among its blobs.
    ensure!(database_runtime::may_read(&rt, caller, &bundle)?, "{app} has no deploy {bundle}: its versions are in apps_list's history");
    let bytes = database_runtime::blob(&rt, &bundle).await?;
    deploy::bundle::module(&bytes)?;
    let builds: Vec<String> = deploy::bundle::services(&bytes).into_values().collect();
    for build in &builds {
        ensure!(database_runtime::may_read(&rt, caller, build)?, "{app}'s bundle names a build of its service, {build}, that isn't here: put it first");
    }
    rt.as_member(caller, content::deploy(app, &bundle, builds)).await?;
    Ok(wait && rt.runs_latest(app, Duration::from_secs(10)).await?)
}

/// Deploys `bundle` as `app`, as `deploy` does, making the app first if there's none by that name, owned by
/// `owner` (by default the caller). → `{ bundle, made, before, here, tools }`: whether it was made, the deploy it ran
/// before, and with `wait`, whether this device runs it now and then its tools (or why it doesn't start).
pub async fn deploy_making(rt: &Rt, caller: &Caller, app: &str, bundle: &str, owner: Option<&str>, wait: bool) -> Result<Value> {
    let before = rt.read(|r| {
        let m = r.db();
        Ok(match m.app(app)? {
            Some(a) => {
                allowed!(m.may(&caller.user, "deploy", &scopes::app(app)), "there's already an app called {app}, and it isn't yours: give yours another name, or fork it");
                Some(a.bundle)
            }
            None => None,
        })
    })?;
    if before.is_none() {
        ensure!(scopes::valid_app_name(app), "{app}: an app's name is lowercase words joined by -, like water-log");
        let owner = owner.map(str::to_string).unwrap_or_else(|| format!("user:{}", caller.user));
        rt.as_member(caller, content::app(app, &owner)).await?;
    }
    let here = deploy(rt, caller, app, bundle, wait).await?;
    let bundle = rt.read(|r| Ok(r.db().app(app)?.and_then(|a| a.bundle.clone())))?;
    let mut out = json!({ "bundle": bundle, "made": before.is_none(), "before": before.flatten(), "here": here });
    if here {
        match rt.describe(app).await {
            Ok(d) => {
                out["tools"] = json!(d["tools"].as_array().into_iter().flatten().filter_map(|o| o["name"].as_str()).filter(|o| !o.starts_with('_')).collect::<Vec<_>>());
                out["lacking"] = json!(deploy::build::lacking(&d));
            }
            Err(e) => out["failed"] = json!(format!("{e:#}")),
        }
    }
    Ok(out)
}

/// An app's code as deployed, its latest or an earlier deploy's: its files, or one of them. A built-in app no one
/// has added gives the release's. Everyone may read every app's code.
pub async fn source(rt: &Rt, app: &str, bundle: Option<&str>, path: Option<&str>) -> Result<Value> {
    let history = rt.read(|r| Ok(r.db().app(app)?.map(|a| a.history)))?;
    let built_in = rt.built_in(app);
    ensure!(history.is_some() || built_in.is_some(), "no app named {app}");
    let history = history.unwrap_or_default();
    let bundle = match bundle {
        Some(b) => history.iter().find(|(h, ..)| h.starts_with(b)).with_context(|| format!("{app} has no deploy {b}"))?.0.clone(),
        None => history.last().map(|h| h.0.clone()).or(built_in).with_context(|| format!("{app} has no deploy yet"))?,
    };
    let mut out = deploy::build::show(&database_runtime::blob(&rt, &bundle).await?, path)?;
    out["app"] = json!(app);
    out["bundle"] = json!(bundle);
    Ok(out)
}
