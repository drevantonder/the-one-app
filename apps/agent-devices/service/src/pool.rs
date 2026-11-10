//! Borrowed devices: an agent borrows a device of its own for as long as it needs a browser, and gives it back, which
//! deletes it. They come from a pool: one spare is kept running, so borrowing is quick. A borrowed device's browser
//! starts with a copy of a browser profile (profiles.rs), which the user signed in to sites in once (sign_in); the
//! copy is thrown away with the device, so only sign_in and save_profile change a profile.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, bail};
use one_service::refuse;
use serde::{Deserialize, Serialize};

use super::linux::{Device, HOME, SIZES};
use super::profiles::Profile;
use super::{Devices, quote, time};

pub const POOL_PREFIX: &str = "pool-";
/// A borrowed device left this long is given back.
pub const IDLE_BORROWED: Duration = Duration::from_secs(15 * 60);
/// A sign-in is saved and closed after this, by default.
pub const SIGN_IN_FOR: Duration = Duration::from_secs(30 * 60);
/// Where the browser tools keep their profiles, in a Linux device.
pub const PROFILES: &str = "/home/cua/.local/state/cua-driver/browser-profiles/";
/// Marks the browser sign_in opens, which the tools didn't, so closing the tools' leftover browsers leaves it open.
const SIGN_IN_CLASS: &str = "--class=one-sign-in";
/// A sign-in whose profile can't be saved is given back unsaved this long after its time is up, as when the app never
/// gives its key back after a restart, or its device won't start.
const SIGN_IN_GRACE: Duration = Duration::from_secs(10 * 60);

/// A borrowed device's loan: who has it and what for, and the browser profile it started with, with its key, kept
/// only in memory: after a restart, the app gives it again when the profile is saved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Loan {
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub r#for: String,
    /// The profile's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    /// Its Vault item.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub item: String,
    #[serde(with = "time", default = "time::zero")]
    pub since: jiff::Timestamp,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub signing_in: bool,
    /// A sign-in's end.
    #[serde(with = "time::maybe", default, skip_serializing_if = "Option::is_none")]
    pub until: Option<jiff::Timestamp>,
    /// Made without a screen: nobody watches it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_screen: bool,
    #[serde(default, skip_serializing_if = "LoanState::is_lent")]
    pub state: LoanState,
    #[serde(skip)]
    pub key: Option<Profile>,
}

impl Loan {
    /// Whether it's a sign-in that lost its profile's key (this program restarted) with time left to save it: only the
    /// app can give the key again, in a call of its own, so giving it back waits for the app's save_profile.
    pub(super) fn waits_for_key(&self) -> bool {
        self.state == LoanState::Lent && self.signing_in && self.key.is_none() && !self.late()
    }

    fn late(&self) -> bool {
        self.until.is_none_or(|u| super::now() > u + jiff::SignedDuration::try_from(SIGN_IN_GRACE).unwrap_or_default())
    }
}

impl Default for Loan {
    fn default() -> Self {
        Loan { by: String::new(), r#for: String::new(), profile: String::new(), item: String::new(), since: time::zero(), signing_in: false, until: None, no_screen: false, state: LoanState::Lent, key: None }
    }
}

/// A loan's state. It starts preparing, its device chosen and being readied; once ready it's lent, and the
/// borrower's; giving it back, it's returning, until the device is deleted and the loan forgotten, or back to lent if
/// that failed. Each step is one of the methods below, all under the pool's lock, and each is saved in lent.json, so a
/// restart carries on: a loan preparing when this program stopped never got ready, and is given back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum LoanState {
    #[default]
    #[serde(rename = "")]
    Lent,
    #[serde(rename = "preparing")]
    Preparing,
    #[serde(rename = "returning")]
    Returning,
}

impl LoanState {
    fn is_lent(&self) -> bool {
        *self == LoanState::Lent
    }
}

/// Borrowed devices by name, and each device's last call.
#[derive(Default)]
pub struct Pool {
    lent: HashMap<String, Loan>,
    used: HashMap<String, jiff::Timestamp>,
}

/// The caches left out of saved profiles and backups, each a path in a Chrome folder (* is one of its profiles, like
/// Default): they are most of a profile's size and rebuild themselves. So are a running browser's locks, which would
/// make a copy look in use by another device. Service Worker/ScriptCache isn't one: it's the only copy of an
/// extension's background script, and without it the extension (1Password) never answers.
const CACHES: [&str; 12] = [
    "*/Cache",
    "*/Code Cache",
    "*/GPUCache",
    "*/DawnGraphiteCache",
    "*/DawnWebGPUCache",
    "*/Service Worker/CacheStorage",
    "GPUPersistentCache",
    "GrShaderCache",
    "ShaderCache",
    "SingletonLock",
    "SingletonSocket",
    "SingletonCookie",
];

/// tar's arguments that leave out the caches of the Chrome folders at `at` (a path from where tar starts, where * is
/// any one folder) and `other`: each only there, from the start of a path, with * within one folder, never a folder of
/// the same name elsewhere, like one in an extension or a user's.
pub fn excludes(at: &str, other: &[&str]) -> Vec<String> {
    let mut args = vec!["--anchored".to_string(), "--no-wildcards-match-slash".to_string()];
    args.extend(CACHES.iter().map(|c| format!("--exclude={at}/{c}")));
    args.extend(other.iter().map(|o| format!("--exclude={o}")));
    args
}

pub fn new_pool_name() -> String {
    format!("{POOL_PREFIX}{}", super::random_hex(3))
}

pub fn spares(all: &[Device], lent: &HashMap<String, Loan>) -> Vec<Device> {
    all.iter().filter(|d| d.pool && !lent.contains_key(&d.name) && d.state != "stopped").cloned().collect()
}

/// Makes the profile one the browser tools will open: they only open profiles they made, as a file in it says.
pub fn owned(name: &str) -> String {
    let dir = format!("{PROFILES}{name}");
    let json = format!("{{\"schema\":\"cua-driver-browser-profile-v1\",\"mode\":\"isolated_named\",\"name\":{}}}", serde_json::to_string(name).unwrap_or_default());
    format!("mkdir -p {dir} && printf '%s' '{json}' > {dir}/.cua-driver-owned-profile.json && chmod 700 {dir} && chown -R cua:cua {HOME}/.local")
}

/// Whether this device keeps a spare agent device: a Linux one with room for it. A Mac's memory is its user's; it
/// makes devices only when asked.
fn keeps_spare() -> bool {
    !cfg!(target_os = "macos") && room_for_one()
}

/// Whether another small device fits: as much memory available as it may use.
pub fn room_for_one() -> bool {
    memory().0 >= (SIZES[0].gb as u64) << 30
}

/// This device's available and total memory, in bytes: available less the swap in use.
pub fn memory() -> (u64, u64) {
    if cfg!(target_os = "macos") {
        return mac_memory();
    }
    let Ok(info) = std::fs::read_to_string("/proc/meminfo") else { return (0, 0) };
    let (mut free, mut total, mut swap_total, mut swap_free) = (0u64, 0u64, 0u64, 0u64);
    for line in info.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let n: u64 = v.trim().trim_end_matches(" kB").trim().parse().unwrap_or(0) << 10;
        match k {
            "MemAvailable" => free = n,
            "MemTotal" => total = n,
            "SwapTotal" => swap_total = n,
            "SwapFree" => swap_free = n,
            _ => {}
        }
    }
    // Swap in use is memory already short: what's available counts only once it's paid back.
    (free.saturating_sub(swap_total.saturating_sub(swap_free)), total)
}

/// macOS's own measure of the memory free, compressed pages and swap counted: memory_pressure's free percentage of
/// the Mac's memory.
fn mac_memory() -> (u64, u64) {
    let Ok(out) = super::linux::cmd("sysctl").args(["-n", "hw.memsize"]).output() else { return (0, 0) };
    let total: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0);
    let Ok(out) = super::linux::cmd("memory_pressure").arg("-Q").output() else { return (0, total) };
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let pct: u64 = text.split("free percentage:").nth(1).map(|p| p.trim().trim_end_matches('%').trim()).and_then(|p| p.parse().ok()).unwrap_or(0);
    (total * pct / 100, total)
}

impl Devices {
    pub(super) fn loans(&self) -> HashMap<String, Loan> {
        self.pool.lock().unwrap().lent.clone()
    }

    pub(super) fn reserve(&self, name: &str, mut l: Loan) {
        let mut pool = self.pool.lock().unwrap();
        l.state = LoanState::Preparing;
        pool.lent.insert(name.into(), l);
        self.save_loans(&pool);
    }

    pub(super) fn lend(&self, name: &str) {
        let mut pool = self.pool.lock().unwrap();
        if let Some(l) = pool.lent.get_mut(name).filter(|l| l.state == LoanState::Preparing) {
            l.state = LoanState::Lent;
            self.save_loans(&pool);
        }
    }

    pub(super) fn change(&self, name: &str, f: impl FnOnce(&mut Loan)) -> bool {
        let mut pool = self.pool.lock().unwrap();
        let Some(l) = pool.lent.get_mut(name).filter(|l| l.state == LoanState::Lent) else { return false };
        f(l);
        self.save_loans(&pool);
        true
    }

    /// Starts giving back a loan, preparing or lent: the loan as it was, which only one caller gets.
    pub(super) fn returning(&self, name: &str) -> Result<Loan> {
        let mut pool = self.pool.lock().unwrap();
        let Some(l) = pool.lent.get_mut(name) else { refuse!("device {name} isn't borrowed") };
        if l.state == LoanState::Returning {
            refuse!("device {name} is being given back already");
        }
        let was = l.clone();
        l.state = LoanState::Returning;
        self.save_loans(&pool);
        Ok(was)
    }

    pub(super) fn return_failed(&self, name: &str) {
        let mut pool = self.pool.lock().unwrap();
        if let Some(l) = pool.lent.get_mut(name) {
            l.state = LoanState::Lent;
            self.save_loans(&pool);
        }
    }

    pub(super) fn end_loan(&self, name: &str) {
        let mut pool = self.pool.lock().unwrap();
        if pool.lent.remove(name).is_some() {
            self.save_loans(&pool);
        }
    }

    fn save_loans(&self, pool: &Pool) {
        let write = || -> Result<()> {
            let path = self.dir.join("lent.json");
            let part = self.dir.join("lent.json.part");
            std::fs::write(&part, serde_json::to_vec_pretty(&pool.lent)?)?;
            std::fs::rename(part, path)?;
            Ok(())
        };
        if let Err(err) = write() {
            tracing::warn!("keeping the loans: {err:#}");
        }
    }

    pub(super) fn load_loans(&self) {
        let mut pool = self.pool.lock().unwrap();
        pool.lent = std::fs::read(self.dir.join("lent.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        for l in pool.lent.values_mut() {
            if l.state == LoanState::Returning {
                l.state = LoanState::Lent; // given back again, as it would be
            }
        }
        let used: HashMap<String, String> = std::fs::read(self.dir.join("used.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        pool.used = used.into_iter().filter_map(|(k, v)| Some((k, v.parse().ok()?))).collect();
    }

    /// Notes a call to a device, which keeps it from being given back or paused while idle.
    pub(super) fn touch(&self, name: &str) {
        self.pool.lock().unwrap().used.insert(name.into(), super::now());
    }

    pub(super) fn used(&self) -> HashMap<String, jiff::Timestamp> {
        self.pool.lock().unwrap().used.clone()
    }

    /// How long since the device's last call, or since this program first saw it.
    pub(super) fn idle(&self, name: &str) -> Duration {
        let mut pool = self.pool.lock().unwrap();
        match pool.used.get(name) {
            Some(t) => super::since(*t),
            None => {
                pool.used.insert(name.into(), super::now());
                Duration::ZERO
            }
        }
    }

    /// Keeps each device's last call on disk, so a restart doesn't make every device look just used, forgetting the
    /// devices not kept.
    fn save_used(&self, kept: impl Fn(&str) -> bool) {
        let mut pool = self.pool.lock().unwrap();
        pool.used.retain(|name, _| kept(name));
        let used: HashMap<&String, String> = pool.used.iter().map(|(k, v)| (k, v.to_string())).collect();
        if let Err(err) = serde_json::to_vec(&used).map_err(anyhow::Error::from).and_then(|b| Ok(std::fs::write(self.dir.join("used.json"), b)?)) {
            tracing::warn!("{err:#}");
        }
    }

    /// Lends the spare device, or one made for it, its browser starting with the profile p, if one is given.
    pub(super) fn borrow(&self, l: Loan, p: Option<Profile>) -> Result<Device> {
        let take = self.take.lock().unwrap();
        self.borrow_taken(take, l, p)
    }

    /// `borrow` with take held, which it lets go once it has chosen the device. The loan is the borrower's only once
    /// the device is ready: on any failure before then it's given back.
    fn borrow_taken(&self, take: std::sync::MutexGuard<()>, mut l: Loan, p: Option<Profile>) -> Result<Device> {
        if let Some(p) = &p {
            (l.profile, l.item, l.key) = (p.name.clone(), p.item.clone(), Some(p.clone()));
        }
        let all = self.list()?;
        let free = spares(&all, &self.loans());
        let d = if let Some(d) = free.first() {
            d.clone()
        } else if !room_for_one() {
            refuse!("this device has no memory to spare for another agent device: give back one you're done with");
        } else {
            self.create(&new_pool_name(), SIZES[0], true, false)?
        };
        l.since = super::now();
        let (by, r#for, profile) = (l.by.clone(), l.r#for.clone(), l.profile.clone());
        self.reserve(&d.name, l);
        self.touch(&d.name);
        drop(take);
        self.poke(); // for another spare
        let ready = (|| -> Result<()> {
            if d.state == "paused" {
                self.rt()?.unpause(&d)?;
            }
            super::linux::ready(&d)?;
            if let Some(p) = &p {
                self.unpack(&d, p)?;
            }
            Ok(())
        })();
        if let Err(err) = ready {
            if let Err(e) = self.give_back(&d.name, &format!("couldn't prepare it: {err:#}")) {
                tracing::warn!("giving back {}: {e:#}", d.name);
            }
            return Err(err);
        }
        self.lend(&d.name);
        tracing::info!("{by} borrowed {} for {for:?}, profile {profile:?}", d.name);
        Ok(d)
    }

    pub(super) fn hold_key(&self, name: &str, p: &Profile) {
        let p = p.clone();
        self.change(name, |l| l.key = Some(p));
    }

    fn open_page(&self, d: &Device, name: &str, url: &str) -> Result<()> {
        let url = if url.is_empty() { "about:blank" } else { url };
        self.sh(
            d,
            &format!(
                "{} && (runuser -u cua -- env DISPLAY=:1 HOME={HOME} setsid google-chrome {SIGN_IN_CLASS} --user-data-dir={PROFILES}{name} --password-store=basic --no-first-run --no-default-browser-check --start-maximized {} >/dev/null 2>&1 </dev/null &)",
                owned(name),
                quote(url)
            ),
        )
    }

    /// Deletes a borrowed device; a sign-in's profile is saved first.
    pub(super) fn give_back(&self, name: &str, why: &str) -> Result<()> {
        let Some(l) = self.loans().get(name).cloned() else { refuse!("device {name} isn't borrowed") };
        let late = l.late();
        if l.waits_for_key() {
            refuse!("not given back: the {} profile's key went when this program restarted, so it's saved by save_profile", l.profile);
        }
        let mut l = self.returning(name)?;
        if let Ok(d) = self.find(name) {
            if l.state == LoanState::Lent && l.signing_in {
                match self.save_profile(&d, l.key.as_mut()) {
                    Err(err) if !late => {
                        self.return_failed(name);
                        bail!("not given back: saving the {} profile failed: {err:#}", l.profile);
                    }
                    Err(err) => tracing::warn!("{name}: giving it back without saving the {} profile: {err:#}", l.profile),
                    Ok(()) => {}
                }
            }
            self.drop_conn(name);
            if let Err(err) = self.remove_device(&d) {
                self.return_failed(name);
                return Err(err);
            }
        }
        tracing::info!("{name}: {why}");
        self.end_loan(name);
        self.poke();
        Ok(())
    }

    /// Saves the device's copy of a profile as its newest save, recorded on its Vault item by the app.
    pub(super) fn save_profile(&self, d: &Device, p: Option<&mut Profile>) -> Result<()> {
        let Some(p) = p else { refuse!("its profile's key is gone, as after this device restarted: save it with save_profile") };
        match d.state.as_str() {
            "paused" => self.rt()?.unpause(d)?,
            "stopped" => self.start_device(d)?,
            _ => {}
        }
        let s = self.pack(d, p)?;
        self.saved(&s)?;
        // The next save starts from this one.
        (p.blob, p.salt) = (s.blob, s.salt);
        Ok(())
    }

    /// Opens a page in a device on the profile, for the user to sign in to sites in, reusing one already signing in
    /// to it. take is held while it looks for one, so two sign-ins to a profile share one device.
    pub(super) fn sign_in(&self, p: &Profile, url: &str, minutes: i64, by: &str) -> Result<(Device, jiff::Timestamp)> {
        let until = super::now() + jiff::SignedDuration::from_mins(minutes);
        let take = self.take.lock().unwrap();
        if let Some((name, l)) = self.loans().into_iter().find(|(_, l)| l.signing_in && l.item == p.item) {
            drop(take);
            if l.state != LoanState::Lent {
                refuse!("the {} profile's sign-in is starting or closing: try again in a moment", p.name);
            }
            let d = self.running(&name)?;
            let p2 = p.clone();
            self.change(&name, move |l| {
                (l.until, l.profile) = (Some(until), p2.name.clone());
                if l.key.is_none() {
                    l.key = Some(p2);
                }
            });
            self.open_page(&d, &p.name, url)?;
            return Ok((d, until));
        }
        let loan = Loan { by: by.into(), r#for: "Signing in to sites".into(), signing_in: true, until: Some(until), ..Default::default() };
        let d = self.borrow_taken(take, loan, Some(p.clone()))?;
        self.open_page(&d, &p.name, url)?;
        Ok((d, until))
    }

    pub(super) fn signing_in_here(&self, item: &str) -> bool {
        self.loans().values().any(|l| l.signing_in && l.item == item)
    }

    pub(super) fn tend(&self) {
        let all = match self.list() {
            Ok(all) => all,
            Err(err) => {
                tracing::warn!("{err:#}");
                return;
            }
        };
        self.tend_vms();
        self.tend_moved(&all);
        let here: std::collections::HashSet<String> = all.iter().map(|d| d.name.clone()).collect();
        let vms: std::collections::HashSet<String> = self.vms().into_iter().map(|v| v.name).collect();
        self.save_used(|name| here.contains(name) || vms.contains(name));
        for (name, l) in self.loans() {
            if vms.contains(&name) || l.state == LoanState::Returning || l.state == LoanState::Preparing && l.since > self.started {
                continue;
            }
            let why = if l.state == LoanState::Preparing {
                "never got ready before this program restarted"
            } else if !here.contains(&name) {
                "gone"
            } else if l.signing_in && l.until.is_none_or(|u| super::now() > u) {
                "sign-in time up"
            } else if !l.signing_in && self.idle(&name) > IDLE_BORROWED {
                "idle for 15 minutes"
            } else {
                continue;
            };
            if let Err(err) = self.give_back(&name, why) {
                tracing::warn!("giving back {name}: {err:#}");
            }
        }

        let _take = self.take.lock().unwrap();
        let Ok(all) = self.list() else { return };
        let lent = self.loans();
        for d in all.iter().filter(|d| d.pool && !lent.contains_key(&d.name) && d.state == "stopped") {
            self.drop_conn(&d.name);
            let _ = self.remove_device(d);
        }
        // A spare on an older image makes way for one on this week's.
        let image = super::linux::image_for(super::now());
        let mut free: Vec<Device> = spares(&all, &lent)
            .into_iter()
            .filter(|d| {
                if d.image == image {
                    return true;
                }
                self.drop_conn(&d.name);
                let _ = self.remove_device(d);
                false
            })
            .collect();
        free.sort_by(|a, b| a.name.cmp(&b.name));
        for d in free.iter().skip(1) {
            self.drop_conn(&d.name);
            let _ = self.remove_device(d);
        }
        if free.is_empty() && keeps_spare() && self.rt.is_some() {
            match self.create(&new_pool_name(), SIZES[0], true, true) {
                Ok(d) => {
                    self.touch(&d.name);
                    self.refresh_if_empty();
                }
                Err(err) => tracing::warn!("making a spare device: {err:#}"),
            }
        }
        self.pause_idle(&all, &lent);
    }
}
