defmodule RemoteShell.Runner do
  @moduledoc """
  Runs a command on its device, as `sh -c` does, and logs each in its folder there, a line of JSON each in
  `ran.jsonl`: when, the command, its folder, its exit code and who ran it.

  Each command is a program of its own, which leads a process group of its own as every program a server starts
  does, so stopping one that took too long stops whatever it started too. What it writes as errors goes to a file
  beside the log while it runs, since a program's output reaches here as one stream.
  """
  use GenServer
  @behaviour OneCore.Service

  # The most of each of what a command prints that's given back: the end of it, where errors usually are.
  @kept 64 * 1024

  @impl OneCore.Service
  def called, do: "Remote Shell's runner"

  @impl OneCore.Service
  def needs do
    "A Linux or Mac machine, with sh. It runs any command it's given, as the user this server runs as, for everyone this server acts for, and keeps a log of each on this machine."
  end

  @impl GenServer
  def init(_) do
    System.find_executable("sh") || raise "This machine has no sh to run commands with"
    {:ok, nil}
  end

  @doc """
  Runs `command` in `dir`, the home folder if `nil`, for at most `timeout` seconds, for `by`. Gives its exit code,
  -1 if it was stopped, and what it printed.
  """
  def run(service, command, dir, timeout, by) do
    dir = dir || System.user_home!()
    if not File.dir?(dir), do: raise("There's no folder #{dir} on this device")
    folder = OneCore.Service.folder(service)
    errors = Path.join(folder, "errors-#{System.unique_integer([:positive])}")

    program =
      Port.open({:spawn_executable, System.find_executable("sh")}, [
        :binary,
        :exit_status,
        args: ["-c", ~s(exec </dev/null 2>"$0"; eval "$1"), errors, command],
        cd: dir
      ])

    {:os_pid, pid} = Port.info(program, :os_pid)
    {exit, stdout} = read(program, pid, System.monotonic_time(:millisecond) + timeout * 1000, [])
    stderr = File.read!(errors)
    File.rm!(errors)
    stderr = if exit == -1, do: stopped(stderr, timeout), else: stderr

    ran = %{time: System.system_time(:millisecond), command: command, dir: dir, exit: exit, by: by}
    File.write!(Path.join(folder, "ran.jsonl"), JSON.encode!(ran) <> "\n", [:append])
    %{exit: exit, stdout: clip(stdout), stderr: clip(stderr)}
  end

  # Reads what the command prints until it ends, or stops it and everything it started once `until` passes.
  defp read(program, pid, until, printed) do
    receive do
      {^program, {:data, data}} -> read(program, pid, until, [printed | data])
      {^program, {:exit_status, status}} -> {status, IO.iodata_to_binary(printed)}
    after
      max(0, until - System.monotonic_time(:millisecond)) ->
        System.cmd("kill", ["-KILL", "--", "-#{pid}"])
        Port.close(program)
        {-1, IO.iodata_to_binary(printed)}
    end
  end

  defp stopped(stderr, timeout) do
    if stderr == "" or String.ends_with?(stderr, "\n"),
      do: stderr <> "(stopped after #{timeout}s)",
      else: stderr <> "\n(stopped after #{timeout}s)"
  end

  # Keeps the end of long output, and says how much was cut.
  defp clip(text) do
    text = String.replace_invalid(text)

    if byte_size(text) <= @kept do
      text
    else
      kept = String.replace_invalid(binary_part(text, byte_size(text) - @kept, @kept), "")
      "(#{byte_size(text) - byte_size(kept)} earlier bytes cut) …" <> kept
    end
  end
end
