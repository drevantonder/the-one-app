//! This device as one of its network's servers. Three state machines, each the one place its state changes:
//!
//! - `server_membership` (`ServerMembership`), whether it's a server: inactive, then joining (`one serve on`:
//!   registering, downloading, installing, starting) until it answers by its key, then active, then leaving
//!   (`one serve off`: draining, deregistering, uninstalling), then inactive again. Either that fails goes back to
//!   where it started, undoing what it did, and says why, as does either that nothing is doing any more, as one the
//!   machine went off during, whichever of this program's starts on the device comes first (`settle`).
//! - `server_daemon_status`, the program the system keeps running (`one serve run`): starting until the server answers
//!   by its key, running, stopping, stopped. Restarts and their back-off are the system's: a systemd user unit on
//!   Linux, a LaunchAgent on a Mac, a scheduled task on Windows (`system`). This program only ends, non-zero where it
//!   failed.
//! - `server_update_status` (`ServerUpdateStatus`), while it's active: up to date, downloading the latest server
//!   build and `one` together and checking the build starts (`prepare`), checked, verifying the pair once it's told
//!   to go (`go`), which is starting both in the server's place, as every update restarts it, and up to date again,
//!   kept or gone back. Its network's updater (`core/edge/updater/updater.ts`) decides when, one server at a time.
//!   A build is started only by the `one` the record pairs it with.
//!
//! ```text
//! server_membership:
//!   inactive ─serve on─► joining (registering → downloading → installing → starting) ─answers─► active
//!                          └─fails, or cut short─► inactive, off the list, nothing left (undo_joining)
//!   active ─serve off─► leaving (draining → deregistering → uninstalling) ─► inactive, nothing left
//!                          └─fails, or cut short─► active, back on the list, run by the system (undo_leaving)
//!   cut short: settled by this program's first start on the device, unit or not (`settle`): `serve run`,
//!   `serve on`, `serve off` (which then leaves again), `update`, the app; joining at starting is `serve run`'s
//!
//! server_update_status:
//!   up to date ─prepare─► downloading (the build, and `one` beside this one, as the server lists them) ─► checking, in
//!     one VM of the build
//!     ─► checked ─go─► verifying: starting (the new `one` put in place) ─said at the edge─► up to date on the new pair
//!                                                 └─doesn't─► up to date, went back
//!     nothing newer, or a download or check that failed ─► up to date, staying on it, saying why
//!   cut short: downloading ─► up to date · checked stays checked · starting ─► started again
//!   every start of `serve run`: the `one` the record pairs with the build is put in place and started first
//!   said at the edge: the build started says it's running at the network's edge, by its key. Nothing else is asked,
//!     so no other server, or an app it runs, has a say: every server can take a build at once
//! ```
//!
//! What it does is reported for Telemetry as it happens (`Reports`), kept on the device until a server is told, so a
//! server that won't start is reported by its machine all the same: each start, with how long it took and why the
//! run before ended (`one.server.start`, an error where it didn't start); each stop, an error where it wasn't asked
//! (`one.server.stop`); ending unasked `LOOP_ENDS` times within `LOOP` (`one.server.crash_loop`); and each update,
//! how it was taken, how long it took, and whether it was kept, went back or stayed, and why (`one.server.update`).
//!
//! A server that missed a release, as one asleep or away during it, is caught up by its network's update once it's
//! back, which it hears of as the server says it's running, or can reach the network again, at the network's edge.
//!
//! Every start is the one way: a build is started and has `START` to say it's running at the network's edge. What
//! each machine is at is one small record, `state.json` in the client's home's `server/`, which every program changes
//! in one step under one lock (`Here::change`), beside `builds/<build>`, each build kept, `runtimes/<build>`, each
//! runtime kept, `programs/<program>/<build>`, each `one` a pair names, kept by which it is (`one` or `one-cli`), so one
//! of the other put in place by a person, as installing the app over `one-cli` does, runs the server from then on
//! rather than being put back, and `home`,
//! the server's own folder, its key among it. A build is put together from its runtime, as hard links, and its code
//! (`Here::assemble`), so it's a whole Erlang release that any start starts. The server is told on its input what
//! changes of it (`OneCore.Release`), and says it to the network, where it's ready once it's active and running. What
//! the two say to each other is `client::serve`'s.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::timeout;

use client::serve::{
    Said, ServerDaemonStatus, ServerMembership, ServerUpdateStage, ServerUpdateStatus, Status, Told,
};
use client::{BUILD, Client, target};

/// Which build of `one` an update takes the latest of: the app, with its window, or `one` built without it.
const PROGRAM: &str = if cfg!(feature = "window") { "one" } else { "one-cli" };
/// The public half of the key the platform signs each build it publishes with, BUILD_SIGNING_KEY in `tooling/release`
/// (in 1Password, "The One App build signing key").
const KEY: &str = "3fd5f3516020dafc3690044ac56dceef578b1f162b72ec4687af2d764192d2f2";
/// Where the core's latest builds are, for a new network's first server, which has no network to download from yet:
/// a list for each kind of machine, `<target>.json`, as the network's `download` gives it, which `tooling/release`
/// keeps as the network's latest.
const RELEASES: &str = "https://github.com/ptc-au/the-one-app/releases/download/builds";
/// How long every start of a build has to say it's running at the network's edge.
const START: Duration = Duration::from_secs(180);
/// How long a server is given to stop: longer than it takes to answer what it took (`OneCore.Application`).
const STOP: Duration = Duration::from_secs(20);
/// How many of the last lines a build said are kept to say why it didn't start.
const LAST: usize = 5;
/// A server that ends without being asked `LOOP_ENDS` times within `LOOP` is in a crash loop.
const LOOP: Duration = Duration::from_secs(600);
const LOOP_ENDS: usize = 3;
/// How long what's reported has to reach a server as this program ends, before it's kept for next time.
const TELLING: Duration = Duration::from_secs(5);

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

/// A server's build and the `one` that runs it, which an update takes, keeps and goes back from together.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Pair {
    build: String,
    program: String,
}

/// What this device is as a server, written in one step (`Here::write`).
#[derive(Serialize, Deserialize, Default, Debug)]
struct Record {
    server_membership: ServerMembership,
    server_membership_stage: Option<ServerMembershipStage>,
    server_update_status: ServerUpdateStatus,
    server_update_stage: Option<ServerUpdateStage>,
    current: Option<Pair>,
    previous: Option<Pair>,
    next: Option<Pair>,
    /// Why the last update didn't take, which the server says until one does.
    went_back: Option<String>,
    /// Why the server's last run ended, as this program saw it, which its next start reports.
    ended: Option<String>,
    /// When it ended without being asked within the last `LOOP`, in milliseconds since 1970.
    #[serde(default)]
    unasked: Vec<u64>,
    /// When the update being taken by restarting was told to go, in milliseconds since 1970.
    update_began: Option<u64>,
    /// Whom the server is for (`ONE_SUBJECT`), as it was joined and says as it starts, so `one serve on` registers it
    /// for them again whoever runs it.
    #[serde(default)]
    subject: Option<String>,
}

impl Record {
    /// What's told the server of itself, as its daemon is `daemon`: on its input, and as any build of it is started.
    fn told(&self, daemon: ServerDaemonStatus) -> Told {
        Told::Status(Status {
            server_membership: self.server_membership,
            server_daemon_status: daemon,
            server_update_status: self.server_update_status,
            server_update_stage: self.server_update_stage,
            next_build: self.next.as_ref().map(|next| next.build.clone()),
            program_name: Some(name()),
            program: self.pair().map(|pair| pair.program),
            next_program: self.next.as_ref().map(|next| next.program.clone()),
            went_back: self.went_back.clone(),
        })
    }

    /// The pair a start of the server runs: the one being verified, or the one it's on.
    fn pair(&self) -> Option<Pair> {
        if self.server_update_status == ServerUpdateStatus::Verifying {
            self.next.clone()
        } else {
            self.current.clone()
        }
    }

    /// Goes back from the pair being verified, which didn't take for `why`, to the one it's on, saying so until an
    /// update takes. The `one` paired with it is put in place as it's started (`run`).
    fn go_back(&mut self, why: impl std::fmt::Display) {
        let build = |pair: &Option<Pair>| pair.as_ref().map(|pair| pair.build.clone()).unwrap_or_default();
        self.went(format!("Went back to {} from {}: {why}", build(&self.current), build(&self.next)));
        self.next = None;
        self.server_update_status = ServerUpdateStatus::UpToDate;
        self.server_update_stage = None;
    }

    /// Says why the last update didn't take, which the server says until one does.
    fn went(&mut self, why: String) {
        println!("{why}");
        self.went_back = Some(why);
    }

    /// Keeps the pair being verified, `next`, which took: the server is on it, up to date.
    fn took(&mut self, next: Pair) {
        self.previous = self.current.replace(next);
        self.next = None;
        self.went_back = None;
        self.server_update_status = ServerUpdateStatus::UpToDate;
        self.server_update_stage = None;
    }

    /// The server ended without being asked to, for `why`, which its next start reports. Gives how many times it has
    /// within `LOOP`.
    fn ended_unasked(&mut self, why: &str) -> usize {
        let now = now();
        self.unasked.retain(|at| now.saturating_sub(*at) < LOOP.as_millis() as u64);
        self.unasked.push(now);
        self.ended = Some(why.to_owned());
        self.unasked.len()
    }
}

/// What this program reports of its server for Telemetry (`Client::report`), each record saying which server.
struct Reports {
    client: Arc<Client>,
    server: String,
}

impl Reports {
    fn report(&self, mut record: Value) {
        record["attributes"]["one.server"] = json!(self.server);
        self.client.report(record);
    }

    /// The server ended without being asked to, `ends` times within `LOOP`: a stop that failed, and a crash loop once
    /// it's `LOOP_ENDS`.
    fn ended_unasked(&self, record: Value, ends: usize, why: &str) {
        self.report(record);
        if ends >= LOOP_ENDS {
            let minutes = LOOP.as_secs() / 60;
            self.report(json!({"name": "one.server.crash_loop", "error": format!("It ended {ends} times in {minutes} minutes without being asked. The last: {why}")}));
        }
    }

    /// An update, as it ended: from which build to which, how long it took, and `went_back` or `stayed` with why where
    /// it wasn't kept.
    fn update(&self, from: &str, to: &str, took: Option<Duration>, error: Option<String>, outcome: &str) {
        let mut record = json!({"name": "one.server.update", "attributes": {
            "one.update.from": from, "one.update.to": to, "one.update.outcome": outcome,
        }});
        if let Some(took) = took {
            record["took"] = json!(took.as_secs_f64() * 1000.0);
        }
        if let Some(error) = error {
            record["error"] = json!(error);
        }
        self.report(record);
    }

    /// Tells a server now what's reported: asking any server anything has the client tell it what's kept, and what
    /// no server answers stays kept on the device for next time. Ending, it waits for that, within `TELLING`.
    async fn tell(&self, ending: bool) {
        let _ = timeout(TELLING, self.client.request("ping", json!({}))).await;
        if ending {
            let _ = timeout(TELLING, self.client.close()).await;
        }
    }
}

/// Now, in milliseconds since 1970.
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// How long since `at`, in milliseconds since 1970.
fn since(at: u64) -> Duration {
    Duration::from_millis(now().saturating_sub(at))
}

/// Makes this device a server of whoever it's for: its person, or the group or network `ONE_SUBJECT` names. It's put
/// on their list, its build downloaded and checked, the system set to run it, and it's started: active once it
/// answers by its key. One that fails is taken back off, and nothing of it is left.
pub async fn on(client: &Client) -> Result<String> {
    let address = client.address().context("A server is for a network with an address, and this one has none.")?;
    let here = Here::of(&client::home()?);
    fs::create_dir_all(&here.folder)?;
    let lock = here.lock()?.context("This device is already becoming a server, or stopping being one.")?;
    let key = here.key()?;
    let who = who(client)?;
    let record = here.read()?;
    match record.server_membership {
        ServerMembership::Active => {
            let who = record.subject.context(
                "This device's server hasn't said whom it's for yet, which it does as it starts on this build: \
                 `one serve on` registers it again once it has.",
            )?;
            let registered = registered(&here, &key, &who).await?;
            return Ok(format!("This device is a server for {who} already, and {registered}. Its key: {key}"));
        }
        ServerMembership::Leaving => {
            undo_leaving(client, &here, &key, true).await?;
            return Ok(format!("This device is a server for {who} again. Its key: {key}"));
        }
        ServerMembership::Inactive | ServerMembership::Joining => {}
    }
    if let Err(error) = join(client, &here, &key, &who).await {
        drop(lock);
        let left = undo_joining(client, &here, &key).await;
        return Err(error.context(format!("This device isn't a server{left}")));
    }
    Ok(format!("This device is a server for {who} now, and starts whenever it does. Its key: {key}\n{address}"))
}

/// Writes again how the system runs this device's server, which is one already, for `who`, as this build says to: what
/// `one serve run` does as it starts, for a server whose registration it couldn't rewrite itself, as one an
/// administrator made. It isn't started: on Windows a task registered again doesn't hold the server that runs, which
/// starting it would run twice. Says whether it changed.
async fn registered(here: &Here, key: &str, who: &str) -> Result<&'static str> {
    let server = kept(here, key, &program()?, unit_env(who.to_owned(), None).await?);
    let changed = client::system::install(&server)
        .map_err(|error| anyhow::anyhow!("{error:#}\nThen `one serve on` again, as yourself, registers it as yours."))?;
    Ok(if changed {
        "the system runs it as this build says to from its next start (its registration changed)"
    } else {
        "the system runs it as this build says to (its registration is unchanged)"
    })
}

async fn join(client: &Client, here: &Here, key: &str, who: &str) -> Result<()> {
    let joining = |stage| {
        move |record: &mut Record| {
            record.server_membership = ServerMembership::Joining;
            record.server_membership_stage = Some(stage);
        }
    };
    let record = here.change(|record| {
        joining(ServerMembershipStage::Registering)(record);
        record.subject = Some(who.to_owned());
    })?;
    listed(client, here, "add_server", key).await?;
    here.change(joining(ServerMembershipStage::Downloading))?;
    if record.current.is_none() {
        let builds = download(client).await?;
        let build = here.take(&builds, &record.told(ServerDaemonStatus::Starting), None).await?;
        here.change(|record| record.current = Some(Pair { build, program: BUILD.to_owned() }))?;
    }
    here.change(joining(ServerMembershipStage::Installing))?;
    let server = kept(here, key, &program()?, unit_env(who.to_owned(), None).await?);
    client::system::install(&server)?;
    here.change(joining(ServerMembershipStage::Starting))?;
    // The system starts `one serve run`, which makes it active once it answers by its key. Where it didn't, its log's
    // last lines say why, as undoing the join removes its folder, and a Mac's or Windows' log with it.
    if let Err(error) = system::start(here, &server).await {
        let log = fs::read_to_string(here.folder.join("server.log")).unwrap_or_default();
        let last: Vec<&str> = log.lines().rev().take(LAST).collect();
        ensure!(last.is_empty(), "{error:#}. Its last lines: {}", last.into_iter().rev().collect::<Vec<_>>().join(" | "));
        return Err(error);
    }
    ensure!(here.read()?.server_membership == ServerMembership::Active, "It started, and didn't answer by its key");
    Ok(())
}

/// Takes back what joining did, from wherever it got to: the system no longer runs it, it's off its owner's list,
/// and its folder goes. Gives all of that couldn't be done, to say with why joining failed.
async fn undo_joining(client: &Client, here: &Here, key: &str) -> String {
    let mut left = Vec::new();
    if let Err(why) = listed(client, here, "remove_server", key).await {
        left.push(format!("it's still on the list of servers: {why:#}"));
    }
    said_left(left.into_iter().chain(removed(here, key)).collect())
}

/// Takes what joining put on this device away: the system no longer runs it, and its folder goes. Gives what of
/// that couldn't be done.
fn removed(here: &Here, key: &str) -> Vec<String> {
    let mut left = Vec::new();
    // Joining that stopped before the system had it has nothing there to remove.
    match client::system::installed(SERVER, key) {
        Ok(true) => left.extend(client::system::remove(SERVER, key).err().map(|why| format!("the system still has it: {why:#}"))),
        Ok(false) => {}
        Err(why) => left.push(format!("the system may still have it: {why:#}")),
    }
    match fs::remove_dir_all(&here.folder) {
        Err(why) if why.kind() != std::io::ErrorKind::NotFound => {
            left.push(format!("its folder {} is still here: {why}", here.folder.display()))
        }
        _ => {}
    }
    left
}

/// What couldn't be undone, as said after why something failed.
fn said_left(left: Vec<String>) -> String {
    if left.is_empty() { String::new() } else { format!(", though {}", left.join("; ")) }
}

/// Makes this device the first server of a new network, at `address`, whose edge was deployed with the secret
/// `first` (`core/edge/README.md`), with the user `owner` its first owner. There's no server yet to sign in to or to
/// download from, so this device isn't signed in, its build is the core's latest release (`RELEASES`), and the server
/// says the secret as it first starts, which writes the network's first list of servers, and then makes `owner` its
/// first owner, saying where they join (`OneCore.Apps.Lifecycle.found/1`). Otherwise it's a server as `on` makes one,
/// the network's, run by the system: its device signs in once its owner has joined, as any does.
pub async fn first(address: &str, first: &str, owner: &str) -> Result<String> {
    let home = client::home()?;
    let network = home.join("network");
    ensure!(!network.exists(), "This device is in a network already, and a new network's first server is in none.");
    fs::create_dir_all(&home)?;
    fs::write(&network, json!({"address": address}).to_string())?;
    let here = Here::of(&home);
    fs::create_dir_all(&here.folder)?;
    let lock = here.lock()?.context("This device is already becoming a server, or stopping being one.")?;
    let key = here.key()?;
    let joining = |stage| {
        move |record: &mut Record| {
            record.server_membership = ServerMembership::Joining;
            record.server_membership_stage = Some(stage);
        }
    };
    let joined = async {
        let record = here.change(|record| {
            joining(ServerMembershipStage::Downloading)(record);
            record.subject = Some("network".to_owned());
        })?;
        let url = format!("{RELEASES}/{}.json", target());
        let builds: Map<String, Value> = client::web()?.get(&url).send().await?.error_for_status()?.json().await?;
        let build = here.take(&builds, &record.told(ServerDaemonStatus::Starting), None).await?;
        here.change(|record| record.current = Some(Pair { build, program: BUILD.to_owned() }))?;
        here.change(joining(ServerMembershipStage::Installing))?;
        // Said only to the first start: `serve run` writes the system's registration again without them.
        let mut env = unit_env("network".to_owned(), None).await?;
        env.extend([("ONE_FIRST", first.to_owned()), ("ONE_FIRST_OWNER", owner.to_owned())]);
        let server = kept(&here, &key, &program()?, env);
        client::system::install(&server)?;
        here.change(joining(ServerMembershipStage::Starting))?;
        system::start(&here, &server).await?;
        ensure!(here.read()?.server_membership == ServerMembership::Active, "It started, and didn't answer by its key");
        anyhow::Ok(())
    }
    .await;
    if let Err(error) = joined {
        drop(lock);
        let mut left = removed(&here, &key);
        left.extend(fs::remove_file(&network).err().map(|why| format!("{} is still here: {why}", network.display())));
        return Err(error.context(format!("This device isn't the network's first server{}", said_left(left))));
    }
    Ok(format!(
        "This device is the first server of {address} now, and starts whenever it does. Its key: {key}\n\
         It deploys the core's apps for the network and makes users/{owner} its first owner, which takes a few \
         minutes, and then says in its log, {}, the link they join at (`grep 'joins at'`), shown once.",
        system::log(&here, &key)
    ))
}

/// Joining or leaving that nothing is doing any more, as one the machine went off during, goes back to where it
/// started, whether or not the system still has `one serve run` to start: it's settled by whichever of this
/// program's starts on this device comes first, `one serve run` as the system starts it, `one serve on`, `one serve
/// off` and `one update`, and the app as it opens. Joining that got to starting, where the system has it, is that start
/// of `one serve run`'s to decide.
pub async fn settle(client: &Client) -> Result<()> {
    settled(client, &Here::of(&client::home()?), true).await.map(|_| ())
}

/// Settles the move on this device that nothing is doing, if there's one, `start`ing the server where a leave goes
/// back, unless this is the server's own start.
async fn settled(client: &Client, here: &Here, start: bool) -> Result<Record> {
    let record = here.read()?;
    if !matches!(record.server_membership, ServerMembership::Joining | ServerMembership::Leaving) {
        return Ok(record);
    }
    let Some(_lock) = here.lock()? else { return Ok(record) };
    let record = here.read()?;
    let key = here.key()?;
    match (record.server_membership, record.server_membership_stage) {
        (ServerMembership::Joining, Some(ServerMembershipStage::Starting)) if client::system::installed(SERVER, &key)? => {}
        (ServerMembership::Joining, _) => {
            println!("Joining didn't finish, so this device isn't a server{}.", undo_joining(client, here, &key).await)
        }
        _ => undo_leaving(client, here, &key, start).await?,
    }
    here.read()
}
/// Who the server is for: whoever `ONE_SUBJECT` names, or this device's person.
fn who(client: &Client) -> Result<String> {
    env::var("ONE_SUBJECT").or_else(|_| client.who().context("This device isn't signed in yet. Sign in on it first."))
}

/// Who this device's server is for once it has begun joining: whom it joined for, as its record says, whoever runs
/// this program now, as its person's own `one serve off` of the network's server.
fn whose(client: &Client, here: &Here) -> Result<String> {
    here.read()?.subject.map_or_else(|| who(client), Ok)
}

/// Puts this device's server on the list of whoever it's for (`add_server`), or takes it off (`remove_server`), asked
/// for them (`whose`), not for whoever runs this.
async fn listed(client: &Client, here: &Here, name: &str, key: &str) -> Result<String> {
    let request = json!({"name": name, "arguments": {"key": key}, "_meta": {"subject": whose(client, here)?}});
    crate::said(client.request("tools/call", request).await?)
}

/// The environment the system runs the server with: the client it updates by, who it's for, and where programs
/// are found, `path` where it's known, as the system gave it to a server that's running, and otherwise its person's
/// login PATH. Windows starts a task with its person's environment as they log in, PATH and all.
async fn unit_env(who: String, #[cfg_attr(not(unix), expect(unused_variables))] path: Option<String>) -> Result<Vec<(&'static str, String)>> {
    let mut env = vec![("ONE_HOME", client::home()?.display().to_string()), ("ONE_SUBJECT", who)];
    #[cfg(unix)]
    env.push((
        "PATH",
        match path {
            Some(path) => path,
            None => login_path().await?,
        },
    ));
    // A network being built has a command say who it acts as, and so does what runs here for it.
    if let Ok(name) = env::var("ONE_AS") {
        env.push(("ONE_AS", name));
    }
    Ok(env)
}

/// The server as the system keeps it running (`client::system`): `one serve run`, with its environment `env`, which is
/// told on a Mac where to say it's running (`system::start`), and given on Windows as its arguments, which `main`
/// takes, as a task has no environment of its own. It has `START` twice over to start: a pair being verified, then the
/// one before it. Only `one serve run` is told to stop, and has the server drain first.
fn kept(here: &Here, key: &str, program: &Path, env: Vec<(&str, String)>) -> client::system::Program {
    #[allow(unused_mut)]
    let mut env: Vec<(String, String)> = env.into_iter().map(|(name, value)| (name.to_owned(), value)).collect();
    #[allow(unused_mut)]
    let mut command = vec![program.display().to_string(), "serve".to_owned(), "run".to_owned()];
    #[cfg(target_os = "macos")]
    env.push(("NOTIFY_SOCKET".to_owned(), here.folder.join("ready").display().to_string()));
    #[cfg(windows)]
    command.extend(env.drain(..).map(|(name, value)| format!("{name}={value}")));
    client::system::Program {
        name: SERVER.to_owned(),
        key: key.to_owned(),
        description: "the One App's server, which `one serve off` removes.".to_owned(),
        command,
        env,
        folder: here.folder.clone(),
        desktop: false,
        start_within: Some(2 * START + Duration::from_secs(30)),
        stop_within: STOP + Duration::from_secs(10),
    }
}

/// What the system knows this device's server by, with its key (`client::system::name`).
const SERVER: &str = "server";

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
    let key = here.key()?;
    match here.read()?.server_membership {
        ServerMembership::Joining => {
            drop(lock);
            let left = undo_joining(client, &here, &key).await;
            return Ok(format!("This device stopped becoming a server{left}"));
        }
        // A leave that was cut short goes back first, so every leave begins where it can be undone from: active, on
        // the list and run by the system.
        ServerMembership::Leaving => undo_leaving(client, &here, &key, true).await?,
        ServerMembership::Active | ServerMembership::Inactive => {}
    }
    if let Err(error) = leave(client, &here, &key, lock).await {
        undo_leaving(client, &here, &key, true).await?;
        return Err(error.context("It's still a server"));
    }
    Ok(format!("This device is no longer a server, and nothing of it is left here: {key}"))
}

async fn leave(client: &Client, here: &Here, key: &str, lock: fs::File) -> Result<()> {
    let leaving = |stage| {
        move |record: &mut Record| {
            record.server_membership = ServerMembership::Leaving;
            record.server_membership_stage = Some(stage);
        }
    };
    // The system stops `one serve run`, which reads that it's leaving and has the server drain as it stops.
    here.change(leaving(ServerMembershipStage::Draining))?;
    client::system::stop(SERVER, key)?;
    here.change(leaving(ServerMembershipStage::Deregistering))?;
    listed(client, here, "remove_server", key).await?;
    here.change(leaving(ServerMembershipStage::Uninstalling))?;
    client::system::remove(SERVER, key)?;
    drop(lock);
    Ok(fs::remove_dir_all(&here.folder)?)
}

/// Takes back what leaving did, from wherever it got to: it's back on its owner's list, as taking it off may have
/// happened whatever the record says, and active, and the system runs it again unless it's what's running this.
async fn undo_leaving(client: &Client, here: &Here, key: &str, start: bool) -> Result<()> {
    listed(client, here, "add_server", key).await.context("It couldn't be put back on the list of servers")?;
    here.change(|record| {
        record.server_membership = ServerMembership::Active;
        record.server_membership_stage = None;
    })?;
    if start {
        let server = kept(here, key, &program()?, unit_env(whose(client, here)?, None).await?);
        client::system::install(&server)?;
        system::start(here, &server).await?;
    }
    Ok(())
}

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

/// Runs this device's server, as the system runs it: starts the build the record says, within `START`, keeps or goes
/// back from a pair being verified, and takes the latest builds when the server's asked. It ends when the server
/// stops, or when the system stops it, after the server has, and non-zero for the system to start it again. What it
/// reported is told before it ends, a failure most of all.
pub async fn run(client: Arc<Client>) -> Result<()> {
    // A server keeps a file open for each database it holds, each connection and each log, and launchd gives a program
    // it runs 256 (systemd 1024), which a server taking an update runs out of ("Too many open files", 19 Nov). So this
    // takes as many as the system lets it, as Go's runtime does as it starts, and the server it runs inherits them: a
    // limit in the LaunchAgent would be read only as it's loaded again, at the next login.
    #[cfg(unix)]
    rlimit::increase_nofile_limit(u64::MAX).context("couldn't raise how many files the server may open")?;
    let here = Here::of(&client::home()?);
    client.service("server");
    let reports = Reports { client: client.clone(), server: here.key()? };
    let ran = serve(&client, &here, &reports).await;
    reports.tell(true).await;
    ran
}

async fn serve(client: &Arc<Client>, here: &Here, reports: &Reports) -> Result<()> {
    // The program's own file, named before an update replaces what's there.
    let program = program()?;
    let key = here.key()?;
    let address = client.address().context("A server is for a network with an address, and this one has none.")?;
    let who = env::var("ONE_SUBJECT").context("ONE_SUBJECT says who the server is for")?;
    if settled(client, here, false).await?.server_membership == ServerMembership::Inactive {
        println!("This device isn't a server: `one serve on` makes it one.");
        return Ok(());
    }
    let log = here.folder.join("server.log");
    system::bound(&log)?;
    // The system runs it as this build says to, from its next start. Where it can't be rewritten, as one changed by
    // hand or an administrator's may not be, it runs as it was set up rather than not at all, and its log says why.
    if let Err(error) = client::system::install(&kept(here, &key, &program, unit_env(who.clone(), env::var("PATH").ok()).await?)) {
        eprintln!("Couldn't rewrite how the system runs it, so it runs as it was set up: {error:#}");
    }

    // A download cut short, as this program or the server stopped, is one that failed: it's up to date again on what
    // it was on. A build being checked when they stopped wasn't kept, and the server starts on the one it was on.
    let mut record = here.change(|record| {
        record.subject = Some(who.clone());
        if record.server_update_status == ServerUpdateStatus::Downloading {
            record.server_update_status = ServerUpdateStatus::UpToDate;
            record.server_update_stage = None;
            record.next = None;
        }
        if record.server_update_status == ServerUpdateStatus::Verifying
            && record.server_update_stage == Some(ServerUpdateStage::Checking)
        {
            record.go_back("it stopped while checking it");
        }
    })?;
    let verifying = record.server_update_status == ServerUpdateStatus::Verifying;
    let mut pair = record.pair().context("The record names no build to run")?;
    // A build is run by the `one` it's paired with, whichever the system started: one put in place for a pair being
    // verified, or the one before for a pair gone back from, has that one put in place, and started instead. Where
    // that one isn't kept, as where a person put another `one` in place by hand, the one they put there runs it.
    if pair.program != BUILD {
        if here.program(&pair.program).exists() {
            here.put_program(&pair.program, &program)?;
            return Err(again(here, &program));
        }
        println!("The one the record pairs with {}, {}, isn't kept here, so {BUILD} runs it.", pair.build, pair.program);
        pair.program = BUILD.to_owned();
        record = here.change(|record| {
            let paired = if verifying { &mut record.next } else { &mut record.current };
            *paired = Some(pair.clone());
        })?;
    }
    let mut server = Command::from(command(release(&here.started(&pair.build)?)));
    server
        .args(["eval", "OneCore.Release.serve()"])
        .env("ONE_SERVER_HOME", here.folder.join("home"))
        .env("ONE_ADDRESS", &address)
        .env("ONE_SUBJECT", &who)
        // The `one` that runs it, which its services ask to have the system keep a program running (`OneCore.System`).
        .env("ONE_PROGRAM", &program)
        .env("ONE_DEVICE", client.key())
        .env("ONE_NAME", client::name())
        .env("ONE_SERVER_STATUS", record.told(ServerDaemonStatus::Starting).to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Erlang's VM on Windows works local time out from its zone's rules itself, and misses daylight saving where it
    // spans the new year, as in Australia, so its log ran an hour behind. Told its zone by its IANA name, as TZ is on
    // Linux and a Mac, it finds no rules of its own by that name and asks Windows instead; Bun reads it as it is.
    #[cfg(windows)]
    if let Some(zone) = jiff::tz::TimeZone::try_system().ok().and_then(|zone| zone.iana_name().map(str::to_owned)) {
        server.env("TZ", zone);
    }
    let spawned = Instant::now();
    let mut server = match server.spawn() {
        Ok(server) => server,
        Err(error) => return didnt_start(here, reports, &pair, verifying, &program, spawned, format!("it couldn't be run: {error}")).await,
    };
    // Everything it starts goes when it does, or when this does.
    #[cfg(windows)]
    let _contained = system::contain(&server)?;
    let mut said = listen(&mut server, log)?;

    let mut last = VecDeque::with_capacity(LAST);
    let started = within("start and say it's running at the network's edge", async {
        let (mut running, mut reported) = (false, false);
        while let Some(heard) = said.recv().await {
            if last.len() == LAST {
                last.pop_front();
            }
            last.push_back(heard.line.clone());
            match heard.said {
                Some(Said::Running { .. }) => running = true,
                Some(Said::Reported { build }) if build == pair.build => reported = true,
                _ => {}
            }
            if running && reported {
                return Ok(());
            }
        }
        bail!("it stopped before it was running")
    });
    if let Err(why) = started.await {
        let _ = server.kill().await;
        let why = format!("{why:#}. Its last lines: {}", Vec::from(last).join(" | "));
        return didnt_start(here, reports, &pair, verifying, &program, spawned, why).await;
    }
    let mut restarted = None;
    let mut began = None;
    let from = record.current.as_ref().map(|current| current.build.clone()).unwrap_or_default();
    record = here.change(|record| {
        restarted = record.ended.take();
        began = record.update_began.take();
        if verifying {
            let was = record.current.as_ref().map(|was| was.build.clone()).unwrap_or_default();
            record.took(pair.clone());
            println!("The server is on the build {} now, from {was}.", pair.build);
        }
        if record.server_membership == ServerMembership::Joining {
            record.server_membership = ServerMembership::Active;
            record.server_membership_stage = None;
        }
    })?;
    here.prune(&record)?;
    tell(&mut server, &record.told(ServerDaemonStatus::Running)).await;
    system::ready(&key)?;
    // A start says why the run before ended: what this program saw, or nothing where it saw nothing, as when it was
    // killed or its machine lost power.
    let why = restarted.unwrap_or_else(|| "It ended without saying why: killed, or its machine lost power".to_owned());
    reports.report(json!({"name": "one.server.start", "took": spawned.elapsed().as_secs_f64() * 1000.0,
        "attributes": {"one.server.build": pair.build, "one.server.restart.reason": why}}));
    if verifying {
        reports.update(&from, &pair.build, began.map(since), None, "took");
    }
    reports.tell(false).await;

    let stopping = stopped();
    tokio::pin!(stopping);
    loop {
        tokio::select! {
            heard = said.recv() => match heard {
                Some(Heard { said: Some(Said::Prepare), .. }) => prepare(here, &mut record, &mut server, &mut said, &program, reports).await?,
                Some(Heard { said: Some(Said::Go), .. }) => go(here, &mut record, &mut server, &program).await?,
                Some(_) => {}
                None => {
                    let why = format!("The server stopped: {}", server.wait().await?);
                    let mut ends = 0;
                    here.change(|record| ends = record.ended_unasked(&why))?;
                    reports.ended_unasked(json!({"name": "one.server.stop", "error": why}), ends, &why);
                    bail!(why)
                }
            },
            // `one serve off` says it's leaving before it has the system stop this, and the server drains.
            _ = &mut stopping => {
                let why = "The system stopped it";
                here.change(|record| record.ended = Some(why.to_owned()))?;
                reports.report(json!({"name": "one.server.stop", "attributes": {"one.server.stop.reason": why}}));
                return stop(&mut server, &here.read()?.told(ServerDaemonStatus::Stopping)).await
            }
        }
    }
}

/// The server on `pair` didn't start, for `why`: it's reported, and a pair being verified is gone back from, and the
/// one before started in its place. One that isn't fails, for the system to start it again.
async fn didnt_start(
    here: &Here,
    reports: &Reports,
    pair: &Pair,
    verifying: bool,
    program: &Path,
    spawned: Instant,
    why: String,
) -> Result<()> {
    let failed = format!("The server on {} didn't start: {why}", pair.build);
    let mut ends = 0;
    let mut began = None;
    let was = here.change(|record| {
        ends = record.ended_unasked(&failed);
        began = record.update_began.take();
        if verifying {
            record.go_back(&why);
        }
    })?;
    let start = json!({"name": "one.server.start", "took": spawned.elapsed().as_secs_f64() * 1000.0, "error": failed,
        "attributes": {"one.server.build": pair.build}});
    reports.ended_unasked(start, ends, &failed);
    if verifying {
        let from = was.current.as_ref().map(|current| current.build.as_str()).unwrap_or_default();
        reports.update(from, &pair.build, began.map(since), Some(why), "went_back");
    }
    ensure!(verifying, failed);
    reports.tell(true).await;
    Err(again(here, program))
}

/// A line the server said, and the message it is, if it's one: only a line on its output is.
struct Heard {
    line: String,
    said: Option<Said>,
}

/// Reads what the server says, on its output and its errors, as it comes, whatever this is doing meanwhile: a
/// server whose output isn't read is held up once the pipe it writes to is full, 16 KB on a Mac, so each line it
/// logs holds up whatever logged it. Gives every line, to act on the messages (that it's running, that it's asked to
/// update) and to say why a build didn't start.
fn listen(server: &mut Child, log: PathBuf) -> Result<UnboundedReceiver<Heard>> {
    let (heard, said) = tokio::sync::mpsc::unbounded_channel();
    let mut errors = BufReader::new(server.stderr.take().context("the server's errors")?).lines();
    let heard_error = heard.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = errors.next_line().await {
            eprintln!("{line}");
            let _ = heard_error.send(Heard { line, said: None });
        }
    });
    let mut lines = BufReader::new(server.stdout.take().context("the server's output")?).lines();
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{line}");
            if let Err(error) = system::bound(&log) {
                eprintln!("Couldn't keep {} small: {error:#}", log.display());
            }
            let _ = heard.send(Heard { said: Said::read(&line), line });
        }
    });
    Ok(said)
}

/// What the server does `doing`, given `START` to do it in.
async fn within<T>(doing: &str, done: impl Future<Output = Result<T>>) -> Result<T> {
    timeout(START, done).await.unwrap_or_else(|_| bail!("it didn't {doing} within {} s", START.as_secs()))
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

/// Tells the server `told`, a line of JSON: what it is, which it says to the network, or to say the latest builds
/// (`OneCore.Release`). A server that has stopped is told nothing, and needs telling nothing.
async fn tell(server: &mut Child, told: &Told) {
    if let Some(input) = server.stdin.as_mut() {
        let _ = input.write_all(format!("{told}\n").as_bytes()).await;
        let _ = input.flush().await;
    }
}

/// Stops a server that's running, once it's told `told`, by closing its input: it takes nothing new and answers what
/// it took already (`OneCore.Application`), and is killed if it hasn't stopped within `STOP`.
async fn stop(server: &mut Child, told: &Told) -> Result<()> {
    tell(server, told).await;
    drop(server.stdin.take());
    if timeout(STOP, server.wait()).await.is_err() {
        server.kill().await?;
    }
    Ok(())
}

/// This program started again in its place, as the system started it, to run the pair the record says: on Windows,
/// which replaces no running program, by ending for Task Scheduler to start it again at once (`system::again`).
fn again(#[cfg_attr(unix, expect(unused_variables))] here: &Here, program: &Path) -> anyhow::Error {
    #[cfg(unix)]
    return std::os::unix::process::CommandExt::exec(std::process::Command::new(program).args(["serve", "run"])).into();
    #[cfg(not(unix))]
    {
        if let Ok(key) = here.key() {
            system::again(&key);
        }
        anyhow::anyhow!("It ended to run {}, which Task Scheduler starts as it ends.", program.display())
    }
}

/// Prepares the latest server build and `one` as a pair, while the server answers on: downloads them, checks the build
/// starts, and says it's checked, to be taken at once once it's told to (`go`). So everything slow is done before it's told to go. Nothing
/// newer, or a download or check that failed, and it's up to date again, on what it was on.
async fn prepare(
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
async fn prepared(
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
async fn go(here: &Here, record: &mut Record, server: &mut Child, program: &Path) -> Result<()> {
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
async fn latest_builds(server: &mut Child, said: &mut UnboundedReceiver<Heard>) -> Result<Map<String, Value>> {
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
async fn take(here: &Here, record: &Record, program: &Path, builds: &Map<String, Value>) -> Result<()> {
    let current = record.current.as_ref().context("a build it's on")?;
    let next = record.next.as_ref().context("a pair it takes")?;
    if next.build != current.build {
        here.take(builds, &record.told(ServerDaemonStatus::Running), Some(&current.build)).await?;
    }
    if next.program != current.program {
        // Both are kept beside the one in place, which they're put in place of only as the pair they're in is taken.
        fs::create_dir_all(here.folder.join("programs").join(PROGRAM))?;
        fs::copy(program.canonicalize()?, here.program(&current.program))?;
        fs::rename(fetched_program(&builds[PROGRAM], program).await?, here.program(&next.program))?;
    }
    Ok(())
}

/// The pair the network's latest `builds` make for a server on `current`, where it's another.
fn latest_pair(builds: &Map<String, Value>, current: &Pair) -> Option<Pair> {
    let next = Pair {
        build: latest(builds).unwrap_or(&current.build).to_owned(),
        program: builds.get(PROGRAM).and_then(|one| one["build"].as_str()).unwrap_or(&current.program).to_owned(),
    };
    (&next != current).then_some(next)
}

/// The latest server build among the network's `builds`: its code's, where there's a runtime for this kind of machine
/// to go with it.
fn latest(builds: &Map<String, Value>) -> Option<&str> {
    builds.get("server-runtime").and(builds.get("server-code")).and_then(|code| code["build"].as_str())
}

/// Downloads the build `one` of the program `program` beside it, and gives where, once it has said it's that build.
/// It's beside the file a link to it names, as on a Mac `~/.local/bin/one` is a link to the program in the One App's
/// bundle.
async fn fetched_program(one: &Value, program: &Path) -> Result<PathBuf> {
    let build = one["build"].as_str().context("a build says which it is")?;
    ensure!(count(build) >= count(BUILD), "The network's build of one, {build}, is older than this one, {BUILD}.");
    let packed = program.canonicalize()?.with_extension("new.gz");
    fetch(&name(), one, &packed).await?;
    let new = packed.with_extension("");
    let unpacked =
        std::io::copy(&mut flate2::read::GzDecoder::new(fs::File::open(&packed)?), &mut fs::File::create(&new)?);
    fs::remove_file(&packed)?;
    unpacked.with_context(|| format!("The build {build} of one didn't unpack."))?;
    #[cfg(unix)]
    fs::set_permissions(&new, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    if build_of(&new).as_deref() != Some(build) {
        fs::remove_file(&new)?;
        bail!("The build {build} of one doesn't start here.")
    }
    Ok(new)
}

/// The build the program `program` says it is (`one --version`), if it starts.
pub fn build_of(program: &Path) -> Option<String> {
    let output = command(program).arg("--version").output().ok()?;
    String::from_utf8_lossy(&output.stdout).split_whitespace().nth(1).map(str::to_owned)
}

/// Puts the program `new` in place of `program`, in one step. Windows replaces or removes no program that's running,
/// but lets it be renamed: the one before is put aside under a name of its own (`one.<ms>.old`), as one put aside
/// before may still be running (the app, or the `one serve run` that's putting this in place, started from it), and
/// those put aside before that nothing runs any more go.
fn put(new: &Path, program: &Path) -> Result<()> {
    let program = program.canonicalize()?;
    #[cfg(windows)]
    {
        let stem = program.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let folder = program.parent().context("a program is in a folder")?;
        for aside in fs::read_dir(folder)?.flatten().map(|entry| entry.path()) {
            let name = aside.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with(&format!("{stem}.")) && name.ends_with(".old") {
                let _ = fs::remove_file(&aside);
            }
        }
        fs::rename(&program, program.with_extension(format!("{}.old", now())))?;
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

/// Downloads the build of `name` the network gives, `build`, by its link into `file`, once it's checked it's the
/// platform's, and checks it's the size and SHA-256 it said.
async fn fetch(name: &str, build: &Value, file: &Path) -> Result<()> {
    signed(name, build)?;
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

/// Checks the network's `build` of `name` (`one/linux-x86_64`, `server-code`) is the platform's: its Ed25519 signature
/// of its name, build and SHA-256, checked with the public half built in here, `KEY`, as Tauri's updater and Sparkle
/// check theirs. So whoever publishes to the network, its owner too, can give this no program but the platform's, nor
/// one in another's place, and as each says its build, an older one is told by it.
fn signed(name: &str, build: &Value) -> Result<()> {
    let which = build["build"].as_str().context("a build says which it is")?;
    let said = format!("{name} {which} {}", build["sha256"].as_str().context("a build says its SHA-256")?);
    let signature = build["signature"].as_str().with_context(|| format!("The build {which} of {name} isn't signed."))?;
    let signature = data_encoding::BASE64.decode(signature.as_bytes()).ok().and_then(|sig| Signature::from_slice(&sig).ok());
    let key = data_encoding::HEXLOWER.decode(KEY.as_bytes()).ok().and_then(|key| VerifyingKey::try_from(key.as_slice()).ok());
    let signed = signature.is_some_and(|sig| key.expect("KEY is a public key").verify_strict(said.as_bytes(), &sig).is_ok());
    ensure!(signed, "The build {which} of {name} isn't signed by the platform.");
    Ok(())
}

/// The count of commits in a build's metadata (`tooling/version`), which only grows on main: `0.25.13+3670.9046435e40b3`
/// is 3670, as `OneCore.Builds.count/1` reads it.
pub fn count(build: &str) -> u64 {
    build.split_once('+').and_then(|(_, metadata)| metadata.split('.').next()?.parse().ok()).unwrap_or(0)
}

/// Calls one of the network's tools and gives what it said, or the error it said.
async fn tool(client: &Client, name: &str, arguments: Value) -> Result<String> {
    crate::said(client.request("tools/call", json!({"name": name, "arguments": arguments})).await?)
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

    /// What this device is as a server: inactive where it has no record.
    fn read(&self) -> Result<Record> {
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
    fn change(&self, change: impl FnOnce(&mut Record)) -> Result<Record> {
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
    async fn save(&self, server: &mut Child, change: impl FnOnce(&mut Record)) -> Result<Record> {
        let record = self.change(change)?;
        tell(server, &record.told(ServerDaemonStatus::Running)).await;
        Ok(record)
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

    /// Puts the build `build` of the program, kept beside it since an update took it or the one before (`take`), in
    /// place of `program`.
    fn put_program(&self, build: &str, program: &Path) -> Result<()> {
        let new = program.canonicalize()?.with_extension("new");
        fs::copy(self.program(build), &new)
            .with_context(|| format!("The build {build} of one, which the server's record pairs with its build, isn't kept here"))?;
        put(&new, program)
    }

    /// The kept `one` of the build `build`, of the kind this one is.
    fn program(&self, build: &str) -> PathBuf {
        self.folder.join("programs").join(PROGRAM).join(build)
    }

    /// The folder of the build `build`.
    fn build(&self, build: &str) -> PathBuf {
        self.folder.join("builds").join(build)
    }

    /// Where the server is started from: its build's folder, but on Windows `current`, the build's files hard linked
    /// there as it starts (`linked`), in place of the ones before. Windows Firewall keeps what its person allowed by
    /// the path a program was started by, and the VM that holds the server's socket is in its build's own folder, so
    /// each update would ask them again. Not a junction: Windows doesn't follow one a person made for a process that
    /// guards against them (Redirection Guard), as an ssh session's do.
    fn started(&self, build: &str) -> Result<PathBuf> {
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
    fn prune(&self, record: &Record) -> Result<()> {
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
    async fn take(&self, builds: &Map<String, Value>, told: &Told, on: Option<&str>) -> Result<String> {
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
    async fn runtime(&self, runtime: &Value) -> Result<PathBuf> {
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
async fn unpack(what: &str, build: &Value, folder: &Path) -> Result<()> {
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
fn linked(from: &Path, to: &Path) -> Result<()> {
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

/// What's the server's own of how the system keeps it running (`client::system`): on Linux, where systemd waits for it to
/// say it's running (`Type=notify`) and keeps what it says in its journal.
#[cfg(target_os = "linux")]
mod system {
    use super::*;

    /// Has systemd start it now, or again where it's running, with what it's run with now. That returns once the
    /// server answers by its key (`ready`), or failed to start.
    pub async fn start(_here: &Here, server: &client::system::Program) -> Result<()> {
        client::system::restart(server)
    }

    /// What the server says is in systemd's journal, which keeps it within its own bounds.
    pub fn bound(_log: &Path) -> Result<()> {
        Ok(())
    }

    /// Where to read what the server says: its unit's journal.
    pub fn log(_here: &Here, key: &str) -> String {
        format!("`journalctl --user -u {}`", client::system::name(SERVER, key))
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
}

/// What's the server's own of how the system keeps it running (`client::system`) on a Mac, where launchd has no limit
/// on how long a start takes, which is why every start has `START` here.
#[cfg(target_os = "macos")]
mod system {
    use std::os::unix::net::UnixDatagram;

    use super::*;

    /// Has launchd load the agent, which starts it, again where it's loaded. That returns once the server says it's
    /// running on a socket in its folder (`ready`), as its environment says.
    pub async fn start(here: &Here, server: &client::system::Program) -> Result<()> {
        let ready = here.folder.join("ready");
        let _ = fs::remove_file(&ready);
        let socket = UnixDatagram::bind(&ready)?;
        socket.set_read_timeout(Some(2 * START))?;
        client::system::restart(server)?;
        let running = socket.recv(&mut [0; 16]);
        fs::remove_file(&ready)?;
        running.with_context(|| {
            format!("The server didn't start: its log is {}", here.folder.join("server.log").display())
        })?;
        Ok(())
    }

    /// Where to read what the server says: `server.log` in its folder.
    pub fn log(here: &Here, _key: &str) -> String {
        here.folder.join("server.log").display().to_string()
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
}

/// What's the server's own of how the system keeps it running (`client::system`) on Windows, as its person's scheduled
/// task. `one` asks for no console window as it's started there (`clients/app/build.rs`). What it says is `server.log`
/// in the server's folder.
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

    /// Where `start` hears that the server is running. A named pipe, as a task runs in its person's session, which
    /// isn't always the one `serve on` is run in.
    fn ready_pipe(key: &str) -> String {
        format!(r"\\.\pipe\one-server-{}-ready", &key[..12])
    }

    /// Has Task Scheduler start it now. That returns once the server answers by its key (`ready`).
    pub async fn start(here: &Here, server: &client::system::Program) -> Result<()> {
        let ready = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .create(ready_pipe(&server.key))?;
        client::system::restart(server)?;
        let log = here.folder.join("server.log");
        timeout(2 * START, ready.connect())
            .await
            .ok()
            .and_then(Result::ok)
            .with_context(|| format!("The server didn't start: its log is {}", log.display()))
    }

    /// Has Task Scheduler start the task again as soon as this program has ended, as one ending to run a newer build
    /// does. A task that's running isn't started (`IgnoreNew`), and a program that ends is no failure to Task
    /// Scheduler, so otherwise only its trigger starts it, within a minute. So a PowerShell that outlives this waits
    /// for it to end and starts the task, as `Start-ScheduledTask` does; where that doesn't, the trigger still does.
    pub fn again(key: &str) {
        let script = format!(
            "Wait-Process -Id {} -ErrorAction SilentlyContinue; Start-ScheduledTask -TaskName '{}'",
            std::process::id(),
            client::system::name(SERVER, key)
        );
        let started = command("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(error) = started {
            eprintln!("Task Scheduler starts it within a minute, as it couldn't be had to at once: {error}");
        }
    }

    /// Tells `start` the server is running, by opening its pipe, which is all `start` waits for. Only to read it,
    /// which a pipe lets everyone do: one made in an ssh session lets nobody but administrators write to it, so
    /// writing to it from the task, its person's own, was refused. Nothing hears it when Task Scheduler started it
    /// by itself.
    pub fn ready(key: &str) -> Result<()> {
        let _ = fs::File::open(ready_pipe(key));
        Ok(())
    }

    /// The log this program writes, which its output and errors are, as a task's go nowhere.
    static LOG: Mutex<Option<fs::File>> = Mutex::new(None);

    /// Where to read what the server says: `server.log` in its folder.
    pub fn log(here: &Here, _key: &str) -> String {
        here.folder.join("server.log").display().to_string()
    }

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
}

/// A phone runs no server. Services on phones stay open for later.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
#[rustfmt::skip]
mod system {
    use super::*;

    const NONE: &str = "A server runs on Linux, macOS and Windows, and this device is none of them.";
    pub async fn start(_: &Here, _: &client::system::Program) -> Result<()> { bail!(NONE) }
    pub fn ready(_: &str) -> Result<()> { bail!(NONE) }
    pub fn bound(_: &Path) -> Result<()> { bail!(NONE) }
    pub fn log(_: &Here, _: &str) -> String { NONE.to_owned() }
}
