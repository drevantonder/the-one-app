//! Installing apps from git repos, as `npx skills` installs agent skills (skills.sh): an app is a folder of a
//! repo's `apps/`, with a `SKILL.md`, and there's nothing to publish. `one install <owner>/<repo>[@<ref>] --app
//! <name>` fetches the repo with git, deploys that folder as a copy of the installer's own, with where it came from
//! (`deploy`'s `from`), which the network keeps with the app and `apps` gives back. In a network's repo (a
//! folder with `apps/`, or an `apps.lock`), `apps.lock` records each too, so `one install` there installs
//! everything it names at the commits it names: that's how a network is made again from its repo. The network
//! itself looks for newer commits once a day, and offers them as updates (`one apps check`, `one apps update`).

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, ExitCode};
use std::time::Duration;
use std::{env, fs};

use anyhow::{Context, Result, bail};
use client::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Where an installer says it installed an app, for the directory of apps (`www/app-directory/README.md`).
const DIRECTORY: &str = "https://one.ptcau.dev/apps/api/installs";
const LOCK: &str = "apps.lock";

/// Where an app comes from: a repo as it's written in a lock file (`owner/repo` on GitHub, a git URL, or a
/// folder's absolute path), and the ref asked for, if one was.
#[derive(Clone)]
struct Source {
    spec: String,
    reference: Option<String>,
}

impl Source {
    /// `owner/repo`, a git URL or a path, each with `@<ref>` after it or not.
    fn parse(written: &str) -> Result<Self> {
        let (repo, reference) = match written.rsplit_once('@') {
            Some((repo, reference)) if !reference.contains('/') && !reference.contains(':') && !repo.is_empty() => {
                (repo, Some(reference.to_owned()))
            }
            _ => (written, None),
        };
        let spec = if Path::new(repo).exists() || Path::new(repo).is_absolute() || repo.starts_with(['.', '~']) {
            fs::canonicalize(repo).with_context(|| format!("There's no folder {repo}"))?.to_string_lossy().into_owned()
        } else if repo.contains("://") || repo.starts_with("git@") {
            repo.to_owned()
        } else if repo.split('/').count() == 2 && !repo.contains(' ') {
            repo.trim_end_matches(".git").to_owned()
        } else {
            bail!("{written} isn't a repo: write owner/repo (on GitHub), a git URL or a folder, with @<ref> or not")
        };
        Ok(Self { spec, reference })
    }

    /// What git fetches it from.
    fn url(&self) -> String {
        if self.github().is_some() && !self.spec.contains("://") {
            format!("https://github.com/{}.git", self.spec)
        } else {
            self.spec.clone()
        }
    }

    fn local(&self) -> bool {
        Path::new(&self.spec).is_absolute()
    }

    /// `owner/repo`, for a repo on GitHub.
    fn github(&self) -> Option<String> {
        let path = self.spec.strip_prefix("https://github.com/").or(self.spec.strip_prefix("git@github.com:"));
        let path = path.unwrap_or(&self.spec).trim_end_matches('/').trim_end_matches(".git");
        (!self.local() && !path.contains(':') && path.split('/').count() == 2).then(|| path.to_owned())
    }

    /// Where this device keeps its copy of the repo, which an agent reads an app's code in before installing it.
    fn folder(&self) -> Result<PathBuf> {
        let mut folder = client::home()?.join("git");
        if self.local() {
            folder.push("local");
            folder.extend(Path::new(&self.spec).components().filter_map(|part| match part {
                Component::Normal(part) => Some(part),
                _ => None,
            }));
            return Ok(folder);
        }
        let named = self.spec.split_once("://").map_or(self.spec.as_str(), |(_, rest)| rest);
        let named = named.replace(':', "/");
        if !self.spec.contains("://") && !self.spec.starts_with("git@") {
            folder.push("github.com");
        }
        for part in named.trim_start_matches("git@").trim_end_matches(".git").split('/') {
            if !part.is_empty() && part != "." && part != ".." {
                folder.push(part);
            }
        }
        Ok(folder)
    }

    /// Fetches the repo, and gives where its copy is and the commit `at` is (a commit, or the ref asked for, or what
    /// the repo has by default), checked out there.
    fn fetch(&self, at: Option<&str>) -> Result<(PathBuf, String)> {
        let folder = self.folder()?;
        if folder.join(".git").exists() {
            git(&folder, &["fetch", "--quiet", "--tags", "--force", "origin"])?;
        } else {
            fs::create_dir_all(folder.parent().context("a repo's folder is in one")?)?;
            let mut clone = vec!["clone", "--quiet", "--no-checkout"];
            // Files are fetched as they're checked out, which for a repo at a URL is only the commits installed.
            if !self.local() {
                clone.push("--filter=blob:none");
            }
            let url = self.url();
            let path = folder.to_string_lossy();
            git(Path::new("."), &[clone.as_slice(), &[url.as_str(), &path]].concat())
                .with_context(|| format!("Couldn't fetch {}", self.spec))?;
        }
        let tried: Vec<String> = match at.or(self.reference.as_deref()) {
            None => vec!["origin/HEAD".to_owned()],
            Some(at) => vec![format!("origin/{at}"), at.to_owned()],
        };
        let commit = tried
            .iter()
            .find_map(|at| git(&folder, &["rev-parse", "--verify", "--quiet", &format!("{at}^{{commit}}")]).ok())
            .with_context(|| format!("{} has no {}", self.spec, tried.last().expect("one is tried")))?;
        git(&folder, &["checkout", "--quiet", "--force", "--detach", &commit])?;
        Ok((folder, commit))
    }

}

fn git(folder: &Path, args: &[&str]) -> Result<String> {
    let ran = Command::new("git").arg("-C").arg(folder).args(args).env("GIT_TERMINAL_PROMPT", "0").output()?;
    if !ran.status.success() {
        bail!("git {} said: {}", args.join(" "), String::from_utf8_lossy(&ran.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&ran.stdout).trim().to_owned())
}

/// An app as `apps.lock` records it, by its name there.
#[derive(Clone, Serialize, Deserialize)]
struct Locked {
    source: String,
    #[serde(rename = "ref")]
    reference: Option<String>,
    commit: String,
    hash: String,
    #[serde(rename = "for")]
    of: String,
    /// The apps it requires, as the server said when it was installed, which come before it.
    #[serde(default)]
    requires: Vec<String>,
}

#[derive(Deserialize)]
struct Lock {
    #[serde(default)]
    apps: BTreeMap<String, Locked>,
}

/// The lock file, where this is run in a network's repo: a folder with `apps/` or an `apps.lock`.
fn lock_file() -> Option<PathBuf> {
    (Path::new(LOCK).exists() || Path::new("apps").is_dir()).then(|| PathBuf::from(LOCK))
}

fn read_lock(file: &Path) -> Result<BTreeMap<String, Locked>> {
    let Ok(text) = fs::read_to_string(file) else { return Ok(BTreeMap::new()) };
    let lock: Lock = serde_json::from_str(&text).with_context(|| format!("{} says an app wrong", file.display()))?;
    Ok(lock.apps)
}

fn write_lock(file: &Path, locked: &BTreeMap<String, Locked>) -> Result<()> {
    let text = serde_json::to_string_pretty(&json!({"version": 1, "apps": locked}))?;
    Ok(fs::write(file, text + "\n")?)
}

/// Calls a tool for `of` (a user, a group or the network), as `ONE_SUBJECT` would have it, and gives its MCP result.
async fn call_raw(client: &Client, of: Option<&str>, tool: &str, arguments: Value) -> Result<Value> {
    let mut params = json!({"name": tool, "arguments": arguments});
    if let Some(of) = of {
        params["_meta"]["subject"] = json!(of);
    }
    client.request("tools/call", params).await
}

/// Calls a tool for `of` (a user, a group or the network), as `ONE_SUBJECT` would have it, and gives its text.
async fn call(client: &Client, of: Option<&str>, tool: &str, arguments: Value) -> Result<String> {
    crate::said(call_raw(client, of, tool, arguments).await?)
}

/// A requirement the server said an app lacks when `deploy` refused it.
struct Missing {
    name: String,
    why: String,
    app: Option<String>,
    deleted: bool,
}

fn missing(result: &Value) -> Vec<Missing> {
    result["structuredContent"]["missing"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Missing {
                name: item["name"].as_str()?.to_owned(),
                why: item["why"].as_str().unwrap_or_default().to_owned(),
                app: item["app"].as_str().map(str::to_owned),
                deleted: item["deleted"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// What's installed for `of` and the network, as the tool `apps` lists it, and who `of` is.
async fn installed(client: &Client, of: Option<&str>) -> Result<(String, Vec<Value>)> {
    let response: Value = serde_json::from_str(&call(client, of, "apps", json!({})).await?)?;
    let of = response["for"].as_str().context("`apps` says who it lists for")?.to_owned();
    Ok((of, response["apps"].as_array().cloned().unwrap_or_default()))
}

/// A repo's apps: each folder of its `apps/` with a `SKILL.md`, with what that says it's called and is for.
fn offered(folder: &Path) -> Vec<(String, String)> {
    let mut offered: Vec<(String, String)> = fs::read_dir(folder.join("apps"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|app| {
            let skill = fs::read_to_string(app.path().join("SKILL.md")).ok()?;
            let front = skill.strip_prefix("---")?.split("\n---").next()?.to_owned();
            let field = |name: &str| {
                front
                    .lines()
                    .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                    .map(|value| value.trim().trim_matches(|quote| quote == '"' || quote == '\'').to_owned())
            };
            Some((
                field("name").unwrap_or_else(|| app.file_name().to_string_lossy().into_owned()),
                field("description")?,
            ))
        })
        .collect();
    offered.sort();
    offered
}

/// Stops with what to build, where an app brings a NIF that isn't built for every kind of machine the network's
/// servers run on, here or published where its `server/checksums.json` says, which the server downloads
/// (`OneCore.Apps.Precompiled`): its source is never deployed, only the library built from it (`OneCore.App`),
/// so the agent installing or deploying it builds it, as `tooling/nifs` does for the apps the One App ships with.
async fn built(client: &Client, of: &str, name: &str, app: &Path) -> Result<()> {
    if !app.join("server/nif").is_dir() {
        return Ok(());
    }
    let checksums = app.join("server/checksums.json");
    let published: Value = match fs::read_to_string(&checksums) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("{} isn't JSON", checksums.display()))?,
        // An app whose NIF nobody has published has no such file yet: only what's built here counts.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(error) => return Err(error).with_context(|| format!("{} can't be read", checksums.display())),
    };
    let builds: Value = serde_json::from_str(&call(client, Some(of), "builds", json!({})).await?)?;
    let mut kinds: Vec<String> = builds["servers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|server| Some(server["target"].as_str()?.to_owned()))
        .collect();
    kinds.sort();
    kinds.dedup();
    for crate_folder in fs::read_dir(app.join("server/nif")).into_iter().flatten().flatten() {
        let crate_folder = crate_folder.path();
        if !crate_folder.join("Cargo.toml").exists() {
            continue;
        }
        let crate_name = crate_folder.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let missing: Vec<&String> = kinds
            .iter()
            .filter(|kind| {
                !crate_folder.join(format!("{kind}.so")).exists()
                    && published.get(format!("server/nif/{crate_name}/{kind}.so")).is_none()
            })
            .collect();
        if missing.is_empty() {
            continue;
        }
        let library = crate_folder.file_name().unwrap_or_default().to_string_lossy().replace('-', "_");
        let commands: Vec<String> = missing
            .iter()
            .map(|kind| {
                let (os, arch) = kind.split_once('-').unwrap_or((kind.as_str(), ""));
                // The same as tooling/nifs builds them: a Windows DLL is kept under the same kind of name.
                let (triple, build, glibc, file) = match os {
                    "macos" => (format!("{arch}-apple-darwin"), "build", "", format!("lib{library}.dylib")),
                    "windows" => (format!("{arch}-pc-windows-msvc"), "xwin build", "", format!("{library}.dll")),
                    _ => (format!("{arch}-unknown-linux-gnu"), "zigbuild", ".2.35", format!("lib{library}.so")),
                };
                format!(
                    "    rustup target add {triple} && CARGO_TARGET_DIR=target cargo {build} --release --target {triple}{glibc} \
                     && cp target/{triple}/release/{file} {kind}.so"
                )
            })
            .collect();
        bail!(
            "{name} has native code, the NIF in {}, which isn't built for {}, where the network's servers run. Build it \
             as tooling/nifs does, Linux's with zig (cargo-zigbuild) for a server's glibc, Windows' with cargo-xwin and a Mac's on a Mac, then \
             run that again:\n\n    cd {}\n{}",
            crate_folder.display(),
            missing.iter().map(|kind| kind.as_str()).collect::<Vec<_>>().join(" and "),
            crate_folder.display(),
            commands.join("\n")
        );
    }
    Ok(())
}

/// Checks `name` can be deployed from `folder` for `of`, reads its files, and builds the `deploy` tool's arguments.
/// `trusted` says whether it's trusted, or untrusted and runs in a sandbox; left out, it's as the version before was,
/// or for a new one as where it came from says. With `new` it's made where one by that name is installed for someone
/// else `of` is in, as `deploy`'s `new` says.
#[expect(clippy::too_many_arguments, reason = "each says something of the one deploy")]
async fn prepare_deploy(
    client: &Client,
    of: &str,
    name: &str,
    folder: &Path,
    from: &Source,
    commit: &str,
    trusted: Option<bool>,
    new: bool,
) -> Result<Value> {
    let app = folder.join("apps").join(name);
    // Deployed over someone else's app, it would be changed for everyone who has it, not copied.
    let (_, apps) = installed(client, Some(of)).await?;
    let own = format!("{of}/apps/{name}");
    if let Some(other) = apps.iter().find(|it| it["name"] == name && it["for"] == of && it["app"] != own.as_str())
    {
        bail!(
            "{name} is installed for {of} already, as {}. `one uninstall --name {name}` it first, to install a copy of \
             your own in its place.",
            other["app"].as_str().unwrap_or_default()
        );
    }
    built(client, of, name, &app).await?;
    println!(
        "{name}'s code, from {} at {}, is in {}. Read it before installing (the skill installing-apps): its \
         server code runs with full access on {of}'s servers, or untrusted, in a sandbox, with only what it's given.",
        from.spec,
        &commit[..12],
        app.display()
    );
    let files = crate::read(&app, false)?;
    let mut deployed =
        json!({"name": name, "files": files, "from": {"source": from.spec, "ref": from.reference, "commit": commit}});
    if let Some(trusted) = trusted {
        deployed["trusted"] = json!(trusted);
    }
    if new {
        deployed["new"] = json!(true);
    }
    Ok(deployed)
}

/// Prints a successful `deploy`'s answer, reports it to the directory, and gives it as locked.
async fn finish_deploy(
    client: &Client,
    of: &str,
    name: &str,
    from: &Source,
    commit: &str,
    answer: &str,
) -> Result<Locked> {
    let mut deployed: Value = serde_json::from_str(answer)?;
    // What a person reads of it goes to the directory, not the terminal: its icons are long.
    let listing = deployed.as_object_mut().and_then(|it| it.remove("listing")).unwrap_or_default();
    println!("{deployed}");
    let (_, apps) = installed(client, Some(of)).await?;
    let hash = apps
        .iter()
        .find(|app| app["name"] == name && app["for"] == of)
        .and_then(|app| app["from"]["hash"].as_str())
        .with_context(|| format!("{name} was deployed, but `apps` doesn't list it for {of}"))?;
    report(from, name, commit, listing).await;
    // `deploy` answers with the apps it requires, by name, each with why.
    let requires = deployed["requires"].as_object().into_iter().flatten().map(|(name, _)| name.clone()).collect();
    Ok(Locked {
        source: from.spec.clone(),
        reference: from.reference.clone(),
        commit: commit.to_owned(),
        hash: hash.to_owned(),
        of: of.to_owned(),
        requires,
    })
}

/// Installs apps from a checked-out repo, resolving each app's missing `requires` transitively from the server's
/// refusal and installing them first (from the repo where it has them, or from what's already deployed in the network).
struct Installer<'a> {
    client: &'a Client,
    of: &'a str,
    folder: &'a Path,
    source: &'a Source,
    commit: &'a str,
    trusted: Option<bool>,
    lock: Option<&'a Path>,
    locked: &'a mut BTreeMap<String, Locked>,
    visiting: Vec<String>,
    done: HashSet<String>,
}

impl<'a> Installer<'a> {
    fn in_repo(&self, name: &str) -> bool {
        !name.is_empty()
            && !name.contains(['/', '\\'])
            && name != "."
            && name != ".."
            && self.folder.join("apps").join(name).join("SKILL.md").exists()
    }

    fn install_app<'b>(&'b mut self, name: &'b str, new: bool) -> Pin<Box<dyn Future<Output = Result<()>> + 'b>> {
        Box::pin(async move {
            if self.done.contains(name) {
                return Ok(());
            }
            self.visiting.push(name.to_owned());
            let outcome = self.deploy_with_requires(name, new).await;
            self.visiting.pop();
            outcome
        })
    }

    async fn deploy_with_requires(&mut self, name: &str, new: bool) -> Result<()> {
        let payload =
            prepare_deploy(self.client, self.of, name, self.folder, self.source, self.commit, self.trusted, new)
                .await?;
        loop {
            let result = call_raw(self.client, Some(self.of), "deploy", payload.clone()).await?;
            let text = crate::text(&result);
            if result["isError"] != true {
                let it = finish_deploy(self.client, self.of, name, self.source, self.commit, &text).await?;
                if let Some(file) = self.lock {
                    self.locked.insert(name.to_owned(), it);
                    write_lock(file, self.locked)?;
                }
                self.done.insert(name.to_owned());
                return Ok(());
            }
            let lacking = missing(&result);
            if lacking.is_empty() {
                bail!("{text}");
            }
            self.check_missing(name, &lacking, &text)?;
            let mut installed_from_network = false;
            for req in lacking {
                if self.done.contains(&req.name) {
                    continue;
                }
                if installed_from_network {
                    let (_, apps) = installed(self.client, Some(self.of)).await?;
                    if apps.iter().any(|it| it["name"] == req.name && it["state"] == "installed") {
                        self.done.insert(req.name);
                        continue;
                    }
                }
                let why = if req.why.is_empty() { String::new() } else { format!(" ({})", req.why) };
                if self.in_repo(&req.name) {
                    println!(
                        "{name} requires {}{why}, which isn't installed for {} or the network, so it's installed from \
                         {} too.",
                        req.name, self.of, self.source.spec
                    );
                    self.install_app(&req.name, false).await?;
                } else if let Some(artifact) = req.app {
                    println!(
                        "{name} requires {}{why}, which isn't installed for {} or the network, so {artifact} is \
                         installed for {} too.",
                        req.name, self.of, self.of
                    );
                    let answer: Value = serde_json::from_str(
                        &call(self.client, Some(self.of), "install", json!({"app": artifact, "name": req.name}))
                            .await?,
                    )?;
                    if let Some(also) = answer["also"].as_array() {
                        for line in also.iter().filter_map(Value::as_str) {
                            println!("{line}");
                        }
                    }
                    self.done.insert(req.name);
                    installed_from_network = true;
                }
            }
        }
    }

    fn check_missing(&self, name: &str, lacking: &[Missing], fallback: &str) -> Result<()> {
        for req in lacking {
            if let Some(at) = self.visiting.iter().position(|it| it == &req.name) {
                if self.visiting.len() - at == 1 {
                    bail!("{name} requires itself, so it can't be installed");
                }
                let mut cycle: Vec<&str> = self.visiting[at..].iter().map(String::as_str).collect();
                cycle.push(&req.name);
                bail!("{} require each other, so none can be installed first", cycle.join(" -> "));
            }
        }
        for req in lacking {
            if req.deleted {
                let why = if req.why.is_empty() { String::new() } else { format!(" ({})", req.why) };
                bail!(
                    "{name} requires {}{why}, which is in Recently Deleted for {}: restore it there (`one soft-delete \
                     restore`) first.",
                    req.name, self.of
                );
            }
        }
        let unfound: Vec<&Missing> =
            lacking.iter().filter(|req| !self.in_repo(&req.name) && req.app.is_none()).collect();
        if !unfound.is_empty() {
            let describe = |req: &Missing| {
                if req.why.is_empty() { req.name.clone() } else { format!("{} ({})", req.name, req.why) }
            };
            if let [one] = unfound.as_slice() {
                bail!(
                    "{name} requires {}, which isn't installed for {} or the network, isn't in {}, and isn't deployed \
                     in this network.",
                    describe(one),
                    self.of,
                    self.source.spec
                );
            }
            let list = unfound.iter().map(|req| describe(req)).collect::<Vec<_>>().join(", ");
            bail!(
                "{name} requires {list}, which aren't installed for {} or the network, aren't in {}, and aren't \
                 deployed in this network.",
                self.of,
                self.source.spec
            );
        }
        // If we already installed one of these in this run and the server still says it's missing, stop instead of looping.
        if lacking.iter().any(|req| self.done.contains(&req.name)) {
            bail!("{fallback}");
        }
        Ok(())
    }
}

/// Tells the directory of apps that an app was installed, with what a person reads of it there (`listing`, as
/// `deploy` gave it) and nothing about who installed it. Not for a
/// folder, nor a repo GitHub doesn't say is public, nor where `DO_NOT_TRACK` or `DISABLE_TELEMETRY` (skills.sh's own)
/// is set. It's never waited on long, and its failing fails nothing.
async fn report(from: &Source, name: &str, commit: &str, listing: Value) {
    let set = |variable| env::var(variable).is_ok_and(|value| !value.is_empty() && value != "0");
    let Some(repo) = from.github().filter(|_| !set("DO_NOT_TRACK") && !set("DISABLE_TELEMETRY")) else { return };
    let Ok(web) = client::web() else { return };
    let public = async {
        let response =
            web.get(format!("https://api.github.com/repos/{repo}")).header("user-agent", "one").send().await.ok()?;
        let response: Value = serde_json::from_str(&response.text().await.ok()?).ok()?;
        Some(response["private"] == false)
    };
    let post = async {
        if public.await == Some(true) {
            let body = json!({"source": repo, "app": name, "commit": commit, "listing": listing}).to_string();
            let _ = web.post(DIRECTORY).header("content-type", "application/json").body(body).send().await;
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(3), post).await;
}

/// `one install <repo>[@<ref>] --app <name> ... [--list] [--trusted | --untrusted] [--new]`, for whoever
/// `ONE_SUBJECT` says, or this device's user. A repo's apps are a stranger's, so they're untrusted and run in a
/// sandbox, which needs no server of the user's, unless `--trusted` says otherwise, which only someone who may install
/// for them may; `--new` makes a second copy where one by that name is installed for the network or a group they're
/// in, which is refused without.
pub async fn install(client: &Client, args: &[String]) -> Result<ExitCode> {
    let source = Source::parse(&args[0])?;
    let (mut names, mut list, mut trusted, mut new) = (Vec::new(), false, Some(false), false);
    let mut flags = args[1..].iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--list" => list = true,
            "--trusted" => trusted = Some(true),
            "--untrusted" => trusted = Some(false),
            "--new" => new = true,
            "--app" => names.push(flags.next().context("--app needs an app's name")?.clone()),
            other => bail!(
                "`{other}` isn't something `one install` takes: --app <name>, --trusted, --untrusted, --new or --list"
            ),
        }
    }
    let (folder, commit) = source.fetch(None)?;
    let offered = offered(&folder);
    if list || names.is_empty() {
        let width = offered.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
        for (name, about) in &offered {
            println!("{name:width$}  {about}");
        }
        println!("\n{} at {}, in {}/apps.", source.spec, &commit[..12], folder.display());
        if !list {
            println!("`one install {} --app <name>` installs one.", args[0]);
        }
        return Ok(ExitCode::SUCCESS);
    }
    let (of, _) = installed(client, None).await?;
    // Each requested app must exist in the repo; missing requirements are resolved transitively from the server's
    // refusal and installed first.
    let mut order: Vec<&String> = Vec::new();
    for name in &names {
        if !folder.join("apps").join(name).join("SKILL.md").exists() {
            bail!("{} has no app {name}: `one install {} --list` lists the ones it has", source.spec, args[0]);
        }
        if !order.contains(&name) {
            order.push(name);
        }
    }
    let lock = lock_file();
    let mut locked = match &lock {
        Some(file) => read_lock(file)?,
        None => BTreeMap::new(),
    };
    let mut installer = Installer {
        client,
        of: &of,
        folder: &folder,
        source: &source,
        commit: &commit,
        trusted,
        lock: lock.as_deref(),
        locked: &mut locked,
        visiting: Vec::new(),
        done: HashSet::new(),
    };
    for name in order {
        installer.install_app(name, new).await?;
    }
    match lock {
        Some(file) => println!("Recorded in {}.", file.display()),
        None => println!(
            "Not in a network's repo, so no {LOCK} is written: the network keeps where it came from (`one apps`)."
        ),
    }
    Ok(ExitCode::SUCCESS)
}

/// `one install` in a folder with an `apps.lock`: installs everything it records, at the commits it records, for
/// whoever it records, as it was, each after the ones it records that it requires. An app whose files hash
/// differently now (a NIF built again) is said so.
pub async fn locked(client: &Client) -> Result<ExitCode> {
    let file = PathBuf::from(LOCK);
    let mut waiting = read_lock(&file)?;
    let count = waiting.len();
    while !waiting.is_empty() {
        let mut deployed = Vec::new();
        for (name, it) in &waiting {
            let source = Source { spec: it.source.clone(), reference: it.reference.clone() };
            let (folder, commit) = source.fetch(Some(&it.commit))?;
            if it.requires.iter().any(|required| required != name && waiting.contains_key(required)) {
                continue;
            }
            let payload = prepare_deploy(client, &it.of, name, &folder, &source, &commit, None, false).await?;
            let answer = call(client, Some(&it.of), "deploy", payload).await?;
            let now = finish_deploy(client, &it.of, name, &source, &commit, &answer).await?;
            if now.hash != it.hash {
                println!(
                    "{name}'s files hash differently from what {LOCK} says, from the same commit: a NIF built again does."
                );
            }
            deployed.push(name.clone());
        }
        if deployed.is_empty() {
            bail!(
                "{} use each other, so none can be installed first",
                waiting.keys().cloned().collect::<Vec<_>>().join(", ")
            );
        }
        waiting.retain(|name, _| !deployed.contains(name));
    }
    println!("Installed the {count} apps {LOCK} records.");
    Ok(ExitCode::SUCCESS)
}


/// `one uninstall --name <name>` in a network's repo: uninstalls it, and takes it out of `apps.lock` if that
/// records it for whoever it was uninstalled for, so `one install` there doesn't install it again.
pub async fn uninstall(client: &Client, name: &str) -> Result<ExitCode> {
    let (of, _) = installed(client, None).await?;
    println!("{}", call(client, None, "uninstall", json!({"name": name})).await?);
    let file = PathBuf::from(LOCK);
    let mut locked = read_lock(&file)?;
    if locked.get(name).is_some_and(|it| it.of == of) {
        locked.remove(name);
        write_lock(&file, &locked)?;
        println!("Removed from {LOCK}.");
    }
    Ok(ExitCode::SUCCESS)
}

/// Whether `commit` is in `descendant`'s history, fetching its remote's first where this repo doesn't have both yet.
fn contains(folder: &Path, descendant: &str, commit: &str) -> Result<bool> {
    let contains = || git(folder, &["merge-base", "--is-ancestor", commit, descendant]).is_ok();
    if contains() || git(folder, &["remote", "get-url", "origin"]).is_err() {
        return Ok(contains());
    }
    git(folder, &["fetch", "--quiet", "origin"])?;
    Ok(contains())
}

/// The commit a folder's files came from, whether it has changes not committed, and the repo it's from, or `None`
/// for a folder in no git repo. A commit the remote's default branch doesn't have, fetched if it isn't here yet, is
/// refused, as `deploy_folder` says.
fn pushed(folder: &Path, name: &str) -> Result<Option<(String, bool, String)>> {
    let Ok(commit) = git(folder, &["rev-parse", "HEAD"]) else { return Ok(None) };
    let dirty = !git(folder, &["status", "--porcelain", "--", "."])?.is_empty();
    let remote = git(folder, &["remote", "get-url", "origin"]);
    if remote.is_ok() {
        // The remote's default branch, as `git clone` names it; a repo that never had it named asks the remote.
        let branch = git(folder, &["rev-parse", "--abbrev-ref", "origin/HEAD"]).or_else(|_| {
            git(folder, &["remote", "set-head", "origin", "--auto"])?;
            git(folder, &["rev-parse", "--abbrev-ref", "origin/HEAD"])
        })?;
        if !contains(folder, &branch, &commit)? {
            bail!(
                "{name} wasn't deployed: commit {} isn't on {branch}, so what's live would come from a commit no \
                 branch has. Rebase onto {branch}, push, then deploy.",
                &commit[..12]
            );
        }
    }
    let source = remote.or_else(|_| git(folder, &["rev-parse", "--show-toplevel"]))?;
    Ok(Some((commit, dirty, source)))
}

/// `one deploy ... --files-from <folder> [--force]`. From a folder in a git repo, the deploy says which commit its
/// files came from (`from`), and `dirty` where the folder has changes not committed, as a build does, which only a
/// network being built takes (`OneCore.Tools.Apps`), so what's live is a commit on the default branch. As `git push`
/// refuses a non-fast-forward, it's refused where the live version came from a commit that one doesn't contain, so
/// it would drop changes that are live, unless `--force`. And, as `tooling/release` refuses a build, it's refused
/// where its repo has a remote and the commit isn't on the remote's default branch, fetched if it isn't there yet,
/// which `--force` doesn't change: a commit nobody pushed, such as one from before a rebase, would be live from no
/// branch, and the next deploy from the branch would be refused for not containing it (19 Nov, twice). That's
/// decided here, since only the repo knows which commits contain which; the network only keeps the commit. From a
/// folder in no git repo, it's deployed as it is. The repo is looked at while the network says what's live.
pub async fn deploy_folder(client: &Client, args: &[String]) -> Result<ExitCode> {
    let force = args.iter().any(|arg| arg == "--force");
    let mut args: Vec<String> = args.iter().filter(|arg| *arg != "--force").cloned().collect();
    let after = |flag: &str| args.iter().position(|arg| arg == flag).and_then(|at| args.get(at + 1)).cloned();
    let (Some(folder), Some(name)) = (after("--files-from"), after("--name")) else {
        return crate::command(client, &args).await;
    };
    let folder = PathBuf::from(folder);
    let looked = tokio::task::spawn_blocking({
        let (folder, name) = (folder.clone(), name.clone());
        move || pushed(&folder, &name)
    });
    let (asked, looked) = tokio::join!(call(client, None, "apps", json!({"name": name})), looked);
    let asked: Value = serde_json::from_str(&asked?)?;
    let of = asked["for"].as_str().context("`apps` says who it lists for")?.to_owned();
    let apps = asked["apps"].as_array().cloned().unwrap_or_default();
    built(client, &of, &name, &folder).await?;
    let Some((commit, dirty, source)) = looked?? else { return crate::command(client, &args).await };

    let live = apps.iter().find(|app| app["name"] == name.as_str() && app["for"] == of.as_str());
    let live = live.and_then(|app| app["from"]["commit"].as_str());
    if let Some(live) = live.filter(|live| !force && *live != commit) {
        if !contains(&folder, &commit, live)? {
            bail!(
                "{name} wasn't deployed: its live version is from commit {}, which yours ({}) doesn't contain, so \
                 deploying it would drop changes that are live. Pull or rebase onto it and deploy again, or pass \
                 --force to replace it anyway.",
                &live[..12],
                &commit[..12]
            );
        }
    }

    let mut from = json!({"source": source, "commit": commit});
    if dirty {
        from["dirty"] = json!(true);
    }
    args.extend(["--from".to_owned(), from.to_string()]);
    let deployed = crate::command(client, &args).await?;
    if dirty && deployed == ExitCode::SUCCESS {
        println!(
            "{} has changes not committed, so this version is marked dirty, from {}-dirty.",
            folder.display(),
            &commit[..12]
        );
    }
    Ok(deployed)
}
