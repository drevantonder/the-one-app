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
    let mut kept: Vec<_> = fs::read_dir(own)?.map(|entry| Ok(entry?.file_name())).collect::<Result<_>>()?;
    kept.sort_by_key(|name| name == "key");
    for name in kept {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// An app signed with no App Group keeps its client in its own folder; a later build that has one moves it
    /// there whole, once, and a cut-short move is done again.
    #[test]
    fn a_phone_keeps_its_client_in_its_group_once_it_has_one() {
        let (own, group) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (own, group) = (own.path(), group.path().join("group"));
        fs::write(own.join("key"), [7; 32]).unwrap();
        fs::write(own.join("network"), b"{}").unwrap();
        fs::create_dir_all(own.join("kept/answers")).unwrap();
        fs::write(own.join("kept/answers/one"), b"kept").unwrap();

        // No group: it stays where it is.
        assert_eq!(home(own, None).unwrap(), own);
        assert_eq!(fs::read(own.join("key")).unwrap(), [7; 32]);
        assert!(!group.exists());

        // A later build with a group, after a move cut short: some of it there already, but not the key.
        fs::create_dir_all(group.join("kept")).unwrap();
        fs::write(group.join("kept/half"), b"half").unwrap();
        assert_eq!(home(own, Some(&group)).unwrap(), group);
        assert_eq!(fs::read(group.join("key")).unwrap(), [7; 32]);
        assert_eq!(fs::read(group.join("network")).unwrap(), b"{}");
        assert_eq!(fs::read(group.join("kept/answers/one")).unwrap(), b"kept");
        assert!(!group.join("kept/half").exists());
        assert_eq!(fs::read_dir(own).unwrap().count(), 0);

        // Once the group holds a key, nothing moves over it.
        fs::write(own.join("key"), [9; 32]).unwrap();
        assert_eq!(home(own, Some(&group)).unwrap(), group);
        assert_eq!(fs::read(group.join("key")).unwrap(), [7; 32]);
    }

    /// A first start with a group and nothing kept yet starts in the group.
    #[test]
    fn a_new_phone_starts_in_its_group() {
        let (own, group) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert_eq!(home(own.path(), Some(group.path())).unwrap(), group.path());
    }
}
