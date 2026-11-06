//! The Macs: each builds the Mac app (and the iPhone app) itself, started over Shell, and says how its builds went.

use super::*;

/// Has each Mac build `commit`'s app now, instead of anything older it's building, then the iPhone app, for the iPhone it
/// reaches; says what its last builds did. True if a Mac was missed.
pub(super) fn follow(macs: &[Value], commit: &str, config_commit: Option<&str>) -> bool {
    let mut missed = false;
    for mac in macs {
        let name = mac["name"].as_str().unwrap_or("");
        if mac["online"] != true {
            println!("  {name}: offline; run this again when it's back");
            missed = true;
            continue;
        }
        match shell(mac, &mac_follow(commit, config_commit), 60) {
            Ok(r) if r["exit"] == 0 => {
                for line in r["stdout"].as_str().unwrap_or("").lines() {
                    if line.starts_with(' ') { println!("  {line}") } else { println!("  {name}: {line}") }
                }
            }
            Ok(r) => {
                missed = true;
                println!("  {name}: couldn't start its build: {}", r["stderr"].as_str().unwrap_or("").trim());
            }
            Err(err) => {
                missed = true;
                println!("  {name}: couldn't start its build: {err:#}");
            }
        }
    }
    missed
}

/// What a Mac's update job files say: the build it was last asked for (`want`) and tried, what it built, and its iPhone.
pub(super) struct MacSaid {
    pub(super) want: String,
    pub(super) tried: String,
    pub(super) built: String,
    pub(super) failed_log: Option<String>,
    pub(super) phone: Option<String>,
}

pub(super) fn mac_said(mac: &Value) -> Option<MacSaid> {
    let script = format!(
        r#"c="$HOME/Library/Caches/{CLI}"
for f in want built tried phone.built phone.installed phone.failed; do printf '%s=%s\n' "$f" "$(cat "$c/$f" 2>/dev/null)"; done
echo ---; tail -n 8 "$c/failed.log" 2>/dev/null"#
    );
    let r = shell(mac, &script, 30).ok().filter(|r| r["exit"] == 0)?;
    let out = r["stdout"].as_str()?;
    let (values, log) = out.split_once("---\n")?;
    let get = |key: &str| values.lines().find_map(|l| l.strip_prefix(&format!("{key}="))).unwrap_or("").trim().to_string();
    let commit_of = |v: String| v.split(' ').next().unwrap_or("").to_string();
    let (want, tried, built) = (commit_of(get("want")), commit_of(get("tried")), commit_of(get("built")));
    let (phone_built, installed, phone_failed) = (commit_of(get("phone.built")), commit_of(get("phone.installed")), commit_of(get("phone.failed")));
    let twelve = |c: &str| c[..c.len().min(12)].to_string();
    let phone = match (phone_built.is_empty(), installed.is_empty()) {
        _ if !phone_failed.is_empty() => Some(format!("iPhone: its build of {} failed", twelve(&phone_failed))),
        (true, true) => None,
        _ if phone_built != installed && !phone_built.is_empty() => Some(format!("iPhone: has {}, {} built and installs when it's reachable and unlocked", if installed.is_empty() { "nothing".to_string() } else { twelve(&installed) }, twelve(&phone_built))),
        _ => Some(format!("iPhone: has {}", twelve(&installed))),
    };
    // Failed: the last thing it was asked is what it tried, and that isn't what it built (a newer ask would differ).
    let failed = !tried.is_empty() && tried == want && tried != built;
    Some(MacSaid { failed_log: failed.then(|| log.trim_end().to_string()), want, tried, built, phone })
}

impl MacSaid {
    /// Why a Mac that doesn't run `commit` yet doesn't.
    pub(super) fn waiting_for(&self, commit: &str, short: &str, build: &str, fetched: &mut std::collections::HashSet<String>) -> String {
        let twelve = |c: &str| c[..c.len().min(12)].to_string();
        if has(commit, &self.built, fetched) {
            format!("built {}, its node is restarting on it", twelve(&self.built))
        } else if has(commit, &self.want, fetched) {
            format!("building {} (on {build})", twelve(&self.want))
        } else {
            format!("on {build}; nothing has asked it to build {short} yet: {CLI} update does")
        }
    }
}

/// Shell for a Mac: start a job in its logged-in session (codesign needs the keychain, and the job outlives the node,
/// which the new app replaces) that builds `commit`'s app in a clean clone and installs it, then leaves the app, zipped,
/// and the node binary as blobs, listed in assets.txt, for a release.
pub(super) fn mac_job(commit: &str, config_commit: Option<&str>) -> String {
    let assets = format!(
        r#"a="$c/assets"; rm -rf "$a"; mkdir "$a"
cp target/release/{bin} "$a/{CLI}-$(rustc -vV | sed -n 's/^host: //p')"
ditto -c -k --keepParent "$HOME/Applications/{NAME}.app" "$a/{zip}"
for f in "$a"/*; do
	n=0; until h=$(target/release/{bin} file put "$f"); do n=$((n + 1)); [ $n -lt 30 ]; sleep 2; done
	echo "$h $(basename "$f")" >> "$c/assets.txt"
done"#,
        zip = app_zip(),
        bin = env!("CARGO_BIN_NAME")
    );
    format!(
        r#"set -e
c="$HOME/Library/Caches/{CLI}"; label=app.{CLI}.update
mkdir -p "$c"; cd "$c"; rm -f update.status assets.txt
cat > update.sh <<'JOB'
c="$HOME/Library/Caches/{CLI}"
exec > "$c/update.log" 2>&1
trap 'echo $? > "$c/update.status"' EXIT
set -e
{checkout}
CONFIG="$config" client/native/disk.sh retry client/native/mac/build.sh --release
{assets}
JOB
{start}"#,
        checkout = mac_checkout(commit, config_commit.unwrap_or("")),
        start = mac_start("update.sh")
    )
}

/// Shell for a Mac: has it build `commit`'s app now, and install it, then the iPhone app, if what that's made of changed,
/// for the iPhone, all in one job in its logged-in session: a newer update stops the build it's on and builds that
/// instead. It installs the iPhone app whenever the phone is reachable, and keeps trying until it is. A job already
/// running takes this update's build scripts at its next build, and tries a build that failed again if they changed.
/// Says how its last builds failed, once, and what the iPhone has.
pub(super) fn mac_follow(commit: &str, config_commit: Option<&str>) -> String {
    let short = &commit[..commit.len().min(12)];
    format!(
        r#"set -e
c="$HOME/Library/Caches/{CLI}"; label=app.{CLI}.update
mkdir -p "$c"; cd "$c"
if [ -f failed ]; then echo "its build of $(cut -c1-12 failed) failed:"; sed 's/^/    /' failed.log; rm -f failed; fi
if [ -f phone.failed ]; then echo "its iPhone build of $(cut -c1-12 phone.failed) failed:"; sed 's/^/    /' phone.failed.log; rm -f phone.failed; fi
if [ -s phone.built ]; then
	b=$(cut -c1-12 phone.built)
	if [ "$(cat phone.built)" = "$(cat phone.installed 2>/dev/null)" ]; then echo "iPhone: installed $b"; else echo "iPhone: not reachable, will install $b when it is"; fi
fi
echo "builds {short} now, instead of anything older, then the iPhone app if it changed"
# The scripts are replaced whole, so a job that's running them keeps the old ones until its next build takes the new.
put() {{ cat > "$1.new"; mv "$1.new" "$1"; }}
put build.sh <<'JOB'
set -e
c="$HOME/Library/Caches/{CLI}"
{checkout}
CONFIG="$config" client/native/disk.sh retry client/native/mac/build.sh
JOB
put phone.sh <<'JOB'
set -e
c="$HOME/Library/Caches/{CLI}"
{checkout}
# Only what the iPhone app is made of: the Mac app's but for what's the Mac's own (its window's code, its node's
# services and helpers, apps), and each network closes the app on the phone.
last=$(cat "$c/phone.built" 2>/dev/null || true)
if [ "${{last#* }}" = "${{2:-}}" ] && git diff --quiet "${{last%% *}}" "$1" -- . ':!client/native/mac' ':!client/native/windows' ':!client/native/android' ':!service' ':!apps' ':!docs' ':!tests' ':!*.md' 2>/dev/null; then exit 3; fi
CONFIG="$config" client/native/disk.sh retry client/native/iphone/build.sh device "$c/phone"
JOB
put follow.sh <<'JOB'
# launchd runs this in the login shell, zsh on a Mac; the job control it uses is bash's.
[ -n "$BASH_VERSION" ] || exec /bin/bash "$0" "$@"
c="$HOME/Library/Caches/{CLI}"
# Each build in a process group of its own, which a newer update stops (client/native/mac/build.sh's install ignores that).
set -m
want() {{ cat "$c/want"; }}
# A build that failed is tried again when its scripts change (a newer update's, if the job is running), and not otherwise.
scripts() {{ cat "$c/build.sh" "$c/phone.sh" | cksum; }}
retry() {{ [ -f "$c/tried.scripts" ] && [ "$(scripts)" != "$(cat "$c/tried.scripts")" ]; }}
run() {{
	stopped=
	nice -n 10 /bin/sh "$@" & pid=$!
	while kill -0 $pid 2>/dev/null; do
		sleep 2
		if [ "$(want)" != "$want" ]; then stopped=1; kill -TERM -- -$pid 2>/dev/null; break; fi
	done
	wait $pid
}}
# The paired iPhone devicectl reaches, unlocked; the kept iPhone app is installed on it, not opened: if it's in use,
# installing replaces it all the same, so it's left for the person to open.
install() {{
	xcrun devicectl list devices --json-output "$c/devices.json" > /dev/null 2>&1 || return 1
	phone=$(jq -r '[.result.devices[] | select(.hardwareProperties.reality == "physical" and .hardwareProperties.deviceType == "iPhone" and .connectionProperties.pairingState == "paired")][0].identifier // empty' "$c/devices.json")
	[ -n "$phone" ] && xcrun devicectl device install app --device "$phone" "$c"/phone/*.app
}}
while :; do
	want=$(want)
	if [ "$want" != "$(cat "$c/tried" 2>/dev/null)" ] || retry; then
		with=$(scripts); rm -f "$c/tried" "$c/tried.scripts"
		# zsh doesn't split "$want" into its commit and config: this is bash.
		set -- ${{want%% *}} "${{want#* }}"
		run "$c/build.sh" "$1" "$2" > "$c/update.log" 2>&1; status=$?
		[ -z "$stopped" ] || continue
		if [ $status = 0 ]; then echo "$want" > "$c/built"; else
			tail -n 25 "$c/update.log" > "$c/failed.log"; echo "$want" | tee "$c/failed" > "$c/tried"; echo "$with" > "$c/tried.scripts"; continue
		fi
		# 3: nothing the iPhone app is made of changed.
		run "$c/phone.sh" "$1" "$2" > "$c/phone.log" 2>&1; status=$?
		[ -z "$stopped" ] || continue
		case $status in
		0) echo "$want" > "$c/phone.built" ;;
		3) ;;
		*) tail -n 25 "$c/phone.log" > "$c/phone.failed.log"; echo "$want" > "$c/phone.failed" ;;
		esac
		echo "$want" > "$c/tried"
	fi
	[ -s "$c/phone.built" ] && [ "$(cat "$c/phone.built")" != "$(cat "$c/phone.installed" 2>/dev/null)" ] || exit 0
	if install >> "$c/phone.log" 2>&1; then cp "$c/phone.built" "$c/phone.installed"; continue; fi
	for _ in $(seq 30); do sleep 2; if [ "$(want)" != "$want" ] || retry; then break; fi; done
done
JOB
# The scripts first: a running job stops its build for this and starts the next with them.
echo "{commit} {config}" > want
pid=$(launchctl list | awk -v l="$label" '$3 == l {{ print $1 }}')
if [ -n "$pid" ] && [ "$pid" != "-" ]; then exit 0; fi
{start}"#,
        config = config_commit.unwrap_or(""),
        checkout = mac_checkout("$1", "${2:-}"),
        start = mac_start("follow.sh")
    )
}

/// Shell that puts the Mac's clean clone at `commit` and sets `config` to the config to build with: the clone's own, or
/// for a custom build whose config is in its own repo, that repo at `config_commit`.
pub(super) fn mac_checkout(commit: &str, config_commit: &str) -> String {
    let config = if CONFIG_REPO.is_empty() {
        format!(r#"config="$c/src/{CONFIG_FILE}""#)
    } else {
        format!(
            r#"[ -d "$c/config/.git" ] || git clone -q {CONFIG_REPO} "$c/config"
git -C "$c/config" fetch -q origin
git -C "$c/config" checkout -q --detach --force "{config_commit}"
config="$c/config/{CONFIG_FILE}""#
        )
    };
    format!(
        r#"[ -d "$c/src/.git" ] || git clone -q https://github.com/{REPO}.git "$c/src"
cd "$c/src"
git fetch -q --tags --force origin
git checkout -q --detach --force "{commit}"
git clean -qfdx -e /target -e /client/native/target
client/native/disk.sh prune
{config}"#
    )
}

/// Shell that runs `script` (in the Mac's cache) as a job of its logged-in session, from a login shell.
pub(super) fn mac_start(script: &str) -> String {
    format!(
        r#"cat > update.plist <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>Label</key><string>$label</string><key>RunAtLoad</key><true/>
<key>ProgramArguments</key><array><string>${{SHELL:-/bin/zsh}}</string><string>-l</string><string>$c/{script}</string></array>
</dict></plist>
PLIST
launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" update.plist"#
    )
}

/// Waits for a Mac's job to finish; on success, the assets it left as `(blob, name)`, and the warnings its build gave.
pub(super) fn await_mac(mac: &Value) -> Result<(Vec<(String, String)>, Vec<String>)> {
    let deadline = Instant::now() + Duration::from_secs(30 * 60);
    let poll = format!(r#"c="$HOME/Library/Caches/{CLI}"; cat "$c/update.status" 2>/dev/null"#);
    // The Mac's node restarts on the new app part way through, and doesn't answer for a moment.
    let status = loop {
        ensure!(Instant::now() < deadline, "its build took more than 30 minutes");
        std::thread::sleep(Duration::from_secs(5));
        if let Ok(r) = shell(mac, &poll, 30)
            && let Some(code) = r["stdout"].as_str().and_then(|s| s.trim().parse::<i32>().ok())
        {
            break code;
        }
    };
    let report = shell(mac, &format!(r#"c="$HOME/Library/Caches/{CLI}"; launchctl bootout "gui/$(id -u)/app.{CLI}.update" 2>/dev/null; cat "$c/assets.txt" 2>/dev/null; echo ---; grep '^warning: ' "$c/update.log"; tail -n 25 "$c/update.log""#), 30)?;
    let out = report["stdout"].as_str().unwrap_or("");
    let (made, log) = out.split_once("---\n").unwrap_or(("", out));
    if status != 0 {
        bail!("its build failed:\n{log}");
    }
    let mut warnings: Vec<String> = log.lines().filter(|l| l.starts_with("warning: ")).map(str::to_string).collect();
    warnings.sort();
    warnings.dedup();
    Ok((made.lines().filter_map(|l| l.split_once(' ')).map(|(h, n)| (h.to_string(), n.to_string())).collect(), warnings))
}
