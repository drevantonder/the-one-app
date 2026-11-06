//! The built-in apps an update deploys, and those it retires when their folder went or was renamed.

use super::*;

/// Deletes the built-in apps whose folder is gone from `src`, renamed or removed, where the network still has them:
/// they'd stay as apps no release ships. `built_in` is the bundle the running release ships under a name, which says
/// it whatever the update sends; `deploys` 0 is one it offers but the network never added. Only one still running the
/// release's own code goes: one an admin deployed their own code to (from its own repo, say) is theirs now, and stays.
/// They wait in Recently Deleted, where an admin could restore one. Gives each renamed one's new name, with where the old one was installed.
pub(super) fn retire_gone(src: &Path, renamed: &[(String, String)]) -> Result<Vec<(String, Vec<String>)>> {
    let mut moved = Vec::new();
    for app in crate::node().get("/v1/apps")?.as_array().into_iter().flatten() {
        let ours = app["bundle"].is_null() || app["bundle"] == app["built_in"];
        let had = ours && app["built_in"].is_string() && app["deploys"].as_u64().is_some_and(|n| n > 0);
        let Some(name) = app["name"].as_str().filter(|_| had) else { continue };
        if app["deleted"].is_null() && !src.join("apps").join(name).join("app.js").is_file() {
            crate::node().post("/v1/retire", json!({ "app": name, "how": "deleted" }))?;
            match renamed.iter().find(|(old, _)| old == name) {
                Some((_, new)) => {
                    println!("Deleted {name}: it's {new} now");
                    let scopes = app["installed"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
                    moved.push((new.clone(), scopes));
                }
                None => println!("Deleted {name}: its folder is gone from apps/, so no release ships it"),
            }
        }
    }
    Ok(moved)
}

/// Adds each renamed built-in app where its old name was installed.
pub(super) fn add_where(moved: &[(String, Vec<String>)]) {
    for (app, scopes) in moved {
        for scope in scopes {
            match crate::node().post("/v1/install", json!({ "app": app, "scope": scope, "installed": true })) {
                Ok(_) => println!("Added {app} to {scope}, where its old name was"),
                Err(err) => println!("  couldn't add {app} to {scope}: {err:#}"),
            }
        }
    }
}

/// Deploys the built-in apps as they are in `src`, where the network has them and what they're built from changed (an
/// app's folder, or one beside it that its links go to, which is why not only the folders named are looked at: an app
/// whose bundle is what's deployed already is left).
pub(super) fn deploy_apps(src: &Path, changed: &[String]) -> Result<()> {
    if changed.is_empty() {
        return Ok(());
    }
    for app in crate::node().get("/v1/apps")?.as_array().into_iter().flatten() {
        let Some(name) = app["name"].as_str().filter(|_| app["built_in"].is_string()) else { continue };
        let dir = src.join("apps").join(name);
        if !dir.join("app.js").is_file() {
            continue;
        }
        let files = one::app::bundle::sources(&dir)?;
        let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
        let bundle = one::node::built_in_bundle(&files)?;
        let blob = crate::node().put_blob(&bundle)?;
        if app["bundle"].as_str() != Some(blob.as_str()) {
            crate::node().post("/v1/deploy", json!({ "app": name, "bundle": blob }))?;
            println!("Deployed {name}'s new version ({})", &blob[..12]);
        }
    }
    Ok(())
}

/// Deploys this device's release's version of each built-in app the network has, where it runs another.
pub(super) fn deploy_built_in() -> Result<()> {
    for app in crate::node().get("/v1/apps")?.as_array().into_iter().flatten() {
        let (Some(name), Some(bundle)) = (app["name"].as_str(), app["built_in"].as_str()) else { continue };
        if app["bundle"].as_str() != Some(bundle) {
            crate::node().post("/v1/deploy", json!({ "app": name, "bundle": bundle }))?;
            println!("Deployed {name}'s new version ({})", &bundle[..12]);
        }
    }
    Ok(())
}
