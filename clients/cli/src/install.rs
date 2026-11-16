//! Installing plugins from git repos, as `npx skills` installs agent skills (skills.sh): a plugin is a folder of a
//! repo's `plugins/`, with a `SKILL.md`, and there's nothing to publish. `one install <owner>/<repo>[@<ref>] --plugin
//! <name>` fetches the repo with git, deploys that folder as a copy of the installer's own, with where it came from
//! (`deploy`'s `from`), which the network keeps with the plugin and `plugins` gives back. In a network's repo (a
//! folder with `plugins/`, or a `plugins.lock`), `plugins.lock` records each too, so `one install` there installs
//! everything it names at the commits it names: that's how a network is made again from its repo. `one plugins
//! check` says which have moved on upstream or were changed since, and `one plugins update` installs the newer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;
use std::{env, fs};

use anyhow::{Context, Result, bail};
use client::Client;
use serde_json::{Value, json};

/// Where an installer says it installed a plugin, for the directory of plugins (`www/plugin-directory/README.md`).
const DIRECTORY: &str = "https://one.drevan.me/plugins/api/installs";
const LOCK: &str = "plugins.lock";

/// Where a plugin comes from: a repo as it's written in a lock file (`owner/repo` on GitHub, a git URL, or a
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
        let spec = if Path::new(repo).exists() || repo.starts_with(['.', '/', '~']) {
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
        self.spec.starts_with('/')
    }

    /// `owner/repo`, for a repo on GitHub.
    fn github(&self) -> Option<String> {
        let path = self.spec.strip_prefix("https://github.com/").or(self.spec.strip_prefix("git@github.com:"));
        let path = path.unwrap_or(&self.spec).trim_end_matches('/').trim_end_matches(".git");
        (!self.local() && !path.contains(':') && path.split('/').count() == 2).then(|| path.to_owned())
    }

    /// Where this device keeps its copy of the repo, which an agent reads a plugin's code in before installing it.
    fn folder(&self) -> Result<PathBuf> {
        let named = self.spec.split_once("://").map_or(self.spec.as_str(), |(_, rest)| rest);
        let named = named.replace(':', "/");
        let mut folder = client::home()?.join("git");
        if self.local() {
            folder.push("local");
        } else if !self.spec.contains("://") && !self.spec.starts_with("git@") {
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

    /// The commit a plugin's folder would be installed at now, if its files differ from those at `commit`.
    fn newer(&self, name: &str, commit: &str) -> Result<Option<String>> {
        let folder = self.folder()?;
        let (_, latest) = self.fetch(None)?;
        let tree = |at: &str| git(&folder, &["rev-parse", "--verify", "--quiet", &format!("{at}:plugins/{name}")]).ok();
        Ok((tree(&latest) != tree(commit)).then_some(latest))
    }
}

fn git(folder: &Path, args: &[&str]) -> Result<String> {
    let ran = Command::new("git").arg("-C").arg(folder).args(args).env("GIT_TERMINAL_PROMPT", "0").output()?;
    if !ran.status.success() {
        bail!("git {} said: {}", args.join(" "), String::from_utf8_lossy(&ran.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&ran.stdout).trim().to_owned())
}

/// A plugin as `plugins.lock` records it, by its name there.
#[derive(Clone)]
struct Locked {
    source: String,
    reference: Option<String>,
    commit: String,
    hash: String,
    of: String,
}

impl Locked {
    fn json(&self) -> Value {
        json!({"source": self.source, "ref": self.reference, "commit": self.commit, "hash": self.hash, "for": self.of})
    }

    fn read(locked: &Value) -> Option<Self> {
        Some(Self {
            source: locked["source"].as_str()?.to_owned(),
            reference: locked["ref"].as_str().map(str::to_owned),
            commit: locked["commit"].as_str()?.to_owned(),
            hash: locked["hash"].as_str()?.to_owned(),
            of: locked["for"].as_str()?.to_owned(),
        })
    }
}

/// The lock file, where this is run in a network's repo: a folder with `plugins/` or a `plugins.lock`.
fn lock_file() -> Option<PathBuf> {
    (Path::new(LOCK).exists() || Path::new("plugins").is_dir()).then(|| PathBuf::from(LOCK))
}

fn read_lock(file: &Path) -> Result<BTreeMap<String, Locked>> {
    let Ok(text) = fs::read_to_string(file) else { return Ok(BTreeMap::new()) };
    let lock: Value = serde_json::from_str(&text).with_context(|| format!("{} isn't JSON", file.display()))?;
    let plugins = lock["plugins"].as_object().into_iter().flatten();
    plugins
        .map(|(name, lock)| {
            Locked::read(lock).map(|locked| (name.clone(), locked)).with_context(|| format!("{LOCK} says {name} wrong"))
        })
        .collect()
}

fn write_lock(file: &Path, locked: &BTreeMap<String, Locked>) -> Result<()> {
    let plugins: serde_json::Map<String, Value> = locked.iter().map(|(name, it)| (name.clone(), it.json())).collect();
    let text = serde_json::to_string_pretty(&json!({"version": 1, "plugins": plugins}))?;
    Ok(fs::write(file, text + "\n")?)
}

/// Calls a tool for `of` (a user, a group or the network), as `ONE_SUBJECT` would have it, and gives its text.
async fn call(client: &Client, of: Option<&str>, tool: &str, arguments: Value) -> Result<String> {
    let mut params = json!({"name": tool, "arguments": arguments});
    if let Some(of) = of {
        params["_meta"]["subject"] = json!(of);
    }
    let result = client.request("tools/call", params).await?;
    let text: Vec<&str> =
        result["content"].as_array().into_iter().flatten().filter_map(|part| part["text"].as_str()).collect();
    if result["isError"] == true {
        bail!("{}", text.join("\n"));
    }
    Ok(text.join("\n"))
}

/// What's installed for `of` and the network, as the tool `plugins` lists it, and who `of` is.
async fn installed(client: &Client, of: Option<&str>) -> Result<(String, Vec<Value>)> {
    let response: Value = serde_json::from_str(&call(client, of, "plugins", json!({})).await?)?;
    let of = response["for"].as_str().context("`plugins` says who it lists for")?.to_owned();
    Ok((of, response["plugins"].as_array().cloned().unwrap_or_default()))
}

/// A repo's plugins: each folder of its `plugins/` with a `SKILL.md`, with what that says it's called and is for.
fn offered(folder: &Path) -> Vec<(String, String)> {
    let mut offered: Vec<(String, String)> = fs::read_dir(folder.join("plugins"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|plugin| {
            let skill = fs::read_to_string(plugin.path().join("SKILL.md")).ok()?;
            let front = skill.strip_prefix("---")?.split("\n---").next()?.to_owned();
            let field = |name: &str| {
                front
                    .lines()
                    .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                    .map(|value| value.trim().trim_matches(|quote| quote == '"' || quote == '\'').to_owned())
            };
            Some((
                field("name").unwrap_or_else(|| plugin.file_name().to_string_lossy().into_owned()),
                field("description")?,
            ))
        })
        .collect();
    offered.sort();
    offered
}

/// The plugins a plugin uses, as its `uses/0` names them (`OneCore.Plugin`).
fn uses(plugin: &Path) -> Vec<String> {
    let mut used = Vec::new();
    for file in fs::read_dir(plugin.join("server")).into_iter().flatten().flatten() {
        let code = fs::read_to_string(file.path()).unwrap_or_default();
        let Some((_, after)) = code.split_once("def uses") else { continue };
        let Some(list) = after.split_once('[').and_then(|(_, list)| list.split_once(']')).map(|(list, _)| list) else {
            continue;
        };
        used.extend(
            list.split(',').map(|name| name.trim().trim_matches('"').to_owned()).filter(|name| !name.is_empty()),
        );
    }
    used
}

/// Stops with what to build, where a plugin brings a NIF that isn't built for every kind of machine the network's
/// servers run on: its source is never deployed, only the library built from it (`OneCore.Plugin`), so the agent
/// installing it builds it, as `tooling/nifs` does for the plugins The One App ships with.
async fn built(client: &Client, of: &str, name: &str, plugin: &Path) -> Result<()> {
    let builds: Value = serde_json::from_str(&call(client, Some(of), "builds", json!({})).await?)?;
    let mut kinds: Vec<String> = builds["servers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|server| Some(server["target"].as_str()?.to_owned()))
        .collect();
    kinds.sort();
    kinds.dedup();
    for crate_folder in fs::read_dir(plugin.join("server/nif")).into_iter().flatten().flatten() {
        let crate_folder = crate_folder.path();
        if !crate_folder.join("Cargo.toml").exists() {
            continue;
        }
        let missing: Vec<&String> =
            kinds.iter().filter(|kind| !crate_folder.join(format!("{kind}.so")).exists()).collect();
        if missing.is_empty() {
            continue;
        }
        let library = crate_folder.file_name().unwrap_or_default().to_string_lossy().replace('-', "_");
        let commands: Vec<String> = missing
            .iter()
            .map(|kind| {
                let (os, arch) = kind.split_once('-').unwrap_or((kind.as_str(), ""));
                let (triple, build, glibc, file) = match os {
                    "macos" => (format!("{arch}-apple-darwin"), "build", "", "dylib"),
                    _ => (format!("{arch}-unknown-linux-gnu"), "zigbuild", ".2.35", "so"),
                };
                format!(
                    "    rustup target add {triple} && CARGO_TARGET_DIR=target cargo {build} --release --target {triple}{glibc} \
                     && cp target/{triple}/release/lib{library}.{file} {kind}.so"
                )
            })
            .collect();
        bail!(
            "{name} has native code, the NIF in {}, which isn't built for {}, where the network's servers run. Build it \
             as tooling/nifs does, Linux's with zig (cargo-zigbuild) for a server's glibc and a Mac's on a Mac, then \
             install it again:\n\n    cd {}\n{}",
            crate_folder.display(),
            missing.iter().map(|kind| kind.as_str()).collect::<Vec<_>>().join(" and "),
            crate_folder.display(),
            commands.join("\n")
        );
    }
    Ok(())
}

/// Deploys the plugin `name` from a repo's copy, checked out at `commit`, as `of`'s own, and gives it as locked. With
/// `sandbox` it runs in sandbox mode; left out, as the version before did. With `new` it's made where one by that
/// name is installed for someone else `of` is in, as `deploy`'s `new` says.
async fn deploy(
    client: &Client,
    of: &str,
    name: &str,
    folder: &Path,
    from: &Source,
    commit: &str,
    sandbox: bool,
    new: bool,
) -> Result<Locked> {
    let plugin = folder.join("plugins").join(name);
    // Deployed over someone else's plugin, it would be changed for everyone who has it, not copied.
    let (_, plugins) = installed(client, Some(of)).await?;
    let own = format!("{of}/plugins/{name}");
    if let Some(other) = plugins.iter().find(|it| it["name"] == name && it["for"] == of && it["plugin"] != own.as_str())
    {
        bail!(
            "{name} is installed for {of} already, as {}. `one uninstall --name {name}` it first, to install a copy of \
             your own in its place.",
            other["plugin"].as_str().unwrap_or_default()
        );
    }
    built(client, of, name, &plugin).await?;
    println!(
        "{name}'s code, from {} at {}, is in {}. Read it before installing (the skill installing-plugins): its \
         server code runs with full access on {of}'s servers, or in sandbox mode with only what it's given.",
        from.spec,
        &commit[..12],
        plugin.display()
    );
    let files = crate::read(&plugin, false)?;
    let mut deployed =
        json!({"name": name, "files": files, "from": {"source": from.spec, "ref": from.reference, "commit": commit}});
    if sandbox {
        deployed["sandbox"] = json!(true);
    }
    if new {
        deployed["new"] = json!(true);
    }
    println!("{}", call(client, Some(of), "deploy", deployed).await?);
    let (_, plugins) = installed(client, Some(of)).await?;
    let hash = plugins
        .iter()
        .find(|plugin| plugin["name"] == name && plugin["for"] == of)
        .and_then(|plugin| plugin["from"]["hash"].as_str())
        .with_context(|| format!("{name} was deployed, but `plugins` doesn't list it for {of}"))?;
    report(from, name, commit).await;
    Ok(Locked {
        source: from.spec.clone(),
        reference: from.reference.clone(),
        commit: commit.to_owned(),
        hash: hash.to_owned(),
        of: of.to_owned(),
    })
}

/// Tells the directory of plugins that a plugin was installed, with nothing about who installed it. Not for a
/// folder, nor a repo GitHub doesn't say is public, nor where `DO_NOT_TRACK` or `DISABLE_TELEMETRY` (skills.sh's own)
/// is set. It's never waited on long, and its failing fails nothing.
async fn report(from: &Source, name: &str, commit: &str) {
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
            let body = json!({"source": repo, "plugin": name, "commit": commit}).to_string();
            let _ = web.post(DIRECTORY).header("content-type", "application/json").body(body).send().await;
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(3), post).await;
}

/// `one install <repo>[@<ref>] --plugin <name> ... [--list] [--sandbox] [--new]`, for whoever `ONE_SUBJECT` says, or
/// this device's user. `--sandbox` runs them in sandbox mode, which needs no server of the user's; `--new` makes a
/// second copy where one by that name is installed for the network or a group they're in, which is refused without.
pub async fn install(client: &Client, args: &[String]) -> Result<ExitCode> {
    let source = Source::parse(&args[0])?;
    let (mut names, mut list, mut sandbox, mut new) = (Vec::new(), false, false, false);
    let mut flags = args[1..].iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--list" => list = true,
            "--sandbox" => sandbox = true,
            "--new" => new = true,
            "--plugin" => names.push(flags.next().context("--plugin needs a plugin's name")?.clone()),
            other => bail!("`{other}` isn't something `one install` takes: --plugin <name>, --sandbox, --new or --list"),
        }
    }
    let (folder, commit) = source.fetch(None)?;
    let offered = offered(&folder);
    if list || names.is_empty() {
        let width = offered.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
        for (name, about) in &offered {
            println!("{name:width$}  {about}");
        }
        println!("\n{} at {}, in {}/plugins.", source.spec, &commit[..12], folder.display());
        if !list {
            println!("`one install {} --plugin <name>` installs one.", args[0]);
        }
        return Ok(ExitCode::SUCCESS);
    }
    let (of, plugins) = installed(client, None).await?;
    let is = |name: &str, at: &str| plugins.iter().find(|plugin| plugin["name"] == name && plugin["for"] == at);
    // Each plugin after the ones it uses that aren't installed for them or the network yet and the repo has.
    let mut order: Vec<String> = Vec::new();
    let mut next: Vec<(String, bool)> = names.iter().rev().map(|name| (name.clone(), false)).collect();
    while let Some((name, placed)) = next.pop() {
        if order.contains(&name) {
            continue;
        }
        let plugin = folder.join("plugins").join(&name);
        if !plugin.join("SKILL.md").exists() {
            bail!("{} has no plugin {name}: `one install {} --list` lists the ones it has", source.spec, args[0]);
        }
        if placed {
            order.push(name);
            continue;
        }
        next.push((name.clone(), true));
        for used in uses(&plugin) {
            let there = is(&used, &of).or(is(&used, "network")).is_some();
            if !there && folder.join("plugins").join(&used).join("SKILL.md").exists() && !order.contains(&used) {
                println!(
                    "{name} uses {used}, which isn't installed for {of} or the network, so it's installed from {} too.",
                    source.spec
                );
                next.push((used, false));
            }
        }
    }
    let lock = lock_file();
    let mut locked = match &lock {
        Some(file) => read_lock(file)?,
        None => BTreeMap::new(),
    };
    for name in &order {
        let it = deploy(client, &of, name, &folder, &source, &commit, sandbox, new).await?;
        if let Some(file) = &lock {
            locked.insert(name.clone(), it);
            write_lock(file, &locked)?;
        }
    }
    match lock {
        Some(file) => println!("Recorded in {}.", file.display()),
        None => println!(
            "Not in a network's repo, so no {LOCK} is written: the network keeps where it came from (`one plugins`)."
        ),
    }
    Ok(ExitCode::SUCCESS)
}

/// `one install` in a folder with a `plugins.lock`: installs everything it records, at the commits it records, for
/// whoever it records, as it was, each after the ones it records that it uses. A plugin whose files hash differently
/// now (a NIF built again) is said so.
pub async fn locked(client: &Client) -> Result<ExitCode> {
    let file = PathBuf::from(LOCK);
    let mut waiting = read_lock(&file)?;
    let count = waiting.len();
    while !waiting.is_empty() {
        let mut deployed = Vec::new();
        for (name, it) in &waiting {
            let source = Source { spec: it.source.clone(), reference: it.reference.clone() };
            let (folder, commit) = source.fetch(Some(&it.commit))?;
            if uses(&folder.join("plugins").join(name)).iter().any(|used| used != name && waiting.contains_key(used)) {
                continue;
            }
            let now = deploy(client, &it.of, name, &folder, &source, &commit, false, false).await?;
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
    println!("Installed the {count} plugins {LOCK} records.");
    Ok(ExitCode::SUCCESS)
}

/// Each plugin installed from a git repo, by its name: the lock file's, in a network's repo, or else what the
/// network keeps of what's installed for whoever this runs for and the network. With `names`, only those.
async fn recorded(client: &Client, names: &[String]) -> Result<BTreeMap<String, Locked>> {
    let all = match lock_file() {
        Some(file) => read_lock(&file)?,
        None => {
            let (_, plugins) = installed(client, None).await?;
            let mut by_name = BTreeMap::new();
            for plugin in plugins.iter().rev() {
                let from = &plugin["from"];
                let mut locked = from.clone();
                locked["for"] = plugin["for"].clone();
                if let (Some(name), Some(locked)) = (plugin["name"].as_str(), Locked::read(&locked)) {
                    by_name.insert(name.to_owned(), locked);
                }
            }
            by_name
        }
    };
    for name in names {
        if !all.contains_key(name) {
            bail!("{name} wasn't installed from a git repo here");
        }
    }
    Ok(all.into_iter().filter(|(name, _)| names.is_empty() || names.contains(name)).collect())
}

/// What's become of each: `(newer commit upstream, changed since it was installed)`.
async fn state(client: &Client, name: &str, it: &Locked) -> Result<(Option<String>, bool)> {
    let source = Source { spec: it.source.clone(), reference: it.reference.clone() };
    let newer = source.newer(name, &it.commit)?;
    let (_, plugins) = installed(client, Some(&it.of)).await?;
    let now = plugins.iter().find(|plugin| plugin["name"] == name && plugin["for"] == it.of.as_str());
    let changed = now.is_none_or(|plugin| plugin["source"].as_str() != Some(&it.hash));
    Ok((newer, changed))
}

/// `one plugins check [names]`: which have a newer version upstream, and which were changed in the network since
/// they were installed. Fails if any has either, as `npx skills check` does.
pub async fn check(client: &Client, names: &[String]) -> Result<ExitCode> {
    let recorded = recorded(client, names).await?;
    let mut behind = false;
    for (name, it) in &recorded {
        let (newer, changed) = state(client, name, it).await?;
        let at = format!("{}@{}", it.source, &it.commit[..12]);
        let status = match (&newer, changed) {
            (None, false) => "up to date".to_owned(),
            (Some(newer), false) => format!("update available: {}", &newer[..12]),
            (None, true) => "changed locally since it was installed".to_owned(),
            (Some(newer), true) => {
                format!("update available: {}, and changed locally since it was installed", &newer[..12])
            }
        };
        behind |= newer.is_some() || changed;
        println!("{name} ({}, {at}): {status}", it.of);
    }
    if recorded.is_empty() {
        println!("Nothing here was installed from a git repo.");
    }
    Ok(if behind { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

/// `one plugins update [names] [--force]`: installs the newer version of each that has one. One changed in the
/// network since it was installed is left as it is, unless `--force` says to overwrite it: nothing is merged.
pub async fn update(client: &Client, args: &[String]) -> Result<ExitCode> {
    let force = args.iter().any(|arg| arg == "--force");
    let names: Vec<String> = args.iter().filter(|arg| *arg != "--force").cloned().collect();
    let lock = lock_file();
    let mut all = match &lock {
        Some(file) => read_lock(file)?,
        None => BTreeMap::new(),
    };
    let mut skipped = false;
    for (name, it) in recorded(client, &names).await? {
        let (newer, changed) = state(client, &name, &it).await?;
        let Some(newer) = newer else {
            println!("{name} is up to date.");
            continue;
        };
        if changed && !force {
            println!(
                "{name} was changed in the network since it was installed, so it's left as it is. `one code --name {name}` \
                 gives it as it is now; `one plugins update {name} --force` overwrites it with {}.",
                &newer[..12]
            );
            skipped = true;
            continue;
        }
        let source = Source { spec: it.source.clone(), reference: it.reference.clone() };
        let (folder, commit) = source.fetch(Some(&newer))?;
        let updated = deploy(client, &it.of, &name, &folder, &source, &commit, false, false).await?;
        if let Some(file) = &lock {
            all.insert(name, updated);
            write_lock(file, &all)?;
        }
    }
    Ok(if skipped { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

/// `one uninstall --name <name>` in a network's repo: uninstalls it, and takes it out of `plugins.lock` if that
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

/// `one deploy ... --files-from <folder> [--force]`. From a folder in a git repo, the deploy says which commit its
/// files came from (`from`), and `dirty` where the folder has changes not committed, as a build does. As `git push`
/// refuses a non-fast-forward, it's refused where the live version came from a commit that one doesn't contain, so
/// it would drop changes that are live, unless `--force`. That's decided here, since only the repo knows which
/// commits contain which; the network only keeps the commit. From a folder in no git repo, it's deployed as it is.
pub async fn deploy_folder(client: &Client, args: &[String]) -> Result<ExitCode> {
    let force = args.iter().any(|arg| arg == "--force");
    let mut args: Vec<String> = args.iter().filter(|arg| *arg != "--force").cloned().collect();
    let after = |flag: &str| args.iter().position(|arg| arg == flag).and_then(|at| args.get(at + 1)).cloned();
    let (Some(folder), Some(name)) = (after("--files-from"), after("--name")) else {
        return crate::command(client, &args).await;
    };
    let folder = PathBuf::from(folder);
    let Ok(commit) = git(&folder, &["rev-parse", "HEAD"]) else { return crate::command(client, &args).await };
    let dirty = !git(&folder, &["status", "--porcelain", "--", "."])?.is_empty();
    let source =
        git(&folder, &["remote", "get-url", "origin"]).or_else(|_| git(&folder, &["rev-parse", "--show-toplevel"]))?;

    let (of, plugins) = installed(client, None).await?;
    let live = plugins.iter().find(|plugin| plugin["name"] == name.as_str() && plugin["for"] == of.as_str());
    let live = live.and_then(|plugin| plugin["from"]["commit"].as_str());
    if let Some(live) = live.filter(|live| !force && *live != commit) {
        // A commit someone else pushed is fetched before it's decided that this one doesn't contain it.
        if git(&folder, &["cat-file", "-e", &format!("{live}^{{commit}}")]).is_err() {
            let _ = git(&folder, &["fetch", "--quiet"]);
        }
        if git(&folder, &["merge-base", "--is-ancestor", live, &commit]).is_err() {
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
