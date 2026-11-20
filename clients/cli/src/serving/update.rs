//! Taking the latest builds: this program's `update`, and a server's pair prepared, checked and gone to (`prepare`, `go`).

use super::*;

/// Takes the latest builds: the network's Update Now, where this device is one of its servers, and otherwise the
/// program's own.
pub async fn update(client: &Client) -> Result<String> {
    let here = Here::of(&client::home()?);
    // A server is taken to the latest build, as every other, by its network's update: this is its Update Now.
    if settled(client, &here, true).await?.server_membership != ServerMembership::Inactive {
        let said = crate::said(crate::tasks::result(client, "update", json!({})).await?)?;
        return Ok(match take_program(client).await? {
            Some(build) => format!("{said}\nThis device's one is the build {build} now, which its server runs from its next start."),
            None => said,
        });
    }
    let builds = download(client).await?;
    let one =
        builds.get(PROGRAM).with_context(|| format!("The network has no build of {PROGRAM} for {} yet.", target()))?;
    if one["build"] == BUILD {
        return Ok(format!("This device is on the latest build already: {BUILD}"));
    }
    let program = program()?;
    put(&fetched_program(one, &program).await?, &program)?;
    Ok(format!("This device is on the build {} now.", one["build"].as_str().unwrap_or_default()))
}

/// Puts the network's latest `one` in place on a device that's also a server, where it runs a server as the `one` the
/// server's record pairs with its build does (its `serving` is that one or older, `OneCore.Builds`): the server goes on
/// as it is and runs it from its next start, and only the app starts again on it, as it hears its program replaced.
/// So the app takes each build of itself at once, while the network's update restarts servers only for a `one` that
/// changed what they run (`prepare`, `go`). Gives the build it put in place, if it did.
pub async fn take_program(client: &Client) -> Result<Option<String>> {
    let here = Here::of(&client::home()?);
    let Some(current) = here.read()?.current else { return Ok(None) };
    let builds = download(client).await?;
    let Some(one) = builds.get(PROGRAM) else { return Ok(None) };
    let (Some(latest), Some(serving)) = (one["build"].as_str(), one["serving"].as_str()) else { return Ok(None) };
    if count(latest) <= count(&current.program).max(count(BUILD)) || count(serving) > count(&current.program) {
        return Ok(None);
    }
    let program = program()?;
    let new = fetched_program(one, &program).await?;
    fs::create_dir_all(here.folder.join("programs").join(PROGRAM))?;
    fs::copy(&new, here.program(latest))?;
    // Put in place under the record's lock, and only while the server takes no update of its own, which pairs the
    // program in place with its build as it prepares.
    let mut taken = Ok(false);
    here.change(|record| {
        if record.server_update_status == ServerUpdateStatus::UpToDate && record.current.as_ref() == Some(&current) {
            taken = put(&new, &program).map(|()| {
                record.current = Some(Pair { build: current.build.clone(), program: latest.to_owned() });
                true
            });
        }
    })?;
    if !taken? {
        let _ = fs::remove_file(&new);
        return Ok(None);
    }
    Ok(Some(latest.to_owned()))
}

/// This program's file, which an update replaces, or `ONE_PROGRAM` where something else runs it: Linux names a
/// program whose file was replaced while it ran `<its path> (deleted)`, and it's the new file that's meant, as an app
/// that's also a server, which doesn't start again on its update, finds on its next.
pub fn program() -> std::io::Result<PathBuf> {
    env::var_os("ONE_PROGRAM").map(PathBuf::from).map_or_else(env::current_exe, Ok).map(|program| {
        let named = program.to_string_lossy();
        named.strip_suffix(" (deleted)").map(PathBuf::from).unwrap_or(program)
    })
}

/// This program's name among the network's builds: `one/macos-aarch64`, `one-cli/linux-x86_64`.
pub fn name() -> String {
    format!("{PROGRAM}/{}", target())
}

/// The network's latest build of this program, if it has one.
pub async fn latest_program(client: &Client) -> Result<Option<String>> {
    Ok(download(client).await?.get(PROGRAM).and_then(|one| one["build"].as_str()).map(str::to_owned))
}

/// Whether this program's file is another build than the one running: an update of this device's server takes the
/// program with its build, and what runs must never stay behind what's there.
pub fn replaced() -> bool {
    let Some(record) = client::home().ok().and_then(|home| Here::of(&home).read().ok()) else { return false };
    let pair = if record.server_update_status == ServerUpdateStatus::Verifying { record.next } else { record.current };
    pair.is_some_and(|pair| pair.program != BUILD)
}

/// This device's server, if it's one or becoming or stopping being one: its key, `server_membership` and the stage.
pub fn server() -> Option<Value> {
    let here = Here::of(&client::home().ok()?);
    let record = here.read().ok().filter(|record| record.server_membership != ServerMembership::Inactive)?;
    let key = here.key().ok()?;
    Some(json!({
        "key": key,
        "server_membership": record.server_membership,
        "server_membership_stage": record.server_membership_stage,
    }))
}

/// Prepares the latest server build and `one` as a pair, while the server answers on: downloads them, checks the build
/// starts, and says it's checked, to be taken at once once it's told to (`go`). So everything slow is done before it's told to go. Nothing
/// newer, or a download or check that failed, and it's up to date again, on what it was on.
pub(super) async fn prepare(
    here: &Here,
    record: &mut Record,
    server: &mut Child,
    said: &mut UnboundedReceiver<Heard>,
    program: &Path,
    reports: &Reports,
) -> Result<()> {
    *record = here
        .save(server, |record| {
            record.server_update_status = ServerUpdateStatus::Downloading;
            record.server_update_stage = None;
            record.next = None;
        })
        .await?;
    let prepared = match latest_builds(server, said).await {
        Ok(builds) => prepared(here, record, server, program, &builds).await,
        Err(error) => Err(error),
    };
    let on = record.current.as_ref().map(|current| current.build.clone()).unwrap_or_default();
    *record = here
        .save(server, |record| match prepared {
            Ok(true) => {}
            Ok(false) => {
                println!("This device is on the latest builds already.");
                record.server_update_status = ServerUpdateStatus::UpToDate;
                record.went_back = None;
            }
            Err(error) => {
                let to = record.next.take().map(|next| next.build).unwrap_or_default();
                reports.update(&on, &to, None, Some(format!("{error:#}")), "stayed");
                record.server_update_status = ServerUpdateStatus::UpToDate;
                record.server_update_stage = None;
                record.went(format!("Stayed on {on}: {error:#}"));
            }
        })
        .await?;
    Ok(())
}

/// Takes the latest pair among the network's `builds` and checks it (`prepare`): `false` where it's on it already.
pub(super) async fn prepared(
    here: &Here,
    record: &mut Record,
    server: &mut Child,
    program: &Path,
    builds: &Map<String, Value>,
) -> Result<bool> {
    let current = record.current.clone().context("a build it's on")?;
    let Some(next) = latest_pair(builds, &current) else { return Ok(false) };
    *record = here
        .save(server, |record| {
            record.next = Some(next.clone());
            record.server_update_stage = Some(ServerUpdateStage::Checking);
        })
        .await?;
    take(here, record, program, builds).await?;
    let taking = if next.build == current.build { format!("one {}", next.program) } else { next.build.clone() };
    println!("Prepared {taking}");
    *record = here
        .save(server, |record| {
            record.server_update_status = ServerUpdateStatus::Checked;
            record.server_update_stage = None;
        })
        .await?;
    Ok(true)
}

/// Takes the pair it has prepared and checked: has the system start both in the server's place, kept once the server
/// says it's on it at the network's edge. Only what it's told to do is done here, so it's quick: a server with nothing
/// checked does nothing.
pub(super) async fn go(here: &Here, record: &mut Record, server: &mut Child, program: &Path) -> Result<()> {
    if record.server_update_status != ServerUpdateStatus::Checked {
        return Ok(());
    }
    let current = record.current.clone().context("a build it's on")?;
    let next = record.next.clone().context("a pair it checked")?;
    println!("Taking {} with one {} by restarting", next.build, next.program);
    *record = here.change(|record| {
        record.server_update_status = ServerUpdateStatus::Verifying;
        record.server_update_stage = Some(ServerUpdateStage::Starting);
        record.update_began = Some(now());
        record.ended = Some(format!("It restarted to take {}", next.build));
    })?;
    stop(server, &record.told(ServerDaemonStatus::Stopping)).await?;
    if next.program != current.program {
        here.put_program(&next.program, program)?;
    }
    Err(again(here, program))
}

/// The latest builds for this kind of machine, by what they are (`server-code`, `one`), each with a link to it, as the
/// server reads them for it: so taking a build asks no other server, or an app one runs, which may be restarting in the
/// same release.
pub(super) async fn latest_builds(server: &mut Child, said: &mut UnboundedReceiver<Heard>) -> Result<Map<String, Value>> {
    tell(server, &Told::Latest { latest: target() }).await;
    within("say the latest builds", async {
        while let Some(heard) = said.recv().await {
            match heard.said {
                Some(Said::Latest { builds }) => return Ok(builds),
                Some(Said::NoLatest { why }) => bail!("It couldn't read the latest builds: {why}"),
                _ => {}
            }
        }
        bail!("it stopped before it said the latest builds")
    })
    .await
}

/// Takes the latest server build and `one` among the network's `builds`, which `record` names as `next`: the build put
/// together and checked, and the program replacing this one, the one before kept to go back to.
pub(super) async fn take(here: &Here, record: &Record, program: &Path, builds: &Map<String, Value>) -> Result<()> {
    let current = record.current.as_ref().context("a build it's on")?;
    let next = record.next.as_ref().context("a pair it takes")?;
    if next.build != current.build {
        here.take(builds, &record.told(ServerDaemonStatus::Running), Some(&current.build)).await?;
    }
    if next.program != current.program {
        // Both are kept beside the one in place, which they're put in place of only as the pair they're in is taken.
        // One downloaded and checked before, as a prepare asked again of the same pair has, is taken as it is.
        fs::create_dir_all(here.folder.join("programs").join(PROGRAM))?;
        fs::copy(program.canonicalize()?, here.program(&current.program))?;
        if !here.program(&next.program).exists() {
            fs::rename(fetched_program(&builds[PROGRAM], program).await?, here.program(&next.program))?;
        }
    }
    Ok(())
}

/// The pair the network's latest `builds` make for a server on `current`, where it's another.
pub(super) fn latest_pair(builds: &Map<String, Value>, current: &Pair) -> Option<Pair> {
    let next = Pair {
        build: latest(builds).unwrap_or(&current.build).to_owned(),
        program: builds.get(PROGRAM).and_then(|one| one["build"].as_str()).unwrap_or(&current.program).to_owned(),
    };
    (&next != current).then_some(next)
}

/// The latest server build among the network's `builds`: its code's, where there's a runtime for this kind of machine
/// to go with it.
pub(super) fn latest(builds: &Map<String, Value>) -> Option<&str> {
    builds.get("server-runtime").and(builds.get("server-code")).and_then(|code| code["build"].as_str())
}
