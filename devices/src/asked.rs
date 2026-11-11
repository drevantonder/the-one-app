//! Someone asking a device to take the newest published build, or the device the Workers were last deployed from to
//! deploy them again: an ask the asker's device signed and the relay left for this one, which checks who signed it and
//! that they may, as who's who says, then starts `<cli> update` (or `<cli> cloudflare deploy`) and does nothing else. A
//! computer's onet hears its device's; its app runtime, its own.

use std::process::Stdio;

use anyhow::{Result, ensure};
use serde_json::Value;

use custom_build::config::{CLI, YOURS};
use custom_build::now_ms;
use key::hex;
use scope::scopes;

/// What the relay left for the device `me` (`asked` in its answer to `POST /relay/computes`), as who's who says: taken
/// if who signed it may ask it, said in this program's log either way. A computer's onet hears its device's.
pub fn heard(who: &scopes::ScopeDb, me: &::key::Key, asked: &Value) {
    match who_asked(who, me, asked).and_then(|(by, ask)| Ok((by, ask.start()?, ask))) {
        Ok((by, (), ask)) => ::device_logs::say(&format!("asked by {by}: starting {CLI} {}", ask.command().join(" "))),
        Err(err) => ::device_logs::say(&format!("an ask to update wasn't taken: {err:#}")),
    }
}

/// The device that signed `asked`, if it's one who may remove the device `me` (an admin's, for the Workers), and it asks
/// that device: else why not.
pub fn who_asked(who: &scopes::ScopeDb, me: &::key::Key, asked: &Value) -> Result<(String, Ask)> {
    let (by, fields) = ::key::check_request(::custom_build::protocol::RELAY, asked, now_ms())?;
    // The relay leaves the Workers' ask for the device they were last deployed from.
    let ask = if fields["cloudflare"] == true { Ask::Deploy } else { Ask::Update };
    ensure!(ask == Ask::Deploy || fields["update"] == hex(me), "it asks another device");
    let asker = who.machine(&by)?.filter(|d| !d.removed);
    let Some(asker) = asker else { anyhow::bail!("{YOURS} doesn't list the device that asked") };
    match ask {
        Ask::Update => ensure!(who.may(&asker.user, "remove", &scopes::device(me)), "{} may not remove this device", asker.name),
        Ask::Deploy => ensure!(who.may(&asker.user, "change-settings", scopes::NETWORK), "{} isn't an admin's device", asker.name),
    }
    Ok((asker.name, ask))
}

/// What a device is asked to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ask {
    /// Take the newest published build.
    Update,
    /// Deploy the Workers again, which were last deployed from this device.
    Deploy,
}

impl Ask {
    fn command(self) -> &'static [&'static str] {
        match self {
            Ask::Update => &["update"],
            Ask::Deploy => &["cloudflare", "deploy"],
        }
    }

    /// Starts it detached. An update replaces this program and then asks this process to stop, so what it says last is
    /// said to nobody: only a failure leaves words here.
    fn start(self) -> Result<()> {
        let words = self.command();
        let started = platform::os::named(CLI)?.args(words).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        std::thread::spawn(move || match started.wait_with_output() {
            Ok(out) => ::device_logs::say(&format!("{CLI} {}: {}{}", words.join(" "), String::from_utf8_lossy(&out.stdout).trim(), String::from_utf8_lossy(&out.stderr).trim())),
            Err(err) => ::device_logs::say(&format!("{CLI} {}: {err}", words.join(" "))),
        });
        Ok(())
    }
}
