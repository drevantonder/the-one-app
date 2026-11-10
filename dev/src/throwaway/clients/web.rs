//! The web client, in a browser of the throwaway network's own on this machine.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use one::config::CLI;
use serde_json::json;

use super::super::devices::approve_code;
use super::super::network::{node_in, spawn};
use crate::agent;

/// The web client's folder in the throwaway network's, for its first user or someone else.
pub(crate) fn web_folder(dir: &Path, user: Option<&str>) -> PathBuf {
    match user {
        None => dir.join("web"),
        Some(name) => dir.join(format!("web-{}", name.to_lowercase().replace(' ', "-"))),
    }
}

/// The web client, in a browser of the throwaway network's own on this machine, with no window: drive it over the DevTools
/// protocol. Its profile is kept with the throwaway network, so it stays signed in. As someone else, they're invited first,
/// by the throwaway network's first user, and join in it.
pub(crate) fn web(dir: &Path, settings: &one::cloudflare::Settings, user: Option<&str>) -> Result<()> {
    let (port, picture) = match user {
        Some(name) => joined(dir, settings, name)?,
        None => {
            let port = browser(dir, settings, None)?;
            let picture = dir.join("web.png");
            let code = web_script(dir, &["sign-in", &port.to_string(), &settings.address, &picture.display().to_string()])?;
            approve(dir, settings, port, &picture, &code)?;
            (port, picture)
        }
    };
    let who = user.map(|name| format!(" as {name}")).unwrap_or_default();
    println!(
        "The web client of this build is signed in to the throwaway network{who}, in a browser of its own on this machine.
  See it: {}
  Use it: its DevTools are at http://127.0.0.1:{port} (Playwright's connectOverCDP, or chrome-devtools-mcp --browserUrl)
  In its page, self.oneNet.net asks the user's nodes as this browser (never import('/net.js'): that's a second copy, which refuses)
  In any browser here: {}",
        picture.display(),
        settings.address
    );
    Ok(())
}

/// `name`'s web client, signed in: the first time, the throwaway network's machine invites them, as `devices invite` does on
/// an admin's machine (keeping the secret for their first device's approval), and they open the link in their own
/// browser, join, and sign in; then their browser, their first device, asks to join and the machine approves it. Pictures
/// of the invite and joining are kept beside theirs. → its DevTools port, and the picture of their first screen.
pub(crate) fn joined(dir: &Path, settings: &one::cloudflare::Settings, name: &str) -> Result<(u16, PathBuf)> {
    let folder = web_folder(dir, Some(name));
    let picture = folder.with_extension("png");
    let port = browser(dir, settings, Some(name))?;
    let mut args = vec!["sign-in".to_string(), port.to_string(), settings.address.clone(), picture.display().to_string()];
    let marker = folder.join("joined");
    if !marker.exists() {
        eprintln!("Inviting {name}…");
        let invited = node_in(dir)?.post("/api/devices/invite", json!({ "name": name }))?;
        args.push(invited["link"].as_str().context("the throwaway network made no invite link")?.to_string());
    }
    let code = web_script(dir, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    std::fs::write(&marker, name)?;
    approve(dir, settings, port, &picture, &code).with_context(|| format!("{name}'s browser wasn't let in"))?;
    Ok((port, picture))
}

/// A web client that shows a code to be approved (`code`, else none) is approved by the throwaway network's machine, as its
/// user's device or as the device of the admin who invited them; then its window is pictured.
pub(crate) fn approve(dir: &Path, settings: &one::cloudflare::Settings, port: u16, picture: &Path, code: &str) -> Result<()> {
    if !code.is_empty() {
        approve_code(dir, code)?;
    }
    web_script(dir, &["picture", &port.to_string(), &settings.address, &picture.display().to_string()])?;
    Ok(())
}

pub(crate) fn web_script(dir: &Path, args: &[&str]) -> Result<String> {
    let script = dir.join("web.js");
    std::fs::write(&script, include_str!("web.js"))?;
    let out = std::process::Command::new(one::build::bun()?).arg(&script).args(args).output()?;
    anyhow::ensure!(out.status.success(), "the web client didn't sign in: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The throwaway network's browser for its first user or someone else, started if it isn't running: → its DevTools port.
pub(crate) fn browser(dir: &Path, settings: &one::cloudflare::Settings, user: Option<&str>) -> Result<u16> {
    // A browser reaches nodes over iroh, as the client's WebAssembly; a build without it can't sign anyone in.
    let page = agent().get(&settings.address).call()?.body_mut().read_to_string()?;
    anyhow::ensure!(!page.contains(r#"<meta name="onet" content="">"#), "this build has no web client's WebAssembly: {CLI}-dev build --dev builds it");
    let web = web_folder(dir, user);
    std::fs::create_dir_all(&web)?;
    let port = std::fs::read_to_string(web.join("port")).ok().and_then(|p| p.trim().parse::<u16>().ok());
    let answers = |port: u16| agent().get(format!("http://127.0.0.1:{port}/json/version")).call().is_ok_and(|r| r.status().is_success());
    let port = match port.filter(|p| answers(*p)) {
        Some(port) => port,
        None => {
            let log = format!("{}.log", web.file_name().unwrap().to_string_lossy());
            let mut chrome = one::build::chrome().context("the web client needs Chrome or Chromium on this machine: none was found (CHROME=<path> names one)")?;
            let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
            spawn(
                dir,
                chrome.args([
                    &format!("--remote-debugging-port={port}"),
                    &format!("--user-data-dir={}", web.join("profile").display()),
                    "--window-size=1280,860",
                    "--hide-scrollbars",
                    &settings.address,
                ]),
                &log,
                true,
            )?;
            std::fs::write(web.join("port"), port.to_string())?;
            let start = Instant::now();
            while !answers(port) {
                anyhow::ensure!(start.elapsed() < Duration::from_secs(30), "the throwaway network's browser didn't start: {}", dir.join(&log).display());
                std::thread::sleep(Duration::from_millis(250));
            }
            port
        }
    };
    Ok(port)
}
