//! The views in the window, and the files each open app's page has loaded.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ::client::{Client, Subscription};
use data_encoding::BASE64;
use serde_json::{Value, json};
use url::Url;

use crate::{app, spawn, views};

/// The views in the window, by name: the window's own page (`client`) and each app that's open.
pub(crate) struct Views {
    pub(crate) client: Arc<Client>,
    pub(crate) subscriptions: Mutex<HashMap<String, Arc<Subscription>>>,
    /// The files each open app's page has loaded, by the app's name.
    pub(crate) loaded: Mutex<HashMap<String, HashSet<String>>>,
    pub(crate) started: Instant,
    /// The windows that are on screen, the Home Screen's (`main`) and on a computer each app's (`hosts`), so its
    /// views ask again when something changed.
    pub(crate) on_screen: Mutex<HashSet<String>>,
    /// Whether a server said something changed while the window wasn't on screen, so its views ask again once it is.
    pub(crate) missed: AtomicBool,
    /// Whether this device hears its notifications yet: from when the window first shows its person's home.
    pub(crate) hearing: AtomicBool,
    /// When each app a server said was just deployed began deploying, by its name, in milliseconds since the Unix
    /// epoch, until its window is next drawn (`deployed`).
    pub(crate) deploys: Mutex<HashMap<String, u64>>,
}

impl Views {
    /// What `view` has been answered. A view that hears `changed` asks again.
    pub(crate) fn subscription(&self, view: &str) -> Arc<Subscription> {
        let mut shown = self.subscriptions.lock().expect("no view panics");
        let new = || {
            let view = view.to_owned();
            Subscription::new(self.client.clone(), move || {
                app().shell.emit("changed", json!(view));
                // The client's list of files is taken once a new version's files are all kept (`keep_whole`): the open
                // apps it no longer names load again then, on it.
                if view == "client" {
                    spawn(async {
                        let Ok(views) = views() else { return };
                        let Ok(listed) = views.listed(None).await else { return };
                        let outdated = views.outdated(None, &listed);
                        if !outdated.is_empty() {
                            eprintln!("{outdated:?} were loaded against a version this client no longer lists");
                            app().shell.emit("stale", json!(outdated));
                        }
                    });
                }
            })
        };
        shown.entry(view.to_owned()).or_insert_with(new).clone()
    }

    /// Says whether the window `label` is on screen.
    pub(crate) fn shown(&self, label: &str, on_screen: bool) {
        let mut shown = self.on_screen.lock().expect("no view panics");
        if on_screen {
            shown.insert(label.to_owned());
        } else {
            shown.remove(label);
        }
    }

    /// Whether any window is on screen.
    pub(crate) fn on_screen(&self) -> bool {
        !self.on_screen.lock().expect("no view panics").is_empty()
    }

    /// Asks again, as a window is back on screen, if something changed while none was.
    pub(crate) async fn caught_up(&self) {
        if self.on_screen() && self.missed.swap(false, Ordering::Relaxed) {
            self.refresh(None).await
        }
    }

    /// Asks again for what every view was answered, all at once, or only of `app`'s tools.
    pub(crate) async fn refresh(&self, app: Option<&str>) {
        let shown: Vec<_> = self.subscriptions.lock().expect("no view panics").values().cloned().collect();
        let mut asks = tokio::task::JoinSet::new();
        for shown in shown {
            let app = app.map(str::to_owned);
            asks.spawn(async move { shown.refresh(app.as_deref()).await });
        }
        while asks.join_next().await.is_some() {}
    }

    /// The files of every app a server lists for `subject`, a group or the network, or for this client's user, as MCP
    /// lists resources.
    pub(crate) async fn listed(&self, subject: Option<&str>) -> anyhow::Result<Value> {
        self.subscription("client").request("resources/list", listing(subject)).await
    }

    /// One file a server listed: what kind it is, and what's in it, which is its text or, for a picture or a font,
    /// base64 as MCP's `blob`. An app's is read for whoever it was listed for, `subject`, as a server finds the app
    /// among theirs. A set of an app's files never changes, so a file that's kept is never asked for again.
    pub(crate) async fn read(&self, resource: &str, subject: Option<&str>) -> anyhow::Result<(String, Vec<u8>)> {
        let mut request = listing(subject);
        request["uri"] = json!(resource);
        let read = match self.client.cached("resources/read", &request) {
            Some(read) => read,
            None => self.client.fresh("resources/read", &request).await?,
        };
        let file = &read["contents"][0];
        let kind = file["mimeType"].as_str().unwrap_or_default().to_owned();
        match file["blob"].as_str() {
            Some(blob) => Ok((kind, BASE64.decode(blob.as_bytes())?)),
            None => Ok((kind, file["text"].as_str().unwrap_or_default().as_bytes().to_vec())),
        }
    }

    /// One file for an app's page: `oneapp://<app>.localhost/<app>/<path>` is `path` among the latest files of
    /// `app` this client knows of. An app opened for a group or the network says so after its name, last part
    /// first (`notes.family.groups.localhost`), and its files are the ones listed for them. One a server didn't
    /// list, as a page loads none of them (a sourcemap), is read by name from the same version.
    pub(crate) async fn file(&self, address: &str) -> anyhow::Result<(String, Vec<u8>)> {
        let address = Url::parse(address)?;
        // A large file a tool answered with, which the client keeps, at the address the answer gave an app for it.
        if let Some(path) = address.path().trim_start_matches('/').strip_prefix(".file/") {
            return self.client.file(path).ok_or_else(|| anyhow::anyhow!("this client keeps no such file"));
        }
        let (resource, subject) = self.resource(&address).await?;
        self.read(&resource, subject.as_deref()).await
    }

    /// The file an app's page names at `address` (`file`), as the server lists it, `ui://<app>/<code>/<path>`, which
    /// never changes, and whoever it's listed for, as what it loaded (`stale`).
    pub(crate) async fn resource(&self, address: &Url) -> anyhow::Result<(String, Option<String>)> {
        let file = address.path().trim_start_matches('/');
        let none = || anyhow::anyhow!("no app here has {file}");
        let (name, path) = file.split_once('/').ok_or_else(none)?;
        // `<app>@<version>` is that version's file, as the edge has it for a browser: an icon a page shows, by what's
        // in it (`ui://icons/<sha256>/…`), which never changes.
        if let Some((name, code)) = name.split_once('@') {
            return Ok((format!("ui://{name}/{code}/{path}"), None));
        }
        let app = address.host_str().and_then(|host| host.strip_suffix(".localhost")).unwrap_or_default();
        // Android's and Windows' web views name the app after the scheme, in a host's name.
        let app = app.strip_prefix("oneapp.").unwrap_or(app);
        let subject: Vec<&str> = app.split('.').skip(1).collect();
        let subject = (!subject.is_empty()).then(|| subject.into_iter().rev().collect::<Vec<_>>().join("/"));
        let listed = self.listed(subject.as_deref()).await?;
        let mut versions = resources(&listed).filter_map(parts).filter(|(of, ..)| *of == name);
        let (_, code, _) = versions.next().ok_or_else(none)?;
        let resource = format!("ui://{name}/{code}/{path}");
        if resources(&listed).any(|listed| listed == resource) {
            self.loaded.lock().expect("no view panics").entry(app.to_owned()).or_default().insert(resource.clone());
        }
        Ok((resource, subject))
    }

    /// The open apps loaded against a version of their app, or a build of the UI library, that `changed` says
    /// isn't the one there is now, each as its address names it: a server says the UI library's build (`library`),
    /// and the version each app of the owners whose apps moved runs at (`apps`, by owner). A version that's
    /// still one of them is current, as an app opened for another owner may be on its own, and so is one this client's
    /// list (`listed`) still names, as its new version's files aren't all kept yet: it loads again once they are, as
    /// the list that names them is taken (`subscription`), not before, onto the same version again. Another app's
    /// files an app shows, as the launcher shows every app's icon, aren't what it runs. They have loaded nothing once
    /// this has named them.
    pub(crate) fn stale(&self, changed: &Value, listed: &Value) -> Vec<String> {
        let listed: HashSet<&str> = resources(listed).collect();
        let gone = |name: &str, code: &str| {
            if name == "ui" {
                return changed["library"].as_str().is_some_and(|library| library != code);
            }
            let owners = changed["apps"].as_object().into_iter().flat_map(|owners| owners.values());
            let now: Vec<&str> = owners.filter_map(|apps| apps[name].as_str()).collect();
            !now.is_empty() && !now.contains(&code)
        };
        let mut loaded = self.loaded.lock().expect("no view panics");
        let stale: Vec<String> = loaded
            .iter()
            .filter(|(app, files)| runs(app, files).any(|(file, name, code)| gone(name, code) && !listed.contains(file)))
            .map(|(app, _)| app.clone())
            .collect();
        loaded.retain(|app, _| !stale.contains(app));
        stale
    }

    /// The open apps for `subject` that run a file a server no longer lists (`runs`): their app or the UI library has
    /// a new version. They have loaded nothing once this has named them.
    pub(crate) fn outdated(&self, subject: Option<&str>, listed: &Value) -> Vec<String> {
        let listed: HashSet<&str> = resources(listed).collect();
        let whose = subject.map(|subject| subject.split('/').rev().collect::<Vec<_>>().join("."));
        let mut loaded = self.loaded.lock().expect("no view panics");
        let outdated: Vec<String> = loaded
            .iter()
            .filter(|(app, _)| app.split_once('.').map(|(_, whose)| whose) == whose.as_deref())
            .filter(|(app, files)| runs(app, files).any(|(file, ..)| !listed.contains(file)))
            .map(|(app, _)| app.clone())
            .collect();
        loaded.retain(|app, _| !outdated.contains(app));
        outdated
    }
}

/// What asks a server for the files of `subject`'s apps, or this client's user's.
pub(crate) fn listing(subject: Option<&str>) -> Value {
    subject.map_or(json!({}), |subject| json!({"_meta": {"subject": subject}}))
}

/// The address of each file in a list of them.
fn resources(listed: &Value) -> impl Iterator<Item = &str> {
    listed["resources"].as_array().into_iter().flatten().filter_map(|resource| resource["uri"].as_str())
}

/// What an open app, by its address's name, runs of the `files` it loaded: its own app's and the UI library's, each
/// as its address, app and version's code. Another app's file it shows, as the launcher shows every app's
/// icon, isn't what it runs.
fn runs<'a>(app: &'a str, files: &'a HashSet<String>) -> impl Iterator<Item = (&'a str, &'a str, &'a str)> {
    let own = app.split('.').next().unwrap_or(app);
    files.iter().filter_map(|file| Some((file.as_str(), parts(file)?))).filter(move |(_, (name, ..))| [own, "ui"].contains(name)).map(|(file, (name, code, _))| (file, name, code))
}

/// An app's file by its address, `ui://<app>/<code>/<path>`: its app, the version's code, and its path.
pub(crate) fn parts(resource: &str) -> Option<(&str, &str, &str)> {
    let mut parts = resource.strip_prefix("ui://")?.splitn(3, '/');
    Some((parts.next()?, parts.next()?, parts.next()?))
}
