---
name: keep-mac-current
description: Keeping the Mac and its iPhone on the newest build of a network. Run on the Mac itself, by its scheduled agent, or when a Mac is behind.
---

# Keep the Mac and its iPhone current

Run this on the Mac, in the logged-in session: only there can a build be signed and the phone reached. Each step is one command. Decide from what changed, so a build happens only when it would show.

For each network, with its address, `<cli>` and app name (`~/Applications/<name>.app`):

1. **What's newest.** `curl -fsSL <address>/updates/newest` (it redirects). Check it before believing it: the signature block goes to `ssh-keygen -Y verify -n release-latest -I release`, against the config's `release_key`. Its `source` line is six fields; the second is the commit, the third when it was made, the sixth the config commit (`-` for none).
2. **What runs.** `~/Applications/<name>.app/Contents/MacOS/<cli> version --json`. Newer than the record: stay on it.
3. **Whether it shows.** `git diff --name-only <running>..<newest>`. Only `*.md`, `docs/`, `tools/`, `tests/`, `dev/`, or what reaches devices over the air (`cloudflare/`, `src/screen/`, `src/onboarding/`, `apps/`): nothing to build. Otherwise build, using judgement about when: hold off while another build is running or the Mac is overloaded, and let changes that land close together go out in one build.
4. **The checkout.** `~/Library/Caches/<cli>/src` keeps its build cache: fetch, and check out the commit. A custom build's config is its clone in `~/Library/Caches/<cli>/config`, at the config commit.
5. **The Mac app.** From the checkout, at low priority so the Mac stays usable: `CONFIG=<config.toml> CARGO_BUILD_JOBS=<half the cores> nice -n 19 client/native/disk.sh retry client/native/mac/build.sh`. It installs and restarts the app.
6. **Publish it**, where this Mac has the release key: `ditto -c -k --keepParent ~/Applications/<name>.app <zip>`, then `<cli> publish-app --zip <zip> --config <config commit or ->`, with `~/.profile` loaded for the password manager's sign-in. That makes it the app every Mac's Update takes (`<cli> update`, from Settings › Devices or a Mac's own terminal).
7. **The iPhone app**, only if what changed reaches it (anything but `client/native/{mac,windows,android}/`, `service/`, `apps/` and step 3's list): `CONFIG=<config.toml> client/native/iphone/build.sh device <dir>`, then `xcrun devicectl device install app --device <paired iPhone> <dir>/*.app`. Not reachable: try next run.

A step that fails: read its output and fix the cause. `one logs <mac>` says what the node saw.

A stall on the Mac leaves every thread's stack in `logs/stall.sample` under its home. Each release keeps its programs' symbols on the Mac, by UUID: `<checkout>/target/release/<cli>-dev symbols <that file>` prints it with the `???` frames named.
