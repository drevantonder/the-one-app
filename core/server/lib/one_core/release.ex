defmodule OneCore.Release do
  @moduledoc """
  A server installed on a machine, as the program that keeps it running starts it (`one serve run`, in
  `clients/cli/src/serving.rs`): `bin/one_core eval 'OneCore.Release.serve()'`. It says on its output when it's
  running, and when it's asked to update, and that program does the rest: it takes the newest build, starts it,
  and goes back to this one if that doesn't start and answer.
  """
  alias OneCore.{Builds, Server}

  @doc "What it says when it's running, with its key after."
  def running, do: "A server is running: "

  @doc "What it says when it's asked to take the newest build."
  def update_asked, do: "Update asked"

  @doc "Starts the server, says so, and runs until it's stopped."
  def serve do
    case Application.ensure_all_started(:one_core) do
      {:ok, _started} ->
        IO.puts(running() <> Server.key())
        Process.sleep(:infinity)

      {:error, {:one_core, {{:not_joined, key}, _start}}} ->
        IO.puts(:stderr, "This server, #{key}, isn't on the list of whoever it's for: `one serve on` puts it there.")
        System.halt(1)

      {:error, why} ->
        IO.puts(:stderr, "The server didn't start: #{inspect(why)}")
        System.halt(1)
    end
  end

  @doc "Says which build this is, once its code and NIFs have loaded: what's checked of a build before it's started."
  def check do
    Application.load(:one_core)
    for module <- Application.spec(:one_core, :modules), do: Code.ensure_loaded!(module)
    IO.puts(Builds.build())
  end
end
