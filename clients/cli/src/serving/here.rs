//! This device's server in the client's home: its record, lock, programs and builds kept.

use super::*;

impl Here {
    pub(super) fn of(home: &Path) -> Self {
        Self { folder: home.join("server") }
    }

    /// The server's key, made the first time: it's put on a list before it's ever started.
    pub(super) fn key(&self) -> Result<String> {
        Ok(client::key(&self.folder.join("home/key"))?.public().to_string())
    }

    /// What this device is as a server: inactive where it has no record.
    pub(super) fn read(&self) -> Result<Record> {
        match fs::read(self.folder.join("state.json")) {
            Ok(record) => Ok(serde_json::from_slice(&record)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Record::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// Changes the record as `change` does, and gives it as changed. It's read and written in one step, under a lock
    /// every change holds while it does, so what another program changed meanwhile, as `one serve off` does while
    /// `one serve run` updates the server, is kept: each changes only what it does of the record, never writes a copy
    /// of it read before. A change where the server's folder has gone, as leaving removes it, fails.
    pub(super) fn change(&self, change: impl FnOnce(&mut Record)) -> Result<Record> {
        let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.folder.join("state.lock"))?;
        lock.lock()?;
        let mut record = self.read()?;
        change(&mut record);
        let writing = self.folder.join("state.json.new");
        fs::write(&writing, serde_json::to_vec_pretty(&record)?)?;
        fs::rename(writing, self.folder.join("state.json"))?;
        Ok(record)
    }

    /// Changes the record, and tells the server, which is running, what it says of it.
    pub(super) async fn save(&self, server: &mut Child, change: impl FnOnce(&mut Record)) -> Result<Record> {
        let record = self.change(change)?;
        tell(server, &record.told(ServerDaemonStatus::Running)).await;
        Ok(record)
    }

    /// Holds the lock `one serve on` and `one serve off` hold while they run, if nothing else holds it.
    pub(super) fn lock(&self) -> Result<Option<fs::File>> {
        let Ok(file) = fs::File::create(self.folder.join("lock")) else { return Ok(None) };
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    /// Puts the build `build` of the program, kept beside it since an update took it or the one before (`take`), in
    /// place of `program`.
    pub(super) fn put_program(&self, build: &str, program: &Path) -> Result<()> {
        let new = program.canonicalize()?.with_extension(format!("{}.new", std::process::id()));
        fs::copy(self.program(build), &new)
            .with_context(|| format!("The build {build} of one, which the server's record pairs with its build, isn't kept here"))?;
        put(&new, program)
    }

    /// The kept `one` of the build `build`, of the kind this one is.
    pub(super) fn program(&self, build: &str) -> PathBuf {
        self.folder.join("programs").join(PROGRAM).join(build)
    }

    /// The folder of the build `build`.
    pub(super) fn build(&self, build: &str) -> PathBuf {
        self.folder.join("builds").join(build)
    }

    /// Where the server is started from: its build's folder, but on Windows `current`, the build's files hard linked
    /// there as it starts (`linked`), in place of the ones before. Windows Firewall keeps what its person allowed by
    /// the path a program was started by, and the VM that holds the server's socket is in its build's own folder, so
    /// each update would ask them again. Not a junction: Windows doesn't follow one a person made for a process that
    /// guards against them (Redirection Guard), as an ssh session's do.
    pub(super) fn started(&self, build: &str) -> Result<PathBuf> {
        #[cfg(windows)]
        {
            let current = self.folder.join("current");
            match fs::remove_dir_all(&current) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error)?,
                _ => {}
            }
            linked(&self.build(build), &current)?;
            Ok(current)
        }
        #[cfg(not(windows))]
        Ok(self.build(build))
    }

    /// Removes the builds and programs the record names as neither current, previous nor next, and the runtimes no
    /// build kept is put together from. Only as the server starts, on current or next.
    pub(super) fn prune(&self, record: &Record) -> Result<()> {
        let pairs = [&record.current, &record.previous, &record.next];
        let named = |name: &str| {
            pairs.iter().filter_map(|pair| pair.as_ref()).any(|pair| pair.build == name || pair.program == name)
        };
        // Programs of the other kind are of a `one` no longer in place.
        for kept in fs::read_dir(self.folder.join("programs")).into_iter().flatten() {
            let kept = kept?;
            if kept.file_name() != PROGRAM {
                let _ = fs::remove_dir_all(kept.path()).or_else(|_| fs::remove_file(kept.path()));
            }
        }
        for folder in ["builds", &format!("programs/{PROGRAM}")] {
            for kept in fs::read_dir(self.folder.join(folder)).into_iter().flatten() {
                let kept = kept?;
                if !named(&kept.file_name().to_string_lossy()) {
                    let _ = fs::remove_dir_all(kept.path()).or_else(|_| fs::remove_file(kept.path()));
                }
            }
        }
        let runtimes: Vec<String> = fs::read_dir(self.folder.join("builds"))
            .into_iter()
            .flatten()
            .filter_map(|build| fs::read_to_string(build.ok()?.path().join("runtime")).ok())
            .map(|runtime| runtime.trim().to_owned())
            .collect();
        for kept in fs::read_dir(self.folder.join("runtimes")).into_iter().flatten() {
            let kept = kept?;
            if !runtimes.contains(&kept.file_name().to_string_lossy().into_owned()) {
                let _ = fs::remove_dir_all(kept.path());
            }
        }
        Ok(())
    }

    /// Takes the latest server build among the network's `builds` (`latest`), put together from its runtime and
    /// its code, and has it say which it is, told what the server is, `told`, which starts its VM and loads its
    /// NIFs: a build that can't do that here goes, as does one older than the build `on`. Gives its name. One that's
    /// here already is taken as it is.
    pub(super) async fn take(&self, builds: &Map<String, Value>, told: &Told, on: Option<&str>) -> Result<String> {
        let name = latest(builds)
            .with_context(|| format!("The network has no server for {} yet: `tooling/release` makes one.", target()))?
            .to_owned();
        let folder = self.build(&name);
        if folder.exists() {
            return Ok(name);
        }
        if let Some(on) = on {
            ensure!(count(&name) >= count(on), "the network's server build, {name}, is older than the one it's on, {on}");
        }
        let unpacking = self.folder.join("builds").join(format!("{name}.unpacking"));
        let _ = fs::remove_dir_all(&unpacking);
        fs::create_dir_all(&unpacking)?;
        let kept = self.runtime(&builds["server-runtime"]).await?;
        linked(&kept, &unpacking).with_context(|| format!("the build {name} couldn't be put together"))?;
        unpack("server-code", &builds["server-code"], &unpacking).await?;
        let mut check = command(release(&unpacking));
        check.args(["eval", "OneCore.Release.check()"]).env("ONE_SERVER_STATUS", told.to_string());
        let output = check.output()?;
        let (out, errors) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        let out = out
            .lines()
            .filter_map(Said::read)
            .find_map(|said| if let Said::Checked { build } = said { Some(build) } else { None })
            .unwrap_or_default();
        if out != name {
            fs::remove_dir_all(&unpacking)?;
            let last: Vec<&str> = errors.lines().rev().take(LAST).collect::<Vec<_>>().into_iter().rev().collect();
            if out.is_empty() {
                bail!("the build {name}'s start-up check failed ({}); its last lines: {}", output.status, last.join(" | "));
            }
            bail!("the build {name} says it's {out}, not what was published");
        }
        fs::rename(&unpacking, &folder)?;
        Ok(name)
    }

    /// The folder of the runtime `runtime`, downloaded and unpacked the first time.
    pub(super) async fn runtime(&self, runtime: &Value) -> Result<PathBuf> {
        let name = runtime["build"].as_str().context("a runtime says which build it is")?;
        let folder = self.folder.join("runtimes").join(name);
        if !folder.exists() {
            let unpacking = folder.with_extension("unpacking");
            let _ = fs::remove_dir_all(&unpacking);
            fs::create_dir_all(&unpacking)?;
            unpack(&format!("server-runtime/{}", target()), runtime, &unpacking).await?;
            fs::rename(&unpacking, &folder)?;
        }
        Ok(folder)
    }
}

/// Downloads the build of `what` that's an archive, `build`, and unpacks it into `folder`.
pub(super) async fn unpack(what: &str, build: &Value, folder: &Path) -> Result<()> {
    let name = build["build"].as_str().context("a build says which it is")?;
    let archive = folder.with_extension("tar.gz");
    fetch(what, build, &archive).await.with_context(|| format!("the build {name} didn't download"))?;
    let unpacked = command("tar").arg("-xzf").arg(&archive).arg("-C").arg(folder).status()?;
    fs::remove_file(&archive)?;
    ensure!(unpacked.success(), "The build {name} didn't unpack.");
    Ok(())
}

/// Puts the files of the folder `from` in the folder `to` as hard links, the same files under another name, as a
/// runtime is in each build put together from it.
pub(super) fn linked(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            linked(&entry.path(), &path)?;
        } else {
            fs::hard_link(entry.path(), &path)?;
        }
    }
    Ok(())
}
