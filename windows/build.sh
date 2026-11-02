#!/usr/bin/env bash
# Builds the Windows app in the Windows VM on this machine (~/devices/windows: WinUI 3 builds only on Windows), then
# installs it there and starts it. CONFIG=path/to/config.toml builds another white label.
set -euo pipefail
cd "$(dirname "$0")"
root=$(cd .. && pwd)
config=$(cd "$(dirname "${CONFIG:-$root/config.toml}")" && pwd)/$(basename "${CONFIG:-config.toml}")
# A top-level field: the [settings] after them are the install's, not the build's.
field() { sed -n "/^\[/q; s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$config"; }
art() { echo "$(dirname "$config")/$(field "$1")"; }
name=$(field name) cli=$(field cli)
commit=$(git -C "$root" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)

stage=$(mktemp -d)/$cli
trap 'rm -rf "$(dirname "$stage")"' EXIT
mkdir -p "$stage/Assets"
cp ./*.cs App.xaml One.csproj app.manifest ./*.ps1 "$stage/"
cp "$(art icon)" "$stage/Assets/Icon.png"
cp "$(art logo)" "$stage/Assets/Logo.png"
cp "$root/vendor/phosphor/icons.json" "$stage/Assets/icons.json"
# C#'s string literals, from the config's text.
quote() { printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"; }
cat > "$stage/Brand.g.cs" <<CS
// Written by build.sh from $(basename "$config"): the white label this build is.
namespace One;

public static class Brand
{
    public const string Name = $(quote "$name");
    public const string Cli = $(quote "$cli");
    public const string Accent = $(quote "$(field accent)");
    /// The install's address, for a white label with one install; empty, the person gives it.
    public const string Address = $(quote "$(field address)");
    public const string Commit = $(quote "$commit");
    /// Where this app keeps what it keeps on this computer.
    public static string Folder => System.IO.Path.Combine(System.Environment.GetFolderPath(System.Environment.SpecialFolder.LocalApplicationData), Name);
}
CS
printf '{"name":%s,"file":%s}\n' "$(quote "$name")" "$(quote "${name//[^A-Za-z0-9]/}")" > "$stage/build.json"

vm=~/devices/windows
ssh=(ssh -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=$HOME/devices/windows-vm/known_hosts -o LogLevel=ERROR -o ConnectTimeout=10 one@127.0.0.1)
"${ssh[@]}" exit 2>/dev/null || { "$vm" start; until "${ssh[@]}" exit 2>/dev/null; do sleep 5; done; }
"${ssh[@]}" "Remove-Item -Recurse -Force build\\$cli -ErrorAction SilentlyContinue; New-Item -ItemType Directory -Force build | Out-Null"
scp -q -r -P 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile="$HOME/devices/windows-vm/known_hosts" -o LogLevel=ERROR "$stage" "one@127.0.0.1:build/"
"${ssh[@]}" "powershell -NoProfile -ExecutionPolicy Bypass -File build\\$cli\\build.ps1"
