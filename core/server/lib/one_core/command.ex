defmodule OneCore.Command do
  @moduledoc """
  Runs a program on this machine for at most a while, as an app's service runs adb, simctl, podman or a person's
  command. It leads a process group of its own, as every program a server starts does (`erl_child_setup` starts each
  in a session of its own), and once it takes too long the group is killed whole, so whatever it started goes too.
  What it writes as errors goes to a file beside while it runs, since a port reads a program's output as one stream.
  It needs `sh`, which a Windows server hasn't.
  """

  @doc """
  Runs `[program | args]` for at most `timeout` seconds. Gives its exit code, -1 where it was stopped (its errors then
  end "(stopped after <timeout>s)"), and all it printed and wrote as errors.

  Options: `env`, the variables it's given beside this server's, as `{name, value}`s; `cd`, the folder it runs in;
  `each`, `{fun, first}`, which is given each line it prints and what it gave for the line before, `first` for the first.
  """
  def run([program | args], timeout, options \\ []) do
    errors = Path.join(System.tmp_dir!(), "one-command-#{System.unique_integer([:positive])}")
    each = options[:each]

    port =
      Port.open(
        {:spawn_executable, System.find_executable("sh")},
        [:binary, :exit_status, args: ["-c", ~s(exec </dev/null 2>"$0"; exec "$@"), errors, program | args]] ++
          Enum.flat_map(options, fn
            {:env, env} -> [env: for({name, value} <- env, do: {String.to_charlist(name), String.to_charlist(value)})]
            {:cd, folder} -> [cd: folder]
            {:each, _} -> [line: 16_384]
          end)
      )

    {exit, stdout} = read(port, System.monotonic_time(:millisecond) + timeout * 1000, [], each)
    stderr = with {:ok, text} <- File.read(errors), do: text, else: (_ -> "")
    File.rm(errors)

    stderr =
      cond do
        exit != -1 -> stderr
        stderr == "" or String.ends_with?(stderr, "\n") -> stderr <> "(stopped after #{timeout}s)"
        true -> stderr <> "\n(stopped after #{timeout}s)"
      end

    %{exit: exit, stdout: stdout, stderr: stderr}
  end

  # Reads what it prints until it ends, or kills its group once `until` passes. Its id is asked for only then: one
  # that has ended has none.
  defp read(port, until, printed, each) do
    receive do
      {^port, {:data, {:noeol, part}}} -> read(port, until, [printed | part], each)
      {^port, {:data, {:eol, line}}} -> read(port, until, [printed, line, "\n"], with({fun, at} <- each, do: {fun, fun.(line, at)}))
      {^port, {:data, data}} -> read(port, until, [printed | data], each)
      {^port, {:exit_status, status}} -> {status, IO.iodata_to_binary(printed)}
    after
      max(0, until - System.monotonic_time(:millisecond)) ->
        with {:os_pid, pid} <- Port.info(port, :os_pid), do: System.cmd("kill", ["-KILL", "--", "-#{pid}"])
        {-1, killed(port, printed)}
    end
  end

  # What it printed before its group was killed, read until its port ends with it, as it does at once. One the kill
  # didn't reach (it left its group) is let go after a second.
  defp killed(port, printed) do
    receive do
      {^port, {:data, {:eol, line}}} -> killed(port, [printed, line, "\n"])
      {^port, {:data, {:noeol, part}}} -> killed(port, [printed | part])
      {^port, {:data, data}} -> killed(port, [printed | data])
      {^port, {:exit_status, _}} -> IO.iodata_to_binary(printed)
    after
      1000 ->
        Port.close(port)
        IO.iodata_to_binary(printed)
    end
  end

  @doc "The end of `text`, at most `max` bytes of it, where errors usually are, saying how much was cut before."
  def clip(text, max) do
    text = String.replace_invalid(text)

    if byte_size(text) <= max do
      text
    else
      tail = String.replace_invalid(binary_part(text, byte_size(text) - max, max), "")
      "(#{byte_size(text) - byte_size(tail)} earlier bytes cut) …" <> tail
    end
  end
end
