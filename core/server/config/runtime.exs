import Config

# The build this is: a release's own version, which `tooling/release` names it, or for `mix network` the commit checked
# out, as `tooling/version` names it.
config :one_core,
  build: System.get_env("RELEASE_VSN") || String.trim(elem(System.cmd(Path.expand("../../../tooling/version", __DIR__), []), 0))

# A server installed on a machine is told where it is by the program that keeps it running (`one serve run`, in
# `clients/cli/src/serving.rs`): its home, its network's address, and who it's for. The rest is the release's own: the
# UI library's build, in `ui/`, and the programs it fetches the first time (`OneCore.Programs`): Bun, and on Windows Git.
# `mix network` says all of this itself. Windows gives its paths with backslashes, which `Path.wildcard/2` reads as
# escapes, finding nothing under them: expanded, they're written with slashes, as Elixir's own paths are.
if home = System.get_env("ONE_SERVER_HOME") do
  home = Path.expand(home)
  root = Path.expand(System.fetch_env!("RELEASE_ROOT"))
  address = System.fetch_env!("ONE_ADDRESS")
  # A program the release names, `{url, sha256}`, or `nil` for one it doesn't, as Git but on Windows.
  program = fn name ->
    case File.read(Path.join(root, name)) do
      {:ok, text} -> text |> String.split() |> List.to_tuple()
      {:error, :enoent} -> nil
    end
  end

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
    bun: program.("bun"),
    git: program.("git"),
    ui: Path.join(root, "ui")
end
