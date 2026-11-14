defmodule OneCore.Call do
  @moduledoc """
  What a tool is called with besides its inputs.

  - `own`: the plugin's own `OneCore.Artifact` for whoever it runs for. Only this plugin reaches it.
  - `folder`: a folder on the machine running this call that is the plugin's own for whoever it runs for, for what
    it keeps there. It may not be there yet, and another server has another.
  - `server`: the key of the server running this call.
  - `for`: the user or group this call runs for.

  The rest is One Core's: which plugin this is, what was asked of it, the plugins it uses, and whether another
  server passed it on.
  """
  alias OneCore.{Plugins, Server}
  alias OneCore.Plugins.Version

  @enforce_keys [:own, :folder, :server, :for, :plugin, :ask, :uses, :passed_on]
  defstruct [:own, :folder, :server, :for, :plugin, :ask, :uses, :passed_on]

  @type t :: %__MODULE__{own: OneCore.Artifact.t(), folder: String.t(), server: String.t(), for: String.t()}

  @doc """
  A plugin this call's plugin uses, or that one of those uses: its module, whose functions take this call and run
  as this call's plugin, for whoever it runs for.

      database = OneCore.Call.plugin(call, "database")
  """
  def plugin(%__MODULE__{uses: uses, plugin: plugin}, name) do
    case uses do
      %{^name => module} -> module
      _ -> raise "#{plugin} doesn't say it uses #{name}. Its uses/0 names the plugins it uses."
    end
  end

  @doc """
  A process that keeps running on this server between calls: the GenServer `module`, which is one of this call's
  plugin's or of a plugin it uses. There's one for each `key`, started when first asked for, with `argument` given
  to its `init/1`, and found again by every call after. It carries on through its plugin's updates as a service
  does, and one that stopped is started by the next call that asks for it.

      counter = OneCore.Call.process(call, Hello.Counter, call.for, 0)
  """
  def process(%__MODULE__{uses: uses, plugin: plugin}, module, key, argument) do
    under = Version.under(module)
    plugin = Enum.find_value(uses, plugin, fn {name, used} -> Version.under(used) == under && name end)

    Plugins.Process.of(plugin, Version.own_name(module), key, argument) ||
      raise "#{inspect(module)} isn't a module of #{plugin} as it runs now"
  end

  @doc """
  Asks another server to run this call instead, for when what it needs is there: a database's lease, say. If that
  server runs it, the call ends here with its answer, and nothing after `pass_on` runs.

  `:no_answer` means it didn't run there: that server couldn't be reached, or would have passed it on in turn. A
  server that stops answering after it was asked raises, since it may have run.
  """
  def pass_on(%__MODULE__{passed_on: true}, _server), do: throw({__MODULE__, :passed_back})

  def pass_on(%__MODULE__{ask: {method, params}}, server) do
    passed_back = Server.passed_back()

    case Server.request(server, method, Map.put(params, :_meta, %{passed_on: true})) do
      {:ok, result} -> throw({__MODULE__, {:passed_on, result}})
      {:error, ^passed_back, _why} -> :no_answer
      {:error, _code, why} -> raise why
      :unreachable -> :no_answer
      :stopped -> raise Server.said(:stopped)
    end
  end
end
