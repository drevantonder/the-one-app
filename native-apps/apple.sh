# Sourced by the Mac's and iPhone's build scripts: config.toml's fields (or $CONFIG's), the build's commit and number,
# the Apple team, and `session`, which runs what needs the keychain in the logged-in session.
# An ssh session starts with 256 open files, which a build of the iOS crates (sccache hashing thousands of sources) runs out of.
ulimit -n 10240 2>/dev/null || true
native=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$native/.." && pwd)
config=$(cd "$(dirname "${CONFIG:-$root/config.toml}")" && pwd)/$(basename "${CONFIG:-config.toml}")
# A top-level field: the [settings] after them are the network's, not the build's.
field() { sed -n "/^\[/q; s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$config"; }
art() { echo "$(dirname "$config")/$(field "$1")"; }
# The commit it's built from, which Settings shows at its foot; and the build number, the commits up to it, so
# devicectl tells which commit an iPhone has.
export BUILD_COMMIT=$(git -C "$root" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
export BUILD_NUMBER=$(git -C "$root" rev-list --count HEAD 2>/dev/null || echo 1)
export CONFIG_NAME=$(field name) CONFIG_CLI=$(field cli) CONFIG_PORT=$(field port)
# The iPhone app's bundle id (build.rs `IPHONE` says the same); its extensions, App Group and Keychain group are made from it.
export CONFIG_IPHONE=$(field iphone)
[ -n "$CONFIG_IPHONE" ] || CONFIG_IPHONE=app.$CONFIG_CLI.phone
# What a user calls the network where they read: a workplace's custom build, which its users join with their work
# email (join_domain), is shared, "the PTC App network"; anyone else's is "your One App network".
if [ -n "$(field join_domain)" ]; then CONFIG_YOURS="${CONFIG_NAME/#The /the } network"; else CONFIG_YOURS="your ${CONFIG_NAME#The } network"; fi
export CONFIG_YOURS
# Signed by the team of this Mac's Apple Development certificate, unless APPLE_TEAM names one.
export APPLE_TEAM=${APPLE_TEAM:-$(security find-certificate -c "Apple Development" -p 2>/dev/null | openssl x509 -noout -subject 2>/dev/null | sed -n 's/.*OU *= *\([A-Z0-9]*\).*/\1/p')}

# One build at a time on this Mac: two at once take it to a load nothing else can run under. `alone "$@"` from a build
# script holds a turn until it ends; what it calls waits for nothing.
alone() {
	[ -z "${ONE_BUILD_TURN:-}" ] || return 0
	export ONE_BUILD_TURN=1
	local lock=${TMPDIR:-/tmp}/one-build.lock
	lockf -k -t 0 "$lock" true 2>/dev/null || echo "Another build is running on this Mac: waiting for it to end."
	exec lockf -k "$lock" "$0" "$@"
}

# Runs a command as a one-shot job in the logged-in session, and prints its output: over SSH the keychain, which
# codesign, notarytool and Xcode's signing need, is out of reach.
session() {
	local job=app.$CONFIG_CLI.session.$$ out status plist
	out=$(mktemp) status=$(mktemp) plist=$(mktemp).plist
	printf '%s\n' '<?xml version="1.0" encoding="UTF-8"?>' '<plist version="1.0"><dict>' \
		"<key>Label</key><string>$job</string><key>RunAtLoad</key><true/><key>WorkingDirectory</key><string>$PWD</string>" \
		'<key>ProgramArguments</key><array><string>/bin/sh</string><string>-c</string>' \
		"<string>\"\$0\" \"\$@\" &gt; '$out' 2&gt;&amp;1; echo \$? &gt; '$status'</string>" \
		"$(printf '<string>%s</string>' "$@")</array></dict></plist>" > "$plist"
	launchctl bootstrap "gui/$(id -u)" "$plist"
	until [ -s "$status" ]; do sleep 1; done
	launchctl bootout "gui/$(id -u)/$job" 2>/dev/null || true
	cat "$out"
	return "$(cat "$status")"
}
# What needs the keychain: tried here, then in the logged-in session.
keychain() { "$@" 2>/dev/null || session "$@"; }
