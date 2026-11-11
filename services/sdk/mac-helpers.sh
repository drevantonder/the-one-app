#!/bin/bash
# Builds each Mac service in Swift (the apps/<name>/service/ with an Info.plist, on services/sdk/swift) into a helper app of its own,
# in target/mac-helpers: "<its CFBundleName> — <the app's name>.app", id app.<cli>.mac.<name>, with its own Info.plist
# and entitlements, signed with the identity SIGN names (ad hoc without), so macOS asks for and records its permission
# under its own name. native-apps/mac/build.sh zips each for the record, and oned unpacks and runs it (oned::helper). CONFIG names another custom build's config.toml; ICON, an .icns for them.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
config=${CONFIG:-$root/config.toml}
field() { sed -n "/^\[/q; s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$config"; }
name=$(field name) cli=$(field cli)
commit=$(git -C "$root" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
out=$root/target/mac-helpers
rm -rf "$out" && mkdir -p "$out"
sign=(codesign --force --options runtime --sign "${SIGN:--}")
[ -z "${SIGN:-}" ] || sign+=(--timestamp)
for plist in "$root"/apps/*/service/Info.plist; do
	[ -e "$plist" ] || continue
	dir=$(dirname "$plist")
	service=$(basename "$(dirname "$dir")")
	# Each one's package and entry point differ only by its name and its Service's struct, so they are written here (git ignores them).
	entry=$(sed -n 's/^struct \([A-Za-z]*\): Service.*/\1/p' "$dir"/*.swift)
	cat > "$dir/Package.swift" <<PACKAGE
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "$service",
    platforms: [.macOS("26.0")],
    dependencies: [.package(path: "../../../services/sdk/swift")],
    targets: [.executableTarget(name: "$service", dependencies: [.product(name: "Helper", package: "swift")], path: ".", exclude: ["Package.swift", "Info.plist", "Helper.entitlements"])]
)
PACKAGE
	printf 'import Helper\n\nserve(%s())\n' "$entry" > "$dir/main.swift"
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
