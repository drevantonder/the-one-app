import Config

# The build this is: a release's own version, which `tooling/release` names it, or for `mix network` the commit checked
# out, as `tooling/version` names it.
build = System.get_env("RELEASE_VSN") || String.trim(elem(System.cmd(Path.expand("../../../tooling/version", __DIR__), []), 0))
config :one_core, build: build

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
    # A new network's first server, as `one serve on` starts it the first time: the secret its edge was deployed with,
    # which writes the network's first list of servers, and the user who becomes its first owner (`OneCore.Apps.Lifecycle.found/1`).
    first: System.get_env("ONE_FIRST"),
    first_owner: System.get_env("ONE_FIRST_OWNER"),
    installed: true,
    bun: program.("bun"),
    git: program.("git"),
    ui: Path.join(root, "ui")
end

# OpenTelemetry's SDK: every span, metric and log line the server gathers is exported once a minute, as OTLP, to the
# apps that keep it (`OneCore.Otlp`), so an idle server exports nothing. Its spans are W3C's, carried between servers
# and clients as `traceparent` (`OneCore.Mcp`, `OneCore.Server.request/4`). Metrics are each minute's (delta), so a
# minute kept is that minute's, and durations are in milliseconds, in buckets as fine as a person reads them.
every = 60_000

config :opentelemetry,
  text_map_propagators: [:trace_context],
  resource: %{service: %{name: "one-server", version: build}},
  processors: [otel_batch_processor: %{exporter: {OneCore.Otlp, :traces}, scheduled_delay_ms: every}]

config :opentelemetry_experimental,
  readers: [
    %{
      module: :otel_metric_reader,
      config: %{
        exporter: {OneCore.Otlp, :metrics},
        export_interval_ms: every,
        default_temporality_mapping: %{
          counter: :temporality_delta,
          observable_counter: :temporality_delta,
          histogram: :temporality_delta,
          observable_gauge: :temporality_delta,
          updown_counter: :temporality_cumulative,
          observable_updowncounter: :temporality_cumulative
        }
      }
    }
  ]
