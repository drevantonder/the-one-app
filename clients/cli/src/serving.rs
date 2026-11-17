//! This device as one of its network's servers. Three state machines, each the one place its state changes:
//!
//! - `server_membership` (`ServerMembership`), whether it's a server: inactive, then joining (`one serve on`:
//!   registering, downloading, installing, starting) until it answers by its key, then active, then leaving
//!   (`one serve off`: draining, deregistering, uninstalling), then inactive again. Either that fails goes back to
//!   where it started, undoing what it did, and says why.
//! - `server_daemon_status`, the program the system keeps running (`one serve run`): starting until the server answers
//!   by its key, running, stopping, stopped. Restarts and their back-off are the system's: a systemd user unit on
//!   Linux, a LaunchAgent on a Mac, a scheduled task on Windows (`system`). This program only ends, non-zero where it
//!   failed.
//! - `server_update_status` (`ServerUpdateStatus`), while it's active: up to date, downloading the latest server
//!   build and `one` together, verifying the pair by starting both, and up to date again, kept or gone back.
//!
//! Every start is the one way: a build is started and has `START` to say it's running and answer by its key. What
//! each machine is at is one small record, `state.json` in the client's home's `server/`, beside `builds/<build>`,
//! each build kept, `programs/<build>`, the `one` before an update, and `home`, the server's own folder, its key among
//! it. The server is told on its input what changes of it (`OneCore.Release`), and says it to the network, where
//! it's ready once it's active and running.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{env, fs};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

use client::{BUILD, Client, target};

/// What the server says when it's running, with its key after, and when it's asked to take the latest builds.
const RUNNING: &str = "A server is running: ";
const UPDATE_ASKED: &str = "Update asked";
/// Which build of `one` an update takes the latest of: the app, with its window, or `one` built without it.
const PROGRAM: &str = if cfg!(feature = "window") { "one" } else { "one-cli" };
/// How long every start of a build has to say it's running and answer by its key.
const START: Duration = Duration::from_secs(180);
/// How long a server is given to stop: longer than it takes to answer what it took (`OneCore.Application`).
const STOP: Duration = Duration::from_secs(20);
/// How many of the last lines a build said are kept to say why it didn't start.
const LAST: usize = 5;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerMembership {
    #[default]
    Inactive,
    Joining,
    Active,
    Leaving,
}

/// Where joining or leaving is: each has the one way out on failure, back to where it began.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerMembershipStage {
    Registering,
    Downloading,
    Installing,
    Starting,
    Draining,
    Deregistering,
    Uninstalling,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerUpdateStatus {
    #[default]
    UpToDate,
    Downloading,
    Verifying,
}

/// What this program tells the server of its daemon: it's starting as it's started, running once it answers by its
/// key, and stopping as it's stopped. Stopped is what the network says of one that's gone.
#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ServerDaemonStatus {
    Running,
    Stopping,
}

/// A server's build and the `one` that runs it, which an update takes, keeps and goes back from together. A record
/// from before pairs doesn't know its program.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Pair {
    build: String,
    program: Option<String>,
}

/// What this device is as a server, written in one step (`Here::write`).
#[derive(Serialize, Deserialize, Default, Debug)]
struct Record {
    server_membership: ServerMembership,
    server_membership_stage: Option<ServerMembershipStage>,
    server_update_status: ServerUpdateStatus,
    current: Option<Pair>,
    previous: Option<Pair>,
    next: Option<Pair>,
    /// Why the last update didn't take, which the server says until one does.
    went_back: Option<String>,
}

impl Record {
    /// What's told the server of itself, on its input and as it starts.
    fn told(&self) -> Value {
        json!({
            "server_membership": self.server_membership,
            "server_update_status": self.server_update_status,
            "went_back": self.went_back,
        })
    }
}

/// Makes this device a server of whoever it's for: its person, or the group or network `ONE_SUBJECT` names. It's put
/// on their list, its build downloaded and checked, the system set to run it, and it's started: active once it
/// answers by its key. One that fails is taken back off, and nothing of it is left.
pub async fn on(client: &Client) -> Result<String> {
    let address = client.address().context("A server is for a network with an address, and this one has none.")?;
    let here = Here::of(&client::home()?);
    fs::create_dir_all(&here.folder)?;
    let lock = here.lock()?.context("This device is already becoming a server, or stopping being one.")?;
    let mut record = here.read()?;
    let key = here.key()?;
    let who = match env::var("ONE_SUBJECT") {
        Ok(who) => who,
        Err(_) => client.who().context("This device isn't signed in yet. Sign in on it first.")?,
    };
    match record.server_membership {
        ServerMembership::Active => return Ok(format!("This device is a server for {who} already. Its key: {key}")),
        ServerMembership::Leaving => {
            undo_leaving(client, &here, &mut record, &key, true).await?;
            return Ok(format!("This device is a server for {who} again. Its key: {key}"));
        }
        ServerMembership::Inactive | ServerMembership::Joining => {}
    }
    if let Err(error) = join(client, &here, &mut record, &key, &who).await {
        drop(lock);
        let left = undo_joining(client, &here, &key).await;
        return Err(error.context(format!("This device isn't a server{left}")));
    }
    Ok(format!("This device is a server for {who} now, and starts whenever it does. Its key: {key}\n{address}"))
}

async fn join(client: &Client, here: &Here, record: &mut Record, key: &str, who: &str) -> Result<()> {
    record.server_membership = ServerMembership::Joining;
    here.stage(record, ServerMembershipStage::Registering)?;
    tool(client, "add_server", json!({"key": key})).await?;
    here.stage(record, ServerMembershipStage::Downloading)?;
    if record.current.is_none() {
        let builds = download(client).await?;
        let build = builds
            .get("server")
            .with_context(|| format!("The network has no server for {} yet: `tooling/release` makes one.", target()))?;
        record.current = Some(Pair { build: here.take(build).await?, program: Some(BUILD.to_owned()) });
    }
    here.stage(record, ServerMembershipStage::Installing)?;
    system::install(here, key, &env::current_exe()?, &unit_env(who.to_owned(), None).await?)?;
    here.stage(record, ServerMembershipStage::Starting)?;
    // The system starts `one serve run`, which makes it active once it answers by its key.
    system::start(here, key).await?;
    ensure!(here.read()?.server_membership == ServerMembership::Active, "It started, and didn't answer by its key");
    Ok(())
}

/// Takes back what joining did, from wherever it got to: the system no longer runs it, it's off its owner's list,
/// and its folder goes. Gives what of that couldn't be done, to say with why joining failed.
async fn undo_joining(client: &Client, here: &Here, key: &str) -> String {
    let _ = system::remove(here, key);
    let listed = tool(client, "remove_server", json!({"key": key})).await.err();
    let _ = fs::remove_dir_all(&here.folder);
    listed.map(|why| format!(", though it's still on the list of servers: {why:#}")).unwrap_or_default()
}

/// The environment the system runs the server with: the client it updates by, who it's for, and where programs
/// are found, `path` where it's known, as the system gave it to a server that's running, and otherwise its person's
/// login PATH. Windows starts a task with its person's environment as they log in, PATH and all.
async fn unit_env(who: String, path: Option<String>) -> Result<Vec<(&'static str, String)>> {
    let mut env = vec![("ONE_HOME", client::home()?.display().to_string()), ("ONE_SUBJECT", who)];
    #[cfg(unix)]
    env.push((
        "PATH",
        match path {
            Some(path) => path,
            None => login_path().await?,
        },
    ));
    #[cfg(not(unix))]
    let _ = path;
    // A network being built has a command say who it acts as, and so does what runs here for it.
    if let Ok(name) = env::var("ONE_AS") {
        env.push(("ONE_AS", name));
    }
    Ok(env)
}

/// The PATH its person has in a terminal on this machine, which their login shell sets. A server's services run
/// programs a person installed (`git`, `go`, `rustc`, `swiftc`) wherever their shell finds them, which isn't on the
/// bare PATH that ssh, or an app opened from the Dock, gives. It's read the way editors that run a person's tools read
/// it (VS Code's shell environment): the shell started as a login and interactive one, its PATH printed between two
/// marks, past whatever its startup files print.
#[cfg(unix)]
async fn login_path() -> Result<String> {
    let shell = unsafe {
        let user = libc::getpwuid(libc::getuid());
        ensure!(!user.is_null(), "this machine says nothing of who's running this");
        std::ffi::CStr::from_ptr((*user).pw_shell).to_string_lossy().into_owned()
    };
    const MARK: &str = "one-path-6f0c";
    let command = Command::new(&shell)
        .args(["-i", "-l", "-c", &format!("printf '%s%s%s' {MARK} \"$PATH\" {MARK}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = timeout(Duration::from_secs(10), command)
        .await
        .with_context(|| format!("Your login shell, {shell}, didn't say its PATH within 10 seconds"))??;
    let output = String::from_utf8_lossy(&output.stdout);
    let path = output.split(MARK).nth(1).filter(|path| !path.is_empty());
    Ok(path.with_context(|| format!("Your login shell, {shell}, didn't say its PATH"))?.to_owned())
}

/// Stops this device being a server: it drains, so the network sends it nothing new and what it took finishes, it's
/// taken off its owner's list, which another of their servers does, and the system no longer runs it, with nothing of
/// it left. One that fails is put back as it was, running again.
pub async fn off(client: &Client) -> Result<String> {
    let here = Here::of(&client::home()?);
    ensure!(here.read()?.server_membership != ServerMembership::Inactive, "This device isn't a server.");
    let lock = here.lock()?.context("This device is already becoming a server, or stopping being one.")?;
    let mut record = here.read()?;
    let key = here.key()?;
    if record.server_membership == ServerMembership::Joining {
        drop(lock);
        let left = undo_joining(client, &here, &key).await;
        return Ok(format!("This device stopped becoming a server{left}"));
    }
    if let Err(error) = leave(client, &here, &mut record, &key, lock).await {
        undo_leaving(client, &here, &mut record, &key, true).await?;
        return Err(error.context("It's still a server"));
    }
    Ok(format!("This device is no longer a server, and nothing of it is left here: {key}"))
}

async fn leave(client: &Client, here: &Here, record: &mut Record, key: &str, lock: fs::File) -> Result<()> {
    record.server_membership = ServerMembership::Leaving;
    // The system stops `one serve run`, which reads that it's leaving and has the server drain as it stops.
    here.stage(record, ServerMembershipStage::Draining)?;
    system::stop(here, key)?;
    here.stage(record, ServerMembershipStage::Deregistering)?;
    tool(client, "remove_server", json!({"key": key})).await?;
    here.stage(record, ServerMembershipStage::Uninstalling)?;
    system::remove(here, key)?;
    drop(lock);
    Ok(fs::remove_dir_all(&here.folder)?)
}

/// Takes back what leaving did, from wherever it got to: it's back on its owner's list where it may have come off,
/// and active, and the system runs it again unless it's what's running this.
async fn undo_leaving(client: &Client, here: &Here, record: &mut Record, key: &str, start: bool) -> Result<()> {
    use ServerMembershipStage::*;
    if matches!(record.server_membership_stage, Some(Deregistering | Uninstalling)) {
        tool(client, "add_server", json!({"key": key}))
            .await
            .context("It couldn't be put back on the list of servers")?;
    }
    record.server_membership = ServerMembership::Active;
    record.server_membership_stage = None;
    here.write(record)?;
    if start {
        let who = env::var("ONE_SUBJECT").or_else(|_| client.who().context("who it's for"))?;
        system::install(here, key, &env::current_exe()?, &unit_env(who, None).await?)?;
        system::start(here, key).await?;
    }
    Ok(())
}

/// Takes the latest builds: this device's server's, if it's one, which it's asked to take, and otherwise the
/// program's own.
pub async fn update(client: &Client) -> Result<String> {
    let here = Here::of(&client::home()?);
    if here.read()?.server_membership != ServerMembership::Inactive {
        return tool(client, "update", json!({"server": here.key()?})).await;
    }
    let builds = download(client).await?;
    let one = builds.get("one").with_context(|| format!("The network has no build of one for {} yet.", target()))?;
    if one["build"] == BUILD {
        return Ok(format!("This device is on the latest build already: {BUILD}"));
    }
    take_program(one, &env::current_exe()?).await?;
    Ok(format!("This device is on the build {} now.", one["build"].as_str().unwrap_or_default()))
}

/// Whether this program's file is another build than the one running: an update of this device's server takes the
/// program with its build, and what runs must never stay behind what's there.
pub fn replaced() -> bool {
    let Some(record) = client::home().ok().and_then(|home| Here::of(&home).read().ok()) else { return false };
    let pair = if record.server_update_status == ServerUpdateStatus::Verifying { record.next } else { record.current };
    pair.and_then(|pair| pair.program).is_some_and(|program| program != BUILD)
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

/// Runs this device's server, as the system runs it: starts the build the record says, within `START`, keeps or goes
/// back from a pair being verified, and takes the latest builds when the server's asked. It ends when the server
/// stops, or when the system stops it, after the server has, and non-zero for the system to start it again.
pub async fn run(client: Arc<Client>) -> Result<()> {
    // The program's own file, named before an update replaces what's there.
    let program = env::current_exe()?;
    let here = Here::of(&client::home()?);
    let key = here.key()?;
    let address = client.address().context("A server is for a network with an address, and this one has none.")?;
    let who = env::var("ONE_SUBJECT").context("ONE_SUBJECT says who the server is for")?;
    let mut record = here.read()?;
    // Joining or leaving that nothing is doing any more, as a device that was switched off part-way, goes back to
    // where it started. Joining that got to starting is this start's to decide.
    if let Some(_lock) = here.lock()? {
        match (record.server_membership, record.server_membership_stage) {
            (ServerMembership::Joining, Some(ServerMembershipStage::Starting)) => {}
            (ServerMembership::Joining, _) => {
                println!(
                    "Joining didn't finish, so this device isn't a server{}.",
                    undo_joining(&client, &here, &key).await
                );
                return Ok(());
            }
            (ServerMembership::Leaving, _) => undo_leaving(&client, &here, &mut record, &key, false).await?,
            _ => {}
        }
    }
    if record.server_membership == ServerMembership::Inactive {
        println!("This device isn't a server: `one serve on` makes it one.");
        return Ok(());
    }
    // The system runs it as this build says to, from its next start. Where it can't be rewritten, as one changed by
    // hand may not be, it runs as it was set up rather than not at all.
    if let Err(error) = system::install(&here, &key, &program, &unit_env(who.clone(), env::var("PATH").ok()).await?) {
        eprintln!("Couldn't rewrite how the system runs it, so it runs as it was set up: {error:#}");
    }
    let log = here.folder.join("server.log");
    system::bound(&log)?;

    let verifying = record.server_update_status == ServerUpdateStatus::Verifying;
    let pair = if verifying { record.next.clone() } else { record.current.clone() };
    let pair = pair.context("The record names no build to run")?;
    let mut server = Command::from(command(release(&here.folder.join("builds").join(&pair.build))));
    server
        .args(["eval", "OneCore.Release.serve()"])
        .env("ONE_SERVER_HOME", here.folder.join("home"))
        .env("ONE_ADDRESS", &address)
        .env("ONE_SUBJECT", &who)
        .env("ONE_DEVICE", client.key())
        .env("ONE_NAME", client::name())
        .env("ONE_SERVER_STATUS", record.told().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut server = server.spawn().with_context(|| format!("couldn't start the build {}", pair.build))?;
    // Everything it starts goes when it does, or when this does.
    #[cfg(windows)]
    let _contained = system::contain(&server)?;
    let (last, mut said) = listen(&mut server, log)?;

    let started = timeout(START, async {
        while let Some(line) = said.recv().await {
            if line.starts_with(RUNNING) {
                return answers(&client, &key, &pair.build).await;
            }
        }
        Err(anyhow::anyhow!("it stopped before it was running"))
    });
    let started = match started.await {
        Ok(started) => started,
        Err(_) => Err(anyhow::anyhow!("it didn't start and answer by its key within {} s", START.as_secs())),
    };
    if let Err(why) = started {
        let _ = server.kill().await;
        let lines = last.lock().unwrap().iter().cloned().collect::<Vec<_>>().join(" | ");
        let why = format!("{why:#}. Its last lines: {lines}");
        if !verifying {
            bail!("The server on {} didn't start: {why}", pair.build)
        }
        let current = record.current.clone().context("a build it was on")?;
        record.went_back = Some(format!("Went back to {} from {}: {why}", current.build, pair.build));
        println!("{}", record.went_back.as_deref().unwrap_or_default());
        here.went_back(&mut record, &program)?;
        return Err(again(&program));
    }
    if verifying {
        let was = record.current.replace(pair.clone());
        record.previous = was.clone();
        record.next = None;
        record.went_back = None;
        record.server_update_status = ServerUpdateStatus::UpToDate;
        println!(
            "The server is on the build {} now, from {}.",
            pair.build,
            was.map(|was| was.build).unwrap_or_default()
        );
    }
    if record.server_membership == ServerMembership::Joining {
        record.server_membership = ServerMembership::Active;
        record.server_membership_stage = None;
    }
    here.write(&record)?;
    here.prune(&record)?;
    let mut told = record.told();
    told["server_daemon_status"] = json!(ServerDaemonStatus::Running);
    tell(&mut server, told).await;
    system::ready(&key)?;

    let stopping = stopped();
    tokio::pin!(stopping);
    loop {
        tokio::select! {
            line = said.recv() => match line {
                Some(line) if line == UPDATE_ASKED => update_server(&here, &mut record, &mut server, &program).await?,
                Some(_) => {}
                None => bail!("The server stopped: {}", server.wait().await?),
            },
            _ = &mut stopping => {
                // `one serve off` says it's leaving before it has the system stop this, and the server drains.
                let mut told = here.read()?.told();
                told["server_daemon_status"] = json!(ServerDaemonStatus::Stopping);
                tell(&mut server, told).await;
                return stop(&mut server).await;
            }
        }
    }
}

/// Reads what the server says, on its output and its errors, as it comes, whatever this is doing meanwhile: a
/// server whose output isn't read is held up once the pipe it writes to is full, 16 KB on a Mac, so each line it
/// logs holds up whatever logged it. Gives its last lines, which say why a build didn't start, and what this acts
/// on: that it's running, and that it's asked to update.
#[allow(clippy::type_complexity)]
fn listen(
    server: &mut Child,
    log: PathBuf,
) -> Result<(Arc<Mutex<Vec<String>>>, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    let last = Arc::new(Mutex::new(Vec::new()));
    let keep = {
        let last = last.clone();
        move |line: &str| {
            let mut last = last.lock().unwrap();
            if last.len() == LAST {
                last.remove(0);
            }
            last.push(line.to_owned());
        }
    };
    let mut errors = BufReader::new(server.stderr.take().context("the server's errors")?).lines();
    let keep_error = keep.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = errors.next_line().await {
            eprintln!("{line}");
            keep_error(&line);
        }
    });
    let (heard, said) = tokio::sync::mpsc::unbounded_channel();
    let mut lines = BufReader::new(server.stdout.take().context("the server's output")?).lines();
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{line}");
            if let Err(error) = system::bound(&log) {
                eprintln!("Couldn't keep {} small: {error:#}", log.display());
            }
            keep(&line);
            if line.starts_with(RUNNING) || line == UPDATE_ASKED {
                let _ = heard.send(line);
            }
        }
    });
    Ok((last, said))
}

/// Whether the server whose key is `key` answers by it, saying it's `build`. It's asked again until it does: the
/// network's relay passes calls to a server that's just started only once it has seen it there.
async fn answers(client: &Client, key: &str, build: &str) -> Result<()> {
    loop {
        let response = tool(client, "builds", json!({"server": key})).await.ok();
        let response = response.and_then(|response| serde_json::from_str::<Value>(&response).ok());
        if response.is_some_and(|response| response["build"] == build) {
            return Ok(());
        }
        sleep(Duration::from_secs(2)).await;
    }
}

/// Once the system asks this program to stop: SIGTERM, as systemd and launchd stop a program. Task Scheduler ends a
/// task's program outright.
async fn stopped() {
    #[cfg(unix)]
    if let Ok(mut terminate) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        terminate.recv().await;
        return;
    }
    std::future::pending::<()>().await
}

/// Tells the server what changed of it: a line of JSON, which it says to the network (`OneCore.Release`). A server
/// that has stopped is told nothing, and needs telling nothing.
async fn tell(server: &mut Child, said: Value) {
    if let Some(input) = server.stdin.as_mut() {
        let _ = input.write_all(format!("{said}\n").as_bytes()).await;
        let _ = input.flush().await;
    }
}

/// Stops a server that's running by closing its input, once it has read what it was told last: it takes nothing new
/// and answers what it took already (`OneCore.Application`), and is killed if it hasn't stopped within `STOP`.
async fn stop(server: &mut Child) -> Result<()> {
    drop(server.stdin.take());
    if timeout(STOP, server.wait()).await.is_err() {
        server.kill().await?;
    }
    Ok(())
}

/// This program started again in its place, as the system started it, to run the pair the record says: on Windows,
/// which replaces no running program, by ending for Task Scheduler to start it.
fn again(program: &Path) -> anyhow::Error {
    #[cfg(unix)]
    return std::os::unix::process::CommandExt::exec(std::process::Command::new(program).args(["serve", "run"])).into();
    #[cfg(not(unix))]
    anyhow::anyhow!("It ended to run {}, which Task Scheduler starts within a minute.", program.display())
}

/// Takes the latest server build and `one` as a pair, and has the system start them to verify them. While they
/// download, the server answers on; nothing newer, or a download that failed, and it's up to date again.
async fn update_server(here: &Here, record: &mut Record, server: &mut Child, program: &Path) -> Result<()> {
    record.server_update_status = ServerUpdateStatus::Downloading;
    here.write(record)?;
    tell(server, record.told()).await;
    // Asked with the token this device has now, renewed if it has ended: this one's own renewing stops where it was
    // told the device is nobody's, and its timer doesn't count while the machine sleeps.
    let taken = match Client::open(client::home()?, Vec::new()).await {
        Ok(now) => {
            let taken = take(&now, here, record, program).await;
            now.close().await;
            taken
        }
        Err(error) => Err(error),
    };
    record.server_update_status = ServerUpdateStatus::UpToDate;
    match taken {
        Ok(Some(next)) => {
            record.next = Some(next);
            record.server_update_status = ServerUpdateStatus::Verifying;
            here.write(record)?;
            let mut told = record.told();
            told["server_daemon_status"] = json!(ServerDaemonStatus::Stopping);
            tell(server, told).await;
            stop(server).await?;
            return Err(again(program));
        }
        Ok(None) => println!("This device is on the latest builds already."),
        Err(error) => {
            let on = record.current.as_ref().map(|current| current.build.clone()).unwrap_or_default();
            record.went_back = Some(format!("Stayed on {on}: {error:#}"));
            println!("{}", record.went_back.as_deref().unwrap_or_default());
        }
    }
    here.write(record)?;
    tell(server, record.told()).await;
    Ok(())
}

/// Takes the latest server build and `one` there's a newer one of: the build unpacked and checked, and the program
/// replacing this one, the one before kept to go back to. Gives the pair, or `None` where both are the latest.
async fn take(client: &Client, here: &Here, record: &Record, program: &Path) -> Result<Option<Pair>> {
    let builds = download(client).await?;
    let current = record.current.clone().context("a build it's on")?;
    let mut next = Pair { build: current.build.clone(), program: Some(BUILD.to_owned()) };
    if let Some(server) = builds.get("server")
        && server["build"] != current.build
    {
        next.build = here.take(server).await?;
    }
    if let Some(one) = builds.get(PROGRAM)
        && one["build"] != BUILD
    {
        fs::create_dir_all(here.folder.join("programs"))?;
        fs::copy(program.canonicalize()?, here.folder.join("programs").join(BUILD))?;
        take_program(one, program).await?;
        next.program = one["build"].as_str().map(str::to_owned);
    }
    Ok((next != Pair { program: Some(BUILD.to_owned()), ..current }).then_some(next))
}

/// Replaces the program `program` with the build `one`, once it has said it's that build. What runs now runs on, and
/// the next start is the new one. It's the file a link to it names that's replaced, as on a Mac `~/.local/bin/one`
/// is a link to the program in The One App's bundle.
async fn take_program(one: &Value, program: &Path) -> Result<()> {
    let build = one["build"].as_str().context("a build says which it is")?;
    let packed = program.canonicalize()?.with_extension("new.gz");
    fetch(one, &packed).await?;
    let new = packed.with_extension("");
    let unpacked =
        std::io::copy(&mut flate2::read::GzDecoder::new(fs::File::open(&packed)?), &mut fs::File::create(&new)?);
    fs::remove_file(&packed)?;
    unpacked.with_context(|| format!("The build {build} of one didn't unpack."))?;
    #[cfg(unix)]
    fs::set_permissions(&new, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    let output = command(&new).arg("--version").output();
    if !output
        .as_ref()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).split_whitespace().nth(1) == Some(build))
    {
        fs::remove_file(&new)?;
        bail!("The build {build} of one doesn't start here.")
    }
    put(&new, program)
}

/// Puts the program `new` in place of `program`, in one step. Windows replaces no program that's running, but lets
/// it be renamed: the one before is put aside, and goes the next time.
fn put(new: &Path, program: &Path) -> Result<()> {
    let program = program.canonicalize()?;
    #[cfg(windows)]
    {
        let aside = program.with_extension("old");
        let _ = fs::remove_file(&aside);
        fs::rename(&program, &aside)?;
    }
    Ok(fs::rename(new, program)?)
}

/// What starts a build of the server: its release's script, a batch file on Windows.
fn release(build: &Path) -> PathBuf {
    build.join(if cfg!(windows) { "bin/one_core.bat" } else { "bin/one_core" })
}

/// A program this one starts, which has no window of its own: on Windows a console program started by one with no
/// console, as a server's is, would open one.
fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    command
}

/// The latest builds for this kind of machine, by what they are (`server`, `one`), each with a link to it.
async fn download(client: &Client) -> Result<Map<String, Value>> {
    let response = tool(client, "download", json!({"target": target()})).await?;
    match serde_json::from_str(&response)? {
        Value::Object(builds) => Ok(builds),
        _ => bail!("the network didn't list its builds"),
    }
}

/// Downloads a build by its link into `file`, and checks it's the size and SHA-256 the network said.
async fn fetch(build: &Value, file: &Path) -> Result<()> {
    let link = &build["link"];
    let method = reqwest::Method::from_bytes(link["method"].as_str().unwrap_or("GET").as_bytes())?;
    let mut request = client::web()?.request(method, link["url"].as_str().context("a build has a link")?);
    for (name, value) in link["headers"].as_object().into_iter().flatten() {
        request = request.header(name, value.as_str().unwrap_or_default());
    }
    let mut response = request.send().await?.error_for_status()?;
    let mut out = tokio::fs::File::create(file).await?;
    let (mut hash, mut size) = (Sha256::new(), 0);
    while let Some(part) = response.chunk().await? {
        hash.update(&part);
        size += part.len() as u64;
        out.write_all(&part).await?;
    }
    out.flush().await?;
    if build["sha256"] != data_encoding::HEXLOWER.encode(&hash.finalize()) || build["size"] != size {
        fs::remove_file(file)?;
        bail!("The build {} didn't download whole.", build["build"]);
    }
    Ok(())
}

/// Calls one of the network's tools and gives what it said, or the error it said.
async fn tool(client: &Client, name: &str, arguments: Value) -> Result<String> {
    let result = client.request("tools/call", json!({"name": name, "arguments": arguments})).await?;
    let lines: Vec<&str> =
        result["content"].as_array().into_iter().flatten().filter_map(|part| part["text"].as_str()).collect();
    let message = lines.join("\n");
    if result["isError"] == true {
        bail!("{message}")
    }
    Ok(message)
}

/// This device's server, in the client's home.
struct Here {
    folder: PathBuf,
}

impl Here {
    fn of(home: &Path) -> Self {
        Self { folder: home.join("server") }
    }

    /// The server's key, made the first time: it's put on a list before it's ever started.
    fn key(&self) -> Result<String> {
        Ok(client::key(&self.folder.join("home/key"))?.public().to_string())
    }

    /// What this device is as a server. A device from before the record had a file for each part of it, which say
    /// the same: `current` and `previous`, links to builds, `next`, a build being tried by the program that's here
    /// now, and `went-back`. They're moved into it the first time.
    fn read(&self) -> Result<Record> {
        match fs::read(self.folder.join("state.json")) {
            Ok(record) => return Ok(serde_json::from_slice(&record)?),
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            Err(_) => {}
        }
        let file = |name: &str| self.folder.join(name);
        let linked =
            |name: &str| fs::read_link(file(name)).ok()?.file_name().map(|build| build.to_string_lossy().into_owned());
        let Some(current) = linked("current") else { return Ok(Record::default()) };
        let next = fs::read_to_string(file("next")).ok().map(|next| next.trim().to_owned());
        let record = Record {
            server_membership: ServerMembership::Active,
            server_update_status: if next.is_some() {
                ServerUpdateStatus::Verifying
            } else {
                ServerUpdateStatus::UpToDate
            },
            // The program running is the current one's, unless it's the next one's, from an update the program before
            // took.
            current: Some(Pair { build: current, program: next.is_none().then(|| BUILD.to_owned()) }),
            previous: linked("previous").map(|build| Pair { build, program: None }),
            next: next.map(|build| Pair { build, program: Some(BUILD.to_owned()) }),
            went_back: fs::read_to_string(file("went-back")).ok(),
            server_membership_stage: None,
        };
        self.write(&record)?;
        for name in ["current", "previous", "next", "went-back"] {
            let _ = fs::remove_file(file(name)).or_else(|_| fs::remove_dir(file(name)));
        }
        Ok(record)
    }

    /// Writes the record in one step: a file beside it, renamed over it.
    fn write(&self, record: &Record) -> Result<()> {
        let writing = self.folder.join("state.json.new");
        fs::write(&writing, serde_json::to_vec_pretty(record)?)?;
        Ok(fs::rename(writing, self.folder.join("state.json"))?)
    }

    /// Writes that joining or leaving is at `stage`.
    fn stage(&self, record: &mut Record, stage: ServerMembershipStage) -> Result<()> {
        record.server_membership_stage = Some(stage);
        self.write(record)
    }

    /// Holds the lock `one serve on` and `one serve off` hold while they run, if nothing else holds it.
    fn lock(&self) -> Result<Option<fs::File>> {
        let Ok(file) = fs::File::create(self.folder.join("lock")) else { return Ok(None) };
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    /// A pair that didn't start goes: the program before it is put back, and it's up to date on what it was on.
    fn went_back(&self, record: &mut Record, program: &Path) -> Result<()> {
        let next = record.next.take();
        let current = record.current.as_ref().and_then(|current| current.program.clone());
        if let Some(before) =
            current.filter(|before| next.as_ref().is_some_and(|next| next.program.as_ref() != Some(before)))
        {
            let kept = self.folder.join("programs").join(&before);
            if kept.exists() {
                let new = program.canonicalize()?.with_extension("new");
                fs::copy(kept, &new)?;
                put(&new, program)?;
            }
        }
        record.server_update_status = ServerUpdateStatus::UpToDate;
        self.write(record)
    }

    /// Removes the builds and programs the record names as neither current, previous nor next.
    fn prune(&self, record: &Record) -> Result<()> {
        let pairs = [&record.current, &record.previous, &record.next];
        let named = |name: &str| {
            pairs
                .iter()
                .filter_map(|pair| pair.as_ref())
                .any(|pair| pair.build == name || pair.program.as_deref() == Some(name))
        };
        for folder in ["builds", "programs"] {
            for kept in fs::read_dir(self.folder.join(folder)).into_iter().flatten() {
                let kept = kept?;
                if !named(&kept.file_name().to_string_lossy()) {
                    let _ = fs::remove_dir_all(kept.path()).or_else(|_| fs::remove_file(kept.path()));
                }
            }
        }
        Ok(())
    }

    /// Downloads the build `build`, unpacks it and has it say which it is, which starts its VM and loads its
    /// NIFs: a build that can't do that here goes. Gives its name. One that's here already is taken as it is.
    async fn take(&self, build: &Value) -> Result<String> {
        let name = build["build"].as_str().context("a build says which it is")?.to_owned();
        let builds = self.folder.join("builds");
        let folder = builds.join(&name);
        if folder.exists() {
            return Ok(name);
        }
        fs::create_dir_all(&builds)?;
        let archive = builds.join(format!("{name}.tar.gz"));
        let unpacking = builds.join(format!("{name}.unpacking"));
        fetch(build, &archive).await.with_context(|| format!("the build {name} didn't download"))?;
        let _ = fs::remove_dir_all(&unpacking);
        fs::create_dir_all(&unpacking)?;
        let unpacked = command("tar").arg("-xzf").arg(&archive).arg("-C").arg(&unpacking).status()?;
        fs::remove_file(&archive)?;
        ensure!(unpacked.success(), "The build {name} didn't unpack.");
        let output = command(release(&unpacking)).args(["eval", "OneCore.Release.check()"]).output()?;
        let (out, errors) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        let out = out.trim();
        if out != name {
            fs::remove_dir_all(&unpacking)?;
            let last: Vec<&str> = errors.lines().rev().take(LAST).collect::<Vec<_>>().into_iter().rev().collect();
            if out.is_empty() {
                bail!("the build {name}'s start-up check failed; its last lines: {}", last.join(" | "));
            }
            bail!("the build {name} says it's {out}, not what was published");
        }
        fs::rename(&unpacking, &folder)?;
        Ok(name)
    }
}

/// How the system keeps the server running, restarts it when it ends non-zero, and backs off one that keeps failing:
/// a systemd user unit on Linux, which starts at boot where the user's processes linger and at their login
/// otherwise. Each is named after the server's key, so a machine can serve more than one network. The unit's file
/// is in the server's folder, and goes with it.
#[cfg(target_os = "linux")]
mod system {
    use super::*;

    fn name(key: &str) -> String {
        format!("one-server-{}.service", &key[..12])
    }

    /// The file systemd reads the unit from, where it knows the unit: wherever it is, as a person may have put it
    /// in their own folder of units by hand.
    fn loaded(key: &str) -> Result<Option<PathBuf>> {
        let done = std::process::Command::new("systemctl")
            .args(["--user", "show", "-p", "FragmentPath", "--value", &name(key)])
            .output()?;
        let path = String::from_utf8_lossy(&done.stdout).trim().to_owned();
        Ok((!path.is_empty()).then(|| PathBuf::from(path)))
    }

    /// Writes the unit, which takes effect from its next start: over the file systemd reads it from, or, new, in the
    /// server's folder, enabled from there. It's restarted 10 s after it ends non-zero, and left failed after 10
    /// starts in 10 minutes, a crash loop. A start has `START` twice over: a pair being verified, then the one before
    /// it. Only `one serve run` is told to stop, and has the server drain first.
    pub fn install(here: &Here, key: &str, program: &Path, env: &[(&str, String)]) -> Result<()> {
        let loaded = loaded(key)?;
        let unit = loaded.clone().unwrap_or_else(|| here.folder.join(name(key)));
        let env: String = env.iter().map(|(name, value)| format!("Environment=\"{name}={value}\"\n")).collect();
        fs::write(
            &unit,
            format!(
                "[Unit]\nDescription=The One App's server\nStartLimitIntervalSec=600\nStartLimitBurst=10\n\n\
                 [Service]\nType=notify\nExecStart=\"{}\" serve run\n{env}Restart=on-failure\nRestartSec=10\n\
                 TimeoutStartSec={}\nKillMode=mixed\nTimeoutStopSec={}\n\n[Install]\nWantedBy=default.target\n",
                program.display(),
                2 * START.as_secs() + 30,
                STOP.as_secs() + 10,
            ),
        )?;
        if loaded.is_none() {
            systemctl(&["enable", &unit.display().to_string()])?;
        }
        systemctl(&["daemon-reload"])
    }

    /// Has systemd start it now, or again where it's running, with what it's run with now. That returns once the
    /// server answers by its key (`ready`), or failed to start.
    pub async fn start(_here: &Here, key: &str) -> Result<()> {
        systemctl(&["reset-failed", &name(key)]).ok();
        systemctl(&["restart", &name(key)])
    }

    pub fn stop(_here: &Here, key: &str) -> Result<()> {
        systemctl(&["stop", &name(key)])
    }

    /// Has systemd forget the unit: one in its own folder of units goes, and one in the server's folder goes with it.
    pub fn remove(_here: &Here, key: &str) -> Result<()> {
        let loaded = loaded(key)?;
        systemctl(&["disable", "--now", &name(key)])?;
        if let Some(unit) = loaded.filter(|unit| unit.exists()) {
            fs::remove_file(unit)?;
        }
        systemctl(&["daemon-reload"])
    }

    /// What the server says is in systemd's journal, which keeps it within its own bounds.
    pub fn bound(_log: &Path) -> Result<()> {
        Ok(())
    }

    /// Tells systemd the server is running, as `Type=notify` has it wait for.
    pub fn ready(_key: &str) -> Result<()> {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::{SocketAddr, UnixDatagram};
        let Some(socket) = env::var_os("NOTIFY_SOCKET") else { return Ok(()) };
        let socket = socket.to_string_lossy();
        let address = match socket.strip_prefix('@') {
            Some(name) => SocketAddr::from_abstract_name(name)?,
            None => SocketAddr::from_pathname(&*socket)?,
        };
        UnixDatagram::unbound()?.send_to_addr(b"READY=1", &address)?;
        Ok(())
    }

    fn systemctl(args: &[&str]) -> Result<()> {
        let done = std::process::Command::new("systemctl").arg("--user").args(args).output()?;
        ensure!(done.status.success(), "systemd didn't: {}", String::from_utf8_lossy(&done.stderr).trim());
        Ok(())
    }
}

/// A LaunchAgent on a Mac, which starts at its person's login and is started again 10 s apart at the soonest when it
/// ends non-zero. launchd has no limit on how long a start takes, which is why every start has `START` here.
#[cfg(target_os = "macos")]
mod system {
    use std::os::unix::net::UnixDatagram;

    use super::*;

    fn label(key: &str) -> String {
        format!("app.theone.server.{}", &key[..12])
    }

    fn agent(key: &str) -> Result<PathBuf> {
        Ok(dirs::home_dir().context("a home folder")?.join(format!("Library/LaunchAgents/{}.plist", label(key))))
    }

    fn domain() -> Result<String> {
        let user = String::from_utf8(std::process::Command::new("id").arg("-u").output()?.stdout)?;
        Ok(format!("gui/{}", user.trim()))
    }

    /// Writes the LaunchAgent, which launchd reads as it loads it: at `start` and each login. Its output is
    /// `server.log` in the server's folder, and it says it's running on a socket there (`ready`).
    pub fn install(here: &Here, key: &str, program: &Path, env: &[(&str, String)]) -> Result<()> {
        let text = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;");
        let env: String = env
            .iter()
            .map(|(name, value)| (*name, value.clone()))
            .chain([("NOTIFY_SOCKET", here.folder.join("ready").display().to_string())])
            .map(|(name, value)| format!("<key>{name}</key><string>{}</string>", text(&value)))
            .collect();
        let log = text(&here.folder.join("server.log").display().to_string());
        let agent = agent(key)?;
        fs::create_dir_all(agent.parent().context("a folder")?)?;
        fs::write(
            &agent,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>Label</key><string>{}</string>\
                 <key>ProgramArguments</key><array><string>{}</string><string>serve</string><string>run</string></array>\
                 <key>EnvironmentVariables</key><dict>{env}</dict>\
                 <key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\
                 <key>ThrottleInterval</key><integer>10</integer><key>ExitTimeOut</key><integer>{}</integer>\
                 <key>StandardOutPath</key><string>{log}</string><key>StandardErrorPath</key><string>{log}</string>\
                 </dict></plist>\n",
                label(key),
                text(&program.display().to_string()),
                STOP.as_secs() + 10,
            ),
        )?;
        Ok(())
    }

    /// Has launchd load the agent, which starts it, again where it's loaded. That returns once the server answers by
    /// its key (`ready`).
    pub async fn start(here: &Here, key: &str) -> Result<()> {
        let ready = here.folder.join("ready");
        let _ = stop(here, key);
        let _ = fs::remove_file(&ready);
        let socket = UnixDatagram::bind(&ready)?;
        socket.set_read_timeout(Some(2 * START))?;
        launchctl(&["bootstrap", &domain()?, &agent(key)?.display().to_string()])?;
        let running = socket.recv(&mut [0; 16]);
        fs::remove_file(&ready)?;
        running.with_context(|| {
            format!("The server didn't start: its log is {}", here.folder.join("server.log").display())
        })?;
        Ok(())
    }

    pub fn stop(_here: &Here, key: &str) -> Result<()> {
        launchctl(&["bootout", &format!("{}/{}", domain()?, label(key))])
    }

    pub fn remove(here: &Here, key: &str) -> Result<()> {
        let _ = stop(here, key);
        Ok(fs::remove_file(agent(key)?)?)
    }

    /// Keeps `server.log` under `LOG`, as launchd keeps no bounds of its own: past it, the log is `server.log.1`, in
    /// place of the one before, and this program's output and errors go to a new one, as a daemon opens its log
    /// again. Only while its output is that file, as launchd started it.
    pub fn bound(log: &Path) -> Result<()> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;

        const LOG: u64 = 10 << 20;
        let Ok(metadata) = fs::metadata(log) else { return Ok(()) };
        let mut out: libc::stat = unsafe { std::mem::zeroed() };
        let writing = unsafe { libc::fstat(1, &mut out) } == 0
            && (out.st_dev as u64, out.st_ino as u64) == (metadata.dev(), metadata.ino());
        if metadata.len() < LOG || !writing {
            return Ok(());
        }
        fs::rename(log, log.with_extension("log.1"))?;
        let new = fs::OpenOptions::new().create(true).append(true).open(log)?;
        for written in [1, 2] {
            ensure!(unsafe { libc::dup2(new.as_raw_fd(), written) } >= 0, "couldn't write a new {}", log.display());
        }
        Ok(())
    }

    /// Tells `start` the server is running. Nothing hears it when launchd started it by itself, as at a login.
    pub fn ready(_key: &str) -> Result<()> {
        let Some(socket) = env::var_os("NOTIFY_SOCKET") else { return Ok(()) };
        let _ = UnixDatagram::unbound()?.send_to(b"READY=1", socket);
        Ok(())
    }

    fn launchctl(args: &[&str]) -> Result<()> {
        let done = std::process::Command::new("launchctl").args(args).output()?;
        ensure!(done.status.success(), "launchd didn't: {}", String::from_utf8_lossy(&done.stderr).trim());
        Ok(())
    }
}

/// A scheduled task of its person's on Windows, which needs no administrator, as a program someone installs for
/// themselves keeps running (Syncthing's way): a Windows service would need one, and would run apart from them.
/// Task Scheduler starts it as they log on, in their session and with their environment, and restarts one that
/// fails, 10 times a minute apart. Its restarting is only for a task that couldn't start, never for a program that
/// ended non-zero, so it's also started each minute it isn't running. `one` asks for no console window as it's
/// started there (`clients/app/build.rs`). What it says is `server.log` in the server's folder.
#[cfg(windows)]
mod system {
    use std::os::windows::io::AsRawHandle;
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    use super::*;

    fn name(key: &str) -> String {
        format!("The One App server {}", &key[..12])
    }

    /// Where `start` hears that the server is running. A named pipe, as a task runs in its person's session, which
    /// isn't always the one `serve on` is run in.
    fn ready_pipe(key: &str) -> String {
        format!(r"\\.\pipe\one-server-{}-ready", &key[..12])
    }

    /// Registers the task. A task has no environment of its own, so the server's is in its arguments, which
    /// `one serve run` takes.
    pub fn install(here: &Here, key: &str, program: &Path, env: &[(&str, String)]) -> Result<()> {
        let user = format!("{}\\{}", env::var("USERDOMAIN")?, env::var("USERNAME")?);
        let text = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let arguments: String = env.iter().map(|(name, value)| format!(" \"{name}={value}\"")).collect();
        let task = format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>The One App's server, which `one serve off` removes.</Description></RegistrationInfo>
  <Triggers>
    <LogonTrigger><UserId>{user}</UserId></LogonTrigger>
    <TimeTrigger><StartBoundary>2000-01-01T00:00:00</StartBoundary><Repetition><Interval>PT1M</Interval></Repetition></TimeTrigger>
  </Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure><Interval>PT1M</Interval><Count>10</Count></RestartOnFailure>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author"><Exec><Command>{}</Command><Arguments>serve run{}</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions>
</Task>
"#,
            text(&program.display().to_string()),
            text(&arguments),
            text(&here.folder.display().to_string()),
        );
        // Task Scheduler reads a task's XML as UTF-16.
        let file = here.folder.join("task.xml");
        let bytes: Vec<u8> = [0xFEFF].into_iter().chain(task.encode_utf16()).flat_map(u16::to_le_bytes).collect();
        fs::write(&file, bytes)?;
        let registered = schtasks(&["/create", "/tn", &name(key), "/xml", &file.display().to_string(), "/f"]);
        fs::remove_file(&file)?;
        registered
    }

    /// Has Task Scheduler start it now. That returns once the server answers by its key (`ready`).
    pub async fn start(here: &Here, key: &str) -> Result<()> {
        let _ = stop(here, key);
        let ready =
            tokio::net::windows::named_pipe::ServerOptions::new().first_pipe_instance(true).create(ready_pipe(key))?;
        schtasks(&["/run", "/tn", &name(key)])?;
        let log = here.folder.join("server.log");
        timeout(2 * START, ready.connect())
            .await
            .ok()
            .and_then(Result::ok)
            .with_context(|| format!("The server didn't start: its log is {}", log.display()))
    }

    pub fn stop(_here: &Here, key: &str) -> Result<()> {
        schtasks(&["/end", "/tn", &name(key)])
    }

    pub fn remove(here: &Here, key: &str) -> Result<()> {
        let _ = stop(here, key);
        schtasks(&["/delete", "/tn", &name(key), "/f"])
    }

    /// Tells `start` the server is running. Nothing hears it when Task Scheduler started it by itself.
    pub fn ready(key: &str) -> Result<()> {
        if let Ok(mut pipe) = fs::OpenOptions::new().write(true).open(ready_pipe(key)) {
            let _ = std::io::Write::write_all(&mut pipe, b"READY=1");
        }
        Ok(())
    }

    /// The log this program writes, which its output and errors are, as a task's go nowhere.
    static LOG: Mutex<Option<fs::File>> = Mutex::new(None);

    /// Has this program's output and errors go to `server.log`, kept under 10 MB: past it, the log is
    /// `server.log.1`, in place of the one before, and a new one is begun.
    pub fn bound(log: &Path) -> Result<()> {
        let mut writing = LOG.lock().unwrap();
        let full = fs::metadata(log).is_ok_and(|metadata| metadata.len() >= 10 << 20);
        if writing.is_some() && !full {
            return Ok(());
        }
        if full {
            fs::rename(log, log.with_extension("log.1"))?;
        }
        let new = fs::OpenOptions::new().create(true).append(true).open(log)?;
        for written in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            ensure!(
                unsafe { SetStdHandle(written, new.as_raw_handle() as HANDLE) } != 0,
                "couldn't write {}",
                log.display()
            );
        }
        *writing = Some(new);
        Ok(())
    }

    /// A job object holding the server and whatever it starts, which are all stopped once it's let go of: when the
    /// server is stopped or this program ends, however it ends. A release's script, which the server is, starts its VM
    /// once it has read two batch files, long after this. On Linux and a Mac they go with it already: systemd stops a
    /// unit's every process, launchd a job's process group, and the server stops when its input closes.
    pub struct Contained(HANDLE);

    impl Drop for Contained {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn contain(server: &Child) -> Result<Contained> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            ensure!(!job.is_null(), "couldn't make a job object: {}", std::io::Error::last_os_error());
            let job = Contained(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = std::mem::size_of_val(&limits) as u32;
            let limited =
                SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, &raw const limits as _, size);
            let process = server.raw_handle().context("the server has no process")?;
            ensure!(
                limited != 0 && AssignProcessToJobObject(job.0, process as HANDLE) != 0,
                "couldn't hold the server in a job object: {}",
                std::io::Error::last_os_error()
            );
            Ok(job)
        }
    }

    fn schtasks(args: &[&str]) -> Result<()> {
        let done = command("schtasks").args(args).output()?;
        ensure!(done.status.success(), "Task Scheduler didn't: {}", String::from_utf8_lossy(&done.stderr).trim());
        Ok(())
    }
}

/// A phone runs no server. Services on phones stay open for later.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
#[rustfmt::skip]
mod system {
    use super::*;

    const NONE: &str = "A server runs on Linux, macOS and Windows, and this device is none of them.";
    pub fn install(_: &Here, _: &str, _: &Path, _: &[(&str, String)]) -> Result<()> { bail!(NONE) }
    pub async fn start(_: &Here, _: &str) -> Result<()> { bail!(NONE) }
    pub fn stop(_: &Here, _: &str) -> Result<()> { bail!(NONE) }
    pub fn remove(_: &Here, _: &str) -> Result<()> { bail!(NONE) }
    pub fn ready(_: &str) -> Result<()> { bail!(NONE) }
    pub fn bound(_: &Path) -> Result<()> { bail!(NONE) }
}
