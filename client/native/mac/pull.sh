#!/bin/bash
# The Mac's job beside its app, run by launchd (app.@CLI@.pull) now and then: it reads the signed record at the network's
# address (src/latest.rs, whose format is the one thing that must never break) and brings this Mac up to it, by the app
# one Mac published, or, if none has, by building it here. It lives outside the app, so nothing it installs can replace it
# while it runs, and needs nothing of the network but a GET: it's what gets a Mac out of being too old to speak the rest.
# Installed by the Mac's oned at its start, or once by `@CLI@-dev update mac-job | ssh <mac> sh` for a Mac too old to have it (src/latest.rs).
# Bad or missing record, or a failed download or build: it stays on what it has, and says why in `why`.
[ -n "${BASH_VERSION:-}" ] || exec /bin/bash "$0" "$@"
set -u
cli=@CLI@ name='@NAME@' address='@ADDRESS@' key='@KEY@' repo='@REPO@' config_repo='@CONFIG_REPO@' config_file=@CONFIG_FILE@ signer='@SIGNER@'
c="$HOME/Library/Caches/$cli"
bin="$HOME/Applications/$name.app/Contents/MacOS/$cli"
case $(uname -m) in arm64 | aarch64) target=aarch64-apple-darwin ;; *) target=x86_64-apple-darwin ;; esac
mkdir -p "$c"; cd "$c" || exit 1

say() { printf '%s\n' "$*" > "$c/why"; }
fail() { say "$*"; exit 0; }

# One build at a time on this Mac, whichever network's job runs it: a build takes the lock (a link to its pid, made in one
# step), and a job that finds it held leaves its build for its next run. A dead job's lock is taken over.
lock="$HOME/Library/Caches/app-build.lock"
building() {
	local holder
	[ "$(readlink "$lock" 2>/dev/null)" != "$$" ] || return 0
	ln -s "$$" "$lock" 2>/dev/null || {
		holder=$(readlink "$lock" 2>/dev/null)
		if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then rm -f "$lock"; ln -s "$$" "$lock" 2>/dev/null || return 1; else return 1; fi
	}
	trap 'rm -f "$lock"' EXIT
}

# A build yields to whatever the Mac is otherwise doing: lowest priority, and half its cores.
yielding() {
	local cores; cores=$(sysctl -n hw.ncpu 2>/dev/null || echo 2)
	CARGO_BUILD_JOBS=$((cores / 2 > 0 ? cores / 2 : 1)) nice -n 19 "$@"
}

# Reads and checks the record; fails, having said why, if it isn't one signed by this build's release key.
record() {
	curl -fsSL -m 30 "$address/updates/newest" -o newest.new 2>/dev/null || { say "couldn't reach $address"; return 1; }
	sed -n '/^-----BEGIN SSH SIGNATURE-----$/,$p' newest.new > newest.sig
	sed '/^-----BEGIN SSH SIGNATURE-----$/,$d' newest.new > newest.body
	printf 'release namespaces="release-latest" %s\n' "$key" > signers
	ssh-keygen -Y verify -f signers -I release -n release-latest -s newest.sig < newest.body > /dev/null 2>&1 || { say "the latest record isn't signed by this build's release key, so it isn't taken"; return 1; }
}

# A thing's line in the record: its six fields, one per word.
line() { awk -v n="$1" '$1 == n && NF == 6' newest.body | head -n 1; }

# Whether two commits are the same one: a build's short form is its first twelve, a custom build's followed by its config.
same() { [ "${1:0:12}" = "${2:0:12}" ]; }

# Runs a build in its own process group, stopped for a newer record; `stopped` says if it was.
run() {
	stopped=
	set -m
	"$@" > "$c/update.log" 2>&1 &
	local pid=$! n=0
	set +m
	while kill -0 "$pid" 2>/dev/null; do
		sleep 2
		if [ $((++n % 30)) = 0 ] && record && ! same "$(line source | cut -d' ' -f2)" "$want_commit"; then
			stopped=1
			kill -TERM -- "-$pid" 2>/dev/null
			break
		fi
	done
	wait "$pid"
}

# The clean clone at a commit, with `config` the config to build with.
checkout() {
	[ -d "$c/src/.git" ] || git clone -q "https://github.com/$repo.git" "$c/src" || return 1
	cd "$c/src" && git fetch -q --tags --force origin && git checkout -q --detach --force "$1" && git clean -qfdx -e /target -e /client/native/target || return 1
	if [ -n "$config_repo" ]; then
		[ -d "$c/config/.git" ] || git clone -q "$config_repo" "$c/config" || return 1
		git -C "$c/config" fetch -q origin && git -C "$c/config" checkout -q --detach --force "$2" || return 1
		config="$c/config/$config_file"
	else
		config="$c/src/$config_file"
	fi
	client/native/disk.sh prune > "$c/prune.out" 2>&1 || { cp "$c/prune.out" "$c/space"; return 1; }
	rm -f "$c/space"
}

# This commit's app, built here because no Mac has published it: then published, if this Mac has the release key.
build_here() {
	local want="$want_commit $want_config" with
	with=$(cksum < "$0")
	if [ "$want" = "$(cat tried 2>/dev/null)" ] && [ "$with" = "$(cat tried.job 2>/dev/null)" ]; then
		[ ! -e failed ] || say "Failed: its build of ${want_commit:0:12} failed: $(tail -n 8 failed.log | tr '\n' ' ')"
		exit 0
	fi
	building || { say "another build is running on this Mac: ${want_commit:0:12} is built when it's done"; exit 0; }
	checkout "$want_commit" "$want_config" > "$c/update.log" 2>&1; status=$?
	cd "$c" || exit 1
	if [ $status != 0 ]; then
		if [ -f space ]; then say "Failed: no room to build: $(tail -n 3 space | tr '\n' ' ')"; else say "couldn't get its code: $(tail -n 4 update.log | tr '\n' ' ')"; fi
		exit 0
	fi
	say "building ${want_commit:0:12}"
	local notarize=""; if same "$(line source-release | cut -d' ' -f2)" "$want_commit"; then notarize=--release; fi
	run yielding bash -c "cd '$c/src' && CONFIG='$config' client/native/disk.sh retry client/native/mac/build.sh $notarize"
	local status=$?
	[ -z "$stopped" ] || exit 0
	echo "$want" > tried; echo "$with" > tried.job
	if [ $status != 0 ]; then
		tail -n 25 update.log > failed.log; touch failed
		say "Failed: its build of ${want_commit:0:12} failed: $(tail -n 8 failed.log | tr '\n' ' ')"
		exit 0
	fi
	rm -f failed
	if [ -n "$signer" ]; then
		ditto -c -k --keepParent "$HOME/Applications/$name.app" app.zip && "$bin" publish-app --zip "$c/app.zip" --config "${want_config:--}" > publish.log 2>&1 || true
	fi
}

# Takes the app published for this commit, once it's the file the record names.
take() {
	local sha=$1 commit=$2
	say "taking ${commit:0:12}"
	curl -fsSL -m 1800 "$address/updates/$sha" -o app.zip.new || fail "couldn't download ${commit:0:12}"
	[ "$(shasum -a 256 app.zip.new | cut -d' ' -f1)" = "$sha" ] || { rm -f app.zip.new; fail "the app the record names for ${commit:0:12} didn't arrive as signed, so it isn't taken"; }
	rm -rf fresh; mkdir fresh; ditto -x -k app.zip.new fresh || fail "couldn't unpack ${commit:0:12}"
	local app; app=$(cd fresh && ls -d -- *.app | head -n 1); [ -n "$app" ] || fail "no app in what's published for ${commit:0:12}"
	CONFIG_NAME="$name" CONFIG_CLI="$cli" /bin/bash "$c/install-app.sh" "fresh/$app" > install.log 2>&1 || fail "couldn't install ${commit:0:12}: $(tail -n 4 install.log | tr '\n' ' ')"
	rm -rf fresh app.zip.new
}

# The paired iPhone devicectl reaches, unlocked; the kept iPhone app is installed on it, not opened: if it's in use,
# installing replaces it all the same, so it's left for the user to open.
install_phone() {
	xcrun devicectl list devices --json-output "$c/devices.json" > /dev/null 2>&1 || return 1
	local phone
	phone=$(jq -r '[.result.devices[] | select(.hardwareProperties.reality == "physical" and .hardwareProperties.deviceType == "iPhone" and .connectionProperties.pairingState == "paired")][0].identifier // empty' "$c/devices.json")
	[ -n "$phone" ] && xcrun devicectl device install app --device "$phone" "$c"/phone/*.app
}

# The iPhone app, if what it's made of changed since the last built here; then installed when its iPhone is reachable.
phone() {
	local want="$want_commit $want_config" last
	last=$(cat phone.built 2>/dev/null || true)
	if [ "$want" != "$last" ] && [ "$want" != "$(cat phone.tried 2>/dev/null)" ] && building; then
		echo "$want" > phone.tried
		if checkout "$want_commit" "$want_config" > phone.log 2>&1; then
			if [ "${last#* }" = "$want_config" ] && git diff --quiet "${last%% *}" "$want_commit" -- . ':!client/native/mac' ':!client/native/windows' ':!client/native/android' ':!service' ':!apps' ':!docs' ':!tests' ':!*.md' 2>/dev/null; then
				:
			elif CONFIG="$config" yielding client/native/disk.sh retry client/native/iphone/build.sh device "$c/phone" >> "$c/phone.log" 2>&1; then
				echo "$want" > "$c/phone.built"
			else
				say "iPhone: its build of ${want_commit:0:12} failed: $(tail -n 4 "$c/phone.log" | tr '\n' ' ')"
				return
			fi
		fi
		cd "$c" || exit 1
	fi
	if [ -s phone.built ] && [ "$(cat phone.built)" != "$(cat phone.installed 2>/dev/null)" ]; then
		if install_phone >> phone.log 2>&1; then cp phone.built phone.installed; say "iPhone: has ${want_commit:0:12}"; else say "iPhone: ${want_commit:0:12} built and installs when it's reachable and unlocked"; fi
	fi
}

record || exit 0
parsed=$(line source)
[ -n "$parsed" ] || exit 0
read -r _ want_commit want_committed _ _ want_config <<< "$parsed"
[ "$want_config" != "-" ] || want_config=""
version=$("$bin" version --json 2>/dev/null || true)
running=$(printf %s "$version" | sed -n 's/.*"commit":"\([^"]*\)".*/\1/p')
running_committed=$(printf %s "$version" | sed -n 's/.*"committed":\([0-9]*\).*/\1/p')
# A record older than what runs here isn't taken: it never goes backwards.
if [ -n "$running_committed" ] && [ "${want_committed:-0}" -lt "$running_committed" ]; then
	say "runs a build newer than the record's ${want_commit:0:12}; staying on it"
	exit 0
fi
if ! same "$running" "$want_commit"; then
	app=$(line "app/$target")
	if [ -n "$app" ] && same "$(cut -d' ' -f2 <<< "$app")" "$want_commit"; then
		take "$(cut -d' ' -f4 <<< "$app")" "$want_commit"
	else
		build_here
	fi
fi
say ""
phone
