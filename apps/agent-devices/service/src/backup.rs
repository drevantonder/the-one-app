//! Backups: a copy of a Linux device's home folder, without caches, as a blob in the network, so an agent device can
//! be restored on another (move). Each device keeps an index of its own backups in backups.json. A blob lives on the
//! devices that have fetched it, not off-site.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use one_service::refuse;
use serde::{Deserialize, Serialize};

use super::linux::{Device, HOME, Size};
use super::pool::{PROFILES, excludes};
use super::{Devices, time};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub on: String,
    #[serde(default)]
    pub blob: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub size: String,
    #[serde(with = "time", default = "time::zero")]
    pub at: jiff::Timestamp,
}

const KEEP: usize = 5;

impl Devices {
    pub(super) fn backups_here(&self) -> Vec<Record> {
        std::fs::read(self.dir.join("backups.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub(super) fn backup(&self, d: &Device) -> Result<Record> {
        let me = self.hosts()?.0;
        let limits = self.limits(std::slice::from_ref(d)).unwrap_or_default();
        let profiles = format!(".{}*", PROFILES.trim_start_matches(HOME));
        let mut args = vec!["tar".to_string(), "-C".into(), HOME.into(), "--warning=no-file-changed".into()];
        args.extend(excludes(&profiles, &["./.cache"]));
        args.extend(["-czf".into(), "-".into(), ".".into()]);
        let mut tar = self.rt()?.exec(d, &args.iter().map(String::as_str).collect::<Vec<_>>());
        tar.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut child = tar.spawn()?;
        let out = child.stdout.take().unwrap();
        let mut err = child.stderr.take().unwrap();
        let said = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut err, &mut s);
            s
        });
        let put = self.host.put_blob(out);
        let status = child.wait()?;
        let said = said.join().unwrap_or_default();
        // Stored, or why not first: tar stops when what it writes to goes.
        let (blob, bytes) = put.map_err(|err| anyhow!("storing the backup: {err:#}"))?;
        // 1: only that files changed while it read them, as a running browser's do.
        if !status.success() && status.code() != Some(1) {
            bail!("packing {}: {status}: {}", d.name, said.trim());
        }
        let rec = Record { device: d.name.clone(), on: me, blob, bytes, size: limits.get(&d.name).copied().unwrap_or_default().size(), at: super::now() };
        let _lock = self.backups.lock().unwrap();
        let mut all = self.backups_here();
        all.push(rec.clone());
        let kept: Vec<Record> = all.iter().enumerate().filter(|(i, r)| all[i + 1..].iter().filter(|o| o.device == r.device).count() < KEEP).map(|(_, r)| r.clone()).collect();
        std::fs::write(self.dir.join("backups.json"), serde_json::to_vec_pretty(&kept)?)?;
        tracing::info!("backed up {}: blob {}, {} bytes", d.name, rec.blob, rec.bytes);
        Ok(rec)
    }

    pub(super) fn backups_all(&self, device: &str) -> (Vec<Record>, std::collections::BTreeMap<String, String>) {
        let mut all = self.backups_here();
        let (got, failed) = self.everywhere("backups", None);
        for r in got.values() {
            all.extend(serde_json::from_value::<Vec<Record>>(r["backups"].clone()).unwrap_or_default());
        }
        let mut out: Vec<Record> = all.into_iter().filter(|r| device.is_empty() || r.device == device).collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.at));
        (out, failed)
    }

    pub(super) fn restore(&self, name: &str, rec: &Record, size: Size) -> Result<Device> {
        if self.find(name).is_ok() {
            refuse!("device {name} already exists: give as another name for the restored one");
        }
        // Another device serves the blob once the entry naming it reaches it, which takes a moment after a backup.
        let mut tries = 0;
        let mut body = loop {
            match self.host.get_blob(&rec.blob) {
                Ok(b) => break b,
                Err(err) if tries == 5 => bail!("fetching the backup: {err:#}"),
                Err(_) => {
                    tries += 1;
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        };
        let d = self.create(name, size, false, true)?;
        let fail = |err: anyhow::Error| -> Result<Device> {
            self.drop_conn(&d.name);
            let _ = self.remove_device(&d);
            Err(err)
        };
        if let Err(err) = self.exec(&d, Some(&mut body), None, &["tar", "-C", HOME, "-xzf", "-"]) {
            return fail(anyhow!("unpacking the backup: {err:#}"));
        }
        if let Err(err) = self.rt()?.restart(&d).and_then(|()| super::linux::ready(&d)) {
            return fail(err);
        }
        Ok(d)
    }

    pub(super) fn find_backup(&self, device: &str, blob: &str) -> Result<Record> {
        let (all, failed) = self.backups_all("");
        if let Some(r) = all.into_iter().find(|r| (!blob.is_empty() && r.blob == blob) || (blob.is_empty() && r.device == device)) {
            return Ok(r);
        }
        if !blob.is_empty() {
            return Ok(Record { device: device.into(), blob: blob.into(), ..Default::default() });
        }
        if failed.is_empty() {
            refuse!("no backup of device {device}");
        }
        bail!("no backup of device {device}: couldn't ask {} devices", failed.len())
    }
}
