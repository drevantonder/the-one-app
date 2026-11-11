//! A service's program in its sandbox (services/src/sandbox.rs) reaches only what its app declares: as the user, the network
//! only through its endpoint, to the hosts declared; in its own unit, on a machine set up for one, not the user's
//! home or another service's folder either.

#![cfg(target_os = "linux")]

use std::io::{BufRead, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::time::Duration;

use services::sandbox::{Declared, Launch};

/// A web server on this machine answering "ok" to anything, at its port.
fn server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            let mut buf = [0u8; 1024];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
    port
}

/// Answers the sandbox's socket in `dir` as a service's endpoint does: only `CONNECT` to what `declared` names; and
/// copies what a unit says (`log`) to `said`.
fn endpoint(dir: &Path, declared: Declared, said: std::sync::mpsc::Sender<String>) {
    let socket = dir.join(services::sandbox::SOCKET);
    let log = dir.join(services::sandbox::LOG);
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&log);
    let (ready, bound) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        runtime.block_on(async move {
            use std::os::unix::fs::PermissionsExt;
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            let logs = tokio::net::UnixListener::bind(&log).unwrap();
            for path in [&socket, &log] {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666)).unwrap();
            }
            ready.send(()).unwrap();
            tokio::spawn(async move {
                while let Ok((conn, _)) = logs.accept().await {
                    let said = said.clone();
                    tokio::task::spawn_blocking(move || {
                        let conn = conn.into_std().unwrap();
                        conn.set_nonblocking(false).unwrap();
                        for line in std::io::BufReader::new(conn).lines().map_while(Result::ok) {
                            let _ = said.send(line);
                        }
                    });
                }
            });
            let router = axum::Router::new().fallback(move |req: axum::extract::Request| {
                let declared = declared.clone();
                async move { services::sandbox::connect(Some(&declared), req).await }
            });
            axum::serve(listener, router).await.unwrap();
        });
    });
    bound.recv_timeout(Duration::from_secs(10)).unwrap();
}

/// The program: says what it could reach, a line each. Written by another process: a program this one wrote and
/// then runs fails "Text file busy" when another thread forks as it's written, the child holding the file's write
/// descriptor until it execs.
fn program(dir: &Path, secret: &Path, declared: u16, undeclared: u16) -> std::path::PathBuf {
    let path = dir.join("program.sh");
    let text = format!(
            "#!/bin/sh
echo \"home=$(cat {secret} 2>/dev/null || echo hidden)\"
echo \"others=$(ls {others} >/dev/null 2>&1 && echo seen || echo hidden)\"
echo \"declared=$(curl -s -m 5 http://127.0.0.1:{declared}/ || echo refused)\"
echo \"undeclared=$(curl -s -m 5 http://127.0.0.1:{undeclared}/ || echo refused)\"
echo \"proxied=$(curl -s -m 5 -p -x \"$HTTPS_PROXY\" http://127.0.0.1:{undeclared}/ || echo refused)\"
echo done
",
            secret = secret.display(),
            others = services::sandbox::dir("sandbox-test").parent().unwrap().display(),
        );
    let mut writer = std::process::Command::new("sh").arg("-c").arg("cat > \"$1\" && chmod 755 \"$1\"").arg("sh").arg(&path).stdin(std::process::Stdio::piped()).spawn().unwrap();
    writer.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    assert!(writer.wait().unwrap().success());
    path
}

/// What the program said, by what it said it of.
fn said(lines: impl Iterator<Item = String>) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for line in lines {
        if line == "done" {
            break;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

/// A file in the user's home, beside every service's sandbox folder, which only a service run as the user reads.
fn secret(of: &str) -> std::path::PathBuf {
    let secret = services::sandbox::dir("sandbox-test").with_file_name(format!("test-secret-{of}-{}", std::process::id()));
    std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
    std::fs::write(&secret, "the user's").unwrap();
    secret
}

/// As the user: their files, but the network only through its endpoint, and only the declared port.
#[test]
fn a_service_reaches_only_what_its_app_declares() {
    let dir = super::temp("sandbox");
    let secret = secret("user");
    let (declared, undeclared) = (server(), server());
    let app = Declared { network: vec![format!("127.0.0.1:{declared}")], user: true };
    let name = format!("sandbox-test-{}", std::process::id());
    let launch = Launch { name: name.clone(), own_network: true, ports: vec![declared], data: Some(dir.join("data")) };
    let script = program(&dir, &secret, declared, undeclared);
    let sandbox = services::sandbox::prepare(&name, &script, Path::new(env!("CARGO_BIN_EXE_one")), "token", &launch).unwrap();
    let (tx, _) = std::sync::mpsc::channel();
    endpoint(&sandbox, app, tx);
    let _ = std::fs::remove_file(sandbox.join(services::sandbox::LOG));
    let out = std::process::Command::new(sandbox.join("oned")).arg0("oned").arg(format!("--sandboxed={}", sandbox.display())).stdout(std::process::Stdio::piped()).output().unwrap();
    let got = said(String::from_utf8_lossy(&out.stdout).lines().map(str::to_string));
    let _ = std::fs::remove_dir_all(&sandbox);
    let _ = std::fs::remove_file(&secret);
    assert_eq!(got.get("home").map(String::as_str), Some("the user's"), "as the user, it reads their files: {got:?} {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(got.get("declared").map(String::as_str), Some("ok"), "{got:?}");
    assert_eq!(got.get("undeclared").map(String::as_str), Some("refused"), "{got:?}");
    assert_eq!(got.get("proxied").map(String::as_str), Some("refused"), "{got:?}");
}

/// In its own unit, as its own user: none of the user's files, nor another service's folder. This machine needs the
/// unit's template (`one services sandbox | sudo sh`), and the test fails without it: a machine that can't run it
/// can't say its services are kept in.
#[test]
fn a_service_in_its_own_unit_sees_none_of_the_users_files() {
    let template = services::sandbox::template();
    assert!(template.exists(), "this machine has no {}: set it up with `one services sandbox | sudo sh`", template.display());
    let dir = super::temp("sandbox-unit");
    let secret = secret("unit");
    let (declared, undeclared) = (server(), server());
    let app = Declared { network: vec![format!("127.0.0.1:{declared}")], user: false };
    let name = "sandbox-test";
    // One run at a time on a machine: the unit is the same for all of them.
    let lock = std::fs::File::create(services::sandbox::dir(name).with_extension("lock")).unwrap();
    // SAFETY: a plain flock on a file this keeps open until it's dropped.
    unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX) };
    let launch = Launch { name: name.into(), own_network: true, ports: vec![declared], data: None };
    let script = program(&dir, &secret, declared, undeclared);
    let sandbox = services::sandbox::prepare(name, &script, Path::new(env!("CARGO_BIN_EXE_one")), "token", &launch).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    endpoint(&sandbox, app, tx);
    let unit = services::sandbox::unit(name);
    let _ = services::sandbox::systemctl(&["stop", &unit]);
    let _ = services::sandbox::systemctl(&["reset-failed", &unit]);
    services::sandbox::systemctl(&["start", &unit]).unwrap();
    let got = said(std::iter::from_fn(|| rx.recv_timeout(Duration::from_secs(30)).ok()));
    let _ = services::sandbox::systemctl(&["stop", &unit]);
    let _ = services::sandbox::systemctl(&["reset-failed", &unit]);
    let _ = std::fs::remove_dir_all(&sandbox);
    let _ = std::fs::remove_file(&secret);
    assert_eq!(got.get("home").map(String::as_str), Some("hidden"), "{got:?}");
    assert_eq!(got.get("others").map(String::as_str), Some("hidden"), "{got:?}");
    assert_eq!(got.get("declared").map(String::as_str), Some("ok"), "{got:?}");
    assert_eq!(got.get("undeclared").map(String::as_str), Some("refused"), "{got:?}");
    assert_eq!(got.get("proxied").map(String::as_str), Some("refused"), "{got:?}");
}
