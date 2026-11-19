defmodule Telemetry do
  @moduledoc """
  Telemetry: what the network's servers, clients and apps did, how long it took and how it ended, kept in the
  network and read back by agents and in its app.

  Each server's OpenTelemetry SDK exports what it gathered, its spans, metrics and logs, as OTLP once a minute
  (`OneCore.Otlp`), and so does each client's of itself (the hook `measured`): Telemetry keeps each export as it is,
  one blob of its own artifact, so a call writes nothing to the bucket, and an idle server writes nothing at all.
  What an app that runs in a VM of its own records goes through the API here (`record`, `Telemetry.Record`). It isn't
  kept in a database, whose log is kept whole forever and whose snapshot is the whole file: what's recorded is kept
  for a while and then let go, which blobs by the hour are (`Telemetry.Store`). It may be lost, and holds no one's
  data.

  What needs a person is kept as alerts (`Telemetry.Alerts`), read here and through `alerts`, whose state is the one
  thing it keeps in its database.
  """
  @behaviour OneCore.App
  require OpenTelemetry.Tracer, as: Tracer
  alias Telemetry.{Alerts, Buffer, Forward, Otlp, Read, Record, Store}
  alias OneCore.Call

  @impl true
  def icon, do: "client/icon.svg"

  # Its data is the network's: every call's spans and every app's errors, for whoever looks after it.
  @impl true
  def own, do: :installed

  @impl true
  def requires,
    do: %{
      "core" => ">= 0.30.0",
      "database" => "to keep its schedule and its alerts"
    }

  @impl true
  def hooks, do: ["exported", "measured"]

  @impl true
  def jobs, do: [%{tool: "prune", every: 3600}, %{tool: "evaluate", every: 60}]

  @impl true
  def entrypoints, do: [%{page: "client/index.html", runtime_version: 4}]

  @impl true
  def tools do
    hours = %{
      type: "integer",
      minimum: 1,
      maximum: Store.days() * 24,
      description: "How many hours back to look, up to #{Store.days() * 24}. 24 if left out"
    }

    ms = &%{type: "number", description: "#{&1}, in milliseconds since 1970"}
    number = &%{type: "number", description: &1}
    maybe = &%{type: ["number", "null"], description: &1}

    quantiles = %{
      count: %{type: "integer"},
      sum: number.("In milliseconds, or the metric's unit"),
      p50: maybe.("The 50th percentile"),
      p95: maybe.("The 95th percentile"),
      p99: maybe.("The 99th percentile")
    }

    failure = %{
      type: "object",
      properties: %{
        where: %{type: ["string", "null"], description: "The span or the log it's from"},
        message: %{type: ["string", "null"]},
        count: %{type: "integer", description: "How often"},
        at: ms.("When it was last"),
        took: maybe.("How long its span took, in milliseconds"),
        trace: %{type: ["string", "null"], description: "The newest one's trace, for `trace`"},
        server: %{type: ["string", "null"]},
        from: %{type: ["string", "null"], description: "The client it's from, or where no client asked, the server or the edge it happened on"},
        whose: %{type: ["string", "null"], description: "Whose call it was, where you may see that"},
        span: %{type: "boolean", description: "Whether it's a span, or a log line"}
      },
      required: ~w(where message count at took trace server from whose span)
    }

    slow = %{
      type: "object",
      description: "A tool, by its app and its name, or an app's view as it opened",
      properties:
        Map.merge(quantiles, %{
          app: %{type: "string"},
          tool: %{type: "string"},
          view: %{type: "string"},
          called: %{type: "string", description: "What a person calls its app"},
          failed: %{type: "integer"},
          should_be_task: %{
            type: "boolean",
            description: "A tool whose 95th percentile is over 10 seconds and whose app doesn't make it an MCP task"
          },
          over_time: %{type: "array", items: %{type: ["number", "null"]}, description: "Its 95th percentile per step"}
        }),
      required: ["called", "failed", "over_time", "count", "p50", "p95", "p99"]
    }

    spent = %{
      type: "object",
      properties: %{
        day: %{type: "string"},
        model: %{type: ["string", "null"]},
        input: %{type: "integer", description: "Tokens in"},
        output: %{type: "integer", description: "Tokens out"},
        cost: maybe.("In US dollars, where the endpoint says")
      },
      required: ["day", "model", "input", "output", "cost"]
    }

    [
      %{
        name: "overview",
        read_only: true,
        description:
          "What's failing, what's slow and what AI cost, over the last hours: what Telemetry' screen shows. Each failure has a trace to read with `trace`",
        inputs: %{type: "object", properties: %{hours: hours}},
        outputs: %{
          type: "object",
          properties: %{
            from: ms.("The range's start"),
            to: ms.("The range's end"),
            failing: %{type: "array", items: failure, description: "The 20 most recent kinds of failure"},
            slowest: %{type: "array", items: slow, description: "The 12 slowest"},
            spending: %{type: "array", items: spent},
            calls: %{
              type: "array",
              description: "Calls and failed calls in about 48 steps",
              items: %{
                type: "object",
                properties: %{at: ms.("The step's start"), calls: %{type: "integer"}, failed: %{type: "integer"}},
                required: ["at", "calls", "failed"]
              }
            },
            names: %{type: "object", additionalProperties: %{type: "string"}, description: "What a person calls each app"},
            retention: %{
              type: "object",
              properties: %{
                since: %{type: ["integer", "null"], description: "The oldest hour kept, in milliseconds since 1970"},
                bytes: %{type: "integer"},
                days: %{type: "integer", description: "How many days it keeps"},
                most_bytes: %{type: "integer", description: "The most it keeps"}
              },
              required: ["since", "bytes", "days", "most_bytes"]
            }
          },
          required: ["from", "to", "failing", "slowest", "spending", "calls", "names", "retention"]
        }
      },
      %{
        name: "errors",
        read_only: true,
        description:
          "The errors of the last hours, each kind once: where it was, its message, how often, when it was last, and the trace of the newest",
        inputs: %{type: "object", properties: %{hours: hours}},
        outputs: %{type: "object", properties: %{errors: %{type: "array", items: failure}}, required: ["errors"]}
      },
      %{
        name: "slowest",
        read_only: true,
        description:
          "Tools by how long they took, slowest first, and how long apps took to open: each one's calls, failures and its 50th, 95th and 99th percentiles in milliseconds",
        inputs: %{type: "object", properties: %{hours: hours}},
        outputs: %{type: "object", properties: %{slowest: %{type: "array", items: slow}}, required: ["slowest"]}
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
        },
        outputs: %{
          type: "object",
          properties: %{
            trace: %{type: "string"},
            spans: %{
              type: "array",
              description: "By when each started",
              items: %{
                type: "object",
                properties: %{
                  span: %{type: "string"},
                  parent: %{type: ["string", "null"]},
                  name: %{type: "string"},
                  depth: %{type: "integer", description: "How far under the trace's first span"},
                  server: %{type: ["string", "null"], description: "The server, or the client it's from"},
                  client: %{type: "boolean", description: "Whether it's a client's span (an app, the CLI, an assistant), not a server's"},
                  start: number.("Milliseconds after the trace began"),
                  took: number.("In milliseconds"),
                  outcome: %{type: "string", enum: ~w(ok error)},
                  message: %{type: ["string", "null"]},
                  attributes: %{type: "object"},
                  events: %{
                    type: "array",
                    items: %{
                      type: "object",
                      properties: %{
                        name: %{type: ["string", "null"]},
                        at: number.("Milliseconds after the trace began"),
                        attributes: %{type: "object"},
                        body: %{description: "What was logged"}
                      },
                      required: ["name", "at"]
                    }
                  }
                },
                required: ~w(span parent name depth server start took outcome message attributes events)
              }
            }
          },
          required: ["trace", "spans"]
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
              description: "Only the data with these attributes, as {\"one.app\": \"backups\"}",
              additionalProperties: %{type: ["string", "number", "boolean"]}
            },
            hours: hours,
            step: %{type: "integer", minimum: 1, description: "Minutes per step. About 48 steps if left out"}
          }
        },
        outputs: %{
          type: "object",
          description: "With a name, that metric's steps; without one, the metrics there are",
          properties: %{
            metrics: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string"},
                  attributes: %{type: "array", items: %{type: "string"}, description: "The names of its attributes"},
                  count: %{type: "integer", description: "How many it counted"}
                },
                required: ["name", "attributes", "count"]
              }
            },
            name: %{type: "string"},
            steps: %{
              type: "array",
              items: %{
                type: "object",
                properties: Map.merge(quantiles, %{at: ms.("The step's start")}),
                required: ["at", "count"]
              }
            }
          }
        }
      },
      %{
        name: "spending",
        read_only: true,
        description:
          "What AI models used, as apps record it, per model per day: tokens in and out, and what they cost where the endpoint says",
        inputs: %{
          type: "object",
          properties: %{
            days: %{
              type: "integer",
              minimum: 1,
              maximum: Store.days(),
              description: "How many days back. 14 if left out"
            }
          }
        },
        outputs: %{type: "object", properties: %{spending: %{type: "array", items: spent}}, required: ["spending"]}
      },
      %{
        name: "record",
        description:
          "Records what an app's tool call measured, under that call: each a number (value), something that took a while and ended now (took, in milliseconds, with an error if it failed), or else a log line, an error if it has one. Names, sizes and outcomes only: never what anyone wrote, was given or answered, and never a secret",
        inputs: %{
          type: "object",
          properties: %{
            records: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string", description: "What it is, in OpenTelemetry's dotted lowercase"},
                  attributes: %{type: "object", additionalProperties: %{type: ["string", "number", "boolean"]}},
                  value: %{type: "number"},
                  took: %{type: "number", minimum: 0},
                  error: %{type: "string"}
                },
                required: ["name"]
              }
            }
          },
          required: ["records"]
        },
        outputs: %{type: "object", properties: %{recorded: %{type: "integer", description: "How many"}}, required: ["recorded"]}
      },
      %{
        name: "alerts",
        read_only: true,
        description:
          "The alerts for what needs a person, open and resolved in the last day, newest first, each kind of alert with what fires it, and where a copy of what's kept is sent",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            alerts: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  id: %{type: "integer"},
                  kind: %{type: "string", enum: Map.keys(Alerts.kinds())},
                  key: %{type: "string", description: "What of its kind it's about: a server's key, a build, an app"},
                  state: %{
                    type: "string",
                    enum: ~w(firing resolved),
                    description: "firing while its symptom is seen, resolved once it has cleared"
                  },
                  title: %{type: "string"},
                  text: %{type: ["string", "null"]},
                  began: ms.("When it fired"),
                  last: ms.("When its symptom was last seen"),
                  ended: %{type: ["integer", "null"], description: "When it cleared, in milliseconds since 1970"}
                },
                required: ~w(id kind key state title text began last ended)
              }
            },
            kinds: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  kind: %{type: "string"},
                  about: %{type: "string", description: "What fires it"}
                },
                required: ["kind", "about"]
              }
            },
            forwarding: %{type: ["string", "null"], description: "Where a copy of what's kept is sent, if anywhere (`forward`)"}
          },
          required: ["alerts", "kinds", "forwarding"]
        }
      },
      %{
        name: "evaluate",
        description:
          "Reads what was recorded since it last did, moves each alert on and sends a copy where `forward` says. The clock calls it every minute",
        inputs: %{type: "object", properties: %{}},
        outputs: %{type: "object", properties: %{}}
      },
      %{
        name: "forward",
        description:
          "Sends a copy of everything kept from now on to an OpenTelemetry endpoint, as OTLP over HTTP (Grafana Cloud, Honeycomb, a collector), or stops with no endpoint. The headers it wants, its key among them, are filled in from a Vault item of yours that's switched on for Telemetry, as Vault's fetch fills them. Only the network's owners may",
        inputs: %{
          type: "object",
          properties: %{
            endpoint: %{
              type: ["string", "null"],
              description: "Its OTLP/HTTP address, to which /v1/traces, /v1/metrics and /v1/logs are added. null stops sending"
            },
            item: %{type: "string", description: "The Vault item its headers are filled in from"},
            headers: %{
              type: "object",
              additionalProperties: %{type: "string"},
              description: "The headers it wants, with {{<field>}} where an item's field goes: {\"x-honeycomb-team\": \"{{key}}\"}"
            }
          },
          required: ["endpoint"]
        },
        outputs: %{
          type: "object",
          properties: %{endpoint: %{type: ["string", "null"]}, item: %{type: ["string", "null"]}},
          required: ["endpoint", "item"]
        }
      },
      %{
        name: "prune",
        description:
          "Puts each hour that's over into one blob, and rolls it up, and lets go of what's older than #{Store.days()} days or past #{div(Store.most_bytes(), 1024 * 1024)} MB. The clock calls it every hour",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            put_together: %{type: "integer", description: "How many hours were put into one blob each"},
            rolled_up: %{type: "integer", description: "How many hours were rolled up, for looks over hours to read"},
            let_go: %{type: "integer", description: "How many hours were let go"}
          },
          required: ["put_together", "rolled_up", "let_go"]
        }
      }
    ]
  end

  @impl true
  # What this server's SDK exported, which is told on a server acting for the network.
  def hook("exported", %{"signal" => signal, "otlp" => otlp}, call), do: Buffer.add(call, signal, Base.decode64!(otlp))

  # What a client exported of itself, as from the device and person its request proved it is, whatever it says of
  # itself.
  def hook("measured", %{"otlp" => exports, "device" => device, "who" => who}, call) do
    proved = %{"one.device" => device, "one.for" => OneCore.User.owner(who || "") || "link"}

    for %{"signal" => signal, "otlp" => otlp} <- exports, signal in Otlp.signals() do
      Buffer.add(call, signal, Otlp.stamp(signal, Base.decode64!(otlp), proved))
    end

    :ok
  end

  # What a client before OTLP reported, as `record` takes it, through the API here.
  def hook("measured", %{"records" => records, "device" => device, "who" => who}, _call) do
    proved = %{"one.device" => device, "one.for" => OneCore.User.owner(who || "") || "link"}
    Record.record(Enum.filter(records, &is_map(&1["attributes"] || %{})), proved)
  end

  @impl true
  def call("overview", inputs, call) do
    # How much is kept is listed as what's kept is read.
    within = OpenTelemetry.Ctx.get_current()

    retention =
      Task.async(fn ->
        OpenTelemetry.Ctx.attach(within)
        Tracer.with_span("telemetry retention", do: Store.retention(call.own))
      end)

    {batches, from, to} = read(call, inputs["hours"] || 24)
    {shown, names} = {shown(call), names(call)}

    Tracer.with_span "telemetry aggregate" do
      %{
        from: div(from, 1_000_000),
        to: div(to, 1_000_000),
        failing: Read.errors(batches, shown) |> Enum.take(20),
        slowest: Read.slowest(batches, from, to, names) |> Enum.take(12),
        spending: Read.spending(batches),
        calls: Read.calls(batches, from, to),
        names: names
      }
    end
    |> Map.put(:retention, Task.await(retention))
  end

  def call("errors", inputs, call) do
    {batches, _from, _to} = read(call, inputs["hours"] || 24)
    %{errors: Read.errors(batches, shown(call))}
  end

  def call("slowest", inputs, call) do
    {batches, from, to} = read(call, inputs["hours"] || 24)
    %{slowest: Read.slowest(batches, from, to, names(call))}
  end

  def call("trace", %{"id" => id} = inputs, call) do
    if not (id =~ ~r/^[0-9a-f]{32}$/), do: raise("A trace's id is 32 lowercase hexadecimal digits.")
    to = System.os_time(:nanosecond)
    batches = Store.whole(call, to - (inputs["hours"] || 24) * 3_600_000_000_000)

    case Read.trace(batches, id, shown(call)) do
      [] -> raise "No trace #{id} in the last #{inputs["hours"] || 24} hours."
      spans -> %{trace: id, spans: spans}
    end
  end

  def call("metric", inputs, call) do
    {batches, from, to} = read(call, inputs["hours"] || 24)

    case inputs["name"] do
      nil -> %{metrics: Read.metrics(batches)}
      name -> Read.series(batches, name, inputs["attributes"] || %{}, from, to, inputs["step"])
    end
  end

  def call("spending", inputs, call) do
    {batches, _from, _to} = read(call, (inputs["days"] || Store.days()) * 24)
    %{spending: Read.spending(batches)}
  end

  def call("record", %{"records" => records}, _call) do
    Record.record(Enum.map(records, &Map.drop(&1, ["time", "traceId", "spanId"])))
    %{recorded: length(records)}
  end

  def call("prune", _inputs, call), do: Store.prune(call.own)

  def call("forward", %{"endpoint" => endpoint} = inputs, call) do
    if endpoint && !inputs["item"], do: raise("Name the Vault item its headers are filled in from, as item.")
    Forward.set(call, endpoint, inputs["item"], inputs["headers"])
  end
  def call("alerts", _inputs, call), do: Alerts.list(call)
  def call("evaluate", _inputs, call) do
    Alerts.evaluate(call)
    %{}
  end

  # What was kept over the last `hours`, and when that is, in nanoseconds since 1970.
  defp read(call, hours) do
    to = System.os_time(:nanosecond)
    from = to - hours * 3_600_000_000_000
    {Store.read(call, from), from, to}
  end

  # Whose a call was is shown to the network's owners, and to whoever it was: to anyone else, how the network is
  # doing is open, and who did what isn't.
  defp shown(%Call{actor: actor} = call) do
    owner = actor != nil and Tracer.with_span("telemetry owns", do: Call.owns?(%{call | subject: "network"}))
    fn whose -> owner or whose == OneCore.User.owner(actor || "") end
  end

  # What a person calls each app, its title, by its name.
  defp names(call), do: Map.new(Call.apps(call), &{&1.name, &1.title})
end
