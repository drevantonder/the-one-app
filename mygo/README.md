# MyGo spike

An experiment (branch `mygo-spike`, never main): whether [MyGo](https://github.com/egoist/mygo) can be the client shell
on Linux, macOS and Windows, with the Rust client (client/ffi) called from Go.

- `client/`: client/ffi from Go. Its API doesn't say how the library is reached: purego on computers
  (`CGO_ENABLED=0`), or a cgo loader for an iPhone (`-tags cgoffi`, which fills the same `lib` table).
- `signin/`: signing in as an OAuth client of the install's sign-in, with a loopback redirect.
- `bridge/`: src/screen/bridge.js's messages answered, given a platform's `Shell`.
- `desktop/`: the MyGo app.

Needs the library built: `cargo build -p one-client-ffi` (it's found in the checkout's target/debug, beside the
program, or at `ONE_CLIENT_LIB`).
