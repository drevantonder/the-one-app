//! Where an iPhone's app keeps its client.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The folder an iPhone's app keeps its client in: its App Group, the `group` iOS gives it if it's signed with one,
/// which it shares with its notification extension, and otherwise its `own` folder. A client kept in its own folder
/// moves to the group the first time there is one, so its person stays signed in. Nothing moves once the group holds
/// a key. The key moves last, so a move that's cut short is done again whole at the next start.
pub fn home(own: &Path, group: Option<&Path>) -> Result<PathBuf> {
    let Some(home) = group else { return Ok(own.into()) };
    if home.join("key").exists() || !own.join("key").exists() {
        return Ok(home.into());
    }
    fs::create_dir_all(home)?;
    let mut names: Vec<_> = fs::read_dir(own)?.map(|entry| Ok(entry?.file_name())).collect::<Result<_>>()?;
    names.sort_by_key(|name| name == "key");
    for name in names {
        let to = home.join(&name);
        // What a move that was cut short left there goes first: it's moved again whole.
        match fs::metadata(&to) {
            Ok(there) if there.is_dir() => fs::remove_dir_all(&to)?,
            Ok(_) => fs::remove_file(&to)?,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        fs::rename(own.join(&name), &to)
            .with_context(|| format!("couldn't move {} to {}", name.display(), home.display()))?;
    }
    Ok(home.into())
}
