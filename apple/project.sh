# Sourced by the build scripts: exports the brand from brand.toml (or $BRAND), turns its artwork into the apps' icons
# and images, and generates Shells.xcodeproj.
root=$(cd .. && pwd)
brand=$(cd "$(dirname "${BRAND:-$root/brand.toml}")" && pwd)/$(basename "${BRAND:-brand.toml}")
field() { sed -n "s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$brand"; }
art() { echo "$(dirname "$brand")/$(field "$1")"; }
export BRAND_NAME=$(field name) BRAND_CLI=$(field cli) BRAND_PORT=$(field port) BRAND_ACCENT=$(field accent)
# The iPhone app signs with the team of this Mac's Apple Development certificate, unless APPLE_TEAM names one.
export APPLE_TEAM=${APPLE_TEAM:-$(security find-certificate -c "Apple Development" -p 2>/dev/null | openssl x509 -noout -subject 2>/dev/null | sed -n 's/.*OU *= *\([A-Z0-9]*\).*/\1/p')}

icon=$(art icon) menubar=$(art menubar) logo=$(art logo)
mkdir -p build
contents() { printf '{"images":[%s],"info":{"author":"xcode","version":1}%s}\n' "$1" "${2:-}"; }

# The Mac's icon is the brand's icon as it is.
rm -rf build/AppIcon.iconset && mkdir build/AppIcon.iconset
for s in 16 32 128 256 512; do
	sips -z $s $s "$icon" --out build/AppIcon.iconset/icon_${s}x${s}.png >/dev/null
	sips -z $((s * 2)) $((s * 2)) "$icon" --out build/AppIcon.iconset/icon_${s}x${s}@2x.png >/dev/null
done
iconutil -c icns build/AppIcon.iconset -o build/AppIcon.icns

# The iPhone's is its tile, full bleed: brand icons are drawn on Apple's Mac grid, an 824-pixel tile in 1024.
# iPhone icons can't be transparent, so what's left of the tile's corners takes the colour of its edge.
icons=build/Phone.xcassets/AppIcon.appiconset
mkdir -p "$icons"
sips -c 824 824 "$icon" --out build/tile.png >/dev/null
swift - build/tile.png "$icons/icon.png" <<'SWIFT'
import AppKit
let a = CommandLine.arguments, tile = NSBitmapImageRep(data: try! Data(contentsOf: URL(filePath: a[1])))!
let cg = CGContext(data: nil, width: 1024, height: 1024, bitsPerComponent: 8, bytesPerRow: 0,
                   space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
cg.setFillColor((tile.colorAt(x: tile.pixelsWide / 2, y: 4) ?? .black).cgColor)
cg.fill(CGRect(x: 0, y: 0, width: 1024, height: 1024))
cg.draw(tile.cgImage!, in: CGRect(x: 0, y: 0, width: 1024, height: 1024))
try! NSBitmapImageRep(cgImage: cg.makeImage()!).representation(using: .png, properties: [:])!.write(to: URL(filePath: a[2]))
SWIFT
contents '{"filename":"icon.png","idiom":"universal","platform":"ios","size":"1024x1024"}' > "$icons/Contents.json"
contents "" > build/Phone.xcassets/Contents.json

# The logo the apps show, and the Mac's menu bar mark (a template, with its @2x).
assets=build/Brand.xcassets
rm -rf "$assets" && mkdir -p "$assets/Logo.imageset" "$assets/MenuBarIcon.imageset"
contents "" > "$assets/Contents.json"
cp "$logo" "$assets/Logo.imageset/logo.png"
contents '{"filename":"logo.png","idiom":"universal"}' > "$assets/Logo.imageset/Contents.json"
cp "$menubar" "$assets/MenuBarIcon.imageset/mark.png"
cp "${menubar%.png}@2x.png" "$assets/MenuBarIcon.imageset/mark@2x.png"
contents '{"filename":"mark.png","idiom":"universal","scale":"1x"},{"filename":"mark@2x.png","idiom":"universal","scale":"2x"}' \
	',"properties":{"template-rendering-intent":"template"}' > "$assets/MenuBarIcon.imageset/Contents.json"

xcodegen generate --quiet
