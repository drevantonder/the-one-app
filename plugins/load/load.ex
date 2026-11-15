defmodule Load do
  @moduledoc "How busy a network's machines are, each measured by a small program on it."
  @behaviour OneCore.Plugin
  alias Load.Meter

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
    now(call, servers)
  end

  @doc """
  How busy each of `servers` is, by their keys: each one's `server`, its `name`, and what its meter measured, or a
  `problem` saying why it couldn't, such as its server not being up. For a plugin that uses this one.
  """
  def now(call, servers \\ nil) do
    servers = servers || OneCore.Service.servers(call, Meter)

    for server <- servers do
      measured =
        try do
          OneCore.Service.call(call, Meter, :now, [], server)
        rescue
          problem -> %{"problem" => Exception.message(problem)}
        end

      Map.merge(measured, %{"server" => server, "name" => OneCore.Service.name(server)})
    end
  end
end
