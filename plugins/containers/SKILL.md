---
name: containers
description: Read before creating or using a Linux container on the user's machines, as a scratch machine, a dev environment, somewhere to run Chromium or a desktop to look at.
---

Containers are Linux containers on the user's own machines, run by Podman. Use one when you need a machine of
your own to install, build, run or break things in, instead of the user's device itself. `machines` lists the
machines that run them.

- `create` takes an `image`, a `name`, what it's for in a few words (`description`) and a `size` (small, medium,
  large). A shell image such as `docker.io/library/ubuntu:24.04` keeps running with no command. It's started at
  once: then `exec` runs commands in it, `read_file` and `write_file` move files, and `logs` reads what its own
  command printed. `list` shows yours and their state.
- `image: "desktop"` is a Linux desktop: a 1280×800 display (Xvfb, `DISPLAY=:99`) with Openbox and Chromium on
  it. A running one is a screen in Computer Use, called what the container is: look at it and act on it there. Chromium's DevTools Protocol is on port 9222 inside it
  (`exec` with `curl -s 127.0.0.1:9222/json`). The first desktop on a machine takes about 2 minutes to build.
- `commit` makes an image of a prepared container, and `create` from that image's name makes throwaway copies of
  it on the same machine. `remove_image` removes it.
- A container reaches the internet but nothing of the machine's own: no files, no host network, no privileges.
  `ports` at `create` publishes ports on the machine's loopback, and `request` makes an HTTP request to one from
  there. A port can't yet be forwarded to your own device.
- `mounts` puts a folder of the machine itself into a container: it's the machine owner's own files, which the
  container can then change. Use it only when the user asked for that folder.

Containers cost memory and CPU on the user's machine while they run. Stop or remove one when you're done with it.
One left running and unused (no `exec`, file, screenshot or request) for `idle_hours`, 4 unless `settings` says,
is stopped on the hour; `start` brings it back with its files. Yours are yours: nobody else sees them but the
network's owner.

A machine runs containers once its service is installed there: `install_service --plugin containers --service
Containers.Podman --server <key>`. It needs Podman, rootless where it can be.
