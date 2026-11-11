<p align="center">
  <img src="icons/ring.png" width="200" alt="">
</p>

<h1 align="center">The One App</h1>

<p align="center"><i>
One app to rule them all, one app to find them,<br>
one app to bring them all, and in the network bind them.
</i></p>

```sh
curl -fsSL theoneapp.drevan.me | sh
```

That starts your first network. Its other computers install from its own address: Settings › Devices › Add Device gives the line.

> [!WARNING]
> **Early days.** This changes every day and things break. I wouldn't use it or rely on it yet; wait until it settles.

The vision for this project is simple: make using AI to its full potential easy. [VISION.md](VISION.md) says what that means.

To learn more, point your agent at this repo and ask it to explain.

## Building it

A Linux machine that builds what `one-dev update` sends (the computers' programs, oned on Cloudflare, the web client and its WebAssembly, the apps' screens and services) needs a C compiler, a clang that can build for WebAssembly, curl and git from its package manager, then rustup and mise to install the rest, as `rust-toolchain.toml` and `mise.toml` say:

```sh
sudo apt install ca-certificates curl git gcc clang    # Ubuntu 26.04
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
curl -fsSL https://mise.run | sh
git clone https://github.com/drevantonder/the-one-app && cd the-one-app
mise install                  # Bun, Node, Go, sccache, wasm-bindgen and binaryen
eval "$(mise activate bash)"  # in your shell's profile, too: mise's shims don't find them from the folders cargo runs sccache in
cargo build                   # the first cargo here has rustup install the Rust and targets rust-toolchain.toml names
```
