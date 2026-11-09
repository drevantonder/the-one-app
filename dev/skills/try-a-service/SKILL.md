---
name: try-a-service
description: Putting your checkout's build of an app's service on one device, before it's sent out to all.
---

# Try a service

It runs until a newer one is published. For the app `<n>` and `<device>`:

    b=$(git rev-parse --short=12 HEAD)

Go, in `apps/<n>/service`, for that device's platform:

    CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -trimpath -ldflags "-X main.built=$b" -o /tmp/one-<n> .

Rust, on a machine like that device, then use `target/release/one-<n>`:

    ONE_BUILD=$b cargo build --release --manifest-path apps/<n>/service/Cargo.toml

Then:

    scp /tmp/one-<n> <device>:.local/bin/one-<n>.new
    ssh <device> 'chmod 755 .local/bin/one-<n>.new && mv .local/bin/one-<n>.new .local/bin/one-<n>'
    one services on <n> --device <device>

A custom build's program is called `<cli>-<n>`.
