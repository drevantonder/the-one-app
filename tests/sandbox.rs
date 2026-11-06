//! A service's program in its sandbox (src/services/sandbox.rs) reaches only what its app declares: as the person, the network
//! only through its endpoint, to the hosts declared; in its own unit, on a machine set up for one, not the person's
//! home or another service's folder either.

#![cfg(target_os = "linux")]

use std::io::{BufRead, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::time::Duration;

use one::services::sandbox::{Declared, Launch};

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
    let socket = dir.join(one::services::sandbox::SOCKET);
    let log = dir.join(one::services::sandbox::LOG);
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
                async move { one::services::sandbox::connect(Some(&declared), req).await }
            });
            axum::serve(listener, router).await.unwrap();
        });
    });
    bound.recv_timeout(Duration::from_secs(10)).unwrap();
}

/// The program: says what it could reach, a line each.
fn program(dir: &Path, secret: &Path, declared: u16, undeclared: u16) -> std::path::PathBuf {
    let path = dir.join("program.sh");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh
echo \"home=$(cat {secret} 2>/dev/null || echo hidden)\"
echo \"others=$(ls {others} >/dev/null 2>&1 && echo seen || echo hidden)\"
echo \"declared=$(curl -s -m 5 http://127.0.0.1:{declared}/ || echo refused)\"
echo \"undeclared=$(curl -s -m 5 http://127.0.0.1:{undeclared}/ || echo refused)\"
echo \"proxied=$(curl -s -m 5 -p -x \"$HTTPS_PROXY\" http://127.0.0.1:{undeclared}/ || echo refused)\"
echo \"web=$(curl -s -m 5 -o /dev/null https://example.com && echo reached || echo refused)\"
echo done
",
            secret = secret.display(),
            others = one::services::sandbox::dir("whatsapp").parent().unwrap().display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
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

#[test]
fn a_service_reaches_only_what_its_app_declares() {
    let dir = super::temp("sandbox");
    // In the person's home, beside every service's sandbox folder.
    let secret = one::services::sandbox::dir("whatsapp").with_file_name(format!("test-secret-{}", std::process::id()));
    std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
    std::fs::write(&secret, "the person's").unwrap();
    let (declared, undeclared) = (server(), server());
    let app = Declared { network: vec![format!("127.0.0.1:{declared}")], person: true };

    // As the person: their files, but only the declared port, and only through its endpoint.
    let name = format!("sandbox-test-{}", std::process::id());
    let launch = Launch { name: name.clone(), own_network: true, ports: vec![declared], data: Some(dir.join("data")) };
    let script = program(&dir, &secret, declared, undeclared);
    let sandbox = one::services::sandbox::prepare(&name, &script, Path::new(env!("CARGO_BIN_EXE_one")), "token", &launch).unwrap();
    let (tx, _) = std::sync::mpsc::channel();
    endpoint(&sandbox, app.clone(), tx);
    let _ = std::fs::remove_file(sandbox.join(one::services::sandbox::LOG));
    let out = std::process::Command::new(sandbox.join("oned")).arg0("oned").arg(format!("--sandboxed={}", sandbox.display())).stdout(std::process::Stdio::piped()).output().unwrap();
    let got = said(String::from_utf8_lossy(&out.stdout).lines().map(str::to_string));
    let _ = std::fs::remove_dir_all(&sandbox);
    assert_eq!(got.get("home").map(String::as_str), Some("the person's"), "as the person, it reads their files: {got:?} {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(got.get("declared").map(String::as_str), Some("ok"), "{got:?}");
    assert_eq!(got.get("undeclared").map(String::as_str), Some("refused"), "{got:?}");
    assert_eq!(got.get("proxied").map(String::as_str), Some("refused"), "{got:?}");
    assert_eq!(got.get("web").map(String::as_str), Some("refused"), "{got:?}");

    // In its own unit, as its own user: none of the person's files, nor another service's folder.
    if !one::services::sandbox::template().exists() {
        let _ = std::fs::remove_file(&secret);
        eprintln!("skipped a service in its own unit: this machine has no {} (`one services sandbox | sudo sh`)", one::services::sandbox::template().display());
        return;
    }
    let name = "sandbox-test";
    // One run at a time on a machine: the unit is the same for all of them.
    let lock = std::fs::File::create(one::services::sandbox::dir(name).with_extension("lock")).unwrap();
    // SAFETY: a plain flock on a file this keeps open until it's dropped.
    unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX) };
    let app = Declared { person: false, ..app };
    let launch = Launch { name: name.into(), own_network: true, ports: vec![declared], data: None };
    let sandbox = one::services::sandbox::prepare(name, &script, Path::new(env!("CARGO_BIN_EXE_one")), "token", &launch).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    endpoint(&sandbox, app, tx);
    let unit = one::services::sandbox::unit(name);
    let _ = one::services::sandbox::systemctl(&["stop", &unit]);
    let _ = one::services::sandbox::systemctl(&["reset-failed", &unit]);
    one::services::sandbox::systemctl(&["start", &unit]).unwrap();
    let got = said(std::iter::from_fn(|| rx.recv_timeout(Duration::from_secs(30)).ok()));
    let _ = one::services::sandbox::systemctl(&["stop", &unit]);
    let _ = one::services::sandbox::systemctl(&["reset-failed", &unit]);
    let _ = std::fs::remove_dir_all(&sandbox);
    let _ = std::fs::remove_file(&secret);
    assert_eq!(got.get("home").map(String::as_str), Some("hidden"), "{got:?}");
    assert_eq!(got.get("others").map(String::as_str), Some("hidden"), "{got:?}");
    assert_eq!(got.get("declared").map(String::as_str), Some("ok"), "{got:?}");
    assert_eq!(got.get("undeclared").map(String::as_str), Some("refused"), "{got:?}");
    assert_eq!(got.get("proxied").map(String::as_str), Some("refused"), "{got:?}");
    assert_eq!(got.get("web").map(String::as_str), Some("refused"), "{got:?}");
}
