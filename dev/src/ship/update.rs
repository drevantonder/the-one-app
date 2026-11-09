//! `update`: each part of a build sent out on its own, by what changed in it since it was last sent, which each part
//! keeps for itself in its own marker. A plain `update` runs them all, and each does nothing if nothing of it changed.

use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Part {
    Apps,
    Mac,
    Services,
    Node,
    Workers,
    Web,
    Dev,
}

/// What a plain `update` runs, in this order: apps first, as a build without a renamed one is about to go; the node before
/// the Workers and web client, whose deploy a change to Rust needs it for; this program last, what the rest was sent with.
pub(super) const ORDER: [Part; 7] = [Part::Apps, Part::Mac, Part::Services, Part::Node, Part::Workers, Part::Web, Part::Dev];

impl Part {
    pub(super) fn name(self) -> &'static str {
        match self {
            Part::Apps => "apps",
            Part::Mac => "mac",
            Part::Services => "services",
            Part::Node => "node",
            Part::Workers => "workers",
            Part::Web => "web",
            Part::Dev => "dev",
        }
    }

    fn named(word: &str) -> Result<Part> {
        ORDER.into_iter().find(|p| p.name() == word).ok_or_else(|| one::cli::Failed::Typed(format!("{word} isn't a part to update: {}", ORDER.map(Part::name).join(", "))).into())
    }

    fn marker(self) -> PathBuf {
        cache().join("sent").join(self.name())
    }

    pub(super) fn last_sent(self) -> Option<String> {
        std::fs::read_to_string(self.marker()).ok()
    }
}

/// Main's tip, or a commit, as this device's clean checkout has it, with the config it's built with: what one `update`
/// sends and marks sent, whatever lands on main meanwhile.
pub(super) struct At {
    pub(super) src: PathBuf,
    pub(super) commit: String,
    pub(super) config: Option<String>,
    pub(super) config_file: PathBuf,
    /// As a build says it (build.rs): the commit's time, or its config's, if that's later.
    pub(super) committed: u64,
}

impl At {
    pub(super) fn at(commit: Option<&str>) -> Result<At> {
        let src = checkout(commit)?;
        let (config_file, config_commit) = config(&src)?;
        let commit = git(&src, &["rev-parse", "HEAD"])?;
        let seconds = |dir: &Path| -> Result<u64> { git(dir, &["log", "-1", "--format=%ct"])?.parse().context("a commit's date") };
        let code = seconds(&src)?;
        let committed = match &config_commit {
            Some(_) => code.max(seconds(config_file.parent().context("the config's folder")?)?),
            None => code,
        };
        Ok(At { src, commit, config: config_commit, config_file, committed })
    }

    /// What a marker holds: the commit and its config's.
    pub(super) fn marked(&self) -> String {
        format!("{} {}", self.commit, self.config.as_deref().unwrap_or(""))
    }

    /// As builds are named: what a device says it runs, and its folder is called.
    pub(super) fn build_name(&self) -> String {
        build_name_of(&self.marked())
    }

    pub(super) fn config_commit(&self) -> &str {
        self.config.as_deref().unwrap_or("-")
    }

    pub(super) fn version(&self) -> Version {
        Version { commit: self.build_name(), target: "-".into(), committed: self.committed }
    }

    /// The folder of this commit's build, if `update node` or `update dev` has made it here.
    pub(super) fn build(&self) -> Option<PathBuf> {
        Some(cache().join("builds").join(self.build_name())).filter(|dir| dir.join("config").is_file())
    }
}

/// The build a marker holds (`At::marked`), as builds are named.
fn build_name_of(marked: &str) -> String {
    let (code, config) = marked.split_once(' ').unwrap_or((marked, ""));
    let code = &code[..12.min(code.len())];
    if config.is_empty() { code.to_string() } else { format!("{code}-{}", &config[..8.min(config.len())]) }
}

/// What `part` has to send since it was last sent, as of `at`.
pub(super) fn due(part: Part, at: &At) -> Parts {
    part.last_sent().map_or_else(Parts::everything, |last| Parts::since(&at.src, &last, &at.marked()))
}

fn sent(part: Part, at: &At) -> Result<()> {
    std::fs::create_dir_all(part.marker().parent().context("the markers' folder")?)?;
    std::fs::write(part.marker(), at.marked())?;
    Ok(())
}

/// The services due: those a change touched, those on what every service in Rust is built on, and any not yet published
/// from here.
pub(super) fn services_due(at: &At, parts: &Parts) -> Vec<String> {
    let released = services_released();
    let here = services(&at.src);
    let mut names: Vec<String> = parts.services.iter().filter(|n| here.contains(n)).cloned().collect();
    for name in &here {
        let rust = service_dir(&at.src, name).join("Cargo.toml").is_file();
        if (parts.all || !released.contains_key(name) || (rust && parts.service)) && !names.contains(name) {
            names.push(name.clone());
        }
    }
    names
}

/// `update [<part>] [--release]`.
pub(super) fn update(part: Option<&str>, release: bool) -> Result<()> {
    let part = part.map(Part::named).transpose()?;
    let _turn = turn()?;
    let at = At::at(None)?;
    let started = Instant::now();
    println!("Sending out {}.", at.build_name());
    let mut failed = Vec::new();
    let mut attempt = |what: &str, result: Result<()>| {
        if let Err(err) = result {
            println!("{what}: {err:#}");
            failed.push(what.to_string());
        }
    };
    match part {
        Some(Part::Workers) => attempt("workers", front(&at, true, false)),
        Some(Part::Web) => attempt("web", front(&at, false, true)),
        Some(Part::Node) => {
            attempt("node", send(Part::Node, &at, release));
            // What went out with the node is only the node; its Workers and web client follow it, as a plain update has them.
            attempt("workers and web", front(&at, true, true));
        }
        Some(part) => attempt(part.name(), send(part, &at, release)),
        None => {
            for part in ORDER.into_iter().filter(|p| !matches!(p, Part::Workers | Part::Web | Part::Dev)) {
                attempt(part.name(), send(part, &at, release));
            }
            // Together: one deploy, not one for each.
            attempt("workers and web", front(&at, true, true));
            attempt("dev", send(Part::Dev, &at, release));
        }
    }
    ensure!(failed.is_empty(), "{} didn't go out: {CLI}-dev update again sends what's still due", failed.join(", "));
    println!("Done in {}s.", started.elapsed().as_secs());
    Ok(())
}

/// Sends `part`, if anything of it changed.
fn send(part: Part, at: &At, release: bool) -> Result<()> {
    let parts = due(part, at);
    let started = Instant::now();
    let services = services_due(at, &parts);
    let wanted = match part {
        Part::Apps => parts.all || !parts.apps.is_empty() || !parts.renamed.is_empty(),
        Part::Mac => parts.mac,
        Part::Services => !services.is_empty(),
        Part::Node => parts.node,
        Part::Dev => parts.dev,
        Part::Workers | Part::Web => unreachable!("sent by `front`"),
    };
    if !wanted {
        println!("{}: nothing changed.", part.name());
        return Ok(());
    }
    match part {
        Part::Apps => {
            retire_gone(&at.src, &parts.renamed)?;
            deploy_apps(&at.src)?;
        }
        Part::Mac => publish_source(at, release)?,
        Part::Services => ensure!(publish_services(&at.src, &services, &release_key()?, &at.commit), "not every service was published"),
        Part::Node => send_node(at)?,
        Part::Dev => send_dev(at)?,
        Part::Workers | Part::Web => unreachable!("sent by `front`"),
    }
    sent(part, at)?;
    println!("{}: sent {} in {}s.", part.name(), at.build_name(), started.elapsed().as_secs());
    Ok(())
}

/// Builds the node and everything in it, publishes it and waits for this device to run it; the built-in apps are then
/// deployed from its build. It is sent once all that is done, so a retry does what didn't happen.
fn send_node(at: &At) -> Result<()> {
    let key = release_key()?;
    let folder = match at.build() {
        Some(folder) => folder,
        None => build_release(at)?,
    };
    let binary = folder.join("one");
    let version = Version::of(&binary).context("ask the new build what it is")?;
    println!("{}", build::published_said(&build::publish(&crate::node(), &binary, &key, &version, None)?, None));
    let here = devices().into_iter().find(|d| d["here"] == true).and_then(|d| d["name"].as_str().map(str::to_string)).context("this device isn't listed")?;
    wait_until(&version.commit, &[here], Duration::from_secs(600))?;
    deploy_built_in()?;
    Ok(())
}

/// Sends the Workers and the web client, each if asked and anything of it changed, with one deploy from their home device.
/// If the node went out at this very commit, they go with its build, which holds both as they are. Otherwise this program
/// puts them together from the checkout, with no build, and the home device deploys them from the record; Rust in the change
/// is then in the node last sent, which that device has to run first. Rust the node hasn't gone out with waits for it.
fn front(at: &At, workers: bool, web: bool) -> Result<()> {
    let started = Instant::now();
    let (w, b) = (workers && due(Part::Workers, at).workers, web && due(Part::Web, at).web);
    if !w && !b {
        println!("{}: nothing changed.", if workers && web { "workers and web" } else if workers { "workers" } else { "web" });
        return Ok(());
    }
    let rust = (w && due(Part::Workers, at).node) || (b && due(Part::Web, at).node);
    if rust {
        ensure!(!due(Part::Node, at).node, "there's Rust in this change, and the node hasn't gone out with it: {CLI}-dev update node sends this with it");
    }
    let key = release_key()?;
    let node = Part::Node.last_sent();
    match at.build().filter(|_| node.as_deref() == Some(at.marked().as_str())) {
        Some(folder) => {
            let version = Version::of(&folder.join("one")).context("ask the build what it is")?;
            deploy_cloudflare(Some(&version.commit))?;
            if b {
                publish_web(&folder, &key)?;
            }
        }
        None => {
            same_rust(at)?;
            one::ui_folder::read_from(&at.src);
            if w {
                publish_workers(at, &key)?;
            }
            if b {
                publish_assembled_web(at, &key)?;
            }
            deploy_cloudflare(rust.then(|| build_name_of(node.as_deref().unwrap_or(""))).as_deref())?;
        }
    }
    for (part, wanted) in [(Part::Workers, w), (Part::Web, b)] {
        if wanted {
            sent(part, at)?;
            println!("{}: sent {} in {}s.", part.name(), at.build_name(), started.elapsed().as_secs());
        }
    }
    Ok(())
}

/// What's compiled into this program (what the Workers and the web client are put together with: their generated files,
/// the client's WebAssembly, its logo and name) is what `at` says, so nothing it adds to them is out of date.
fn same_rust(at: &At) -> Result<()> {
    let (code, built_config) = one::config::BUILD.split('+').next().unwrap_or("").split_once('-').map_or((one::config::BUILD.split('+').next().unwrap_or(""), None), |(c, d)| (c, Some(d)));
    let stale = || format!("{CLI}-dev is from {}, and what's built into it has changed since: {CLI}-dev update dev", one::config::BUILD);
    let files = git(&at.src, &["diff", "--name-only", code, &at.commit]).with_context(stale)?;
    // What is built in is also what `update dev` sends: so whatever makes this program stale, it has something to send. A
    // change to dev/ alone doesn't, or any push to it would hold back the Workers and the web client until `update dev`.
    ensure!(!Parts::of(files.lines()).is_built_in() &&built_config.is_none_or(|c| at.config.as_deref().is_some_and(|d| d.starts_with(c))), "{}", stale());
    Ok(())
}

/// Installs this commit's `{CLI}-dev` over this program, and on the user's other Linux devices, for their throwaways.
fn send_dev(at: &At) -> Result<()> {
    let folder = match at.build() {
        Some(folder) => folder,
        None => build_release(at)?,
    };
    let built = folder.join("one-dev");
    let me = one::this_program()?;
    let next = me.with_extension("new");
    std::fs::copy(&built, &next)?;
    std::fs::rename(&next, &me).with_context(|| format!("install it at {}", me.display()))?;
    let name = format!("{CLI}-dev");
    for device in devices().iter().filter(|d| d["online"] == true && d["here"] != true && d["target"] == TARGET) {
        let host = device["name"].as_str().unwrap_or("");
        // Over ssh's own channel: not every device's ssh server takes scp's.
        let copied = std::fs::File::open(&built).is_ok_and(|program| {
            Command::new("ssh").args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, &format!("cat > .local/bin/{name}.new && chmod 755 .local/bin/{name}.new && mv .local/bin/{name}.new .local/bin/{name}")]).stdin(program).status().is_ok_and(|s| s.success())
        });
        println!("  {host}: {}", if copied { "installed" } else { "couldn't copy it there" });
    }
    Ok(())
}

/// Packs the Workers' sources from the checkout and names them in the record, for their home device to deploy.
fn publish_workers(at: &At, key: &ssh_key::PrivateKey) -> Result<()> {
    let packed = one::web::bundle::Bundle { commit: at.build_name(), committed: at.committed, shell: one::web::SHELL, files: one::ui_folder::cloudflare() }.packed()?;
    let file = cache().join("workers").join(format!("{}.bundle", at.build_name()));
    std::fs::create_dir_all(file.parent().context("its folder")?)?;
    std::fs::write(&file, packed)?;
    build::publish_latest(&crate::node(), key, &[(Thing::WORKERS.into(), Some(file), at.version(), at.config_commit().to_string())])?;
    println!("Published the Workers of {}.", at.build_name());
    Ok(())
}

/// Puts the web client together from the checkout and publishes it, unless its files are what was last published.
fn publish_assembled_web(at: &At, key: &ssh_key::PrivateKey) -> Result<()> {
    let bundle = one::web::bundle::Bundle { commit: at.build_name(), committed: at.committed, shell: one::web::SHELL, files: one::web::with_client() };
    publish_web_bundle(&bundle, &at.version(), at.config_commit(), key)
}
