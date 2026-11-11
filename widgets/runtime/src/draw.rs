//! Every widget and badge of the apps a page shows, drawn for who's looking.

use std::time::Duration;

use anyhow::Result;
use api::Caller;
use api::runtime::PageApp;
use api::runtime::prelude::*;
use scope::scopes::Scope;
use serde_json::{Value, json};

const BADGE_LIMIT: Duration = Duration::from_secs(5);

/// Every widget, rendered for `viewer`, all at once and each within `WIDGET_LIMIT_MS` so a slow one holds back only
/// itself; with `scope` (`user`, `group:<id>` or `global`), those of the apps added there, for a shell's page of
/// that scope, each reaching only it. Each comes at its own options at every size it draws, with its options
/// (choices worked out now), and as each widget the viewer put on that page with options of their own (`placed`,
/// by its id, at its size). With `only` (an app and a widget), that one alone, however long it takes.
pub(crate) async fn widgets(rt: &Rt, viewer: Caller, scope: Option<&str>, only: Option<(&str, &str)>) -> Result<Vec<Value>> {
    let page = scope.map(|s| if s == format!("user:{}", viewer.user) { "user" } else { s });
    let placed: Vec<Value> = page
        .and_then(|p| rt.read_or(Default::default(), |r| Ok(r.arrangements(&viewer.user))).remove(p))
        .and_then(|a| a["items"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|item| item["widget"].is_string() && item["options"].as_object().is_some_and(|o| !o.is_empty()))
        .collect();
    let mut pages = rt.page_apps(&viewer, scope)?;
    if let Some((app, _)) = only {
        pages.retain(|p| p.app == app);
    }
    let described = futures_util::future::try_join_all(pages.iter().map(|page| rt.describe(&page.app))).await?;
    let drawing: Vec<(&PageApp, Vec<Value>)> = pages
        .iter()
        .zip(described)
        .map(|(page, d)| (page, d["widgets"].as_array().into_iter().flatten().filter(|w| only.is_none_or(|(_, name)| w["name"] == name)).cloned().collect::<Vec<_>>()))
        .filter(|(_, widgets)| !widgets.is_empty())
        .collect();
    // The keys of the database of each app here that draws a widget, those the caller didn't hand, asked for at once
    // (428), as the Vault does: an app's render reaches its data in its own process, which can't say what it lacked as
    // a need, so a widget would only say it stopped working until something else had them.
    let want: Vec<(Scope, &str, &str)> = drawing.iter().flat_map(|(p, _)| p.scopes.iter().map(|s| (s.clone(), p.app.as_str(), key::APP_DATABASE))).collect();
    rt.needs(&viewer, &want).await?;
    let placed = &placed;
    let apps = futures_util::future::join_all(drawing.iter().map(|(page, widgets)| futures_util::future::join_all(widgets.iter().map(|w| within(page, w, placed, only.is_none()))))).await;
    Ok(apps.into_iter().flatten().collect())
}

/// One widget of `widgets`: with `limited`, as `late` if it takes longer than `WIDGET_LIMIT_MS`.
async fn within(page: &PageApp, w: &Value, placed: &[Value], limited: bool) -> Value {
    let app = page.app.as_str();
    let said = json!({ "app": app, "widget": w["name"], "title": w["title"], "description": w["description"], "size": w["size"].as_str().unwrap_or("medium") });
    let drawing = drawn(page, w, placed, said.clone());
    if !limited {
        return drawing.await;
    }
    let late = || {
        tracing::info!("{app} {} took longer than {} ms; it comes alone", w["name"], custom_build::protocol::WIDGET_LIMIT_MS);
        let mut one = said.clone();
        one["late"] = json!(true);
        one["root"] = json!({ "t": "text", "text": custom_build::protocol::LATE_WIDGET, "tone": "muted" });
        one
    };
    tokio::time::timeout(Duration::from_millis(custom_build::protocol::WIDGET_LIMIT_MS), drawing).await.unwrap_or_else(|_| late())
}

/// A widget at every size it draws, with its options and the viewer's placed copies, added to `said`.
async fn drawn(page: &PageApp, w: &Value, placed: &[Value], mut said: Value) -> Value {
    let (app, draws) = (page.app.as_str(), &page.draws);
    let widget = w["name"].as_str().unwrap_or_default();
    let size = w["size"].as_str().unwrap_or("medium");
    let none = json!({});
    let options = async {
        match w["options"].as_array().is_some_and(|o| !o.is_empty()) {
            true => draws.widget_options(widget).await.unwrap_or_else(|err| {
                tracing::warn!("{app} {widget}'s options: {err:#}");
                w["options"].clone()
            }),
            false => Value::Null,
        }
    };
    let (options, root) = tokio::join!(options, draws.widget(widget, size, &none));
    let root = match root {
        Ok(root) => root,
        // A shell that knows `error` says the widget is broken; one that doesn't still draws `root`.
        Err(err) => {
            said["error"] = json!(format!("{err:#}"));
            said["root"] = json!({ "t": "text", "text": "This widget stopped working.", "tone": "muted" });
            return said;
        }
    };
    // Each other size it draws, as its own tree; one that fails is left out, and shells draw `root`.
    let others = w["sizes"].as_array().into_iter().flatten().filter_map(|s| s.as_str()).filter(|s| *s != size);
    let others = futures_util::future::join_all(others.map(|other| {
        let none = &none;
        async move { (other, draws.widget(widget, other, none).await) }
    }));
    let mine = placed.iter().filter(|i| i["app"] == app && i["widget"] == widget).map(|item| async {
        let at = item["size"].as_str().unwrap_or(size);
        let tree = match draws.widget(widget, at, &item["options"]).await {
            Ok(tree) => json!({ "sizes": { at: tree } }),
            Err(err) => json!({ "error": format!("{err:#}") }),
        };
        (item["id"].as_str().unwrap_or_default().to_string(), tree)
    });
    let (others, mine) = tokio::join!(others, futures_util::future::join_all(mine));
    let mut sizes: serde_json::Map<String, Value> = others.into_iter().filter_map(|(other, tree)| Some((other.to_string(), tree.ok()?))).collect();
    sizes.insert(size.to_string(), root.clone());
    said["sizes"] = Value::Object(sizes);
    said["root"] = root;
    if !options.is_null() {
        said["options"] = options;
    }
    if !mine.is_empty() {
        said["placed"] = Value::Object(mine.into_iter().collect());
    }
    said
}

/// Each app's badge, the count its icon shows, for `viewer`, as `widgets` reaches scopes; apps with none, or 0,
/// are left out, as is one that fails or takes longer than `BADGE_LIMIT`: a page waits for its badges.
pub(crate) async fn badges(rt: &Rt, viewer: Caller, scope: Option<&str>) -> Result<Value> {
    let mut asked = tokio::task::JoinSet::new();
    for page in rt.page_apps(&viewer, scope)? {
        let rt = rt.clone();
        asked.spawn(async move {
            let badge = async { if rt.describe(&page.app).await?["badge"] == true { page.draws.badge().await } else { Ok(Value::Null) } };
            match tokio::time::timeout(BADGE_LIMIT, badge).await {
                Ok(Ok(n)) => n.as_f64().is_some_and(|n| n > 0.0).then_some((page.app.clone(), n)),
                Ok(Err(err)) => {
                    tracing::warn!("{} badge: {err:#}", page.app);
                    None
                }
                Err(_) => {
                    tracing::warn!("{} badge took longer than {}s; left out", page.app, BADGE_LIMIT.as_secs());
                    None
                }
            }
        });
    }
    Ok(Value::Object(asked.join_all().await.into_iter().flatten().collect()))
}
