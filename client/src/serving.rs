//! This device as a server of its person's: `one serve on` makes it one, kept running by the system's own way
//! (a systemd user unit on Linux, a LaunchAgent on a Mac), `one serve off` stops it and removes it, and the system
//! runs `one serve run`, which keeps it running and updates it. The app's Devices screen does the same with these.
//!
//! What a server runs is a build of One Core for this kind of machine, which the network keeps (`OneCore.Builds`):
//! downloaded the first time and on each update, checked against the SHA-256 the network gives for it, and started
//! once to say its build before it's used. All of it is in the client's home, `server/`:
//!
//! - `builds/<build>`: each build kept, and `current`, which the server runs, and `previous`, the one before.
//! - `next`: a build to try. `one serve run` starts it instead, and once it's running and answers by its key, it's
//!   `current`. One that doesn't, within `START`, is stopped and `current` is started again.
//! - `home`: the server's own folder, its key among it, whichever build runs.
//!
//! The server says on its output when it's running and when it's asked to update (`OneCore.Release`): asked, this
//! takes the newest builds, the program's too, then stops the server and starts itself again in its place. While
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
use tokio::time::timeout;

use crate::{BUILD, Client, target};

/// What the server says when it's running, with its key after, and when it's asked to take the newest builds.
const RUNNING: &str = "A server is running: ";
const UPDATE_ASKED: &str = "Update asked";
/// How long a build being tried has to start and answer before the one before it is started again: a server
/// acting for the network deploys what it ships with that the network has never had before it's running.
const START: Duration = Duration::from_secs(180);

/// Makes this device a server of whoever it's for: its person, or the group or network `ONE_FOR` names. It's put
/// on their list, its build is downloaded and checked, and the system starts it now and whenever it starts.
pub async fn on(client: &Client) -> Result<String> {
    let address = client.address().context("A server is for a network with an address, and this one has none.")?;
    let here = Here::of(&crate::home()?);
    let key = here.key()?;
    let who = match env::var("ONE_FOR") {
        Ok(who) => who,
        Err(_) => client.who().context("This device is nobody's yet: sign in on it first.")?,
    };
    tool(client, "add_server", json!({"key": key})).await?;
    if here.current().is_err() {
        let builds = download(client).await?;
        let build = builds
            .get("server")
            .with_context(|| format!("The network has no server for {} yet: `./release` makes one.", target()))?;
        let build = here.take(build).await?;
        here.point("current", &build)?;
    }
    // The server runs with the client it updates by, and finds programs where its person's terminal does.
    let mut said = vec![
        ("ONE_HOME", crate::home()?.display().to_string()),
        ("ONE_FOR", who.clone()),
        ("PATH", login_path().await?),
    ];
    // A network being built has a command say who it acts as, and so does what runs here for it.
    if let Ok(name) = env::var("ONE_AS") {
        said.push(("ONE_AS", name));
    }
    system::start(&here, &key, &env::current_exe()?, &said)?;
    tool(client, "builds", json!({"server": key})).await.context("The server started, and didn't answer")?;
    Ok(format!("This device is a server for {who} now, and starts whenever it does. Its key: {key}\n{address}"))
}

/// The PATH its person has in a terminal on this machine, which their login shell sets, read as `serve on` runs, and
/// again each time it's run. A server's services run programs a person installed (`git`, `go`, `rustc`, `swiftc`)
/// wherever their shell finds them, which isn't on the bare PATH that ssh, or an app opened from the Dock, gives.
/// It's read the way editors that run a person's tools read it (VS Code's shell environment): the shell started as a
/// login and interactive one, its PATH printed between two marks, past whatever its startup files print.
async fn login_path() -> Result<String> {
    // The shell the system says is theirs, whatever started this.
    let shell = unsafe {
        let user = libc::getpwuid(libc::getuid());
        ensure!(!user.is_null(), "this machine says nothing of who's running this");
        std::ffi::CStr::from_ptr((*user).pw_shell).to_string_lossy().into_owned()
    };
    const MARK: &str = "one-path-6f0c";
    let asked = Command::new(&shell)
        .args(["-i", "-l", "-c", &format!("printf '%s%s%s' {MARK} \"$PATH\" {MARK}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let said = timeout(Duration::from_secs(10), asked)
        .await
        .with_context(|| format!("Your login shell, {shell}, didn't say its PATH within 10 seconds"))??;
    let said = String::from_utf8_lossy(&said.stdout);
    let path = said.split(MARK).nth(1).filter(|path| !path.is_empty());
    Ok(path.with_context(|| format!("Your login shell, {shell}, didn't say its PATH"))?.to_owned())
}

/// Stops this device's server, takes it off the list of whoever it was for, which another of their servers does,
/// and removes it from the device. Where they won't take it off, it's started again as it was.
pub async fn off(client: &Client) -> Result<String> {
    let here = Here::of(&crate::home()?);
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

/// Takes the newest builds: this device's server's, if it's one, which it's asked to take, and otherwise the
/// program's own.
pub async fn update(client: &Client) -> Result<String> {
    let here = Here::of(&crate::home()?);
    if here.current().is_ok() {
        return tool(client, "update", json!({"server": here.key()?})).await;
    }
    let builds = download(client).await?;
    let one = builds.get("one").with_context(|| format!("The network has no build of one for {} yet.", target()))?;
    if one["build"] == BUILD {
        return Ok(format!("This device is on the newest build already: {BUILD}"));
    }
    take_program(one, &env::current_exe()?).await?;
    Ok(format!("This device is on the build {} now.", one["build"].as_str().unwrap_or_default()))
}

/// This device's server's key, if it's a server.
pub fn server() -> Option<String> {
    let here = Here::of(&crate::home().ok()?);
    here.current().ok()?;
    here.key().ok()
}

/// Keeps this device's server running, as the system runs it: tries a build there's one to try, starts the
/// server, and takes the newest builds when it's asked. It ends when the server stops, for the system to start it
/// again.
pub async fn run(client: Arc<Client>) -> Result<()> {
    // The program's own file, named before an update replaces what's there.
    let program = env::current_exe()?;
    let here = Here::of(&crate::home()?);
    let key = here.key()?;
    let address = client.address().context("A server is for a network with an address, and this one has none.")?;
    let who = env::var("ONE_FOR").context("ONE_FOR says who the server is for")?;
    loop {
        let trying = here.next();
        let build = match &trying {
            Some(build) => build.clone(),
            None => here.current()?,
        };
        let mut server = Command::new(here.folder.join("builds").join(&build).join("bin/one_core"))
            .args(["eval", "OneCore.Release.serve()"])
            .env("ONE_SERVER_HOME", here.folder.join("home"))
            .env("ONE_ADDRESS", &address)
            .env("ONE_FOR", &who)
            .env("ONE_DEVICE", client.key())
            .env("ONE_NAME", crate::name())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("couldn't start the build {build}"))?;
        let mut said = BufReader::new(server.stdout.take().context("the server's output")?).lines();
        let running = async {
            while let Some(line) = said.next_line().await? {
                println!("{line}");
                if line.starts_with(RUNNING) {
                    return Ok::<_, anyhow::Error>(true);
                }
            }
            Ok(false)
        };
        let running = match &trying {
            Some(_) => timeout(START, running).await.unwrap_or(Ok(false))?,
            None => running.await?,
        };
        if let Some(build) = &trying {
            fs::remove_file(here.folder.join("next"))?;
            if running && answers(&client, &key, build).await {
                let was = here.current()?;
                here.point("previous", &was)?;
                here.point("current", build)?;
                here.prune()?;
                println!("The server is on the build {build} now, from {was}.");
            } else {
                server.kill().await?;
                println!("The build {build} didn't start and answer, so the server is back on {}.", here.current()?);
                continue;
            }
        }
        if !running {
            bail!("The server stopped before it was running: {}", server.wait().await?)
        }
        system::ready()?;
        while let Some(line) = said.next_line().await? {
            println!("{line}");
            if line == UPDATE_ASKED {
                match take(&client, &here, &program).await {
                    Ok(true) => {
                        server.kill().await?;
                        return Err(again(&program));
                    }
                    Ok(false) => println!("This device is on the newest builds already."),
                    Err(error) => println!("The newest builds weren't taken: {error:#}"),
                }
            }
        }
        bail!("The server stopped: {}", server.wait().await?)
    }
}

/// Starts this program again in this one's place, as the system started it: the newest of it, which tries the
/// newest server.
fn again(program: &Path) -> anyhow::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(program).args(["serve", "run"]).exec().into()
}

/// Takes the newest builds this device isn't on: the server's, to try next, and the program's. Gives whether it
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
    if let Some(one) = builds.get("one")
        && one["build"] != BUILD
    {
        take_program(one, program).await?;
        took = true;
    }
    Ok(took)
}

/// Replaces this program with the build `one`, once it has said it's that build. What runs now runs on, and the
/// next start is the new one.
async fn take_program(one: &Value, program: &Path) -> Result<()> {
    let build = one["build"].as_str().context("a build says which it is")?;
    let new = program.with_extension("new");
    let packed = program.with_extension("new.gz");
    fetch(one, &packed).await?;
    let unpacked = std::process::Command::new("gzip").arg("-df").arg(&packed).status()?;
    ensure!(unpacked.success(), "The build {build} of one didn't unpack.");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&new, fs::Permissions::from_mode(0o755))?;
    let said = std::process::Command::new(&new).arg("--version").output();
    if !said.as_ref().is_ok_and(|said| String::from_utf8_lossy(&said.stdout).split_whitespace().nth(1) == Some(build)) {
        fs::remove_file(&new)?;
        bail!("The build {build} of one doesn't start here.")
    }
    fs::rename(&new, program)?;
    Ok(())
}

/// Whether the server whose key is `key` answers by it, saying it's `build`.
async fn answers(client: &Client, key: &str, build: &str) -> bool {
    let asked = timeout(START, tool(client, "builds", json!({"server": key}))).await;
    let said = asked.ok().and_then(Result::ok).and_then(|said| serde_json::from_str::<Value>(&said).ok());
    said.is_some_and(|said| said["build"] == build)
}

/// The newest builds for this kind of machine, by what they are (`server`, `one`), each with a link to it.
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
    let mut asked = crate::web()?.request(method, link["url"].as_str().context("a build has a link")?);
    for (name, value) in link["headers"].as_object().into_iter().flatten() {
        asked = asked.header(name, value.as_str().unwrap_or_default());
    }
    let mut answer = asked.send().await?.error_for_status()?;
    let mut out = tokio::fs::File::create(file).await?;
    let (mut hash, mut size) = (Sha256::new(), 0);
    while let Some(part) = answer.chunk().await? {
        hash.update(&part);
        size += part.len() as u64;
        out.write_all(&part).await?;
    }
    out.flush().await?;
    let whole = build["sha256"] == crate::hex(&hash.finalize()) && build["size"] == size;
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
        Ok(crate::key(&self.folder.join("home/key"))?.public().to_string())
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

    /// Has `name` (`current`, `previous`) name `build` from now on, in one step.
    fn point(&self, name: &str, build: &str) -> Result<()> {
        let pointing = self.folder.join(format!("{name}.new"));
        let _ = fs::remove_file(&pointing);
        std::os::unix::fs::symlink(Path::new("builds").join(build), &pointing)?;
        Ok(fs::rename(pointing, self.folder.join(name))?)
    }

    /// Removes the builds that are neither `current` nor `previous`.
    fn prune(&self) -> Result<()> {
        let kept: Vec<_> =
            ["current", "previous"].iter().filter_map(|name| fs::read_link(self.folder.join(name)).ok()).collect();
        for build in fs::read_dir(self.folder.join("builds"))? {
            let build = build?.path();
            if !kept.iter().any(|kept| build.ends_with(kept)) {
                fs::remove_dir_all(build)?;
            }
        }
        Ok(())
    }

    /// Downloads the build `build`, unpacks it and has it say which it is, which starts its VM and loads its
    /// add-ons: a build that can't do that here goes. Gives its name. One that's here already is taken as it is.
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
        fetch(build, &archive).await?;
        let _ = fs::remove_dir_all(&unpacking);
        fs::create_dir_all(&unpacking)?;
        let unpacked =
            std::process::Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&unpacking).status()?;
        fs::remove_file(&archive)?;
        ensure!(unpacked.success(), "The build {name} didn't unpack.");
        let said = std::process::Command::new(unpacking.join("bin/one_core"))
            .args(["eval", "OneCore.Release.check()"])
            .output()?;
        if String::from_utf8_lossy(&said.stdout).trim() != name {
            fs::remove_dir_all(&unpacking)?;
            bail!("The build {name} doesn't start here: {}", String::from_utf8_lossy(&said.stderr).trim());
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
    pub fn start(here: &Here, key: &str, program: &Path, said: &[(&str, String)]) -> Result<()> {
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

    /// Tells systemd the server is running, as `Type=notify` has it wait for.
    pub fn ready() -> Result<()> {
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

    fn systemctl(asked: &[&str]) -> Result<()> {
        let done = std::process::Command::new("systemctl").arg("--user").args(asked).output()?;
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
    pub fn start(here: &Here, key: &str, program: &Path, said: &[(&str, String)]) -> Result<()> {
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
        let heard = UnixDatagram::bind(&ready)?;
        heard.set_read_timeout(Some(START))?;
        launchctl(&["bootstrap", &format!("gui/{}", user()?), &agent.display().to_string()])?;
        let running = heard.recv(&mut [0; 16]);
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

    /// Tells `start` the server is running. Nothing hears it when launchd started it by itself, as at a login.
    pub fn ready() -> Result<()> {
        let Some(socket) = env::var_os("NOTIFY_SOCKET") else { return Ok(()) };
        let _ = UnixDatagram::unbound()?.send_to(b"READY=1", socket);
        Ok(())
    }

    fn launchctl(asked: &[&str]) -> Result<()> {
        let done = std::process::Command::new("launchctl").args(asked).output()?;
        ensure!(done.status.success(), "launchd didn't: {}", String::from_utf8_lossy(&done.stderr).trim());
        Ok(())
    }
}

/// A phone runs no server, and Windows has nothing here yet to keep one running.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod system {
    use super::*;

    const NONE: &str = "A server runs on Linux and macOS, and this device is neither.";

    pub fn start(_here: &Here, _key: &str, _program: &Path, _said: &[(&str, String)]) -> Result<()> {
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

    pub fn ready() -> Result<()> {
        bail!(NONE)
    }
}
