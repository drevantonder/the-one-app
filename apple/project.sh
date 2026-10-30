# Sourced by the build scripts: exports the brand from brand.toml (or $BRAND), draws its icons and generates Shells.xcodeproj.
root=$(cd .. && pwd)
brand=$(cd "$(dirname "${BRAND:-$root/brand.toml}")" && pwd)/$(basename "${BRAND:-brand.toml}")
field() { sed -n "s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$brand"; }
export BRAND_NAME=$(field name) BRAND_CLI=$(field cli) BRAND_PORT=$(field port) BRAND_ACCENT=$(field accent)
# The iPhone app signs with the team of this Mac's Apple Development certificate, unless APPLE_TEAM names one.
export APPLE_TEAM=${APPLE_TEAM:-$(security find-certificate -c "Apple Development" -p 2>/dev/null | openssl x509 -noout -subject 2>/dev/null | sed -n 's/.*OU *= *\([A-Z0-9]*\).*/\1/p')}

icons=build/Phone.xcassets/AppIcon.appiconset
mkdir -p "$icons"
swift icon.swift "$BRAND_ACCENT" build
mv build/AppIcon.png "$icons/icon.png"
echo '{"images":[{"filename":"icon.png","idiom":"universal","platform":"ios","size":"1024x1024"}],"info":{"author":"xcode","version":1}}' > "$icons/Contents.json"
echo '{"info":{"author":"xcode","version":1}}' > build/Phone.xcassets/Contents.json
xcodegen generate --quiet
