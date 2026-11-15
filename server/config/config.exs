import Config

# What a server records is spans of OpenTelemetry's, which `OneCore.Telemetry` is the one processor of: it hands
# them to Insights in the network, and nothing is exported anywhere else.
config :opentelemetry,
  processors: [{OneCore.Telemetry, %{}}],
  traces_exporter: :none,
  text_map_propagators: [:trace_context]
