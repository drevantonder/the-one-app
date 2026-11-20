//! Becoming a server, and ceasing to be one: `one serve on` and `off`, a network's first server, and settling a join or leave that was cut short.

use super::*;

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
pub(super) async fn registered(here: &Here, key: &str, who: &str) -> Result<&'static str> {
    let server = kept(here, key, &program()?, unit_env(who.to_owned(), None).await?);
    let changed = client::system::install(&server)
        .map_err(|error| anyhow::anyhow!("{error:#}\nThen `one serve on` again, as yourself, registers it as yours."))?;
    Ok(if changed {
        "the system runs it as this build says to from its next start (its registration changed)"
    } else {
        "the system runs it as this build says to (its registration is unchanged)"
    })
}

pub(super) async fn join(client: &Client, here: &Here, key: &str, who: &str) -> Result<()> {
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
pub(super) async fn undo_joining(client: &Client, here: &Here, key: &str) -> String {
    let mut left = Vec::new();
    if let Err(why) = listed(client, here, "remove_server", key).await {
        left.push(format!("it's still on the list of servers: {why:#}"));
    }
    said_left(left.into_iter().chain(removed(here, key)).collect())
}

/// Takes what joining put on this device away: the system no longer runs it, and its folder goes. Gives what of
/// that couldn't be done.
pub(super) fn removed(here: &Here, key: &str) -> Vec<String> {
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
pub(super) fn said_left(left: Vec<String>) -> String {
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
pub(super) async fn settled(client: &Client, here: &Here, start: bool) -> Result<Record> {
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
pub(super) fn who(client: &Client) -> Result<String> {
    env::var("ONE_SUBJECT").or_else(|_| client.who().context("This device isn't signed in yet. Sign in on it first."))
}

/// Who this device's server is for once it has begun joining: whom it joined for, as its record says, whoever runs
/// this program now, as its person's own `one serve off` of the network's server.
pub(super) fn whose(client: &Client, here: &Here) -> Result<String> {
    here.read()?.subject.map_or_else(|| who(client), Ok)
}

/// Puts this device's server on the list of whoever it's for (`add_server`), or takes it off (`remove_server`), asked
/// for them (`whose`), not for whoever runs this.
pub(super) async fn listed(client: &Client, here: &Here, name: &str, key: &str) -> Result<String> {
    let request = json!({"name": name, "arguments": {"key": key}, "_meta": {"subject": whose(client, here)?}});
    crate::said(client.request("tools/call", request).await?)
}

/// The environment the system runs the server with: the client it updates by, who it's for, and where programs
/// are found, `path` where it's known, as the system gave it to a server that's running, and otherwise its person's
/// login PATH. Windows starts a task with its person's environment as they log in, PATH and all.
pub(super) async fn unit_env(who: String, #[cfg_attr(not(unix), expect(unused_variables))] path: Option<String>) -> Result<Vec<(&'static str, String)>> {
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
pub(super) fn kept(here: &Here, key: &str, program: &Path, env: Vec<(&str, String)>) -> client::system::Program {
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
        description: "the One App server, which `one serve off` removes.".to_owned(),
        command,
        env,
        folder: here.folder.clone(),
        desktop: false,
        start_within: Some(2 * START + Duration::from_secs(30)),
        stop_within: STOP + Duration::from_secs(10),
    }
}

/// What the system knows this device's server by, with its key (`client::system::name`).
pub(super) const SERVER: &str = "server";

/// The PATH its person has in a terminal on this machine, which their login shell sets. A server's services run
/// programs a person installed (`git`, `go`, `rustc`, `swiftc`) wherever their shell finds them, which isn't on the
/// bare PATH that ssh, or an app opened from the Dock, gives. It's read the way editors that run a person's tools read
/// it (VS Code's shell environment): the shell started as a login and interactive one, its PATH printed between two
/// marks, past whatever its startup files print.
#[cfg(unix)]
pub(super) async fn login_path() -> Result<String> {
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

pub(super) async fn leave(client: &Client, here: &Here, key: &str, lock: fs::File) -> Result<()> {
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
pub(super) async fn undo_leaving(client: &Client, here: &Here, key: &str, start: bool) -> Result<()> {
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
