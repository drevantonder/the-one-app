//! The installers, `install.sh` and Windows' `install.ps1`, each taking the newest build for its machine from a folder
//! laid out as the network's `/updates/` is (`latest`), checked as `one update` checks it: the network's own, which its
//! address serves them for (`/install`, `/install.ps1`), or a GitHub release's, for someone with no network yet.

use custom_build::config::CLI;

/// install.sh, taking builds from `from`.
pub fn sh(from: &str) -> String {
    filled(include_str!("../../../install.sh"), from, |name, value| format!("{name}='{value}'"))
}

/// install.ps1, taking builds from `from`.
pub fn powershell(from: &str) -> String {
    filled(include_str!("../../../install.ps1"), from, |name, value| format!("${name} = '{value}'"))
}

/// What the network at `address` serves at its address: each installer, by its path, taking builds from the address.
pub fn served(address: &str) -> [(String, Vec<u8>); 2] {
    let from = format!("{}/{}", address.trim_end_matches('/'), crate::latest::FOLDER);
    [(custom_build::protocol::INSTALL.into(), sh(&from).into_bytes()), (custom_build::protocol::INSTALL_POWERSHELL.into(), powershell(&from).into_bytes())]
}

/// `script` with its empty `from`, `cli` and `key` (the release key a device here takes builds by), each a line as
/// `line` writes one, said.
fn filled(script: &str, from: &str, line: impl Fn(&str, &str) -> String) -> String {
    [("from", from), ("cli", CLI), ("key", platform::mode::here().release_key())].iter().fold(script.to_string(), |out, (name, value)| out.replacen(&format!("\n{}\n", line(name, "")), &format!("\n{}\n", line(name, value)), 1))
}

#[cfg(test)]
mod tests {
    use crate::latest::{Latest, Thing, sha256};
    use ssh_key::PrivateKey;
    use ssh_key::private::Ed25519Keypair;

    fn key(seed: u8) -> PrivateKey {
        PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
    }

    /// What fills them in is a line each script has empty.
    #[test]
    fn each_installer_says_the_release_key() {
        for script in [super::sh("x"), super::powershell("x")] {
            assert!(script.contains(platform::mode::here().release_key()));
        }
    }

    /// The installers are shell and PowerShell, so they can't read the table of platforms: each names what the record
    /// names for every one of them.
    #[test]
    fn the_installers_take_each_platforms_build() {
        let (sh, powershell) = (include_str!("../../../install.sh"), include_str!("../../../install.ps1"));
        for p in &crate::platform::PLATFORMS {
            let (arch, rest) = p.target.split_once('-').unwrap();
            match p.target {
                crate::platform::WINDOWS => assert!(powershell.contains(&format!("'{}'", Thing::program_name(p.target))), "install.ps1 takes {}", p.target),
                target => {
                    let named = if crate::latest::is_mac(target) { Thing::app_name(&format!("$arch-{rest}")) } else { Thing::program_name(&format!("$arch-{rest}")) };
                    assert!(sh.contains(&format!("fetch \"{named}\"")), "install.sh takes {named} for {target}");
                    assert!(sh.contains(&format!("arch={arch}")), "install.sh knows the processor {arch}");
                }
            }
        }
    }

    /// install.sh's own reading of the record and download, from a folder (curl reads file:// too) laid out as the
    /// network's /updates/ is: only a record the release key signed, and only the file it names by its hash.
    #[test]
    fn the_installer_takes_only_what_the_release_key_signed() {
        let release = key(1);
        let dir = std::env::temp_dir().join(format!("one-install-{}", rand::random::<u32>()));
        let (served, tmp) = (dir.join("served"), dir.join("tmp"));
        std::fs::create_dir_all(&served).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        let program = b"program".to_vec();
        let thing = Thing { name: "node/x".into(), commit: "c".into(), committed: 1, sha256: sha256(&program), size: program.len() as u64, config: "-".into() };
        let record = |signer: &PrivateKey| Latest { things: vec![thing.clone()] }.signed(signer).unwrap();
        std::fs::write(served.join(&thing.sha256), &program).unwrap();
        let script = super::sh(&format!("file://{}", served.display()));
        let functions = &script[script.find("newest() {").unwrap()..script.find("case $(uname -m)").unwrap()];
        let installs = || {
            let sh = format!("set -eu\ntmp={}; from=file://{}; key='{}'\n{functions}\nnewest; fetch node/x program", tmp.display(), served.display(), release.public_key().to_openssh().unwrap());
            std::process::Command::new("sh").args(["-c", &sh]).output().unwrap().status.success()
        };
        std::fs::write(served.join("newest"), record(&release)).unwrap();
        assert!(installs());
        assert_eq!(std::fs::read(tmp.join("program")).unwrap(), program);
        std::fs::write(served.join(&thing.sha256), b"programme").unwrap();
        assert!(!installs(), "a download that isn't what the record names");
        std::fs::write(served.join(&thing.sha256), &program).unwrap();
        std::fs::write(served.join("newest"), record(&key(2))).unwrap();
        assert!(!installs(), "a record another key signed");
        std::fs::write(served.join("newest"), record(&release).replacen("node/x c 1", "node/x c 2", 1)).unwrap();
        assert!(!installs(), "a record changed after it was signed");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
