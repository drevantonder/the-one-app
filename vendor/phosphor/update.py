#!/usr/bin/env python3
"""Writes icons.json from a Phosphor release: python3 update.py 2.1.1

Each icon, regular as `name` and filled as `name-fill`, is its path data on a 256-point grid, drawn filled.
"""
import io, json, re, sys, tarfile, urllib.request

version = sys.argv[1]
url = f"https://registry.npmjs.org/@phosphor-icons/core/-/core-{version}.tgz"
icons = {}
with tarfile.open(fileobj=io.BytesIO(urllib.request.urlopen(url).read())) as tar:
    for member in tar.getmembers():
        found = re.fullmatch(r"package/assets/(regular|fill)/([a-z0-9-]+)\.svg", member.name)
        if found:
            svg = tar.extractfile(member).read().decode()
            assert 'viewBox="0 0 256 256"' in svg and not re.search(r"<(?!svg|path|/)", svg), member.name
            icons[found[2]] = "".join(re.findall(r'<path d="([^"]+)"', svg))
        elif member.name == "package/LICENSE":
            open("LICENSE", "wb").write(tar.extractfile(member).read())
with open("icons.json", "w") as f:
    json.dump(dict(sorted(icons.items())), f, separators=(",", ":"))
    f.write("\n")
print(len(icons), "icons")
