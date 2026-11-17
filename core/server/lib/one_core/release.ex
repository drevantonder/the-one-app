defmodule OneCore.Release do
  @moduledoc """
  A server installed on a machine, as the program that keeps it running starts it (`one serve run`, in
  `clients/cli/src/serving.rs`): `bin/one_core eval 'OneCore.Release.serve()'`. It says on its output when it's
  running, and when it's asked to update, and that program does the rest: it takes the latest build, starts it,
  and goes back to this one if that doesn't start and answer.

  That program tells it where an update is as it moves on, a line of JSON on its input each time, which it says in
  `builds` (`OneCore.Builds.updating/1`): `{"update": <stage>}`, and `{"went_back": <why>}` where one didn't take.
  An update's stages, in order:

  - `downloading`: the latest build is downloaded, unpacked and checked to start, while this one answers on.
  - `restarting`: this one is stopping, to start that one. It says so at the network's edge first, so the other
    servers wait for it rather than take it for off, and its input closes.
  - `starting`: the new build is starting, which it's told as it starts (`ONE_UPDATE`). It's taken once it's running
    and answers by its key, and then there's no stage. One that doesn't, within three minutes, is stopped and the one
    before starts again, saying why in `went_back`. Being reached is part of being up: until then it isn't
    (`OneCore.Server.up?/0`), so a build that can't be reached takes on nothing the others would then wait on it for.

  With no stage, no update is under way: none was asked, the server was on the latest builds already, or one took.
  """
  require Logger
  alias OneCore.{Builds, Server}

  @doc "What it says when it's running, with its key after."
  def running, do: "A server is running: "

  @doc "What it says when it's asked to take the latest build."
  def update_asked, do: "Update asked"

  @doc """
  Starts the server, says so, and runs until it's stopped: until its input closes, as the program that started it
  stops it, or as a system stops a program, with SIGTERM. Either way it takes nothing new and answers what it took
  already (`OneCore.Application`). Until then, each line on its input says where an update is.
  """
  def serve do
    case Application.ensure_all_started(:one_core) do
      {:ok, _started} ->
        IO.puts(running() <> Server.key())
        told()
        System.stop()
        Process.sleep(:infinity)

      {:error, {:one_core, {{:not_joined, key}, _start}}} ->
        IO.puts(:stderr, "This server, #{key}, isn't on the list of whoever it's for: `one serve on` puts it there.")
        System.halt(1)

      {:error, why} ->
        IO.puts(:stderr, "The server didn't start: #{inspect(why)}")
        System.halt(1)
    end
  end

  # Reads what the program that started it tells it, until its input closes.
  defp told do
    with line when is_binary(line) <- IO.read(:line) do
      case JSON.decode(line) do
        {:ok, %{} = said} -> say(said)
        _ -> Logger.warning("The program keeping this server running said what it doesn't read: #{String.trim(line)}")
      end

      told()
    end
  end

  # Where an update is is said, even where the edge or the bucket can't be reached just now: what this program says
  # next carries on.
  defp say(said) do
    Builds.updating(said)
  rescue
    error -> Logger.warning("Couldn't say where its update is: #{Exception.message(error)}")
  end

  @doc """
  Has the program keeping this server running take the latest builds, unless an update is under way already. Gives
  the update's stage once that program has said it, or `nil` where it says none: one from before it said stages.
  """
  def update do
    if Builds.update() == nil do
      Builds.await_update(fn -> IO.puts(update_asked()) end)
    else
      Builds.update()
    end
  end

  @doc "Says which build this is, once its code and NIFs have loaded: what's checked of a build before it's started."
  def check do
    Application.load(:one_core)
    for module <- Application.spec(:one_core, :modules), do: Code.ensure_loaded!(module)

    for module <- Application.spec(:one_core, :modules),
        function_exported?(module, :__nif__, 0),
        do: OneCore.Nif.build(module)

    IO.puts(Builds.build())
  end
end
