import Config

# The build this is: a release's own version, which `tooling/release` names it, or for `mix network` the commit checked
# out, as `tooling/version` names it.
config :one_core,
  build: System.get_env("RELEASE_VSN") || String.trim(elem(System.cmd(Path.expand("../../../tooling/version", __DIR__), []), 0))

# A server installed on a machine is told where it is by the program that keeps it running (`one serve run`, in
# `clients/cli/src/serving.rs`): its home, its network's address, and who it's for. The rest is the release's own: the
# UI library's build, in `ui/`, and the Bun it builds apps with, which it fetches the first time (`OneCore.Entrypoint`).
# `mix network` says all of this itself.
if home = System.get_env("ONE_SERVER_HOME") do
  root = System.fetch_env!("RELEASE_ROOT")
  address = System.fetch_env!("ONE_ADDRESS")
  [url, sha256] = String.split(File.read!(Path.join(root, "bun")))

  config :one_core,
    home: home,
    address: address,
    bucket: {OneCore.Bucket.S3, address},
    owner: System.get_env("ONE_SUBJECT", "network"),
    device: System.get_env("ONE_DEVICE"),
    # What its machine is called, as its owner named it, which the server is listed by.
    name: System.get_env("ONE_NAME"),
    # What it is, as that program says (`OneCore.Release`): whether it's a server, where an update is, and why the
    # last didn't take.
    status: JSON.decode!(System.get_env("ONE_SERVER_STATUS", "{}")),
    # A network being built has its person's commands say who they act as, which only it lets them (`OneCore.User`),
    # and it was one of those that installed this server.
    development: System.get_env("ONE_AS") != nil,
    installed: true,
    bun: {url, sha256},
    ui: Path.join(root, "ui")
end
