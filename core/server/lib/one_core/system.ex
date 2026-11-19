defmodule OneCore.System do
  @moduledoc """
  A program the system keeps running in its person's session, as `one` keeps its server: a systemd user unit on Linux,
  a LaunchAgent on a Mac and a scheduled task of theirs on Windows, written and registered by the same code
  (`client::system`), which this asks of the `one` that runs this server (`one system`, `ONE_PROGRAM`). A server's
  service runs one for what must be on its person's screen, as Computer Use's Cua Driver.

  A program is a map, as `client::system::Program` says: its `name` and its server's `key` (which name it), a
  `description`, its `command` and `env` (`{name, value}` pairs), its own `folder`, whether it's on its person's screen
  (`desktop`), and `start_within` (nil, as it doesn't say it's running) and `stop_within`, in milliseconds. What the
  system won't do is raised.
  """

  @doc "Writes how the system runs `program`, which takes effect from its next start, where it isn't that already. Says whether it changed."
  def install(program), do: one(["install", json(program)]) == "true"

  @doc "Has the system start `program`, unless it's running."
  def start(program), do: one(["start", json(program)]) && :ok

  @doc "Has the system start `program` again, as it's written now."
  def restart(program), do: one(["restart", json(program)]) && :ok

  def stop(name, key), do: one(["stop", name, key]) && :ok

  @doc "Has the system forget the program `name` of the server `key`, stopped."
  def remove(name, key), do: one(["remove", name, key]) && :ok

  defp json(program), do: JSON.encode!(%{program | env: Enum.map(program.env, &Tuple.to_list/1)})

  defp one(arguments) do
    one = System.get_env("ONE_PROGRAM") || raise "This server says no `one` runs it (ONE_PROGRAM), which keeps what the system runs"

    case System.cmd(one, ["system" | arguments], stderr_to_stdout: true) do
      {said, 0} -> said |> String.trim() |> String.split("\n") |> List.last()
      {said, _} -> raise String.trim(said)
    end
  end
end
