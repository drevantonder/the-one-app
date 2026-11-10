//! `one update`, the one path that must never break. Each test runs the real program, by a hard link of it in a folder of
//! its own (the file it replaces), against a small web server standing in for the network's address, with no background
//! program, node or relay anywhere: what `one update` depends on is only what these give it. A build here is a shell
//! script, since all that's asked of one is that it says what it is.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use one::config::{COMMITTED, TARGET};
use one::latest::{Latest, Thing, sha256};
use one::mode::Throwaway;
use one::release::Version;

const NEWER: u64 = COMMITTED + 100;

/// The network's address, as far as a device's update can tell: a GET of a file under `updates/`.
#[derive(Clone)]
struct Address {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    at: String,
}

impl Address {
    fn start() -> Address {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = Address { files: Default::default(), at: format!("http://{}", listener.local_addr().unwrap()) };
        let files = address.files.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let files = files.clone();
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let mut asked = Vec::new();
                    let mut byte = [0u8; 1];
                    while !asked.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                        asked.push(byte[0]);
                    }
                    let asked = String::from_utf8_lossy(&asked).into_owned();
                    let path = asked.split(' ').nth(1).unwrap_or("").trim_start_matches('/').to_string();
                    let found = files.lock().unwrap().get(&path).cloned();
                    let head = match &found {
                        Some(body) => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()),
                        None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                    };
                    let _ = stream.write_all(head.as_bytes());
                    if let Some(body) = found {
                        let _ = stream.write_all(&body);
                    }
                });
            }
        });
        address
    }

    fn serve(&self, name: &str, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(format!("updates/{name}"), bytes);
    }
}

/// A device: the program, as a hard link of the built one (so replacing it is seen by its inode), its home, and the release
/// key it was told, with no program running and only what `one update` reads in its home.
struct Device {
    dir: PathBuf,
    program: PathBuf,
    home: PathBuf,
    key: ssh_key::PrivateKey,
    address: Address,
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn device() -> Device {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("update-{}", rand::random::<u64>()));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let program = dir.join("one");
    std::fs::hard_link(env!("CARGO_BIN_EXE_one"), &program).unwrap();
    let key = Throwaway::own_release_key(&home).unwrap();
    let address = Address::start();
    one::update::keep_address(&home, &address.at).unwrap();
    Device { dir, program, home, key, address }
}

/// A build: a program that says it's `commit` for `target`, made at `committed`.
fn build(commit: &str, target: &str, committed: u64) -> Vec<u8> {
    let says = Version { commit: commit.into(), target: target.into(), committed }.json();
    format!("#!/bin/sh\necho '{says}'\n").into_bytes()
}

impl Device {
    fn inode(&self) -> u64 {
        std::fs::metadata(&self.program).unwrap().ino()
    }

    /// Publishes `bytes` as the build named in the record, signed with `key`.
    fn publish(&self, name: &str, commit: &str, committed: u64, bytes: Vec<u8>, key: &ssh_key::PrivateKey) {
        let sha = sha256(&bytes);
        self.address.serve(&sha, bytes.clone());
        let record = Latest { things: vec![Thing { name: name.into(), commit: commit.into(), committed, sha256: sha, size: bytes.len() as u64, config: "-".into() }] };
        self.address.serve("newest", record.signed(key).unwrap().into_bytes());
    }

    fn published(&self, commit: &str, committed: u64, bytes: Vec<u8>) {
        self.publish(&Thing::node_name(TARGET), commit, committed, bytes, &self.key);
    }

    /// Runs `update` on this device: → whether it succeeded, what it said to stdout, and to stderr.
    fn update(&self) -> (bool, String, String) {
        let mut command = std::process::Command::new(&self.program);
        Throwaway::tell(&mut command, &self.home, "127.0.0.1:1");
        let out = command.arg("update").output().unwrap();
        (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    }

    fn leftovers(&self) -> Vec<String> {
        std::fs::read_dir(&self.dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("one.")).collect()
    }

    /// The program is as it was before `update`.
    fn unchanged(&self, inode: u64) {
        assert_eq!(self.inode(), inode, "the program was replaced");
        assert_eq!(self.leftovers(), Vec::<String>::new());
        assert_eq!(Version::of(&self.program).unwrap(), Version::this());
    }
}

#[test]
fn a_newer_build_replaces_the_program_whole() {
    let device = device();
    let was = device.inode();
    // A second name for the program it replaces: had it been written over in place, this would change too.
    let before = device.dir.join("before");
    std::fs::hard_link(&device.program, &before).unwrap();
    let new = build("newer", TARGET, NEWER);
    let sha = sha256(&new);
    device.published("newer", NEWER, new.clone());
    let (ok, said, err) = device.update();
    assert!(ok, "{err}");
    assert!(said.contains("newer"), "{said}");
    assert_ne!(device.inode(), was, "replaced by a rename, not written in place");
    assert_eq!(std::fs::read(&device.program).unwrap(), new);
    assert_eq!(std::fs::metadata(&before).unwrap().ino(), was);
    assert_eq!(Version::of(&before).unwrap(), Version::this(), "the old file is whole, for what's still running it");
    assert_eq!(device.leftovers(), Vec::<String>::new());
    assert_eq!(one::update::taken(&device.home), sha, "a device's own check agrees it's taken");
}

#[test]
fn a_bad_signature_changes_nothing() {
    let device = device();
    let was = device.inode();
    let other = Throwaway::own_release_key(&device.dir.join("other")).unwrap();
    let new = build("newer", TARGET, NEWER);
    device.publish(&Thing::node_name(TARGET), "newer", NEWER, new.clone(), &other);
    let (ok, _, err) = device.update();
    assert!(!ok, "signed by another key");
    assert!(err.contains("signed") || err.contains("key"), "{err}");
    device.unchanged(was);

    device.published("newer", NEWER, new);
    let record = device.address.files.lock().unwrap().get("updates/newest").cloned().unwrap();
    device.address.serve("newest", String::from_utf8(record).unwrap().replace("newer", "other").into_bytes());
    let (ok, _, _) = device.update();
    assert!(!ok, "a line changed after it was signed");
    device.unchanged(was);
}

#[test]
fn an_older_commit_changes_nothing() {
    let device = device();
    let was = device.inode();
    device.published("older", COMMITTED - 100, build("older", TARGET, COMMITTED - 100));
    let (ok, said, err) = device.update();
    assert!(ok, "being current isn't a failure: {err}");
    assert!(said.contains("current"), "{said}");
    device.unchanged(was);

    // The record says newer, but the build itself says it's older: its own word is signed in with it, the record's isn't for this.
    device.published("older", NEWER, build("older", TARGET, COMMITTED - 100));
    let (ok, _, _) = device.update();
    assert!(!ok, "a build older than its record claims");
    device.unchanged(was);
}

#[test]
fn the_wrong_target_is_refused() {
    let device = device();
    let was = device.inode();
    device.publish(&Thing::node_name("riscv64gc-unknown-linux-gnu"), "newer", NEWER, build("newer", "riscv64gc-unknown-linux-gnu", NEWER), &device.key);
    let (ok, _, err) = device.update();
    assert!(!ok, "the record has no build for this computer");
    assert!(err.contains(TARGET), "{err}");
    device.unchanged(was);

    device.published("newer", NEWER, build("newer", "riscv64gc-unknown-linux-gnu", NEWER));
    let (ok, _, _) = device.update();
    assert!(!ok, "the build says it's for another computer");
    device.unchanged(was);
}

#[test]
fn a_build_that_isnt_what_the_record_names_changes_nothing() {
    let device = device();
    let was = device.inode();
    let new = build("newer", TARGET, NEWER);
    let sha = sha256(&new);
    device.published("newer", NEWER, new);
    device.address.serve(&sha, b"something else".to_vec());
    let (ok, _, _) = device.update();
    assert!(!ok, "its hash doesn't match");
    device.unchanged(was);

    let not_it = build("another", TARGET, NEWER);
    device.published("newer", NEWER, not_it);
    let (ok, _, _) = device.update();
    assert!(!ok, "it says it's another commit than the record says");
    device.unchanged(was);
}

/// A Mac: the program runs from inside its app, which `one update` replaces whole with the app the record names, by the
/// installer that `build.sh` uses too. Everything Apple's the installer calls is a stand-in here; what's held is that
/// the app named is the one installed, and one that isn't what the record says changes nothing.
#[test]
fn a_mac_takes_the_app_the_record_names() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("update-mac-{}", rand::random::<u64>()));
    let (cli, name) = (one::config::CLI, one::config::NAME);
    let user = dir.join("user");
    let installed = user.join(format!("Applications/{name}.app/Contents/MacOS/{cli}"));
    let home = dir.join("home");
    let bin = dir.join("bin");
    for folder in [installed.parent().unwrap(), &home, &bin] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::hard_link(env!("CARGO_BIN_EXE_one"), &installed).unwrap();
    let stub = |name: &str, body: &str| {
        std::fs::write(bin.join(name), format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(bin.join(name), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    };
    for nothing in ["osascript", "open", "launchctl"] {
        stub(nothing, "exit 0");
    }
    for none in ["pgrep", "pkill"] {
        stub(none, "exit 1");
    }
    // ditto -x -k <zip> <into>: our "zip" is the program itself, put where an unpacked app has it. ditto <app> <to>: copies.
    let inside = format!("{name}.app/Contents/MacOS");
    stub("ditto", &format!("if [ \"$1\" = -x ]; then mkdir -p \"$4/{inside}\"; cat \"$3\" > \"$4/{inside}/{cli}\"; chmod 755 \"$4/{inside}/{cli}\"; else cp -R \"$1\" \"$2\"; fi"));
    let key = Throwaway::own_release_key(&home).unwrap();
    let address = Address::start();
    one::update::keep_address(&home, &address.at).unwrap();
    let device = Device { dir: dir.clone(), program: installed.clone(), home, key, address };
    let update = || {
        let mut command = std::process::Command::new(&installed);
        Throwaway::tell(&mut command, &device.home, "127.0.0.1:1");
        let out = command.arg("update").env("HOME", &user).env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())).output().unwrap();
        (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let was = std::fs::read(&installed).unwrap();

    // An app that says it's another build than the record names isn't installed.
    device.published("newer", NEWER, build("another", TARGET, NEWER));
    let (ok, _, _) = update();
    assert!(!ok, "it says it's another commit than the record says");
    assert_eq!(std::fs::read(&installed).unwrap(), was);

    // The one it names is: after this returns, apart from it.
    let new = build("newer", TARGET, NEWER);
    device.published("newer", NEWER, new.clone());
    let (ok, said, err) = update();
    assert!(ok, "{err}");
    assert!(said.contains("Updating this Mac"), "{said}");
    for _ in 0..200 {
        if std::fs::read(&installed).is_ok_and(|b| b == new) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(std::fs::read(&installed).unwrap(), new, "the installer didn't run");
}

/// A process named as `oned`'s program, standing in for it: `one update` tells it apart from a stranger's only by that.
fn sleeping_as(dir: &Path, name: &str) -> std::process::Child {
    use std::os::unix::process::CommandExt;
    std::fs::create_dir_all(dir).unwrap();
    let sleeper = dir.join(name);
    // A link, not a copy: a program just written can't be started while another test's fork still holds it open (ETXTBSY).
    let _ = std::fs::remove_file(&sleeper);
    std::os::unix::fs::symlink(std::fs::canonicalize("/bin/sleep").unwrap(), &sleeper).unwrap();
    std::process::Command::new(&sleeper).arg0("sleep").arg("600").spawn().unwrap()
}

fn exits(child: &mut std::process::Child) -> bool {
    for _ in 0..200 {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

#[test]
fn it_makes_oned_exit_and_leaves_a_stranger_alone() {
    let device = device();
    device.published("newer", NEWER, build("newer", TARGET, NEWER));
    let mut oned = sleeping_as(&device.dir.join("as-oned"), &one::oned::name());
    std::fs::write(device.home.join(one::update::PID), oned.id().to_string()).unwrap();
    let (ok, said, err) = device.update();
    assert!(ok, "{err}");
    assert!(said.contains("restarting"), "{said}");
    assert!(exits(&mut oned), "oned didn't exit");

    let device = self::device();
    device.published("newer", NEWER, build("newer", TARGET, NEWER));
    let mut stranger = sleeping_as(&device.dir.join("as-stranger"), "stranger");
    std::fs::write(device.home.join(one::update::PID), stranger.id().to_string()).unwrap();
    let (ok, said, err) = device.update();
    assert!(ok, "{err}");
    assert!(said.contains("Nothing was running"), "{said}");
    assert!(stranger.try_wait().unwrap().is_none(), "it stopped a process that wasn't oned");
    stranger.kill().unwrap();
    stranger.wait().unwrap();
}

/// The program may already be the newest while the oned running is the one before it (the device's own check put it in
/// place and hasn't restarted onto it): Update restarts that one, and says so, instead of "current" and nothing.
#[test]
fn update_restarts_an_oned_still_on_the_build_before() {
    let device = device();
    let build = one::config::BUILD;
    device.published(build, COMMITTED, self::build(build, TARGET, COMMITTED));
    let mut oned = sleeping_as(&device.dir.join("as-oned"), &one::oned::name());
    std::fs::write(device.home.join(one::update::PID), oned.id().to_string()).unwrap();
    std::fs::write(device.home.join("running"), "the build before").unwrap();
    let (ok, said, err) = device.update();
    assert!(ok, "{err}");
    assert!(said.contains("restarting"), "{said}");
    assert!(exits(&mut oned), "oned didn't exit");

    // One that began on this build is left running.
    let mut oned = sleeping_as(&device.dir.join("as-oned"), &one::oned::name());
    std::fs::write(device.home.join(one::update::PID), oned.id().to_string()).unwrap();
    one::update::note_running(&device.home).unwrap();
    let (ok, said, err) = device.update();
    assert!(ok, "{err}");
    assert!(said.contains("current"), "{said}");
    assert!(oned.try_wait().unwrap().is_none(), "it stopped an oned that was current");
    oned.kill().unwrap();
    oned.wait().unwrap();
}

/// What `one update` runs on must work with the network gone: so nothing of it may reach for the node, the relay or who's who.
#[test]
fn updating_imports_nothing_of_the_network() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let allowed: &[(&str, &[&str])] = &[
        ("src/update.rs", &["cli", "config", "home", "latest", "mode", "oned::name", "os", "release"]),
        ("src/update/mac.rs", &["config", "latest", "release"]),
        ("src/update/cli.rs", &["cli", "config"]),
        ("src/latest.rs", &["build::Platform", "hedge", "keys", "protocol"]),
        ("src/release.rs", &["config", "os"]),
    ];
    for (file, may) in allowed {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        let text: String = text.lines().filter(|l| !l.trim_start().starts_with("//") && !l.contains("#[path")).collect::<Vec<_>>().join("\n");
        let mut rest = text.as_str();
        while let Some(at) = rest.find("crate::") {
            rest = &rest[at + "crate::".len()..];
            assert!(may.iter().any(|m| rest.starts_with(m)), "{file} reaches for crate::{}, which updating must not depend on", rest.split(|c: char| !c.is_alphanumeric() && c != '_' && c != ':').next().unwrap());
        }
    }
}
