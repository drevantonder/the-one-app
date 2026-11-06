#!/bin/bash
# Builds each Mac service in Swift (apps/<name>/service/Package.swift, on service/swift) into a helper app of its own,
# in target/mac-helpers: "<its CFBundleName> — <the app's name>.app", id app.<cli>.mac.<name>, with its own Info.plist
# and entitlements, signed with the identity SIGN names (ad hoc without), so macOS asks for and records its permission
# under its own name. The Mac app copies them into its Contents/Helpers before it's signed itself, where oned finds
# them. CONFIG names another custom build's config.toml; ICON, an .icns for them.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
config=${CONFIG:-$root/config.toml}
field() { sed -n "/^\[/q; s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$config"; }
name=$(field name) cli=$(field cli)
commit=$(git -C "$root" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
out=$root/target/mac-helpers
rm -rf "$out" && mkdir -p "$out"
sign=(codesign --force --options runtime --sign "${SIGN:--}")
[ -z "${SIGN:-}" ] || sign+=(--timestamp)
for package in "$root"/apps/*/service/Package.swift; do
	[ -e "$package" ] || continue
	dir=$(dirname "$package")
	service=$(basename "$(dirname "$dir")")
	swift build -c release --arch arm64 --package-path "$dir" -q
	bin=$(swift build -c release --arch arm64 --package-path "$dir" --show-bin-path)
	title=$(plutil -extract CFBundleName raw "$dir/Info.plist")
	app="$out/$title — $name.app"
	mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
	cp "$bin/$service" "$app/Contents/MacOS/$cli-$service"
	info=$app/Contents/Info.plist
	cp "$dir/Info.plist" "$info"
	set_() { plutil -replace "$1" "-$2" "$3" "$info"; }
	set_ CFBundleName string "$title — $name"
	set_ CFBundleDisplayName string "$title — $name"
	set_ CFBundleIdentifier string "app.$cli.mac.$service"
	set_ CFBundleExecutable string "$cli-$service"
	set_ CFBundlePackageType string APPL
	set_ CFBundleShortVersionString string 1.0
	set_ CFBundleVersion string "$(git -C "$root" rev-list --count HEAD 2>/dev/null || echo 1)"
	set_ LSMinimumSystemVersion string 26.0
	set_ LSUIElement bool true
	set_ BuildCommit string "$commit"
	if [ -n "${ICON:-}" ]; then
		cp "$ICON" "$app/Contents/Resources/AppIcon.icns"
		set_ CFBundleIconFile string AppIcon
	fi
	"${sign[@]}" --entitlements "$dir/Helper.entitlements" "$app"
	echo "Built $app"
done
