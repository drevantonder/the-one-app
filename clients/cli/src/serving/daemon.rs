//! `one serve run`: the program the system keeps running, which starts the server and watches it.

use super::*;

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
    let reports = Reports { client: client.clone(), server: here.key()? };
    let ran = serve(&client, &here, &reports).await;
    reports.tell(true).await;
    ran
}

pub(super) async fn serve(client: &Arc<Client>, here: &Here, reports: &Reports) -> Result<()> {
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
pub(super) async fn didnt_start(
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

/// Reads what the server says, on its output and its errors, as it comes, whatever this is doing meanwhile: a
/// server whose output isn't read is held up once the pipe it writes to is full, 16 KB on a Mac, so each line it
/// logs holds up whatever logged it. Gives every line, to act on the messages (that it's running, that it's asked to
/// update) and to say why a build didn't start.
pub(super) fn listen(server: &mut Child, log: PathBuf) -> Result<UnboundedReceiver<Heard>> {
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
pub(super) async fn within<T>(doing: &str, done: impl Future<Output = Result<T>>) -> Result<T> {
    timeout(START, done).await.unwrap_or_else(|_| bail!("it didn't {doing} within {} s", START.as_secs()))
}

/// Once the system asks this program to stop: SIGTERM, as systemd and launchd stop a program. Task Scheduler ends a
/// task's program outright.
pub(super) async fn stopped() {
    #[cfg(unix)]
    if let Ok(mut terminate) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        terminate.recv().await;
        return;
    }
    std::future::pending::<()>().await
}

/// Tells the server `told`, a line of JSON: what it is, which it says to the network, or to say the latest builds
/// (`OneCore.Release`). A server that has stopped is told nothing, and needs telling nothing.
pub(super) async fn tell(server: &mut Child, told: &Told) {
    if let Some(input) = server.stdin.as_mut() {
        let _ = input.write_all(format!("{told}\n").as_bytes()).await;
        let _ = input.flush().await;
    }
}

/// Stops a server that's running, once it's told `told`, by closing its input: it takes nothing new and answers what
/// it took already (`OneCore.Application`), and is killed if it hasn't stopped within `STOP`.
pub(super) async fn stop(server: &mut Child, told: &Told) -> Result<()> {
    tell(server, told).await;
    drop(server.stdin.take());
    if timeout(STOP, server.wait()).await.is_err() {
        server.kill().await?;
    }
    Ok(())
}

/// This program started again in its place, as the system started it, to run the pair the record says: on Windows,
/// which replaces no running program, by ending for Task Scheduler to start it again at once (`system::again`).
pub(super) fn again(#[cfg_attr(unix, expect(unused_variables))] here: &Here, program: &Path) -> anyhow::Error {
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
