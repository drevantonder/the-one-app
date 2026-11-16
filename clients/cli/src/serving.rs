//! This device as a server of its person's: `one serve on` makes it one, kept running by the system's own way
//! (a systemd user unit on Linux, a LaunchAgent on a Mac, a scheduled task on Windows), `one serve off` stops it and
//! removes it, and the system runs `one serve run`, which keeps it running and updates it. The app's Devices screen
//! does the same with these.
//!
//! What a server runs is a build of One Core for this kind of machine, which the network keeps (`OneCore.Builds`):
//! downloaded the first time and on each update, checked against the SHA-256 the network gives for it, and started
//! once to say its build before it's used. All of it is in the client's home, `server/`:
//!
//! - `builds/<build>`: each build kept, and `current`, which the server runs, and `previous`, the one before.
//! - `next`: a build to try. `one serve run` starts it instead, and once it's running and answers by its key, it's
//!   `current`. One that doesn't, within `START`, is stopped and `current` is started again.
//! - `home`: the server's own folder, its key among it, whichever build runs.
//! - `went-back`: why the last update didn't take, with the last lines the new build said, which the server says
//!   in `builds` (`OneCore.Builds.started/1`) until an update takes.
//!
//! The server says on its output when it's running and when it's asked to update (`OneCore.Release`): asked, this
//! takes the latest builds, the program's too, then stops the server and starts itself again in its place. While
//! it's stopped, the network's other servers take its work over, and it takes its share back once it's running.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use std::{env, fs};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::{sleep, timeout};

use client::{BUILD, Client, target};

/// What the server says when it's running, with its key after, and when it's asked to take the latest builds.
const RUNNING: &str = "A server is running: ";
const UPDATE_ASKED: &str = "Update asked";
/// Which build of `one` this is, which an update takes the latest of: the app, with its window, or `one` built
/// without it, for a Linux machine with no screen.
const PROGRAM: &str = if cfg!(feature = "window") { "one" } else { "one-cli" };
/// How long a build being tried has to start and answer before the one before it is started again: a server
/// acting for the network deploys what it ships with that the network has never had before it's running.
const START: Duration = Duration::from_secs(180);

/// Makes this device a server of whoever it's for: its person, or the group or network `ONE_FOR` names. It's put
/// on their list, its build is downloaded and checked, and the system starts it now and whenever it starts.
pub async fn on(client: &Client) -> Result<String> {
    let address = client.address().context("A server is for a network with an address, and this one has none.")?;
    let here = Here::of(&client::home()?);
    let key = here.key()?;
    let who = match env::var("ONE_FOR") {
        Ok(who) => who,
        Err(_) => client.who().context("This device isn't signed in yet. Sign in on it first.")?,
    };
    tool(client, "add_server", json!({"key": key})).await?;
    if here.current().is_err() {
        let builds = download(client).await?;
        let build = builds
            .get("server")
            .with_context(|| format!("The network has no server for {} yet: `tooling/release` makes one.", target()))?;
        let build = here.take(build).await?;
        here.point("current", &build)?;
    }
    // The server runs with the client it updates by, and finds programs where its person's terminal does. Windows
    // starts a task with its person's environment as they log in, PATH and all.
    let mut said = vec![("ONE_HOME", client::home()?.display().to_string()), ("ONE_FOR", who.clone())];
    #[cfg(unix)]
    said.push(("PATH", login_path().await?));
    // A network being built has a command say who it acts as, and so does what runs here for it.
    if let Ok(name) = env::var("ONE_AS") {
        said.push(("ONE_AS", name));
    }
    system::start(&here, &key, &env::current_exe()?, &said).await?;
    tool(client, "builds", json!({"server": key})).await.context("The server started, and didn't answer")?;
    Ok(format!("This device is a server for {who} now, and starts whenever it does. Its key: {key}\n{address}"))
}

/// The PATH its person has in a terminal on this machine, which their login shell sets, read as `serve on` runs, and
/// again each time it's run. A server's services run programs a person installed (`git`, `go`, `rustc`, `swiftc`)
/// wherever their shell finds them, which isn't on the bare PATH that ssh, or an app opened from the Dock, gives.
/// It's read the way editors that run a person's tools read it (VS Code's shell environment): the shell started as a
/// login and interactive one, its PATH printed between two marks, past whatever its startup files print.
#[cfg(unix)]
async fn login_path() -> Result<String> {
    // The shell the system says is theirs, whatever started this.
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
    let said = timeout(Duration::from_secs(10), command)
        .await
        .with_context(|| format!("Your login shell, {shell}, didn't say its PATH within 10 seconds"))??;
    let said = String::from_utf8_lossy(&said.stdout);
    let path = said.split(MARK).nth(1).filter(|path| !path.is_empty());
    Ok(path.with_context(|| format!("Your login shell, {shell}, didn't say its PATH"))?.to_owned())
}

/// Stops this device's server, takes it off the list of whoever it was for, which another of their servers does,
/// and removes it from the device. Where they won't take it off, it's started again as it was.
pub async fn off(client: &Client) -> Result<String> {
    let here = Here::of(&client::home()?);
    ensure!(here.current().is_ok(), "This device isn't a server.");
    let key = here.key()?;
    system::stop(&here, &key)?;
    if let Err(error) = tool(client, "remove_server", json!({"key": key})).await {
        system::again(&here, &key)?;
        return Err(error.context("It's still a server"));
    }
    system::forget(&here, &key)?;
    fs::remove_dir_all(&here.folder)?;
    Ok(format!("This device is no longer a server, and nothing of it is left here: {key}"))
}

/// Takes the latest builds: this device's server's, if it's one, which it's asked to take, and otherwise the
/// program's own.
pub async fn update(client: &Client) -> Result<String> {
    let here = Here::of(&client::home()?);
    if here.current().is_ok() {
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

/// This device's server's key, if it's a server.
pub fn server() -> Option<String> {
    let here = Here::of(&client::home().ok()?);
    here.current().ok()?;
    here.key().ok()
}

/// Keeps this device's server running, as the system runs it: tries a build there's one to try, starts the
/// server, and takes the latest builds when it's asked. It ends when the server stops, for the system to start it
/// again.
pub async fn run(client: Arc<Client>) -> Result<()> {
    // The program's own file, named before an update replaces what's there.
    let program = env::current_exe()?;
    let here = Here::of(&client::home()?);
    let key = here.key()?;
    let address = client.address().context("A server is for a network with an address, and this one has none.")?;
    let who = env::var("ONE_FOR").context("ONE_FOR says who the server is for")?;
    // What the server says goes on, in what the system keeps of this program's output.
    let log = here.folder.join("server.log");
    system::bound(&log)?;
    let say = |line: &str| {
        println!("{line}");
        system::bound(&log)
    };
    loop {
        let trying = here.next();
        let build = match &trying {
            Some(build) => build.clone(),
            None => here.current()?,
        };
        let mut server = Command::from(command(release(&here.folder.join("builds").join(&build))));
        server
            .args(["eval", "OneCore.Release.serve()"])
            .env("ONE_SERVER_HOME", here.folder.join("home"))
            .env("ONE_ADDRESS", &address)
            .env("ONE_FOR", &who)
            .env("ONE_DEVICE", client.key())
            .env("ONE_NAME", client::name())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let (None, Ok(went_back)) = (&trying, fs::read_to_string(here.folder.join("went-back"))) {
            server.env("ONE_WENT_BACK", went_back);
        }
        let mut server = server.spawn().with_context(|| format!("couldn't start the build {build}"))?;
        // Everything it starts goes when it does, or when this does.
        let _contained = system::contain(&server)?;
        // The last lines it said, on its output and its errors, which say why a build being tried didn't take.
        let last = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        let keep = {
            let last = last.clone();
            move |line: &str| {
                let mut last = last.lock().unwrap();
                if last.len() == LAST {
                    last.pop_front();
                }
                last.push_back(line.to_owned());
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
        let mut said = BufReader::new(server.stdout.take().context("the server's output")?).lines();
        let running = async {
            while let Some(line) = said.next_line().await? {
                say(&line)?;
                keep(&line);
                if line.starts_with(RUNNING) {
                    return Ok::<_, anyhow::Error>(Started::Running);
                }
            }
            Ok(Started::Stopped)
        };
        let running = match &trying {
            Some(_) => timeout(START, running).await.unwrap_or(Ok(Started::Late))?,
            None => running.await?,
        };
        if let Some(build) = &trying {
            fs::remove_file(here.folder.join("next"))?;
            let why = match running {
                Started::Running if answers(&client, &key, build).await => None,
                Started::Running => Some(format!("it was running but didn't answer by its key within {} s", START.as_secs())),
                Started::Late => Some(format!("it didn't report running within {} s", START.as_secs())),
                Started::Stopped => Some(format!("it stopped before it was running ({})", server.wait().await?)),
            };
            let was = here.current()?;
            match why {
                None => {
                    here.point("previous", &was)?;
                    here.point("current", build)?;
                    here.prune()?;
                    let _ = fs::remove_file(here.folder.join("went-back"));
                    println!("The server is on the build {build} now, from {was}.");
                }
                Some(why) => {
                    server.kill().await?;
                    let lines: Vec<String> = last.lock().unwrap().iter().cloned().collect();
                    went_back(&here, &format!("Went back to {was} from {build}: {why}."), &lines)?;
                    continue;
                }
            }
        }
        if running != Started::Running {
            bail!("The server stopped before it was running: {}", server.wait().await?)
        }
        system::ready(&key)?;
        while let Some(line) = said.next_line().await? {
            say(&line)?;
            if line == UPDATE_ASKED {
                // Asked with the token this device has now, renewed if it has ended: this one's own renewing stops
                // where it was told the device is nobody's, and its timer doesn't count while the machine sleeps.
                let took = match Client::open(client::home()?, Vec::new()).await {
                    Ok(now) => {
                        let took = take(&now, &here, &program).await;
                        now.close().await;
                        took
                    }
                    Err(error) => Err(error),
                };
                match took {
                    Ok(true) => {
                        stop(&mut server).await?;
                        return Err(system::again_here(&program));
                    }
                    Ok(false) => println!("This device is on the latest builds already."),
                    Err(error) => {
                        // It's said in `builds`, which the server says as it starts: it's started again to say it.
                        went_back(&here, &format!("Stayed on {}: {error:#}.", here.current()?), &[])?;
                        stop(&mut server).await?;
                        return Err(system::again_here(&program));
                    }
                }
            }
        }
        bail!("The server stopped: {}", server.wait().await?)
    }
}

/// Stops a server that's running as a system stops a program, with SIGTERM, or on Windows, which has none, by closing
/// its input (`OneCore.Release`): it takes nothing new and answers what it took already (`OneCore.Application`), and
/// is killed if it hasn't stopped within `STOP`.
async fn stop(server: &mut tokio::process::Child) -> Result<()> {
    #[cfg(unix)]
    if let Some(id) = server.id() {
        unsafe { libc::kill(id as libc::pid_t, libc::SIGTERM) };
    }
    drop(server.stdin.take());
    if timeout(STOP, server.wait()).await.is_ok() {
        return Ok(());
    }
    Ok(server.kill().await?)
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

/// How long a server is given to stop: longer than it takes to answer what it took (`OneCore.Application`).
const STOP: Duration = Duration::from_secs(20);

/// How a build that was started went.
#[derive(PartialEq)]
enum Started {
    Running,
    Stopped,
    Late,
}

/// How many of the last lines a build said are kept to say why it didn't take.
const LAST: usize = 5;

/// Keeps why an update didn't take, with the last lines the build said, in `went-back`, for the server to say.
fn went_back(here: &Here, why: &str, lines: &[String]) -> Result<()> {
    let said = match lines {
        [] => why.to_owned(),
        lines => format!("{why} Its last lines: {}", lines.join(" | ")),
    };
    println!("{said}");
    Ok(fs::write(here.folder.join("went-back"), said)?)
}

/// Takes the latest builds this device isn't on: the server's, to try next, and the program's. Gives whether it
/// took any.
async fn take(client: &Client, here: &Here, program: &Path) -> Result<bool> {
    let builds = download(client).await?;
    let mut took = false;
    if let Some(server) = builds.get("server")
        && server["build"] != here.current()?
    {
        let build = here.take(server).await?;
        fs::write(here.folder.join("next"), build)?;
        took = true;
    }
    if let Some(one) = builds.get(PROGRAM)
        && one["build"] != BUILD
    {
        take_program(one, program).await?;
        took = true;
    }
    Ok(took)
}

/// Replaces this program with the build `one`, once it has said it's that build. What runs now runs on, and the
/// next start is the new one. It's the file a link to it names that's replaced, as on a Mac `~/.local/bin/one` is a
/// link to the program in The One App's bundle.
async fn take_program(one: &Value, program: &Path) -> Result<()> {
    let build = one["build"].as_str().context("a build says which it is")?;
    let program = &program.canonicalize()?;
    let new = program.with_extension("new");
    let packed = program.with_extension("new.gz");
    fetch(one, &packed).await?;
    let unpacked = std::io::copy(&mut flate2::read::GzDecoder::new(fs::File::open(&packed)?), &mut fs::File::create(&new)?);
    fs::remove_file(&packed)?;
    unpacked.with_context(|| format!("The build {build} of one didn't unpack."))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&new, fs::Permissions::from_mode(0o755))?;
    }
    let said = command(&new).arg("--version").output();
    if !said.as_ref().is_ok_and(|said| String::from_utf8_lossy(&said.stdout).split_whitespace().nth(1) == Some(build)) {
        fs::remove_file(&new)?;
        bail!("The build {build} of one doesn't start here.")
    }
    // Windows replaces no program that's running, but lets it be renamed: the one before is put aside, and goes the
    // next time.
    #[cfg(windows)]
    {
        let aside = program.with_extension("old");
        let _ = fs::remove_file(&aside);
        fs::rename(program, &aside)?;
    }
    fs::rename(&new, program)?;
    Ok(())
}

/// Whether the server whose key is `key` answers by it, saying it's `build`, within `START`. It's asked again
/// until then: the network's relay passes calls to a server that's just started only once it has seen it there.
async fn answers(client: &Client, key: &str, build: &str) -> bool {
    let asking = async {
        loop {
            let said = tool(client, "builds", json!({"server": key})).await.ok();
            if said
                .and_then(|said| serde_json::from_str::<Value>(&said).ok())
                .is_some_and(|said| said["build"] == build)
            {
                return;
            }
            sleep(Duration::from_secs(5)).await;
        }
    };
    timeout(START, asking).await.is_ok()
}

/// The latest builds for this kind of machine, by what they are (`server`, `one`), each with a link to it.
async fn download(client: &Client) -> Result<Map<String, Value>> {
    let said = tool(client, "download", json!({"target": target()})).await?;
    match serde_json::from_str(&said)? {
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
    let mut answer = request.send().await?.error_for_status()?;
    let mut out = tokio::fs::File::create(file).await?;
    let (mut hash, mut size) = (Sha256::new(), 0);
    while let Some(part) = answer.chunk().await? {
        hash.update(&part);
        size += part.len() as u64;
        out.write_all(&part).await?;
    }
    out.flush().await?;
    let whole = build["sha256"] == data_encoding::HEXLOWER.encode(&hash.finalize()) && build["size"] == size;
    if !whole {
        fs::remove_file(file)?;
        bail!("The build {} didn't download whole.", build["build"]);
    }
    Ok(())
}

/// Calls one of the network's tools and gives what it said, or the error it said.
async fn tool(client: &Client, name: &str, arguments: Value) -> Result<String> {
    let result = client.request("tools/call", json!({"name": name, "arguments": arguments})).await?;
    let said: Vec<&str> =
        result["content"].as_array().into_iter().flatten().filter_map(|part| part["text"].as_str()).collect();
    let said = said.join("\n");
    if result["isError"] == true {
        bail!("{said}")
    }
    Ok(said)
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

    /// The build `current` names: what the server runs.
    fn current(&self) -> Result<String> {
        let current = fs::read_link(self.folder.join("current")).context("This device isn't a server.")?;
        Ok(current.file_name().context("`current` names a build")?.to_string_lossy().into_owned())
    }

    /// The build to try, if there's one.
    fn next(&self) -> Option<String> {
        fs::read_to_string(self.folder.join("next")).ok().map(|next| next.trim().to_owned())
    }

    /// Has `name` (`current`, `previous`) name `build` from now on, in one step: a symlink, or on Windows, which
    /// lets only an administrator make one, a junction.
    fn point(&self, name: &str, build: &str) -> Result<()> {
        let pointing = self.folder.join(format!("{name}.new"));
        let _ = fs::remove_file(&pointing);
        #[cfg(unix)]
        std::os::unix::fs::symlink(Path::new("builds").join(build), &pointing)?;
        #[cfg(windows)]
        {
            let _ = fs::remove_dir(&pointing);
            junction::create(self.folder.join("builds").join(build), &pointing)?;
        }
        Ok(fs::rename(pointing, self.folder.join(name))?)
    }

    /// Removes the builds that are neither `current` nor `previous`.
    fn prune(&self) -> Result<()> {
        let retained: Vec<_> = ["current", "previous"]
            .iter()
            .filter_map(|name| fs::read_link(self.folder.join(name)).ok()?.file_name().map(|build| build.to_owned()))
            .collect();
        for build in fs::read_dir(self.folder.join("builds"))? {
            let build = build?;
            if !retained.contains(&build.file_name()) {
                fs::remove_dir_all(build.path())?;
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
        let said = command(release(&unpacking)).args(["eval", "OneCore.Release.check()"]).output()?;
        let (out, errors) = (String::from_utf8_lossy(&said.stdout), String::from_utf8_lossy(&said.stderr));
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

/// How the system keeps the server running: a systemd user unit on Linux, which starts at boot where the user's
/// processes linger and at their login otherwise, and a LaunchAgent on a Mac, which starts at their login. Each is
/// named after the server's key, so a machine can serve more than one network.
#[cfg(target_os = "linux")]
mod system {
    use super::*;

    fn name(key: &str) -> String {
        format!("one-server-{}.service", &key[..12])
    }

    /// Writes the unit in the server's folder and has systemd start it now, or again with what it's run with now
    /// where it's running, and with the user's processes. That returns once the server is running (`ready`), or
    /// failed to start.
    pub async fn start(here: &Here, key: &str, program: &Path, said: &[(&str, String)]) -> Result<()> {
        let unit = here.folder.join(name(key));
        let said: String = said.iter().map(|(name, value)| format!("Environment=\"{name}={value}\"\n")).collect();
        fs::write(
            &unit,
            format!(
                "[Unit]\nDescription=The One App's server\nStartLimitIntervalSec=0\n\n\
                 [Service]\nType=notify\nExecStart=\"{}\" serve run\n{said}Restart=always\nRestartSec=2\n\
                 TimeoutStartSec={}\n\n[Install]\nWantedBy=default.target\n",
                program.display(),
                START.as_secs()
            ),
        )?;
        systemctl(&["enable", &unit.display().to_string()])?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["restart", &name(key)])
    }

    pub fn stop(_here: &Here, key: &str) -> Result<()> {
        systemctl(&["stop", &name(key)])
    }

    pub fn again(_here: &Here, key: &str) -> Result<()> {
        systemctl(&["start", &name(key)])
    }

    /// Has systemd forget the unit, which goes with the server's folder.
    pub fn forget(_here: &Here, key: &str) -> Result<()> {
        systemctl(&["disable", &name(key)])
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

    /// What it starts goes with it already: systemd stops a unit's every process, and the server stops when its
    /// input closes, as it does when this program ends.
    pub fn contain(_server: &tokio::process::Child) -> Result<()> {
        Ok(())
    }

    /// Starts this program again in this one's place, as the system started it: the latest of it, which tries the
    /// latest server.
    pub fn again_here(program: &Path) -> anyhow::Error {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(program).args(["serve", "run"]).exec().into()
    }

    fn systemctl(args: &[&str]) -> Result<()> {
        let done = std::process::Command::new("systemctl").arg("--user").args(args).output()?;
        ensure!(done.status.success(), "systemd didn't: {}", String::from_utf8_lossy(&done.stderr).trim());
        Ok(())
    }
}

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

    fn user() -> Result<String> {
        Ok(String::from_utf8(std::process::Command::new("id").arg("-u").output()?.stdout)?.trim().to_owned())
    }

    /// Writes the LaunchAgent and has launchd start it now, or again with what it's run with now where it's
    /// running, and at each login, keeping it running. Its output is `server.log` in the server's folder. That
    /// returns once the server is running: it says so (`ready`) on a socket in its folder, as systemd hears it.
    pub async fn start(here: &Here, key: &str, program: &Path, said: &[(&str, String)]) -> Result<()> {
        let ready = here.folder.join("ready");
        let text = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;");
        let said: String = said
            .iter()
            .map(|(name, value)| (*name, value.clone()))
            .chain([("NOTIFY_SOCKET", ready.display().to_string())])
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
                 <key>EnvironmentVariables</key><dict>{said}</dict>\
                 <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>\
                 <key>StandardOutPath</key><string>{log}</string><key>StandardErrorPath</key><string>{log}</string>\
                 </dict></plist>\n",
                label(key),
                text(&program.display().to_string())
            ),
        )?;
        // launchd won't load an agent that's loaded already, as it is where `serve on` is run again.
        let _ = stop(here, key);
        let _ = fs::remove_file(&ready);
        let socket = UnixDatagram::bind(&ready)?;
        socket.set_read_timeout(Some(START))?;
        launchctl(&["bootstrap", &format!("gui/{}", user()?), &agent.display().to_string()])?;
        let running = socket.recv(&mut [0; 16]);
        fs::remove_file(&ready)?;
        running.with_context(|| {
            format!("The server didn't start: its log is {}", here.folder.join("server.log").display())
        })?;
        Ok(())
    }

    pub fn stop(_here: &Here, key: &str) -> Result<()> {
        launchctl(&["bootout", &format!("gui/{}/{}", user()?, label(key))])
    }

    pub fn again(_here: &Here, key: &str) -> Result<()> {
        launchctl(&["bootstrap", &format!("gui/{}", user()?), &agent(key)?.display().to_string()])
    }

    pub fn forget(_here: &Here, key: &str) -> Result<()> {
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

    /// What it starts goes with it already: systemd stops a unit's every process, and the server stops when its
    /// input closes, as it does when this program ends.
    pub fn contain(_server: &tokio::process::Child) -> Result<()> {
        Ok(())
    }

    /// Starts this program again in this one's place, as the system started it: the latest of it, which tries the
    /// latest server.
    pub fn again_here(program: &Path) -> anyhow::Error {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(program).args(["serve", "run"]).exec().into()
    }

    fn launchctl(args: &[&str]) -> Result<()> {
        let done = std::process::Command::new("launchctl").args(args).output()?;
        ensure!(done.status.success(), "launchd didn't: {}", String::from_utf8_lossy(&done.stderr).trim());
        Ok(())
    }
}

/// How Windows keeps the server running: a scheduled task of its person's, which needs no administrator, as a
/// program someone installs for themselves keeps running (Syncthing's way). A Windows service would need one, and
/// would run apart from them, in a session of its own. Task Scheduler starts it as they log on, in their session and
/// with their environment, as a LaunchAgent does on a Mac, and each minute it isn't running, which is how it's
/// started again once it stops: a task's restarting on failure is only for one that couldn't start, never for a
/// program's ending. `one` asks for no console window as it's started there (`clients/app/build.rs`). What it says is
/// `server.log` in the server's folder, and its starts and ends are in the task's history.
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

    /// Registers the task and has Task Scheduler start it now, or again with what it's run with now where it's
    /// running. That returns once the server is running (`ready`). A task has no environment of its own, so the
    /// server's is in its arguments, which `one serve run` takes.
    pub async fn start(here: &Here, key: &str, program: &Path, said: &[(&str, String)]) -> Result<()> {
        let user = format!("{}\\{}", env::var("USERDOMAIN")?, env::var("USERNAME")?);
        let text = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let arguments: String = said.iter().map(|(name, value)| format!(" \"{name}={value}\"")).collect();
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
        let _ = stop(here, key);
        let registered = schtasks(&["/create", "/tn", &name(key), "/xml", &file.display().to_string(), "/f"]);
        fs::remove_file(&file)?;
        registered?;
        let ready = tokio::net::windows::named_pipe::ServerOptions::new().first_pipe_instance(true).create(ready_pipe(key))?;
        schtasks(&["/run", "/tn", &name(key)])?;
        let log = here.folder.join("server.log");
        timeout(START, ready.connect())
            .await
            .ok()
            .and_then(Result::ok)
            .with_context(|| format!("The server didn't start: its log is {}", log.display()))
    }

    pub fn stop(_here: &Here, key: &str) -> Result<()> {
        schtasks(&["/end", "/tn", &name(key)])
    }

    pub fn again(_here: &Here, key: &str) -> Result<()> {
        schtasks(&["/run", "/tn", &name(key)])
    }

    pub fn forget(_here: &Here, key: &str) -> Result<()> {
        schtasks(&["/delete", "/tn", &name(key), "/f"])
    }

    /// This program as Task Scheduler starts it next: within a minute of this one's ending, as a task that's running
    /// isn't started again.
    pub fn again_here(_program: &Path) -> anyhow::Error {
        anyhow::anyhow!("It stopped to take the latest builds, which Task Scheduler starts within a minute.")
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
            ensure!(unsafe { SetStdHandle(written, new.as_raw_handle() as HANDLE) } != 0, "couldn't write {}", log.display());
        }
        *writing = Some(new);
        Ok(())
    }

    /// A job object holding the server and whatever it starts, which are all stopped once it's let go of: when the
    /// server is stopped or this program ends, however it ends. A release's script, which the server is, starts its VM
    /// once it has read two batch files, long after this.
    pub struct Contained(HANDLE);

    impl Drop for Contained {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn contain(server: &tokio::process::Child) -> Result<Contained> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            ensure!(!job.is_null(), "couldn't make a job object: {}", std::io::Error::last_os_error());
            let job = Contained(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = std::mem::size_of_val(&limits) as u32;
            let limited = SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, &raw const limits as _, size);
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
mod system {
    use super::*;

    const NONE: &str = "A server runs on Linux, macOS and Windows, and this device is none of them.";

    pub async fn start(_here: &Here, _key: &str, _program: &Path, _said: &[(&str, String)]) -> Result<()> {
        bail!(NONE)
    }

    pub fn stop(_here: &Here, _key: &str) -> Result<()> {
        bail!(NONE)
    }

    pub fn again(_here: &Here, _key: &str) -> Result<()> {
        bail!(NONE)
    }

    pub fn forget(_here: &Here, _key: &str) -> Result<()> {
        bail!(NONE)
    }

    pub fn again_here(_program: &Path) -> anyhow::Error {
        anyhow::anyhow!(NONE)
    }

    pub fn contain(_server: &tokio::process::Child) -> Result<()> {
        bail!(NONE)
    }

    pub fn ready(_key: &str) -> Result<()> {
        bail!(NONE)
    }

    pub fn bound(_log: &Path) -> Result<()> {
        bail!(NONE)
    }
}
