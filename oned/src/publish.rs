//! Publishing to the signed record at the network's address (`latest`): a build of this program or a service's, what a
//! Mac built (`<cli> publish-mac`), signed with the release key from the publisher's password manager.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use custom_build::config::CLI;
use serde_json::json;

use oned_update::release::Version;

/// Puts `things`' files where devices take them, and adds them to the record at the network's address, signed with the
/// release key: each `(name, file, version, config commit)`, with no file for what's built, not fetched (`source`).
pub fn publish_latest(node: &::api::Client, key: &ssh_key::PrivateKey, things: &[(String, Option<PathBuf>, Version, String)]) -> Result<()> {
    if let Some((_, _, version, _)) = things.iter().find(|(_, _, v, _)| v.commit.ends_with("+dirty")) {
        bail!("{} has uncommitted changes; commit, build again, then publish", version.commit);
    }
    use oned_update::latest::{Latest, Thing};
    let mut made = Vec::new();
    for (name, path, version, config) in things {
        let (sha256, size) = match path {
            None => ("-".to_string(), 0),
            Some(path) => {
                let put = node.post("/api/latest/put", json!({ "path": path }))?;
                (put["sha256"].as_str().context("no hash")?.to_string(), put["size"].as_u64().unwrap_or(0))
            }
        };
        made.push(Thing { name: name.clone(), commit: version.commit.clone(), committed: version.committed, sha256, size, config: config.clone() });
    }
    let public = key.public_key().to_openssh()?;
    for _ in 0..16 {
        let now = node.get("/api/latest")?;
        let here = now["text"].as_str().and_then(|t| Latest::read(&public, t).ok()).unwrap_or_default();
        let text = here.merged(made.iter().cloned()).signed(key)?;
        if node.post("/api/latest", json!({ "text": text, "tag": now["tag"] }))?["replaced"] == true {
            return Ok(());
        }
    }
    bail!("couldn't update the record: another publisher kept changing it")
}

/// The release key's private half, from the publisher's password manager by way of the config's release_signer, kept
/// only in memory. It must be the key every build checks releases against.
pub fn release_key() -> Result<ssh_key::PrivateKey> {
    use ::custom_build::config::{RELEASE_KEY, RELEASE_SIGNER};
    anyhow::ensure!(!RELEASE_KEY.is_empty() && !RELEASE_SIGNER.is_empty(), "this custom build has no release key, so nothing it builds can be released");
    let out = Command::new("sh").args(["-c", RELEASE_SIGNER]).stdin(std::process::Stdio::null()).output().context("run release_signer")?;
    anyhow::ensure!(
        out.status.success(),
        "couldn't get the release key with `{RELEASE_SIGNER}`: {}\nIs the password manager it reads reachable, and unlocked?",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let key = ssh_key::PrivateKey::from_openssh(&out.stdout).map_err(|_| anyhow::anyhow!("`{RELEASE_SIGNER}` didn't give an OpenSSH private key"))?;
    anyhow::ensure!(key.public_key().key_data() == ssh_key::PublicKey::from_openssh(RELEASE_KEY)?.key_data(), "`{RELEASE_SIGNER}` gave a key, but not the release key in config.toml");
    Ok(key)
}

/// What publishes what a Mac built (dev/skills/keep-mac-current), on a Mac with the release key.
pub fn commands() -> ::api::cli::Command {
    ::api::cli::Command {
        help: vec![("publish-mac <folder> [--config <commit>]".into(), "publish what native-apps/mac/build.sh made in <folder> (its window, program and services), for every Mac to take with update".into())],
        run: |g| {
            let words = g.words();
            let ["publish-mac", folder] = words.as_slice() else { return Ok(false) };
            let folder = Path::new(folder);
            let version = Version::of(&folder.join(CLI)).with_context(|| format!("{} has no program of this Mac's that says what it is", folder.display()))?;
            publish_latest(g.node, &release_key()?, &mac_things(folder, &version, g.flag("config").unwrap_or("-"))?)?;
            println!("Published {} for {}: every Mac takes it with {CLI} update.", version.commit, version.target);
            Ok(true)
        },
    }
}

/// What a Mac built for the record (native-apps/mac/build.sh), as `publish_latest` takes it: in `folder`, its window
/// zipped (`app.zip`), its program (`<cli>`), each service in Rust (`<cli>-<name>`) and each in Swift, an app of its own
/// zipped (`<cli>-<name>.zip`), all of `version`.
pub fn mac_things(folder: &Path, version: &Version, config: &str) -> Result<Vec<(String, Option<PathBuf>, Version, String)>> {
    use oned_update::latest::Thing;
    let target = &version.target;
    let thing = |name: String, file: PathBuf| (name, Some(file), version.clone(), config.to_string());
    let mut things = vec![thing(Thing::app_name(target), folder.join("app.zip")), thing(Thing::node_name(target), folder.join(CLI))];
    for entry in std::fs::read_dir(folder)?.flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        let Some(service) = file.strip_prefix(&format!("{CLI}-")) else { continue };
        let name = service.strip_suffix(".zip").unwrap_or(service);
        ensure!(scope::scopes::valid_app_name(name), "{file} in {} isn't a service's", folder.display());
        things.push(thing(Thing::service_name(name, target), entry.path()));
    }
    for (_, file, ..) in &things {
        let file = file.as_ref().context("a file")?;
        ensure!(file.is_file(), "{} isn't there", file.display());
    }
    Ok(things)
}
