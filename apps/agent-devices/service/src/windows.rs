//! Windows 11 in QEMU on a Linux x86 machine with KVM: UEFI Secure Boot (OVMF) and a TPM (swtpm), as Windows 11
//! needs. A device's folder has its disk, its UEFI variables and its TPM; the machine reaches it through QEMU's monitor
//! (pictures, mouse and keys), and through SSH forwarded to a port here (commands), as the Windows account one that
//! trusts this machine's key. Its network is passt's, which reaches the internet as this machine does but none of this
//! machine's own 127.0.0.1, where QEMU's own would put it at 10.0.2.2 for any program in Windows to reach what only this
//! machine's programs use; only its SSH comes in. A copy is a disk on top of a lasting device's base: a copy of its
//! disk taken when it's copied, kept for the next copy until the lasting device next stops, and for each copy on it
//! until it's gone.
//!
//! Stopping one keeps what it was doing (its memory and devices, as QEMU migrates them, the TPM's state too) in its
//! folder, beside the disk as it was then, and starting it carries on from there in seconds, not a boot of a minute or
//! more; a copy of a stopped device starts from its base's. Its disk is on AHCI, not NVMe, which QEMU can't migrate.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

use super::Devices;
use super::kinds::{Action, Kind, Vm, fetch, outliving, run, run_in, run_outliving, upstream_dns};
use super::linux::{cmd, found};

pub struct Windows;

const OVMF_CODE: &str = "/usr/share/OVMF/OVMF_CODE_4M.secboot.fd";
const WINDOWS_USER: &str = "one";

/// A device's bases: what its copies are made on, each a folder with a copy of its disk, UEFI variables and TPM (and
/// its saved state, if it was stopped), taken when it's first copied and used for each copy after until it next stops.
/// bases/current names the one copies are made on now; another is deleted once no copy is on it (`prune_bases`), never
/// written again, so a copy's base stays as it was made on.
fn bases_of(from: &Vm) -> PathBuf {
    from.file("bases")
}

/// There's no saved state to carry on from.
#[derive(Debug)]
struct NoState;

impl std::fmt::Display for NoState {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("no saved state")
    }
}

impl std::error::Error for NoState {}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

impl Kind for Windows {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn title(&self) -> &'static str {
        "Windows"
    }

    fn limits(&self) -> &'static str {
        "mouse, keyboard and commands; the user watches and uses it through view"
    }

    fn input(&self) -> bool {
        true
    }

    fn port(&self) -> (u16, u16) {
        (22220, 1)
    }

    fn can(&self) -> String {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return "Windows runs on Linux x86 machines with KVM".into();
        }
        if std::fs::OpenOptions::new().read(true).write(true).open("/dev/kvm").is_err() {
            return "Windows needs KVM (/dev/kvm) here".into();
        }
        for c in ["qemu-system-x86_64", "qemu-img", "swtpm", "passt"] {
            if !found(c) {
                return format!("Windows needs {c} here");
            }
        }
        if !Path::new(OVMF_CODE).exists() {
            return format!("Windows needs OVMF with Secure Boot here ({OVMF_CODE})");
        }
        String::new()
    }

    fn adopt(&self, _: &Devices, v: &mut Vm) -> Result<()> {
        if !v.file("disk.qcow2").exists() || v.port == 0 || v.display == 0 {
            return Err(one_service::refused(format!("give dir (its folder, with its disk.qcow2, OVMF_VARS.fd, tpm and monitor.sock), port (its SSH, forwarded here) and display (its VNC display)")));
        }
        Ok(())
    }

    fn make(&self, ds: &Devices, v: &mut Vm, from: Option<&Vm>) -> Result<()> {
        let Some(from) = from else { bail!("a new Windows device is made from scratch only by installing Windows, which isn't built yet: borrow a copy of one instead") };
        v.dir = ds.dir.join("windows").join(&v.name).to_string_lossy().to_string();
        v.display = v.port - 22220 + 10;
        (v.cpus, v.gb) = (from.cpus, from.gb);
        std::fs::create_dir_all(&v.dir)?;
        let made = (|| -> Result<()> {
            let base = self.base(ds, from)?;
            v.base = base.to_string_lossy().to_string();
            let disk = v.file("disk.qcow2");
            run("qemu-img", &["create", "-q", "-f", "qcow2", "-b", &base.join("disk.qcow2").to_string_lossy(), "-F", "qcow2", &disk.to_string_lossy()])?;
            run("cp", &[&base.join("vars.fd").to_string_lossy(), &v.file("OVMF_VARS.fd").to_string_lossy()])?;
            run("cp", &["-a", &base.join("tpm").to_string_lossy(), &v.file("tpm").to_string_lossy()])?;
            // It starts from the base's saved state, if it has one: a link, which its start removes, leaving the base's.
            if let Ok(id) = std::fs::read(base.join("state.id"))
                && std::fs::hard_link(base.join("state"), v.file("state")).is_ok()
            {
                let _ = std::fs::write(v.file("state.id"), id);
            }
            Ok(())
        })();
        if made.is_err() {
            let _ = std::fs::remove_dir_all(&v.dir);
        }
        made
    }

    fn running(&self, _: &Devices, v: &Vm) -> bool {
        let pid: i32 = read(&v.file("qemu.pid")).trim().parse().unwrap_or(0);
        // SAFETY: signal 0 only checks the process is there.
        pid > 0 && unsafe { libc::kill(pid, 0) } == 0
    }

    /// Carries on from where it was stopped, or boots it if it can't.
    fn start(&self, ds: &Devices, v: &Vm) -> Result<()> {
        match self.carry_on(ds, v) {
            Ok(()) => return Ok(()),
            Err(err) if err.downcast_ref::<NoState>().is_some() => {}
            Err(err) => tracing::warn!("{} didn't carry on where it stopped, so it boots: {err:#}", v.name),
        }
        self.launch(v, None)?;
        if !super::wait_for(Duration::from_secs(240), Duration::from_secs(5), || self.ssh(v, "echo ready").is_ok()) {
            bail!("{} started but didn't answer in 4 minutes: look at it with screen", v.name);
        }
        Ok(())
    }

    /// Keeps what it's doing and ends it, or if that fails, shuts Windows down, as its power button would, and pulls
    /// the plug if it hasn't in 3 minutes. Its base is taken again at the next copy.
    fn stop(&self, ds: &Devices, v: &Vm) -> Result<()> {
        if !self.running(ds, v) {
            return Ok(());
        }
        if v.of.is_empty() {
            // Copies made from now on are of it as it is next; those made on its current base stay on that.
            let _ = std::fs::remove_file(bases_of(v).join("current"));
        }
        let Err(err) = self.save(ds, v) else { return Ok(()) };
        tracing::warn!("{} shuts down, as saving what it's doing failed: {err:#}", v.name);
        self.forget(v);
        let _ = monitor(v, "system_powerdown");
        super::wait_for(Duration::from_secs(180), Duration::from_secs(2), || !self.running(ds, v));
        if self.running(ds, v) {
            let _ = monitor(v, "quit");
        }
        Ok(())
    }

    fn pause(&self, _: &Devices, v: &Vm) -> Result<()> {
        monitor(v, "stop").map(|_| ())
    }

    fn resume(&self, _: &Devices, v: &Vm) -> Result<()> {
        monitor(v, "cont")?;
        let v = v.clone();
        std::thread::spawn(move || set_clock(&v));
        Ok(())
    }

    fn remove(&self, ds: &Devices, v: &Vm) -> Result<()> {
        if self.running(ds, v) {
            let _ = monitor(v, "quit");
            std::thread::sleep(Duration::from_secs(1));
        }
        match std::fs::remove_dir_all(&v.dir) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
            _ => Ok(()),
        }
    }

    fn screen(&self, _: &Devices, v: &Vm) -> Result<image::DynamicImage> {
        let file = std::env::temp_dir().join(format!("agent-devices-screen-{}.png", super::random_hex(6)));
        let shot = monitor(v, &format!("screendump {} -f png", file.display())).and_then(|_| super::picture::decode_file(&file));
        let _ = std::fs::remove_file(&file);
        shot
    }

    fn act(&self, _: &Devices, v: &Vm, a: &Action) -> Result<()> {
        super::viewer::vnc_act(&v.file("vnc.sock"), a)
    }

    fn open(&self, _: &Devices, v: &Vm, url: &str) -> Result<()> {
        // Windows shows nothing an SSH session starts, so a task signed in as the user's account opens it.
        self.ssh(
            v,
            &format!(
                "$a = New-ScheduledTaskAction -Execute 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe' -Argument '--start-maximized {}'\n$p = New-ScheduledTaskPrincipal -UserId {WINDOWS_USER} -LogonType Interactive\nRegister-ScheduledTask -TaskName agent-devices-open -Action $a -Principal $p -Force | Out-Null\nStart-ScheduledTask -TaskName agent-devices-open",
                ps_quote(url)
            ),
        )
        .map(|_| ())
    }

    fn install(&self, _: &Devices, v: &Vm, app: &str) -> Result<String> {
        if app.is_empty() {
            bail!("install what? a file, a URL or a winget id");
        }
        if !app.contains(['/', '\\']) {
            return self.ssh(v, &format!("winget install --silent --accept-package-agreements --accept-source-agreements --id {}", ps_quote(app)));
        }
        let (path, temp) = fetch(app)?;
        let done = (|| -> Result<String> {
            let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let dest = format!("C:/Users/{WINDOWS_USER}/Downloads/{file}");
            let mut args = ssh_args(v, "-P");
            args.extend([path.to_string_lossy().to_string(), format!("{WINDOWS_USER}@127.0.0.1:{dest}")]);
            run("scp", &args.iter().map(String::as_str).collect::<Vec<_>>())?;
            match Path::new(&file).extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
                Some("msi") => self.ssh(v, &format!("Start-Process msiexec -Wait -ArgumentList '/i','{dest}','/qn'; 'Installed.'")),
                Some("msix" | "msixbundle" | "appx") => self.ssh(v, &format!("Add-AppxPackage -Path '{dest}'; 'Installed.'")),
                _ => self.ssh(v, &format!("Start-Process -Wait '{dest}' -ArgumentList '/S'; 'Ran its installer with /S.'")),
            }
        })();
        if let Some(dir) = temp {
            let _ = std::fs::remove_dir_all(dir);
        }
        done
    }

    fn run(&self, _: &Devices, v: &Vm, command: &str) -> Result<String> {
        self.ssh(v, command)
    }

    /// Forwards the port inside Windows to this machine's own, over SSH from here, until Windows stops.
    fn reach(&self, _: &Devices, v: &Vm, port: u16) -> Result<String> {
        let reached = format!("{} reaches this machine's port {port} at 127.0.0.1:{port}.", v.name);
        // A port proxy reach once left there, to QEMU's 10.0.2.2, which is nothing now, holds the port: it goes.
        let out = self.ssh(
            v,
            &format!(
                "netsh interface portproxy delete v4tov4 listenaddress=127.0.0.1 listenport={port} | Out-Null; (Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort {port} -State Listen -ErrorAction SilentlyContinue | % {{ (Get-Process -Id $_.OwningProcess).Name }}) -join ','"
            ),
        )?;
        match out.trim() {
            "" => {}
            "sshd" => return Ok(reached),
            other => bail!("{} has {other} on its port {port} already", v.name),
        }
        let mut args = ssh_args(v, "-p");
        args.extend(["-f", "-N", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=15", "-R"].map(String::from));
        args.extend([format!("127.0.0.1:{port}:127.0.0.1:{port}"), format!("{WINDOWS_USER}@127.0.0.1")]);
        let (p, a) = outliving("ssh", &args.iter().map(String::as_str).collect::<Vec<_>>());
        run(p, &a.iter().map(String::as_str).collect::<Vec<_>>())?;
        Ok(reached)
    }
}

impl Windows {
    /// The lasting device's current base, which copies are made on: taken now if it has none. A running device's disk
    /// is copied by QEMU as it runs, as a sudden power cut would leave it, which a throwaway copy recovers from.
    fn base(&self, ds: &Devices, from: &Vm) -> Result<PathBuf> {
        let bases = bases_of(from);
        let current = read(&bases.join("current"));
        if !current.trim().is_empty() {
            let dir = bases.join(current.trim());
            if dir.join("disk.qcow2").exists() {
                return Ok(dir);
            }
        }
        let generation = base36(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        let dir = bases.join(&generation);
        std::fs::create_dir_all(&dir)?;
        let taken = (|| -> Result<()> {
            let tmp = dir.join("disk.qcow2.part");
            if self.running(ds, from) {
                monitor(from, &format!("drive_backup -f d0 {} qcow2", tmp.display()))?;
                loop {
                    if monitor(from, "info block-jobs")?.contains("No active jobs") {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
                if !tmp.exists() {
                    bail!("copying {}'s disk failed", from.name);
                }
            } else {
                run("cp", &["--reflink=auto", &from.file("disk.qcow2").to_string_lossy(), &tmp.to_string_lossy()])?;
                if let Ok(id) = std::fs::read(from.file("state.id"))
                    && std::fs::hard_link(from.file("state"), dir.join("state")).is_ok()
                {
                    let _ = std::fs::write(dir.join("state.id"), id);
                }
            }
            run("cp", &[&from.file("OVMF_VARS.fd").to_string_lossy(), &dir.join("vars.fd").to_string_lossy()])?;
            run("cp", &["-a", &from.file("tpm").to_string_lossy(), &dir.join("tpm").to_string_lossy()])?;
            let _ = std::fs::remove_file(dir.join("tpm").join("sock"));
            std::fs::rename(&tmp, dir.join("disk.qcow2"))?;
            std::fs::write(bases.join("current.part"), &generation)?;
            std::fs::rename(bases.join("current.part"), bases.join("current"))?;
            Ok(())
        })();
        if let Err(err) = taken {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(err);
        }
        Ok(dir)
    }

    /// Starts it from its saved state, which goes once loaded: Windows changes its disk from then on.
    fn carry_on(&self, ds: &Devices, v: &Vm) -> Result<()> {
        let state = v.file("state");
        if !state.exists() {
            return Err(NoState.into());
        }
        let done = (|| -> Result<()> {
            let id = read(&v.file("state.id"));
            let want = self.machine(v);
            if want.is_empty() || id != want {
                bail!("it was saved by another QEMU, or at another size");
            }
            if let Err(err) = self.launch(v, Some(&state)) {
                self.halt(ds, v);
                return Err(err);
            }
            let start = Instant::now();
            loop {
                let status = monitor(v, "info status");
                if status.is_err() || !self.running(ds, v) {
                    self.halt(ds, v);
                    bail!("loading its state: {}", status.err().map(|e| format!("{e:#}")).unwrap_or_else(|| "it stopped".into()));
                }
                if !status.unwrap_or_default().contains("inmigrate") {
                    break;
                }
                if start.elapsed() > Duration::from_secs(120) {
                    self.halt(ds, v);
                    bail!("loading its state took over 2 minutes");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            self.forget(v);
            if let Err(err) = monitor(v, "cont") {
                self.halt(ds, v);
                return Err(err);
            }
            // Its SSH server answering is enough: a command over SSH takes seconds more, Windows starting PowerShell.
            if !greets(v, Duration::from_secs(60)) {
                self.halt(ds, v);
                bail!("it carried on but its SSH didn't answer in a minute");
            }
            let v = v.clone();
            std::thread::spawn(move || set_clock(&v));
            Ok(())
        })();
        self.forget(v);
        done
    }

    /// Deletes its saved state.
    fn forget(&self, v: &Vm) {
        let _ = std::fs::remove_file(v.file("state"));
        let _ = std::fs::remove_file(v.file("state.id"));
    }

    /// Starts its TPM, its network and QEMU, booting it, or with incoming, loading that saved state and waiting.
    fn launch(&self, v: &Vm, incoming: Option<&Path>) -> Result<()> {
        std::fs::create_dir_all(v.file("tpm"))?;
        let f = |name: &str| v.file(name).to_string_lossy().to_string();
        run_outliving("swtpm", &["socket", "--tpm2", "--tpmstate", &format!("dir={}", f("tpm")), "--ctrl", &format!("type=unixio,path={}", f("tpm/sock")), "--terminate", "--daemon", "--log", &format!("file={}", f("tpm/log"))])?;
        // One passt for each start, which ends when QEMU does; one left by a start that failed goes first, with its port.
        let pid: i32 = read(&v.file("passt.pid")).trim().parse().unwrap_or(0);
        if pid > 0 {
            // SAFETY: a signal to the passt this left.
            unsafe { libc::kill(pid, libc::SIGTERM) };
            std::thread::sleep(Duration::from_secs(1));
        }
        let sock = f("passt.sock");
        let _ = std::fs::remove_file(&sock);
        let mut args = vec!["--one-off".to_string(), "--no-map-gw".into(), "--socket".into(), sock, "--pid".into(), f("passt.pid"), "-t".into(), format!("127.0.0.1/{}:22", v.port), "--quiet".into()];
        for dns in upstream_dns() {
            args.extend(["-D".into(), dns]);
        }
        run_outliving("passt", &args.iter().map(String::as_str).collect::<Vec<_>>())?;
        let mut qemu = vec!["-name".to_string(), format!("agent-devices-{}", v.name), "-daemonize".into(), "-pidfile".into(), f("qemu.pid")];
        qemu.extend(hardware(v));
        if let Some(incoming) = incoming {
            qemu.extend(["-incoming".into(), format!("file:{}", incoming.display()), "-S".into()]);
        }
        run_outliving("qemu-system-x86_64", &qemu.iter().map(String::as_str).collect::<Vec<_>>())?;
        Ok(())
    }

    /// Names QEMU's version and the machine it makes for v, wherever its folder is: a saved state loads only into the
    /// same.
    fn machine(&self, v: &Vm) -> String {
        use sha2::Digest;
        let Ok(version) = run("qemu-system-x86_64", &["--version"]) else { return String::new() };
        let version = version.split('\n').next().unwrap_or_default();
        let hw = hardware(v).join("\x00").replace(&v.dir, "");
        data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(format!("{version}\x00{hw}").as_bytes()))
    }

    /// Ends its QEMU at once, which ends its TPM and network.
    fn halt(&self, ds: &Devices, v: &Vm) {
        let _ = monitor(v, "quit");
        super::wait_for(Duration::from_secs(5), Duration::from_millis(100), || !self.running(ds, v));
    }

    /// Keeps what it's doing in its folder, for its next start to carry on from, and ends it.
    fn save(&self, ds: &Devices, v: &Vm) -> Result<()> {
        let part = v.file("state.part");
        let _ = std::fs::remove_file(&part);
        monitor(v, "stop")?;
        let fail = |err: anyhow::Error| -> Result<()> {
            let _ = monitor(v, "migrate_cancel");
            let _ = monitor(v, "cont");
            let _ = std::fs::remove_file(&part);
            Err(err)
        };
        let _ = monitor(v, "migrate_set_parameter max-bandwidth 100G");
        if let Err(err) = monitor(v, &format!("migrate -d file:{}", part.display())) {
            return fail(err);
        }
        let start = Instant::now();
        loop {
            let out = match monitor(v, "info migrate") {
                Ok(out) => out,
                Err(err) => return fail(err),
            };
            if out.contains("Migration status: completed") {
                break;
            }
            if !out.contains("Migration status: active") && !out.contains("Migration status: setup") || start.elapsed() > Duration::from_secs(180) {
                return fail(anyhow!("saving it: {out}"));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        if let Err(err) = std::fs::write(v.file("state.id"), self.machine(v)) {
            return fail(err.into());
        }
        if let Err(err) = std::fs::rename(&part, v.file("state")) {
            return fail(err.into());
        }
        self.halt(ds, v);
        Ok(())
    }

    /// Runs a PowerShell command in the device, as its account.
    fn ssh(&self, v: &Vm, command: &str) -> Result<String> {
        let mut args = ssh_args(v, "-p");
        args.extend([format!("{WINDOWS_USER}@127.0.0.1"), "powershell -NoProfile -NonInteractive -Command -".into()]);
        run_in(Some(command.as_bytes()), "ssh", &args.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

/// The machine QEMU makes for it, which a saved state must be for.
fn hardware(v: &Vm) -> Vec<String> {
    let (cpus, gb) = (v.cpus.max(2), v.gb.max(4));
    let f = |name: &str| v.file(name).to_string_lossy().to_string();
    [
        "-machine".into(),
        "q35,smm=on,accel=kvm".into(),
        "-global".into(),
        "driver=cfi.pflash01,property=secure,value=on".into(),
        "-cpu".into(),
        "host,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_time".into(),
        "-smp".into(),
        format!("{cpus},sockets=1,cores={cpus},threads=1"),
        "-m".into(),
        format!("{gb}G"),
        "-drive".into(),
        format!("if=pflash,format=raw,unit=0,file={OVMF_CODE},readonly=on"),
        "-drive".into(),
        format!("if=pflash,format=raw,unit=1,file={}", f("OVMF_VARS.fd")),
        "-chardev".into(),
        format!("socket,id=chrtpm,path={}", f("tpm/sock")),
        "-tpmdev".into(),
        "emulator,id=tpm0,chardev=chrtpm".into(),
        "-device".into(),
        "tpm-crb,tpmdev=tpm0".into(),
        "-drive".into(),
        format!("file={},if=none,id=d0,discard=unmap", f("disk.qcow2")),
        "-device".into(),
        "ide-hd,drive=d0,bus=ide.0,serial=one,bootindex=0".into(),
        "-netdev".into(),
        format!("stream,id=n0,server=off,addr.type=unix,addr.path={}", f("passt.sock")),
        "-device".into(),
        "e1000e,netdev=n0".into(),
        "-device".into(),
        "qemu-xhci".into(),
        "-device".into(),
        "usb-tablet".into(),
        "-vga".into(),
        "std".into(),
        "-rtc".into(),
        "base=utc,clock=host".into(),
        "-audio".into(),
        "none".into(),
        "-display".into(),
        "none".into(),
        "-vnc".into(),
        format!("unix:{}", f("vnc.sock")),
        "-monitor".into(),
        format!("unix:{},server,nowait", f("monitor.sock")),
    ]
    .to_vec()
}

fn base36(mut n: u128) -> String {
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while n > 0 {
        out.push(digits[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

fn ssh_args(v: &Vm, port_flag: &str) -> Vec<String> {
    vec![
        port_flag.into(),
        v.port.to_string(),
        "-o".into(),
        "StrictHostKeyChecking=no".into(),
        "-o".into(),
        format!("UserKnownHostsFile={}", v.file("known_hosts").display()),
        "-o".into(),
        "LogLevel=ERROR".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
    ]
}

/// Waits for its SSH server's greeting.
fn greets(v: &Vm, limit: Duration) -> bool {
    super::wait_for(limit, Duration::from_millis(100), || {
        let Ok(mut c) = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], v.port).into(), Duration::from_secs(1)) else { return false };
        let _ = c.set_read_timeout(Some(Duration::from_secs(2)));
        let mut b = [0u8; 4];
        c.read_exact(&mut b).is_ok() && &b == b"SSH-"
    })
}

/// Sets Windows' clock to this machine's: one carrying on from a pause or a saved state is behind by as long as it was
/// stopped. It takes seconds, which nothing waits for: the time goes once PowerShell asks for it.
fn set_clock(v: &Vm) {
    let script = "[Console]::Out.WriteLine('time?'); [Console]::Out.Flush(); Set-Date -Date ([DateTimeOffset]::FromUnixTimeMilliseconds([long]@($input)[0]).LocalDateTime) | Out-Null";
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut args = ssh_args(v, "-p");
    args.extend([format!("{WINDOWS_USER}@127.0.0.1"), format!("powershell -NoProfile -NonInteractive -EncodedCommand {}", data_encoding::BASE64.encode(&utf16))]);
    let set = (|| -> Result<()> {
        let mut child = cmd("ssh").args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line)?;
        let mut stdin = child.stdin.take().unwrap();
        let asked = line.trim() == "time?";
        if asked {
            let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
            writeln!(stdin, "{ms}")?;
        }
        drop(stdin);
        let status = child.wait()?;
        if !asked {
            bail!("it said {line:?}");
        }
        if !status.success() {
            bail!("{status}");
        }
        Ok(())
    })();
    if let Err(err) = set {
        tracing::warn!("setting {}'s clock: {err:#}", v.name);
    }
}

/// Runs a command at a device's QEMU monitor: what it said.
pub fn monitor(v: &Vm, command: &str) -> Result<String> {
    let mut c = UnixStream::connect(v.file("monitor.sock")).map_err(|err| anyhow!("{}'s monitor: {err}", v.name))?;
    c.set_read_timeout(Some(Duration::from_secs(120)))?;
    c.set_write_timeout(Some(Duration::from_secs(120)))?;
    let mut r = BufReader::new(c.try_clone()?);
    let prompt = |r: &mut BufReader<UnixStream>| -> Result<String> {
        let mut b = Vec::new();
        let mut byte = [0u8; 1];
        while !b.ends_with(b"(qemu) ") {
            r.read_exact(&mut byte)?;
            b.push(byte[0]);
        }
        Ok(String::from_utf8_lossy(&b).to_string())
    };
    prompt(&mut r)?;
    c.write_all(format!("{command}\n").as_bytes())?;
    let out = prompt(&mut r)?;
    let out = out.strip_suffix("(qemu) ").unwrap_or(&out);
    // It echoes the command, with terminal escapes, on its first line.
    let out = out.split_once('\n').map(|(_, rest)| rest).unwrap_or(out);
    let out = out.replace('\r', "").trim().to_string();
    if out.starts_with("Error") || out.contains("unknown command") {
        bail!("{}: {out}", command.split_whitespace().next().unwrap_or_default());
    }
    Ok(out)
}

impl Devices {
    /// Deletes each Windows device's bases that no copy is on and copies aren't made on now: one stopped since it was
    /// taken.
    pub(super) fn prune_bases(&self) {
        let _creating = self.creating.lock().unwrap(); // no copy is being made on one
        let all = self.vms();
        for from in all.iter().filter(|v| v.kind == "windows" && v.of.is_empty() && !v.dir.is_empty()) {
            let on = |base: &str| all.iter().any(|c| c.of == from.name && c.base == base);
            let current = read(&bases_of(from).join("current"));
            for d in std::fs::read_dir(bases_of(from)).into_iter().flatten().flatten() {
                let base = bases_of(from).join(d.file_name());
                if d.path().is_dir() && d.file_name().to_string_lossy() != current.trim() && !on(&base.to_string_lossy()) {
                    tracing::info!("deleting a base of {} no copy is on", from.name);
                    let _ = std::fs::remove_dir_all(&base);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Windows copy's base is its disk's backing file: deleting it while the copy is there, or writing a new one at
    /// its path, loses the copy. Bases go only once no copy is on them and copies aren't made on them now.
    #[test]
    fn bases_stay_while_a_copy_is_on_them() {
        let dir = std::env::temp_dir().join(format!("agent-devices-bases-{}", super::super::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let ds = Devices::for_test(&dir);
        let lasting = Vm { name: "win".into(), kind: "windows".into(), dir: dir.join("windows").join("win").to_string_lossy().into(), ..Default::default() };
        let bases = bases_of(&lasting);
        for generation in ["used", "now", "stale"] {
            std::fs::create_dir_all(bases.join(generation)).unwrap();
            std::fs::write(bases.join(generation).join("disk.qcow2"), b"").unwrap();
        }
        std::fs::write(bases.join("current"), "now").unwrap();
        ds.save_vm(&lasting).unwrap();
        ds.save_vm(&Vm { name: "copy".into(), kind: "windows".into(), of: "win".into(), base: bases.join("used").to_string_lossy().into(), ..Default::default() }).unwrap();

        ds.prune_bases();
        for (generation, kept) in [("used", true), ("now", true), ("stale", false)] {
            assert_eq!(bases.join(generation).exists(), kept, "base {generation}");
        }

        ds.forget_vm("copy");
        std::fs::remove_file(bases.join("current")).unwrap();
        ds.prune_bases();
        for gone in [bases.join("used"), bases.join("now")] {
            assert!(!gone.exists(), "{} stayed with no copy on it", gone.display());
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
