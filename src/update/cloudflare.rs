//! Deploying the Workers on Cloudflare and the Cloudflare node, from their one home.

use super::*;

/// Deploys to Cloudflare, with the build being sent out (`commit`), from the one of `linux` that is the Workers' home
/// (`Settings::at_home`; another refuses, which says so and is passed over): only there are its secrets
/// (`cloudflare deploy` in its home's cloudflare/). It waits for each to run that build, so it never
/// deploys the one before, and tries again what may pass (a link dropping as nodes restart, Cloudflare). Every network
/// has its Workers on Cloudflare, so it fails if no device deployed them. True if it failed.
pub(super) fn deploy_cloudflare(linux: &[Value], commit: &str) -> bool {
    let home = format!("${{{}_HOME:-${{XDG_DATA_HOME:-$HOME/.local/share}}/{CLI}}}", CLI.to_uppercase());
    // The node's own build deploys it (what's called the CLI there may be something else), once it's the one sent out:
    // `exit::NOT_YET` says it isn't yet, `exit::NO_SETTINGS` that this device has none to deploy. Shell's program, which runs this on another
    // device, has its own folder in services/ as its home, so its home is two up, and its address (`_ADDR`) is the services' registry's, not
    // the node's. It runs as a login would: its profile is where 1Password's sign-in, which the Cloudflare secrets are read with, usually is,
    // and ~/.local/bin where op may be.
    let deploy = format!(
        r#"h="{home}"; [ -f "$h/pid" ] || h=$(cd "$h/../.." && pwd); [ -f "$h/cloudflare/settings.json" ] || exit {no_settings}; node=/proc/$(cat "$h/pid")/exe
case "$("$node" version --json)" in *'"commit":"{commit}"'*) ;; *) echo "still on $("$node" version --json)"; exit {not_yet} ;; esac
. "$HOME/.profile" >/dev/null 2>&1; unset {var}_ADDR; {var}_HOME="$h" PATH="$HOME/.local/bin:$PATH" "$node" cloudflare deploy"#,
        var = CLI.to_uppercase(),
        no_settings = one::protocol::exit::NO_SETTINGS,
        not_yet = one::protocol::exit::NOT_YET,
    );
    let (here, others): (Vec<&Value>, Vec<&Value>) = linux.iter().filter(|d| d["online"] == true).partition(|d| d["here"] == true);
    let (mut failed, mut deployed) = (false, false);
    // This device first: if its settings are here, nothing is asked of another's Shell.
    for device in here.into_iter().chain(others) {
        if deployed {
            break;
        }
        let name = device["name"].as_str().unwrap_or("");
        let local = device["here"] == true;
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut failures = 0;
        let said = loop {
            let said = if local { run_here(&deploy) } else { shell(device, &deploy, 600) };
            let again = match &said {
                Err(_) => true,
                Ok(r) if r["exit"] == one::protocol::exit::NOT_YET => true,
                Ok(r) if r["exit"] == one::protocol::exit::REFUSED || r["exit"] == one::protocol::exit::NO_SETTINGS => false,
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
            Ok(r) if r["exit"] == one::protocol::exit::NO_SETTINGS => {}
            Ok(r) if r["exit"] == one::protocol::exit::REFUSED => println!("  {name}: {}", r["stderr"].as_str().unwrap_or("").trim()),
            Ok(r) if r["exit"] == 0 => {
                deployed = true;
                println!("  {name}: deployed to Cloudflare at {commit}");
            }
            Ok(r) => {
                failed = true;
                println!("  {name}: couldn't deploy to Cloudflare:\n{}{}", r["stdout"].as_str().unwrap_or(""), r["stderr"].as_str().unwrap_or("").trim());
            }
            Err(err) => {
                failed = true;
                let hint = if one::protocol::Coded::is(&err, one::protocol::code::NOT_INSTALLED) { " (add Shell in the App Store, or run one update on the device with the Cloudflare settings)" } else { "" };
                println!("  {name}: couldn't deploy to Cloudflare: {err:#}{hint}");
            }
        }
    }
    if !deployed && !failed {
        println!("  no device deployed to Cloudflare: none of them online has the settings");
    }
    failed || !deployed
}
