defmodule Insights do
  @moduledoc """
  Insights: what the network's servers, clients and plugins did, how long it took and how it ended, kept in the
  network and read back by agents and on a screen of Settings.

  Servers record it themselves in OpenTelemetry's shapes (`OneCore.Telemetry`), and each hands what it recorded to
  this plugin about once a minute (the hook `recorded`), which keeps each batch as one blob of its own artifact:
  so a call writes nothing to the bucket, and an idle server writes nothing at all. It isn't kept in a database,
  whose log is kept whole forever and whose snapshot is the whole file: what's recorded is kept for a while and then
  let go, which blobs by the hour are (`Insights.Kept`). It may be lost, and holds no one's data.
  """
  @behaviour OneCore.Plugin
  alias Insights.{Kept, Read}
  alias OneCore.Call

  @impl true
  def uses, do: ["database"]

  @impl true
  def hooks, do: ["recorded"]

  @impl true
  def jobs, do: [%{tool: "tidy", every: 3600}]

  @impl true
  def apps, do: [%{name: "Insights", page: "app/index.html", icon: "icon.svg", settings: true}]

  @impl true
  def tools do
    hours = %{
      type: "integer",
      minimum: 1,
      maximum: Kept.days() * 24,
      description: "How many hours back to look, up to #{Kept.days() * 24}. 24 if left out"
    }

    [
      %{
        name: "overview",
        read_only: true,
        description:
          "What's failing, what's slow and what AI cost, over the last hours: what Insights' screen shows. Each failure has a trace to read with `trace`",
        inputs: %{type: "object", properties: %{hours: hours}}
      },
      %{
        name: "errors",
        read_only: true,
        description:
          "The errors of the last hours, each kind once: where it was, its message, how often, when it was last, and the trace of the newest",
        inputs: %{type: "object", properties: %{hours: hours}}
      },
      %{
        name: "slowest",
        read_only: true,
        description:
          "Tools by how long they took, slowest first, and how long apps took to open: each one's calls, failures and its 50th, 95th and 99th percentiles in milliseconds",
        inputs: %{type: "object", properties: %{hours: hours}}
      },
      %{
        name: "trace",
        read_only: true,
        description:
          "One trace whole, across every server it went through: each span with its depth, server, start and duration in milliseconds, outcome and events",
        inputs: %{
          type: "object",
          properties: %{
            id: %{type: "string", description: "The trace's id: 32 hexadecimal digits"},
            hours: hours
          },
          required: ["id"]
        }
      },
      %{
        name: "metric",
        read_only: true,
        description:
          "A metric over time as its count, sum and 50th, 95th and 99th percentiles per step, such as one.tool.duration in milliseconds. Without a name, lists the metrics there are, with their attributes",
        inputs: %{
          type: "object",
          properties: %{
            name: %{type: "string", description: "The metric's name, as one.tool.duration"},
            attributes: %{
              type: "object",
              description: "Only the data with these attributes, as {\"one.plugin\": \"ai-gateway\"}",
              additionalProperties: %{type: ["string", "number", "boolean"]}
            },
            hours: hours,
            step: %{type: "integer", minimum: 1, description: "Minutes per step. About 48 steps if left out"}
          }
        }
      },
      %{
        name: "spending",
        read_only: true,
        description:
          "What AI Gateway's calls used, per model per day: tokens in and out, and what they cost where the endpoint says",
        inputs: %{
          type: "object",
          properties: %{
            days: %{
              type: "integer",
              minimum: 1,
              maximum: Kept.days(),
              description: "How many days back. 14 if left out"
            }
          }
        }
      },
      %{
        name: "tidy",
        description:
          "Puts each hour that's over into one blob, and lets go of what's older than #{Kept.days()} days or past #{div(Kept.most_bytes(), 1024 * 1024)} MB. The clock calls it every hour",
        inputs: %{type: "object", properties: %{}}
      }
    ]
  end

  @impl true
  def hook("recorded", batch, call), do: Kept.keep(call.own, batch)

  @impl true
  def call("overview", inputs, call) do
    {batches, from, to} = read(call, inputs["hours"] || 24)

    %{
      from: div(from, 1_000_000),
      to: div(to, 1_000_000),
      failing: Read.errors(batches, shown(call)) |> Enum.take(20),
      slowest: Read.slowest(batches, from, to, names(call)) |> Enum.take(12),
      spending: Read.spending(batches),
      calls: Read.calls(batches, from, to),
      names: names(call),
      kept: Kept.kept(call.own)
    }
  end

  def call("errors", inputs, call) do
    {batches, _from, _to} = read(call, inputs["hours"] || 24)
    Read.errors(batches, shown(call))
  end

  def call("slowest", inputs, call) do
    {batches, from, to} = read(call, inputs["hours"] || 24)
    Read.slowest(batches, from, to, names(call))
  end

  def call("trace", %{"id" => id} = inputs, call) do
    if not (id =~ ~r/^[0-9a-f]{32}$/), do: raise("A trace's id is 32 lowercase hexadecimal digits.")
    {batches, _from, _to} = read(call, inputs["hours"] || 24)

    case Read.trace(batches, id, shown(call)) do
      [] -> raise "No trace #{id} in the last #{inputs["hours"] || 24} hours."
      spans -> %{trace: id, spans: spans}
    end
  end

  def call("metric", inputs, call) do
    {batches, from, to} = read(call, inputs["hours"] || 24)

    case inputs["name"] do
      nil -> Read.metrics(batches)
      name -> Read.series(batches, name, inputs["attributes"] || %{}, from, to, inputs["step"])
    end
  end

  def call("spending", inputs, call) do
    {batches, _from, _to} = read(call, (inputs["days"] || Kept.days()) * 24)
    Read.spending(batches)
  end

  def call("tidy", _inputs, call), do: Kept.tidy(call.own)

  # What was kept over the last `hours`, and when that is, in nanoseconds since 1970.
  defp read(call, hours) do
    to = System.os_time(:nanosecond)
    from = to - hours * 3_600_000_000_000
    {Kept.read(call.own, from), from, to}
  end

  # Whose a call was is shown to the network's owners, and to whoever it was: to anyone else, how the network is
  # doing is open, and who did what isn't.
  defp shown(%Call{by: by} = call) do
    owner = by != nil and Call.owns?(%{call | for: "network"})
    fn whose -> owner or whose == OneCore.User.owner(by || "") end
  end

  # What a person calls each plugin: its first app's name, where it has one.
  defp names(call) do
    for %{name: name, apps: [%{name: app} | _]} <- Call.plugins(call), into: %{}, do: {name, app}
  end
end
