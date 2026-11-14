defmodule Load do
  @moduledoc "How busy a network's machines are, each measured by a small program on it."
  @behaviour OneCore.Plugin
  alias Load.Meter

  @impl true
  def guide do
    """
    Load says how busy each machine is: how many processors it has, and its load over the last 1, 5 and 15
    minutes. A load as high as the number of processors is a machine with no time to spare, so use `now` before
    giving a machine heavy work, to pick the one with most to spare.

    Each machine is measured by Load's service `Load.Meter`, which brings a small program written in Rust and
    builds it on the machine, so the machine needs `rustc`. `services` says which servers run it, and
    `install_service` runs it on one.
    """
  end

  @impl true
  def tools do
    [
      %{
        name: "now",
        read_only: true,
        description: "Says how busy each machine is right now, and since when it has been measured",
        inputs: %{
          type: "object",
          properties: %{server: %{type: "string", description: "The key of one server to ask. All, if left out"}}
        }
      }
    ]
  end

  @impl true
  def call("now", inputs, call) do
    servers = List.wrap(inputs["server"] || OneCore.Service.servers(call, Meter))
    if servers == [], do: raise("No machine is measured yet. `install_service` runs Load.Meter on one.")
    for server <- servers, do: Map.put(OneCore.Service.call(call, Meter, :now, [], server), "server", server)
  end
end
