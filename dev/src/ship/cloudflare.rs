//! Deploying the Workers on Cloudflare and the Cloudflare node, from their one home.

use super::*;

/// Deploys to Cloudflare from the one of this user's Linux devices that is the Workers' home (`Settings::at_home`;
/// another refuses, which says so and is passed over): only there are its secrets (`cloudflare deploy` in its home's
/// cloudflare/), and it reads the Workers and the web client from the signed record, or its own build where that's newer.
/// With `build`, the full name of a build (`<commit>` or a custom build's `<commit>-<config>`, as its folder is named), it
/// first waits for that device to run it: a change to Rust needs the program deploying to be that build. It tries again
/// what may pass (a link dropping as nodes restart, Cloudflare). Every network has its Workers on Cloudflare, so it fails
/// if no device deployed them.
pub(super) fn deploy_cloudflare(build: Option<&str>) -> Result<()> {
    // As they are now, not before the nodes restarted onto this build: a device that was away then may be back, and the
    // Workers' home must not be left out because the list is old.
    let linux: Vec<Value> = devices().into_iter().filter(|d| d["target"] == TARGET).collect();
    let home = format!("${{{}:-${{XDG_DATA_HOME:-$HOME/.local/share}}/{CLI}}}", one::config::env("HOME"));
    // `exit::NOT_YET` says it isn't on that build yet, `exit::NO_SETTINGS` that this device has none to deploy. Shell's
    // program, which runs this on another device, has its own folder in services/ as its home, so its home is two up, and
    // its address (`_ADDR`) is the services' registry's, not the node's. It runs as a login would: its profile is where
    // 1Password's sign-in, which the Cloudflare secrets are read with, usually is, and ~/.local/bin where op may be.
    let wait = build.map_or(String::new(), |build| format!(r#"case "$("$node" version --json)" in *'"commit":"{build}"'*) ;; *) echo "still on $("$node" version --json)"; exit {} ;; esac"#, one::cli::exit::NOT_YET));
    let deploy = format!(
        r#"h="{home}"; [ -f "$h/pid" ] || h=$(cd "$h/../.." && pwd); [ -f "$h/cloudflare/settings.json" ] || exit {no_settings}; node=/proc/$(cat "$h/pid")/exe
{wait}
. "$HOME/.profile" >/dev/null 2>&1; unset {addr}; {home_var}="$h" PATH="$HOME/.local/bin:$PATH" "$node" cloudflare deploy"#,
        addr = one::config::env("ADDR"),
        home_var = one::config::env("HOME"),
        no_settings = one::cli::exit::NO_SETTINGS,
    );
    let (online, offline): (Vec<&Value>, Vec<&Value>) = linux.iter().partition(|d| d["online"] == true);
    let (here, others): (Vec<&Value>, Vec<&Value>) = online.into_iter().partition(|d| d["here"] == true);
    let (mut failed, mut deployed) = (false, false);
    // This device first: if its settings are here, nothing is asked of another's Shell.
    for device in here.into_iter().chain(others) {
        if deployed {
            break;
        }
        let name = device["name"].as_str().unwrap_or("");
        let local = device["here"] == true;
        println!("  {name}: deploying to Cloudflare");
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut failures = 0;
        let said = loop {
            let said = if local { run_here(&deploy) } else { shell(device, &deploy, 600) };
            let again = match &said {
                Err(_) => true,
                Ok(r) if r["exit"] == one::cli::exit::NOT_YET => true,
                Ok(r) if r["exit"] == one::cli::exit::REFUSED || r["exit"] == one::cli::exit::NO_SETTINGS => false,
                Ok(r) if r["exit"] != 0 => {
                    failures += 1;
                    failures < 3
                }
                Ok(_) => false,
            };
            if !again || Instant::now() > deadline {
                break said;
            }
            std::thread::sleep(Duration::from_secs(10));
        };
        match said {
            Ok(r) if r["exit"] == one::cli::exit::NO_SETTINGS => {}
            Ok(r) if r["exit"] == one::cli::exit::REFUSED => {
                let said = r["stderr"].as_str().unwrap_or("").trim();
                println!("  {name}: {}", said.strip_prefix(&format!("{CLI}: ")).unwrap_or(said));
            }
            Ok(r) if r["exit"] == 0 => {
                deployed = true;
                println!("  {name}: {}", r["stdout"].as_str().unwrap_or("").lines().find(|l| l.contains(" from ")).unwrap_or("deployed to Cloudflare"));
            }
            Ok(r) => {
                failed = true;
                println!("  {name}: couldn't deploy to Cloudflare:\n{}{}", r["stdout"].as_str().unwrap_or(""), r["stderr"].as_str().unwrap_or("").trim());
            }
            Err(err) => {
                failed = true;
                let hint = if one::protocol::Coded::is(&err, one::protocol::code::NOT_ADDED) { " (add Shell in the App Store, or run this on the device with the Cloudflare settings)" } else { "" };
                println!("  {name}: couldn't deploy to Cloudflare: {err:#}{hint}");
            }
        }
    }
    if !deployed && !failed {
        let away: Vec<&str> = offline.iter().filter_map(|d| d["name"].as_str()).collect();
        let away = if away.is_empty() { String::new() } else { format!(" ({} offline: deploy again when it's back)", away.join(", ")) };
        println!("  no device deployed to Cloudflare: none of the online ones has the settings{away}");
    }
    ensure!(deployed && !failed, "it isn't deployed to Cloudflare");
    Ok(())
}
