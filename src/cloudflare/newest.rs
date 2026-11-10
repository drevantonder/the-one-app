//! What a deploy sends: the Workers' sources and the web client, from the signed record at the network's address where
//! that's newer than what this program was built with, so a change to them alone reaches Cloudflare with no node build
//! and no wait for this device to take one.

use anyhow::{Context, Result};

use crate::config::COMMITTED;
use crate::latest::{Latest, Thing};
use crate::web::bundle::{Bundle, open};

use super::Files;

/// The Workers' sources and the web client's files a deploy sends, and which commit each came from.
pub struct Newest {
    pub workers: Files,
    pub web: Files,
    pub said: String,
}

impl Newest {
    /// As this program was built with them.
    pub fn built() -> Newest {
        Newest { workers: crate::ui_folder::cloudflare(), web: crate::web::with_client(), said: format!("Workers and web client from this build, {}", crate::config::BUILD) }
    }

    /// The newest of each, as the record at `address` names them or as this program was built with them. Failing to read
    /// a record that's there is an error: deploying what's built in instead could take Cloudflare backwards.
    pub fn read(address: &str) -> Result<Newest> {
        let Some(text) = crate::latest::fetch(address)? else { return Ok(Newest::built()) };
        let record = Latest::read(crate::mode::here().release_key(), &text).context("the record at the network's address")?;
        Newest::from_record(&record, COMMITTED, &|thing| crate::latest::download(address, thing))
    }

    /// Each of the two as `record` names it, if that's at least as new as `committed`, this program's build, else as
    /// this program has it.
    fn from_record(record: &Latest, committed: u64, get: &dyn Fn(&Thing) -> Result<Vec<u8>>) -> Result<Newest> {
        let built = Newest::built();
        let newer = |name: &str| record.get(name).filter(|t| t.committed >= committed).cloned();
        let (workers, from_workers) = match newer(Thing::WORKERS) {
            Some(thing) => (Bundle::unpacked(&get(&thing)?).context("the Workers' sources in the record")?.files, thing.commit),
            None => (built.workers, "this build".to_string()),
        };
        let (web, from_web) = match newer(Thing::WEB) {
            Some(thing) => (open(crate::mode::here().release_key(), &get(&thing)?).context("the web client in the record")?.files, thing.commit),
            None => (built.web, "this build".to_string()),
        };
        Ok(Newest { workers, web, said: format!("Workers from {from_workers}, web client from {from_web}") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thing(name: &str, commit: &str, committed: u64, sha256: &str) -> Thing {
        Thing { name: name.into(), commit: commit.into(), committed, sha256: sha256.into(), size: 0, config: "-".into() }
    }

    #[test]
    fn a_deploy_never_takes_the_workers_backwards() {
        let packed = Bundle { commit: "w1".into(), committed: 50, shell: 1, files: vec![("auth/worker.js".into(), b"new".to_vec())] }.packed().unwrap();
        let get = |thing: &Thing| -> Result<Vec<u8>> {
            anyhow::ensure!(thing.name == Thing::WORKERS, "only the Workers' line is read here");
            Ok(packed.clone())
        };
        let record = Latest { things: vec![thing(Thing::WORKERS, "w1", 50, "aa")] };
        let newest = Newest::from_record(&record, 40, &get).unwrap();
        assert_eq!(newest.workers, vec![("auth/worker.js".to_string(), b"new".to_vec())], "the record's is newer than this build");
        assert!(newest.said.contains("w1"));
        let newest = Newest::from_record(&record, 60, &get).unwrap();
        assert_ne!(newest.workers[0].1, b"new", "this build is newer than the record's");
        assert!(Newest::from_record(&Latest { things: vec![thing(Thing::WORKERS, "w1", 50, "aa")] }, 40, &|_| anyhow::bail!("no such file")).is_err(), "a record it can't follow is an error, not what's built in");
    }
}
