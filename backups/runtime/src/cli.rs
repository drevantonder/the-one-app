//! Opening a backup on a disk with the recovery key alone. This is recovery from nothing, so it's a command on the
//! machine the disk is in, and asks no device: there may be none, and no network. The key stays in this process.

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use api::cli::{Command, Given};

pub fn commands() -> Command {
    Command {
        help: vec![("backups check --folder <path> --recovery-key <file>".into(), "open a backup on a disk with the recovery key alone, from a file or - for stdin, and check what comes back: no device or network needed".into())],
        run,
    }
}

fn run(g: &Given) -> Result<bool> {
    let (["backups", "check"], Some(folder)) = (g.words().as_slice(), g.flag("folder")) else { return Ok(false) };
    let key = key::recovery_key_in(g.flag("recovery-key").context("give --recovery-key: the file the recovery key is in, or - for stdin")?)?;
    let report = check_folder(folder, &key)?;
    println!("{}", if g.json { report.to_string() } else { api::tools::said(&report) });
    Ok(true)
}

/// Every network the backup in `folder` holds, each opened and checked.
fn check_folder(folder: &str, recovery_key: &str) -> Result<Value> {
    let secret = key::unhex(recovery_key).context("a recovery key is 64 hex characters")?;
    let place = bucket::Place::Folder(folder.into());
    let mut out = Vec::new();
    for network in crate::engine::networks(&place)? {
        let keys = crate::engine::open_keys(&place, &network, &secret)?;
        let mut r = crate::engine::check(&crate::engine::Check { place: &place, network: &network, keys: &keys, root: None })?.0;
        r["place"] = json!(folder);
        out.push(r);
    }
    ensure!(!out.is_empty(), "no backup in {folder}");
    Ok(json!(out))
}
