import Config

# A server installed on a machine is told where it is by the program that keeps it running (`one serve run`, in
# `clients/cli/src/serving.rs`): its home, its network's address, and who it's for. The rest is the release's own: the
# plugins it ships with, beside it in `shipped/`, and the Bun it builds apps with, which it fetches the first time
# (`OneCore.App`). `mix network` says all of this itself.
if home = System.get_env("ONE_SERVER_HOME") do
  root = System.fetch_env!("RELEASE_ROOT")
  address = System.fetch_env!("ONE_ADDRESS")
  [url, sha256] = String.split(File.read!(Path.join(root, "bun")))

  config :one_core,
    home: home,
    address: address,
    bucket: {OneCore.Bucket.S3, address},
    for: System.get_env("ONE_FOR", "network"),
    device: System.get_env("ONE_DEVICE"),
    # What its machine is called, as its owner named it, which the server is listed by.
    name: System.get_env("ONE_NAME"),
    # A network being built has its person's commands say who they act as, which only it lets them (`OneCore.User`),
    # and it was one of those that installed this server.
    development: System.get_env("ONE_AS") != nil,
    installed: true,
    bun: {url, sha256},
    shipped: Map.new(Path.wildcard(Path.join(root, "shipped/*")), &{Path.basename(&1), &1})
end
