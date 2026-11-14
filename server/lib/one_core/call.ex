defmodule OneCore.Call do
  @moduledoc """
  What a tool is called with besides its inputs.

  - `own`: the plugin's own `OneCore.Artifact` for whoever it runs for. Only this plugin reaches it.
  - `server`: the key of the server running this call.

  The rest is One Core's: which tool this is, its inputs, the plugins it uses, and whether another server passed it
  on.
  """
  alias OneCore.Server

  @enforce_keys [:own, :server, :plugin, :tool, :inputs, :uses, :passed_on]
  defstruct [:own, :server, :plugin, :tool, :inputs, :uses, :passed_on]

  @type t :: %__MODULE__{own: OneCore.Artifact.t(), server: String.t()}

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
  Asks another server to run this call instead, for when what it needs is there: a database's lease, say. If that
  server runs it, the call ends here with its answer, and nothing after `pass_on` runs.

  `:no_answer` means it didn't run there: that server couldn't be reached, or would have passed it on in turn. A
  server that stops answering after it was asked raises, since it may have run.
  """
  def pass_on(%__MODULE__{passed_on: true}, _server), do: throw({__MODULE__, :passed_back})

  def pass_on(%__MODULE__{} = call, server) do
    params = %{name: "#{call.plugin}_#{call.tool}", arguments: call.inputs, _meta: %{passed_on: true}}

    case Server.ask(server, JSON.encode!(%{jsonrpc: "2.0", id: 1, method: "tools/call", params: params})) do
      {:ok, answer} ->
        case JSON.decode!(answer) do
          %{"result" => result} ->
            throw({__MODULE__, {:passed_on, result}})

          %{"error" => %{"code" => code, "message" => why}} ->
            if code == Server.passed_back(), do: :no_answer, else: raise(why)
        end

      :unreachable ->
        :no_answer

      :stopped ->
        raise "The server this was passed on to stopped answering, so it may or may not have been done"
    end
  end
end
