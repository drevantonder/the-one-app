//! Android phones in the Android SDK's emulator, on a Linux x86 machine with KVM or on a Mac, with no window: each an
//! AVD of the SDK's, reached with adb at its console's port. A lasting device is an AVD of its own; a copy is an AVD
//! made for one loan, from a copy of a stopped lasting device's (reflinked where the disk can), or with the lasting one
//! running, a new phone from the same system image, deleted when it's given back.
//!
//! Stopping one keeps what it was doing (the emulator's Quick Boot snapshot), and starting it carries on from there in
//! seconds, not a boot of half a minute or more; a copy of a stopped one carries on from its snapshot too. The emulator
//! boots instead when a snapshot won't load.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use super::Devices;
use super::kinds::{ADB_PORT, Action, Kind, TimedOut, Vm, capture, fetch, outliving, run, run_within, upstream_dns};
use super::linux::{cmd, found};
use super::quote;

pub struct Android;

/// Each phone's memory, whatever its AVD says (the SDK's are 4 GB): Android 36 with the Play Store boots in 2, and a
/// machine holding several phones beside builds runs out of memory at 4.
const PHONE_GB: u16 = 2;

/// In an AVD's folder, says its snapshot is what it was doing when it last stopped.
const KEPT: &str = "agent-devices-kept";

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// The Android SDK here, if any.
fn sdk() -> Option<PathBuf> {
    let dirs = [std::env::var_os("ANDROID_HOME").map(PathBuf::from), std::env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from), Some(home().join("Android/Sdk")), Some(home().join("Library/Android/sdk"))];
    dirs.into_iter().flatten().find(|d| !d.as_os_str().is_empty() && d.join("emulator/emulator").exists())
}

fn adb_path() -> PathBuf {
    sdk().unwrap_or_default().join("platform-tools/adb")
}

fn avd_home() -> PathBuf {
    std::env::var_os("ANDROID_AVD_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".android/avd"))
}

/// The AVDs here, by name.
fn avds() -> Vec<String> {
    let mut all: Vec<String> = std::fs::read_dir(avd_home())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".ini").map(str::to_string))
        .collect();
    all.sort();
    all
}

/// Runs adb for v, an error saying so if it has no answer in 10 seconds (a tap, a key, a look, an app opened).
fn adb(v: &Vm, args: &[&str]) -> Result<String> {
    adb_within(v, Duration::from_secs(10), args)
}

/// Runs adb for v, giving up on it at limit: a phone whose Android has hung (a WebView update ends the app it ran) leaves
/// adb waiting for it for as long as the caller does, which is minutes.
fn adb_within(v: &Vm, limit: Duration, args: &[&str]) -> Result<String> {
    let serial = format!("emulator-{}", v.port);
    // A call allowed long (an install, a command) is for a phone that answers: asked first, so one that doesn't is told
    // in seconds, not when the call's own limit is up.
    if limit > Duration::from_secs(10) {
        run_within(Duration::from_secs(5), adb_path(), &["-s", &serial, "shell", "true"]).map_err(|err| stopped(v, err))?;
    }
    let mut all = vec!["-s", &serial];
    all.extend(args);
    run_within(limit, adb_path(), &all).map_err(|err| stopped(v, err))
}

/// An adb error as one that says what to do, if it was for no answer.
fn stopped(v: &Vm, err: anyhow::Error) -> anyhow::Error {
    match err.downcast_ref::<TimedOut>() {
        Some(t) => anyhow!("{} stopped answering: adb had no answer in {} s. Its Android is stuck (an app or WebView update can do that): give it back and borrow another, or stop it and start it again", v.name, t.0.as_secs()),
        None => err,
    }
}

/// An AVD's folder, as its .ini says.
fn avd_dir(name: &str) -> Result<PathBuf> {
    let ini = std::fs::read_to_string(avd_home().join(format!("{name}.ini"))).map_err(|_| anyhow!("no Android Virtual Device {name} here"))?;
    let path = ini.lines().find_map(|l| l.strip_prefix("path=")).ok_or_else(|| anyhow!("{name}.ini says no path"))?;
    Ok(PathBuf::from(path.trim()))
}

/// The Android version of an AVD's system image, as its .ini names it.
fn target(dir: &Path) -> String {
    let config = std::fs::read_to_string(dir.join("config.ini")).unwrap_or_default();
    config
        .lines()
        .find_map(|l| l.strip_prefix("image.sysdir.1=system-images/"))
        .and_then(|rest| rest.split('/').next())
        .filter(|t| !t.is_empty())
        .unwrap_or("android-36")
        .to_string()
}

fn log_file(ds: &Devices, v: &Vm) -> PathBuf {
    ds.dir.join(format!("android-{}.log", v.name))
}

/// The processes running v's AVD: the emulator, and on Linux its pasta.
fn emulators(v: &Vm) -> Vec<i32> {
    let out = cmd("ps").args(["-axo", "pid=,args="]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let needle = format!(" -avd {} ", v.id);
    out.lines()
        .filter_map(|line| {
            let (pid, args) = line.trim().split_once(' ')?;
            format!("{args} ").contains(&needle).then(|| pid.parse().ok()).flatten()
        })
        .collect()
}

/// Tells adb's server of an emulator with adb at port, as the emulator itself would if it could reach the server: in
/// a network of its own it can't. The server takes an emulator it knows already as told once.
fn announce(port: u16) {
    let _ = run_within(Duration::from_secs(10), adb_path(), &["start-server"]);
    let Ok(mut c) = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], ADB_PORT).into(), Duration::from_secs(1)) else { return };
    let msg = format!("host:emulator:{port}");
    let _ = write!(c, "{:04x}{msg}", msg.len());
}

/// Sets the phone's clock to this machine's: one carrying on from a pause is behind by as long as it was paused, and
/// from a snapshot, can be.
fn set_clock(v: &Vm) {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().to_string();
    let _ = adb(v, &["shell", "cmd", "alarm", "set-time", &ms]);
}

/// A key as Android names it: enter, back, home, or KEYCODE_ anything.
fn android_key(key: &str) -> String {
    let lower = key.to_lowercase();
    let k = lower.strip_prefix("keycode_").unwrap_or(&lower).to_uppercase();
    let k = match k.as_str() {
        "RETURN" => "ENTER",
        "ESCAPE" | "ESC" => "BACK",
        "BACKSPACE" => "DEL",
        "DELETE" => "FORWARD_DEL",
        "UP" => "DPAD_UP",
        "DOWN" => "DPAD_DOWN",
        "LEFT" => "DPAD_LEFT",
        "RIGHT" => "DPAD_RIGHT",
        "RECENTS" => "APP_SWITCH",
        k => k,
    };
    format!("KEYCODE_{k}")
}

impl Kind for Android {
    fn name(&self) -> &'static str {
        "android"
    }

    fn title(&self) -> &'static str {
        "Android"
    }

    fn limits(&self) -> &'static str {
        "taps, swipes, typing and keys, and shell commands; the user watches and uses it through view"
    }

    fn input(&self) -> bool {
        true
    }

    fn port(&self) -> (u16, u16) {
        (5560, 2)
    }

    fn can(&self) -> String {
        if cfg!(target_os = "linux") {
            if !cfg!(target_arch = "x86_64") {
                return "Android runs on Linux x86 machines with KVM, and on Macs".into();
            }
            if std::fs::OpenOptions::new().read(true).write(true).open("/dev/kvm").is_err() {
                return "Android needs KVM (/dev/kvm) here".into();
            }
            // Without its own network, a phone's apps would reach this machine's 127.0.0.1 and adb (start).
            if !found("pasta") {
                return "Android needs pasta here (the passt package), for a network of its own".into();
            }
        } else if !cfg!(target_os = "macos") {
            return "Android runs on Linux x86 machines with KVM, and on Macs".into();
        }
        if sdk().is_none() {
            return "Android needs the Android SDK's emulator here (ANDROID_HOME)".into();
        }
        if avds().is_empty() {
            return "Android needs an Android Virtual Device here to start from".into();
        }
        String::new()
    }

    fn adopt(&self, _: &Devices, v: &mut Vm) -> Result<()> {
        if avd_dir(&v.id).is_err() || v.port == 0 {
            bail!("give id (its AVD's name) and port (its console port, like 5554)");
        }
        Ok(())
    }

    fn make(&self, ds: &Devices, v: &mut Vm, from: Option<&Vm>) -> Result<()> {
        v.id = format!("agent-devices-{}", v.name);
        v.gb = PHONE_GB;
        let src = match from {
            Some(from) => avd_dir(&from.id)?,
            None => avd_dir(avds().first().ok_or_else(|| anyhow!("there's no Android Virtual Device here to start from"))?)?,
        };
        let dst = avd_home().join(format!("{}.avd", v.id));
        // The machine's AVDs are every network's (a `one-dev throwaway` beside the real one): never another's.
        if dst.exists() || avd_home().join(format!("{}.ini", v.id)).exists() {
            bail!("there's an Android device called {} on this machine already: pick another name", v.name);
        }
        // Stopped, and done saving its snapshot, which it does after adb stops answering.
        match from.filter(|from| !self.running(ds, from) && emulators(from).is_empty()) {
            Some(from) => {
                run("cp", &["-a", "--reflink=auto", &src.to_string_lossy(), &dst.to_string_lossy()])?;
                remove_locks(&dst);
                // Its snapshots name the AVD they're of, which the emulator loads them only into: now this one.
                for snap in std::fs::read_dir(dst.join("snapshots")).into_iter().flatten().flatten() {
                    let f = snap.path().join("hardware.ini");
                    let Ok(ini) = std::fs::read_to_string(&f) else { continue };
                    let ini = ini.replace(&format!("/{}.avd/", from.id), &format!("/{}.avd/", v.id));
                    let ini: Vec<String> = ini
                        .split('\n')
                        .map(|l| {
                            for key in ["avd.name = ", "avd.id = "] {
                                if l.strip_prefix(key) == Some(from.id.as_str()) {
                                    return format!("{key}{}", v.id);
                                }
                            }
                            l.to_string()
                        })
                        .collect();
                    let _ = std::fs::write(&f, ini.join("\n"));
                }
            }
            None => {
                std::fs::create_dir_all(&dst)?;
                run("cp", &[&src.join("config.ini").to_string_lossy(), &dst.join("config.ini").to_string_lossy()])?;
            }
        }
        let ini = format!("avd.ini.encoding=UTF-8\npath={}\npath.rel=avd/{}.avd\ntarget={}\n", dst.display(), v.id, target(&src));
        std::fs::write(avd_home().join(format!("{}.ini", v.id)), ini)?;
        Ok(())
    }

    fn running(&self, _: &Devices, v: &Vm) -> bool {
        adb_within(v, Duration::from_secs(5), &["get-state"]).is_ok_and(|out| out.trim() == "device")
    }

    fn start(&self, ds: &Devices, v: &Vm) -> Result<()> {
        // Its snapshot only if its last stop kept it: loading one also takes its disk back to then, losing what it did
        // since if it ended any other way.
        let mut saved = false;
        if let Ok(dir) = avd_dir(&v.id) {
            saved = dir.join(KEPT).exists();
            let _ = std::fs::remove_file(dir.join(KEPT));
        }
        let booted = self.boot(ds, v, saved);
        if let Err(err) = &booted
            && saved
        {
            // A snapshot that loads but never finishes starting: it boots instead.
            tracing::warn!("{} didn't carry on where it stopped, so it boots: {err:#}", v.name);
            self.kill(v);
            return self.boot(ds, v, false);
        }
        booted
    }

    /// Ends the emulator, which keeps what the phone was doing as it goes, a few seconds' work it's waited for: a copy
    /// taken before it's done would be half of one.
    fn stop(&self, ds: &Devices, v: &Vm) -> Result<()> {
        if !self.running(ds, v) {
            self.kill(v); // starting, or stuck: nothing to keep
            return Ok(());
        }
        adb(v, &["emu", "kill"])?;
        if super::wait_for(Duration::from_secs(120), Duration::from_millis(200), || emulators(v).is_empty()) {
            if let Ok(dir) = avd_dir(&v.id)
                && std::fs::read_to_string(log_file(ds, v)).unwrap_or_default().contains("Saving snapshot 'default_boot' took")
            {
                let _ = std::fs::write(dir.join(KEPT), b"");
            }
            return Ok(());
        }
        self.kill(v);
        bail!("{} took over 2 minutes to stop, so it was ended: it boots at its next start", v.name)
    }

    fn pause(&self, _: &Devices, v: &Vm) -> Result<()> {
        adb(v, &["emu", "avd", "pause"]).map(|_| ())
    }

    fn resume(&self, _: &Devices, v: &Vm) -> Result<()> {
        adb(v, &["emu", "avd", "resume"])?;
        set_clock(v);
        Ok(())
    }

    fn remove(&self, ds: &Devices, v: &Vm) -> Result<()> {
        self.kill(v);
        if let Ok(dir) = avd_dir(&v.id) {
            let _ = std::fs::remove_dir_all(dir);
        }
        let _ = std::fs::remove_file(avd_home().join(format!("{}.ini", v.id)));
        let _ = std::fs::remove_file(log_file(ds, v));
        let _ = std::fs::remove_dir_all(ds.dir.join(format!("android-{}.run", v.name)));
        Ok(())
    }

    fn screen(&self, _: &Devices, v: &Vm) -> Result<image::DynamicImage> {
        let mut c = cmd(adb_path());
        c.args(["-s", &format!("emulator-{}", v.port), "exec-out", "screencap", "-p"]);
        let (status, out, _) = capture(c, None, Some(Duration::from_secs(10))).map_err(|err| stopped(v, err))?;
        if !status.success() {
            bail!("screencap: {status}");
        }
        super::picture::decode(&out)
    }

    fn act(&self, _: &Devices, v: &Vm, a: &Action) -> Result<()> {
        let n = |f: f64| (f as i64).to_string();
        let (x, y, tx, ty) = (n(a.x), n(a.y), n(a.to_x), n(a.to_y));
        match a.r#do.as_str() {
            "click" => adb(v, &["shell", "input", "tap", &x, &y]),
            "double_click" => {
                let _ = adb(v, &["shell", "input", "tap", &x, &y]);
                adb(v, &["shell", "input", "tap", &x, &y])
            }
            "right_click" => adb(v, &["shell", "input", "swipe", &x, &y, &x, &y, "800"]),
            "scroll" | "swipe" => adb(v, &["shell", "input", "swipe", &x, &y, &tx, &ty, "300"]),
            // input text takes no spaces, and the device's shell reads the rest.
            "type" => adb(v, &["shell", "input", "text", &quote(&a.text.replace(' ', "%s"))]),
            "key" => adb(v, &["shell", "input", "keyevent", &android_key(&a.key)]),
            other => bail!("no action {other:?}"),
        }
        .map(|_| ())
    }

    fn open(&self, _: &Devices, v: &Vm, url: &str) -> Result<()> {
        adb(v, &["shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", &quote(url)]).map(|_| ())
    }

    fn install(&self, _: &Devices, v: &Vm, app: &str) -> Result<String> {
        if app.is_empty() {
            bail!("install what? an .apk file here or its URL");
        }
        if !app.contains('/') {
            // A package: only the Play Store installs it, signed in, by the user or an agent tapping Install.
            adb(v, &["shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", &quote(&format!("market://details?id={app}"))])?;
            return Ok("Opened its page in the Play Store: tap Install there (it needs a Google account signed in).".into());
        }
        let (path, temp) = fetch(app)?;
        let done = adb_within(v, Duration::from_secs(300), &["install", "-r", "-g", &path.to_string_lossy()]);
        if let Some(dir) = temp {
            let _ = std::fs::remove_dir_all(dir);
        }
        done
    }

    fn reach(&self, _: &Devices, v: &Vm, port: u16) -> Result<String> {
        let p = format!("tcp:{port}");
        adb(v, &["reverse", &p, &p])?;
        Ok(format!("{} reaches this machine's port {port} at 127.0.0.1:{port}.", v.name))
    }

    fn run(&self, _: &Devices, v: &Vm, command: &str) -> Result<String> {
        adb_within(v, Duration::from_secs(120), &["shell", command])
    }
}

/// The locks an emulator leaves, which would keep the next from starting.
fn remove_locks(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().ends_with(".lock") {
            let p = e.path();
            let _ = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
        }
    }
}

impl Android {
    /// Starts the emulator, carrying on from its snapshot with saved, and waits for Android to have started.
    fn boot(&self, ds: &Devices, v: &Vm, saved: bool) -> Result<()> {
        self.kill(v); // one left starting, or stuck, which adb doesn't answer for: the emulator runs one at a time
        let sdk = sdk().ok_or_else(|| anyhow!("Android needs the Android SDK's emulator here (ANDROID_HOME)"))?;
        let out = std::fs::File::create(log_file(ds, v))?;
        let (port, memory) = (v.port.to_string(), (PHONE_GB as u32 * 1024).to_string());
        let emulator = sdk.join("emulator/emulator").to_string_lossy().to_string();
        let mut args: Vec<String> = [emulator.as_str(), "-avd", &v.id, "-port", &port, "-no-window", "-gpu", "swiftshader_indirect", "-accel", "on", "-no-audio", "-no-boot-anim", "-memory", &memory].map(String::from).to_vec();
        if !saved {
            args.push("-no-snapshot-load".into());
        }
        let limit = Duration::from_secs(if saved { 120 } else { 300 });
        // The emulator's own network puts this machine's 127.0.0.1 at 10.0.2.2 for any app in the phone, and has no way
        // not to. On Linux it runs in a network of its own (pasta), reaching the internet as this machine does but none
        // of its 127.0.0.1, which takes only its console and adb's port in; and asks this machine's resolvers, not its
        // stub. A Mac has nothing like it, so there a phone's apps reach the Mac's 127.0.0.1 (ARCHITECTURE.md).
        let apart = cfg!(target_os = "linux");
        if apart {
            let ports = format!("127.0.0.1/{}-{}", v.port, v.port + 1);
            let mut pasta: Vec<String> = ["pasta", "--config-net", "--no-map-gw", "-T", "none", "-U", "none", "-u", "none", "-t", &ports, "--quiet", "--"].map(String::from).to_vec();
            pasta.extend(args);
            args = pasta;
            let dns = upstream_dns();
            if !dns.is_empty() {
                args.extend(["-dns-server".into(), dns.join(",")]);
            }
        }
        // Its own runtime folder, for a netsimd (its radios) of its own: one an emulator in another network started is
        // unreachable here, and a phone that waits for it never starts.
        let run_dir = ds.dir.join(format!("android-{}.run", v.name));
        std::fs::create_dir_all(&run_dir)?;
        let mut all = vec!["env".to_string(), format!("XDG_RUNTIME_DIR={}", run_dir.display())];
        all.extend(args);
        let (program, rest) = outliving(&all[0], &all[1..].iter().map(String::as_str).collect::<Vec<_>>());
        let mut c = cmd(&program);
        c.args(&rest).env("ANDROID_HOME", &sdk).env("ANDROID_SDK_ROOT", &sdk).stdin(Stdio::null()).stdout(out.try_clone()?).stderr(out);
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: only an async-signal-safe call, between fork and exec.
            unsafe {
                c.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }
        let mut child = c.spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        let booted = super::wait_for(limit, Duration::from_millis(500), || {
            if apart {
                announce(v.port + 1);
            }
            adb_within(v, Duration::from_secs(5), &["shell", "getprop", "sys.boot_completed"]).is_ok_and(|out| out.trim() == "1")
        });
        if !booted {
            bail!("{} didn't start in {} minutes: {}", v.name, limit.as_secs() / 60, log_file(ds, v).display());
        }
        let _ = adb(v, &["shell", "svc", "power", "stayon", "true"]);
        let _ = adb(v, &["shell", "input", "keyevent", "KEYCODE_WAKEUP"]);
        let _ = adb(v, &["shell", "wm", "dismiss-keyguard"]);
        set_clock(v);
        Ok(())
    }

    /// Ends v's emulator at once, keeping nothing, and the locks it leaves, which would keep the next from starting.
    fn kill(&self, v: &Vm) {
        for pid in emulators(v) {
            // SAFETY: a signal to the emulator's own processes.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        super::wait_for(Duration::from_secs(5), Duration::from_millis(100), || emulators(v).is_empty());
        if let Ok(dir) = avd_dir(&v.id) {
            remove_locks(&dir);
        }
    }
}
