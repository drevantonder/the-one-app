---
name: another-computer
description: Giving a throwaway network a second or third computer of your own, for anything that crosses devices: services called from another device, presence, pairing, updates.
---

# Another computer on a throwaway

`one-dev throwaway client linux` (and `--as sam`) is a computer of its own already: its home is the throwaway's
`linux/computer`, its oned a unit of your systemd. Its CLI is that home's `bin/one` with the unit's environment
(`systemctl --user cat oned-throwaway-<hash>.service` shows it). Each is listed by this machine's name with a number
after it (`linux 2`), which `--device` and `add --node` take.

A bare one, joined with a pairing code, in a home and port of its own (`$H` the throwaway's folder):

    C=$H/third/computer; mkdir -p $C/bin; cp target/debug/one target/debug/one-apps $C/bin/; ln -sf one $C/bin/oned
    PORT=<a free port>; KEY=$(cat $H/release-key.pub)
    systemd-run --user --unit=one-throwaway-third --setenv=ONE_HOME=$C --setenv=ONE_THROWAWAY=$C \
      --setenv=ONE_ADDR=127.0.0.1:$PORT --setenv="ONE_RELEASE_KEY=$KEY" $C/bin/oned
    one-dev throwaway pair        # prints: one join <code>
    ONE_HOME=$C ONE_THROWAWAY=$C ONE_ADDR=127.0.0.1:$PORT ONE_RELEASE_KEY="$KEY" $C/bin/one join <code>

It's listed by this machine's name with the next number (its `one status` says it). A service's program goes beside its
`one` (`cp target/debug/one-shell $C/bin/`), then `one services on shell --device "<that name>"`. `systemctl --user stop one-throwaway-third` when you're done: `one-dev
throwaway stop` doesn't know it.

A Windows one is the same on a borrowed copy (`one agent-devices borrow --kind windows --copy`): `one agent-devices
reach` each of the throwaway's ports (its address, auth and bucket: `cloudflare/settings.json`, `bucket-addr`), its
`one.exe` and `one-apps.exe` cross-built in release (`cargo build --release --target x86_64-pc-windows-gnu --bin one
--bin one-apps`: a debug build doesn't link) and copied over with scp, `oned.exe` a hard link to `one.exe`, and oned
started by a scheduled task (what SSH starts dies with its session). `one-dev throwaway publish --windows` gives it an
update to take.
