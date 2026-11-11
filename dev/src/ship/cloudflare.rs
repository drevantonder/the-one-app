//! Deploying the Workers on Cloudflare and the Cloudflare node, from this machine.

use super::*;

/// Deploys to Cloudflare from this machine, with the network's program here (`cloudflare deploy`, which takes the Workers'
/// settings from 1Password), which reads the Workers and the web client from the signed record, or its own build where
/// that's newer. With `build`, the full name of a build (`<commit>` or a custom build's `<commit>-<config>`, as its
/// folder is named), it first waits for this device to run it: a change to Rust needs the program deploying to be that build.
pub(super) fn deploy_cloudflare(build: Option<&str>) -> Result<()> {
    if let Some(build) = build {
        let here = devices().into_iter().find(|d| d["here"] == true).and_then(|d| d["name"].as_str().map(str::to_string)).context("this device isn't listed")?;
        wait_until(build, &[here], Duration::from_secs(600))?;
    }
    println!("  deploying to Cloudflare from this device");
    let program = platform::mode::here().bin().join(CLI);
    let out = Command::new(&program).args(["cloudflare", "deploy"]).output().with_context(|| format!("run {}", program.display()))?;
    let (said, err) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    ensure!(out.status.success(), "it isn't deployed to Cloudflare:\n{said}{}", err.trim());
    if !err.trim().is_empty() {
        eprintln!("  {}", err.trim());
    }
    println!("  {}", said.lines().find(|l| l.contains(" from ")).unwrap_or("deployed to Cloudflare"));
    Ok(())
}
