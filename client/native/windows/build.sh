#!/usr/bin/env bash
# Builds the Windows app: its program (client/native) cross-built here for Windows (x86_64-pc-windows-gnu, with
# mingw-w64, beside the WebView2 loader it starts its window with), then packaged with its helper (Helper.csproj: the
# Windows App SDK's, which builds only on Windows) in the Windows VM on this machine (Agent Devices' windows, or
# DEVICE=<name>), installed there and started. CONFIG=path/to/config.toml builds another custom build; PROFILE=dev
# builds the program as `cargo build` does (what `one-dev throwaway client windows` runs: quicker, and drivable by its
# CONTROL port), release otherwise; NO_START=1 installs it without starting it.
#   --ssh <ssh's arguments>   builds on a Windows machine ssh reaches instead (`--ssh surface`): its user needs to be
#                             signed in at its desktop. It's built in a folder of its own there, which goes after, and
#                             installed for that user, who gets the .NET 10 SDK and a signing certificate (setup.ps1).
set -euo pipefail
target=
if [ "${1:-}" = --ssh ]; then target=${2:?--ssh needs the machine, as ssh names it}; fi
cd "$(dirname "$0")"
native=$(cd .. && pwd)
root=$(cd ../../.. && pwd)
config=$(cd "$(dirname "${CONFIG:-$root/config.toml}")" && pwd)/$(basename "${CONFIG:-config.toml}")
profile=${PROFILE:-release}
# A top-level field: the [settings] after them are the network's, not the build's.
field() { sed -n "/^\[/q; s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$config"; }
art() { echo "$(dirname "$config")/$(field "$1")"; }
name=$(field name) cli=$(field cli)

stage=$(mktemp -d)/$cli
# Where it builds, from the user's home there (the same under either shell ssh gives): the VM's own folder, kept for the
# next build, or on another machine one of this run's, which goes after.
built=build\\$cli
cleanup() { rm -rf "$(dirname "$stage")"; }
if [ -n "$target" ]; then
  # shellcheck disable=SC2206  # ssh's arguments, split on purpose
  ssh=(ssh -o ConnectTimeout=10 $target)
  built=one-build-$$
  cleanup() {
    rm -rf "$(dirname "$stage")"
    "${ssh[@]}" "powershell -NoProfile -Command \"Remove-Item -Recurse -Force $built -ErrorAction SilentlyContinue\"" || true
  }
fi
trap cleanup EXIT
mkdir -p "$stage/Assets" "$stage/app"
cp ./*.cs Helper.csproj ./*.ps1 "$stage/"
# The package: named for the custom build, its widgets' COM class steady for it, its version newer each build (days
# since 2026, then the minute of that day), so installing it updates the one there, keeping its sign-in.
file=${name//[^A-Za-z0-9]/}
guid() { printf '%s' "$1" | sha1sum | sed -E 's/^(.{8})(.{4})(.{4})(.{4})(.{12}).*/\1-\2-\3-\4-\5/'; }
widgets=$(guid "$cli widgets") notes=$(guid "$cli notifications")
# Pushes come by the custom build's Microsoft Entra app, if it has one: none, and the package doesn't name it.
wns_app=$(field wns_app) wns_object=$(field wns_object)
if [ -n "$wns_app" ]; then wns='/<!--\/\{0,1\}wns-->/d'; else wns='/<!--wns-->/,/<!--\/wns-->/d'; fi
now=$(date -u +%s)
version="1.$(( (now - $(date -u -d 2026-01-01 +%s)) / 86400 )).$(( now % 86400 / 60 )).0"
# Links to the config's address open the app; a custom build without one names a host no link has.
host=$(field address | sed -E 's|^[a-z]+://||; s|[/:].*||'); host=${host:-$cli.invalid}
xml() { printf '%s' "$1" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g; s/"/\&quot;/g; s/[&|\\]/\\&/g'; }
sed -e "s|@ID@|$file|g; s|@PUBLISHER@|CN=$(xml "${name//[^A-Za-z0-9 ]/}")|g; s|@VERSION@|$version|g; s|@NAME@|$(xml "$name")|g" \
  -e "s|@FILE@|$file|g; s|@CLI@|$(xml "$cli")|g; s|@WIDGETS@|$widgets|g; s|@HOST@|$(xml "$host")|g; s|@NOTES@|$notes|g; s|@WNS_APP@|$(xml "$wns_app")|g" \
  -e "$wns" Package.appxmanifest |
  awk '/<Definition /{d=1} d{b=b $0 "\n"} !d{print} /<\/Definition>/{d=0; for (n=1; n<=4; n++) {x=b; gsub(/@N@/, n, x); printf "%s", x}}' > "$stage/Package.appxmanifest"
cp "$(art icon)" "$stage/Assets/Icon.png"
# C#'s string literals, from the config's text.
quote() { printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"; }
cat > "$stage/CustomBuild.g.cs" <<CS
// Written by build.sh from $(basename "$config"): the custom build this build is.
namespace One;

public static class CustomBuild
{
    public const string Name = $(quote "$name");
    public const string Cli = $(quote "$cli");
    /// The app's program, beside the helper: <File>.exe.
    public const string File = $(quote "$file");
    /// The COM class the Widgets board starts the helper as, for its widgets (Package.appxmanifest).
    public const string WidgetClass = "$widgets";
    /// The custom build's Microsoft Entra app, which WNS knows this app by (config.toml's wns_app, and its service
    /// principal's object id, wns_object): empty for none, and it gets no pushes.
    public const string WnsApp = $(quote "$wns_app");
    public const string WnsObject = $(quote "$wns_object");
}
CS
# The app's program, for Windows, built here with mingw-w64; its debug lines go, which only make it slow to send.
(cd "$native" && CONFIG=$config nice cargo build -q --target x86_64-pc-windows-gnu $([ "$profile" = release ] && echo --release))
out=$native/target/x86_64-pc-windows-gnu/$([ "$profile" = release ] && echo release || echo debug)
x86_64-w64-mingw32-strip -o "$stage/app/$file.exe" "$out/one-native.exe"
loader=$(ls -t "$out"/build/webview2-com-sys-*/out/x64/WebView2Loader.dll | head -1)
cp "$loader" "$stage/app/"
printf '{"name":%s,"file":%s,"publisher":%s,"start":%s}\n' "$(quote "$name")" "$(quote "$file")" "$(quote "CN=${name//[^A-Za-z0-9 ]/}")" "$([ -n "${NO_START:-}" ] && echo false || echo true)" > "$stage/build.json"

if [ -z "$target" ]; then
  # The device's SSH port and folder, as Agent Devices keeps them.
  device=${DEVICE:-windows}
  kept=$HOME/.local/share/one/agent-devices/devices/$device.json
  port=$(sed -n 's/.*"port": *\([0-9]*\).*/\1/p' "$kept") dir=$(sed -n 's/.*"dir": *"\([^"]*\)".*/\1/p' "$kept")
  opts=(-o StrictHostKeyChecking=no -o "UserKnownHostsFile=$dir/known_hosts" -o LogLevel=ERROR -o ConnectTimeout=10)
  ssh=(ssh -p "$port" "${opts[@]}" one@127.0.0.1)
  "${ssh[@]}" exit 2>/dev/null || { one agent-devices start "$device" >/dev/null; until "${ssh[@]}" exit 2>/dev/null; do sleep 5; done; }
fi
# One stream into Windows' own tar, run by ssh's shell itself (cmd or PowerShell), since a stream of more than about
# 20 MB through another PowerShell breaks off: scp takes seconds a file there. A VM just started may drop it: again.
for try in 1 2 3 4 5 6; do
  "${ssh[@]}" "powershell -NoProfile -Command \"Remove-Item -Recurse -Force $built -ErrorAction SilentlyContinue; New-Item -ItemType Directory -Force $built | Out-Null\""
  tar -C "$stage" -cf - . | "${ssh[@]}" "tar -xf - -C $built" && break
  [ "$try" = 6 ] && exit 1
  sleep 10
done
"${ssh[@]}" "powershell -NoProfile -ExecutionPolicy Bypass -File $built\\build.ps1"
