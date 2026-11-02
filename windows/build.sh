#!/usr/bin/env bash
# Builds the Windows app in the Windows VM on this machine (Agent Devices' windows, or DEVICE=<name>, a copy borrowed for
# it: WinUI 3 builds only on Windows), then installs it there and starts it. CONFIG=path/to/config.toml builds another
# white label.
set -euo pipefail
cd "$(dirname "$0")"
root=$(cd .. && pwd)
config=$(cd "$(dirname "${CONFIG:-$root/config.toml}")" && pwd)/$(basename "${CONFIG:-config.toml}")
# A top-level field: the [settings] after them are the install's, not the build's.
field() { sed -n "/^\[/q; s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$config"; }
art() { echo "$(dirname "$config")/$(field "$1")"; }
name=$(field name) cli=$(field cli)
# What a person calls the install where they read: a workplace's white label, which its people join with their work
# email (join_domain), is shared, "the PTC App"; anyone else's is "your One App".
if [ -n "$(field join_domain)" ]; then yours=${name/#The /the }; else yours="your ${name#The }"; fi
commit=$(git -C "$root" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)

stage=$(mktemp -d)/$cli
trap 'rm -rf "$(dirname "$stage")"' EXIT
mkdir -p "$stage/Assets"
cp ./*.cs App.xaml One.csproj app.manifest ./*.ps1 "$stage/"
# The package: named for the white label, its widgets' COM class steady for it, its version newer each build (days
# since 2026, then the minute of that day), so installing it updates the one there, keeping its sign-in.
file=${name//[^A-Za-z0-9]/}
widgets=$(printf '%s widgets' "$cli" | sha1sum | sed -E 's/^(.{8})(.{4})(.{4})(.{4})(.{12}).*/\1-\2-\3-\4-\5/')
now=$(date -u +%s)
version="1.$(( (now - $(date -u -d 2026-01-01 +%s)) / 86400 )).$(( now % 86400 / 60 )).0"
# Links to the config's address open the app; a white label without one has the app add the address it signs in to
# (Shell.Claim), and names a host no link has.
host=$(field address | sed -E 's|^[a-z]+://||; s|[/:].*||'); host=${host:-$cli.invalid}
xml() { printf '%s' "$1" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g; s/"/\&quot;/g; s/[&|\\]/\\&/g'; }
sed -e "s|@ID@|$file|g; s|@PUBLISHER@|CN=$(xml "${name//[^A-Za-z0-9 ]/}")|g; s|@VERSION@|$version|g; s|@NAME@|$(xml "$name")|g" \
  -e "s|@FILE@|$file|g; s|@CLI@|$(xml "$cli")|g; s|@WIDGETS@|$widgets|g; s|@HOST@|$(xml "$host")|g" Package.appxmanifest |
  awk '/<Definition /{d=1} d{b=b $0 "\n"} !d{print} /<\/Definition>/{d=0; for (n=1; n<=4; n++) {x=b; gsub(/@N@/, n, x); printf "%s", x}}' > "$stage/Package.appxmanifest"
cp "$(art icon)" "$stage/Assets/Icon.png"
cp "$(art logo)" "$stage/Assets/Logo.png"
cp "$root/vendor/phosphor/icons.json" "$stage/Assets/icons.json"
# C#'s string literals, from the config's text.
quote() { printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"; }
cat > "$stage/WhiteLabel.g.cs" <<CS
// Written by build.sh from $(basename "$config"): the white label this build is.
namespace One;

public static class WhiteLabel
{
    public const string Name = $(quote "$name");
    public const string Cli = $(quote "$cli");
    /// What a person calls the install: "your One App", or a workplace's, "the PTC App".
    public const string Yours = $(quote "$yours");
    public const string Accent = $(quote "$(field accent)");
    /// The install's address, for a white label with one install; empty, the person gives it.
    public const string Address = $(quote "$(field address)");
    public const string Commit = $(quote "$commit");
    /// The COM class the Widgets board starts this program as, for its widgets (Package.appxmanifest).
    public const string WidgetClass = "$widgets";
    /// Where this app keeps what it keeps on this computer.
    public static string Folder => System.IO.Path.Combine(System.Environment.GetFolderPath(System.Environment.SpecialFolder.LocalApplicationData), Name);
}
CS
printf '{"name":%s,"file":%s,"publisher":%s}\n' "$(quote "$name")" "$(quote "$file")" "$(quote "CN=${name//[^A-Za-z0-9 ]/}")" > "$stage/build.json"

# The device's SSH port and folder, as Agent Devices keeps them.
device=${DEVICE:-windows}
kept=$HOME/.local/share/one/agent-devices/devices/$device.json
port=$(sed -n 's/.*"port": *\([0-9]*\).*/\1/p' "$kept") dir=$(sed -n 's/.*"dir": *"\([^"]*\)".*/\1/p' "$kept")
opts=(-o StrictHostKeyChecking=no -o "UserKnownHostsFile=$dir/known_hosts" -o LogLevel=ERROR -o ConnectTimeout=10)
ssh=(ssh -p "$port" "${opts[@]}" one@127.0.0.1)
"${ssh[@]}" exit 2>/dev/null || { one agent-devices start "$device" >/dev/null; until "${ssh[@]}" exit 2>/dev/null; do sleep 5; done; }
"${ssh[@]}" "Remove-Item -Recurse -Force build\\$cli -ErrorAction SilentlyContinue; New-Item -ItemType Directory -Force build | Out-Null"
scp -q -r -P "$port" "${opts[@]}" "$stage" "one@127.0.0.1:build/"
"${ssh[@]}" "powershell -NoProfile -ExecutionPolicy Bypass -File build\\$cli\\build.ps1"
