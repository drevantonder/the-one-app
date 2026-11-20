//! A program the system keeps running for its person, in their session, with no administrator, as a program someone
//! installs for themselves keeps running (Syncthing's way): a systemd user unit on Linux, a LaunchAgent on a Mac, and
//! a scheduled task of theirs on Windows, where a Windows service would need an administrator and run apart from them.
//! It's started again when it ends non-zero, 10 s apart, and left failed after 10 starts in 10 minutes, a crash loop. A
//! device's server is one (`one serve on`), and so is what a server's service runs on its person's screen, as
//! Computer Use's Cua Driver (`OneCore.System`). Each is named for what it is and the server it's for, so a machine can
//! serve more than one network.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};

/// What the system keeps running.
pub struct Program {
    /// What it is, which names it: `server`, `computer-use`.
    pub name: String,
    /// The key of the server it's for, whose first 12 characters name it too.
    pub key: String,
    pub description: String,
    /// The program and its arguments.
    pub command: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Its own folder, where what it says is kept on a Mac, `<name>.log` (systemd keeps a unit's in its journal), and
    /// where a task starts on Windows, with what's registered.
    pub folder: PathBuf,
    /// Whether it runs on its person's screen: on Linux it starts and stops with their graphical session, given the
    /// display this process has where it has one, and on a Mac it's run only in a login's session, as an app is.
    pub desktop: bool,
    /// How long it has to say it's running, where it says so (systemd's `Type=notify`).
    pub start_within: Option<Duration>,
    /// How long it's given to stop before it's killed.
    pub stop_within: Duration,
}

/// The name the system knows `name` for the server `key` by: a unit's, a LaunchAgent's label, a task's.
pub fn name(name: &str, key: &str) -> String {
    let key = &key[..key.len().min(12)];
    if cfg!(target_os = "macos") {
        format!("app.theone.{name}.{key}")
    } else if cfg!(windows) {
        format!("the One App {name} {key}")
    } else {
        format!("one-{name}-{key}.service")
    }
}

/// Whether `program` is there to start: a person may have put another `one serve on` made there by hand.
pub fn installed(name: &str, key: &str) -> Result<bool> {
    imp::installed(&self::name(name, key))
}

/// Writes how the system runs `program`, which takes effect from its next start, where it isn't that already. Says
/// whether it changed.
pub fn install(program: &Program) -> Result<bool> {
    imp::install(program, &name(&program.name, &program.key))
}

/// Has the system start it, unless it's running.
pub fn start(program: &Program) -> Result<()> {
    imp::start(program, &name(&program.name, &program.key), false)
}

/// Has the system start it again, as it's written now: on a Mac and Windows, where it's ended at once.
pub fn restart(program: &Program) -> Result<()> {
    imp::start(program, &name(&program.name, &program.key), true)
}

pub fn stop(name: &str, key: &str) -> Result<()> {
    imp::stop(&self::name(name, key))
}

/// Has the system forget it, stopped.
pub fn remove(name: &str, key: &str) -> Result<()> {
    imp::remove(&self::name(name, key))
}

/// Writes `text` to `file` where it isn't that already; says whether it wasn't.
fn written(file: &Path, text: &[u8]) -> Result<bool> {
    if fs::read(file).is_ok_and(|was| was == text) {
        return Ok(false);
    }
    fs::create_dir_all(file.parent().context("a folder")?)?;
    fs::write(file, text)?;
    Ok(true)
}

fn ran(program: &str, args: &[&str], what: &str) -> Result<()> {
    let done = command(program).args(args).output()?;
    ensure!(done.status.success(), "{what} didn't: {}", String::from_utf8_lossy(&done.stderr).trim());
    Ok(())
}

/// A program this one runs, which has no window of its own: on Windows a console program started by one with no
/// console, as a server's is, would open one.
fn command(program: &str) -> std::process::Command {
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    command
}

#[cfg(target_os = "linux")]
mod imp {
    use std::env;

    use super::*;

    /// The display a program on its person's screen is given, where this process has one.
    const DISPLAY: [&str; 3] = ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY"];

    /// The file systemd reads the unit from, where it knows the unit: wherever it is, as a person may have put it in
    /// their own folder of units by hand, and a server's before 20 Nov is in its folder.
    fn loaded(name: &str) -> Result<Option<PathBuf>> {
        let done = command("systemctl").args(["--user", "show", "-p", "FragmentPath", "--value", name]).output()?;
        let path = String::from_utf8_lossy(&done.stdout).trim().to_owned();
        Ok((!path.is_empty()).then(|| PathBuf::from(path)))
    }

    pub fn installed(name: &str) -> Result<bool> {
        Ok(loaded(name)?.is_some())
    }

    pub fn install(program: &Program, name: &str) -> Result<bool> {
        let loaded = loaded(name)?;
        let unit = match &loaded {
            Some(unit) => unit.clone(),
            None => dirs::config_dir().context("a config folder")?.join("systemd/user").join(name),
        };
        let display = DISPLAY.iter().filter_map(|name| Some((name.to_string(), env::var(name).ok()?)));
        let env = program.env.iter().cloned().chain(display.filter(|_| program.desktop));
        let env: String =
            env.map(|(name, value)| format!("Environment={}\n", quoted(&format!("{name}={value}")))).collect();
        let (wanted, session) = match program.desktop {
            true => ("graphical-session.target", "PartOf=graphical-session.target\nAfter=graphical-session.target\n"),
            false => ("default.target", ""),
        };
        let ready = match program.start_within {
            Some(within) => format!("Type=notify\nTimeoutStartSec={}\n", within.as_secs()),
            None => String::new(),
        };
        let text = format!(
            "[Unit]\nDescription={}\nStartLimitIntervalSec=600\nStartLimitBurst=10\n{session}\n\
             [Service]\n{ready}ExecStart={}\n{env}Restart=on-failure\nRestartSec=10\n\
             KillMode=mixed\nTimeoutStopSec={}\n\n[Install]\nWantedBy={wanted}\n",
            program.description,
            program.command.iter().map(|word| quoted(word).replace('$', "$$")).collect::<Vec<_>>().join(" "),
            program.stop_within.as_secs(),
        );
        let changed = written(&unit, text.as_bytes())?;
        if loaded.is_none() {
            systemctl(&["enable", &unit.display().to_string()])?;
        }
        systemctl(&["daemon-reload"])?;
        Ok(changed || loaded.is_none())
    }

    /// One on its person's screen starts with their graphical session, unless this process has a display to give it.
    pub fn start(program: &Program, name: &str, again: bool) -> Result<()> {
        let session = || command("systemctl").args(["--user", "is-active", "graphical-session.target"]).output();
        if program.desktop && !DISPLAY.iter().any(|name| env::var_os(name).is_some()) && !session()?.status.success() {
            return Ok(());
        }
        systemctl(&["reset-failed", name]).ok();
        systemctl(&[if again { "restart" } else { "start" }, name])
    }

    pub fn stop(name: &str) -> Result<()> {
        systemctl(&["stop", name])
    }

    /// One in its person's own folder of units goes, and one in a folder of its own goes with it.
    pub fn remove(name: &str) -> Result<()> {
        let loaded = loaded(name)?;
        systemctl(&["disable", "--now", name])?;
        if let Some(unit) = loaded.filter(|unit| unit.exists()) {
            fs::remove_file(unit)?;
        }
        systemctl(&["daemon-reload"])
    }

    /// A word of a unit's, quoted, as systemd reads one, with `%` its own (and a command's `$`, `$$`).
    fn quoted(word: &str) -> String {
        format!("\"{}\"", word.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%"))
    }

    fn systemctl(args: &[&str]) -> Result<()> {
        ran("systemctl", &[&["--user"][..], args].concat(), "systemd")
    }
}

/// launchd has no limit on how long a start takes, so `start_within` is its program's own to keep.
#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    fn agent(name: &str) -> Result<PathBuf> {
        Ok(dirs::home_dir().context("a home folder")?.join(format!("Library/LaunchAgents/{name}.plist")))
    }

    fn domain() -> Result<String> {
        let user = String::from_utf8(command("id").arg("-u").output()?.stdout)?;
        Ok(format!("gui/{}", user.trim()))
    }

    pub fn installed(name: &str) -> Result<bool> {
        Ok(agent(name)?.exists())
    }

    /// launchd reads it as it loads it: at `start` and at each login.
    pub fn install(program: &Program, name: &str) -> Result<bool> {
        let text = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let strings =
            |words: &[String]| words.iter().map(|word| format!("<string>{}</string>", text(word))).collect::<String>();
        let env: String = program
            .env
            .iter()
            .map(|(name, value)| format!("<key>{}</key><string>{}</string>", text(name), text(value)))
            .collect();
        let log = text(&program.folder.join(format!("{}.log", program.name)).display().to_string());
        let desktop = match program.desktop {
            true => {
                "<key>LimitLoadToSessionType</key><string>Aqua</string><key>ProcessType</key><string>Interactive</string>"
            }
            false => "",
        };
        // The app it's of, by its bundle's identifier, which System Settings' Login Items lists it under, by that app's
        // name and icon, where it named its program's signer.
        let bundle = match program.command.first().and_then(|program| bundle_identifier(Path::new(program))) {
            Some(id) => format!("<key>AssociatedBundleIdentifiers</key><array><string>{}</string></array>", text(&id)),
            None => String::new(),
        };
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>Label</key><string>{name}</string>{bundle}\
             <key>ProgramArguments</key><array>{}</array>\
             <key>EnvironmentVariables</key><dict>{env}</dict>{desktop}\
             <key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\
             <key>ThrottleInterval</key><integer>10</integer><key>ExitTimeOut</key><integer>{}</integer>\
             <key>StandardOutPath</key><string>{log}</string><key>StandardErrorPath</key><string>{log}</string>\
             </dict></plist>\n",
            strings(&program.command),
            program.stop_within.as_secs(),
        );
        written(&agent(name)?, plist.as_bytes())
    }

    /// The identifier of the app bundle the program `program` is in, through any link to it, as `~/.local/bin/one`
    /// is to the One App's.
    fn bundle_identifier(program: &Path) -> Option<String> {
        let program = program.canonicalize().ok()?;
        let contents = program.parent().filter(|folder| folder.ends_with("Contents/MacOS"))?.parent()?;
        let info = plist::Value::from_file(contents.join("Info.plist")).ok()?;
        Some(info.as_dictionary()?.get("CFBundleIdentifier")?.as_string()?.to_owned())
    }

    pub fn start(_program: &Program, name: &str, again: bool) -> Result<()> {
        let loaded = command("launchctl").args(["print", &format!("{}/{name}", domain()?)]).output()?.status.success();
        if loaded && !again {
            return Ok(());
        }
        // One that takes long to stop on launchd's signal (Cua Driver takes the 20 s launchd lets it) is still loaded
        // after `bootout` returns, and `bootstrap` then fails (5: Input/output error), so it's killed first, with its
        // process group, as Windows' `/end` ends a task.
        if loaded {
            let _ = launchctl(&["kill", "SIGKILL", &format!("{}/{name}", domain()?)]);
            let _ = stop(name);
        }
        launchctl(&["bootstrap", &domain()?, &agent(name)?.display().to_string()])
    }

    pub fn stop(name: &str) -> Result<()> {
        launchctl(&["bootout", &format!("{}/{name}", domain()?)])
    }

    pub fn remove(name: &str) -> Result<()> {
        let _ = stop(name);
        Ok(fs::remove_file(agent(name)?)?)
    }

    fn launchctl(args: &[&str]) -> Result<()> {
        ran("launchctl", args, "launchd")
    }
}

/// Task Scheduler starts a task as its person logs on, in their session and with their environment, and restarts one
/// that fails, 10 times a minute apart. Its restarting is only for a task that couldn't start, never for a program
/// that ended non-zero, so it's also started each minute it isn't running. It has no limit on how long a start takes,
/// nor a time to stop: `/end` ends it at once.
#[cfg(windows)]
mod imp {
    use super::*;

    pub fn installed(name: &str) -> Result<bool> {
        Ok(command("schtasks").args(["/query", "/tn", name]).output()?.status.success())
    }

    /// Registers the task, where it isn't as it was registered last (`task.xml` in its folder). A task has no
    /// environment of its own, so a program that's given one is started by a batch file in its folder that sets it,
    /// in a console with no window, conhost's headless one, as cmd needs a console: conhost quotes again the
    /// arguments it's given, as C programs do, which cmd doesn't read, so the batch file is cmd's own. It's its
    /// person's by their SID, as `whoami` gives it: their account's name needs `USERDOMAIN`, which OpenSSH sets to
    /// `WORKGROUP`, and Task Scheduler finds no account by that.
    pub fn install(program: &Program, name: &str) -> Result<bool> {
        let whoami = command("whoami").args(["/user", "/fo", "csv", "/nh"]).output()?;
        let user = String::from_utf8(whoami.stdout)?;
        let user = user.trim().rsplit(',').next().map(|sid| sid.trim_matches('"').to_owned());
        let user = user.filter(|sid| sid.starts_with("S-")).context("whoami gave no SID")?;
        let text = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let windows = |path: &str| path.replace('/', "\\");
        let mut changed = false;
        let (run, arguments) = if program.env.is_empty() {
            let arguments = program.command[1..].iter().map(|word| quoted(word)).collect::<Vec<_>>().join(" ");
            (windows(&program.command[0]), arguments)
        } else {
            // In UTF-8, as a person's folder may be named in any script, with a batch file's `%` written twice.
            let batch = |text: &str| text.replace('%', "%%");
            let mut command = program.command.clone();
            command[0] = windows(&command[0]);
            let lines: Vec<String> = ["@chcp 65001 >nul".to_owned()]
                .into_iter()
                .chain(program.env.iter().map(|(name, value)| format!("@set \"{name}={}\"", batch(value))))
                .chain([format!(
                    "@{}",
                    command.iter().map(|word| format!("\"{}\"", batch(word))).collect::<Vec<_>>().join(" ")
                )])
                .collect();
            let script = program.folder.join(format!("{}.cmd", program.name));
            changed |= written(&script, (lines.join("\r\n") + "\r\n").as_bytes())?;
            (
                "conhost.exe".to_owned(),
                format!("--headless cmd.exe /d /c \"{}\"", windows(&script.display().to_string())),
            )
        };
        let task = format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>{}</Description></RegistrationInfo>
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
  <Actions Context="Author"><Exec><Command>{}</Command><Arguments>{}</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions>
</Task>
"#,
            text(&program.description),
            text(&run),
            text(&arguments),
            text(&windows(&program.folder.display().to_string())),
        );
        // Task Scheduler reads a task's XML as UTF-16.
        let bytes: Vec<u8> = [0xFEFF].into_iter().chain(task.encode_utf16()).flat_map(u16::to_le_bytes).collect();
        let registered = program.folder.join("task.xml");
        changed |= written(&registered, &bytes)?;
        let there = installed(name)?;
        if changed || !there {
            let created = schtasks(&["/create", "/tn", name, "/xml", &registered.display().to_string(), "/f"]);
            if let Err(error) = created {
                fs::remove_file(&registered)?;
                // A task an administrator registered, as one made over ssh is, only an administrator may replace.
                ensure!(
                    !there,
                    "Task Scheduler didn't replace the task \"{name}\" that's there ({error:#}). Where an administrator \
                     registered it, only one may: in PowerShell run as administrator, `schtasks /delete /tn \"{name}\" \
                     /f`, which leaves what it runs running."
                );
                return Err(error);
            }
        }
        Ok(changed)
    }

    /// A task that's running isn't started (`IgnoreNew`), so one started again is ended first.
    pub fn start(_program: &Program, name: &str, again: bool) -> Result<()> {
        if again {
            let _ = stop(name);
        }
        schtasks(&["/run", "/tn", name])
    }

    pub fn stop(name: &str) -> Result<()> {
        schtasks(&["/end", "/tn", name])
    }

    pub fn remove(name: &str) -> Result<()> {
        let _ = stop(name);
        schtasks(&["/delete", "/tn", name, "/f"])
    }

    /// An argument as a C program reads its command line (`CommandLineToArgvW`): quoted where it has a space or a
    /// quote, with a quote's backslashes, and the closing quote's, doubled.
    fn quoted(word: &str) -> String {
        if !word.is_empty() && !word.contains([' ', '\t', '"']) {
            return word.to_owned();
        }
        let mut quoted = String::from("\"");
        let mut slashes = 0;
        for c in word.chars() {
            if c == '\\' {
                slashes += 1;
                continue;
            }
            quoted.push_str(&"\\".repeat(if c == '"' { 2 * slashes + 1 } else { slashes }));
            quoted.push(c);
            slashes = 0;
        }
        quoted.push_str(&"\\".repeat(2 * slashes));
        quoted.push('"');
        quoted
    }

    fn schtasks(args: &[&str]) -> Result<()> {
        ran("schtasks", args, "Task Scheduler")
    }
}

/// A phone runs nothing for the system to keep running.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
#[rustfmt::skip]
mod imp {
    use super::*;

    const NONE: &str = "The system keeps a program running on Linux, macOS and Windows, and this device is none of them.";
    pub fn installed(_: &str) -> Result<bool> { Ok(false) }
    pub fn install(_: &Program, _: &str) -> Result<bool> { anyhow::bail!(NONE) }
    pub fn start(_: &Program, _: &str, _: bool) -> Result<()> { anyhow::bail!(NONE) }
    pub fn stop(_: &str) -> Result<()> { anyhow::bail!(NONE) }
    pub fn remove(_: &str) -> Result<()> { anyhow::bail!(NONE) }
}
