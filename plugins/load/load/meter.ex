defmodule Load.Meter do
  @moduledoc """
  Measures the machine it runs on, with a program of its own: `meter/main.rs`, which it builds there and starts.

  It holds the running program between calls. Asked for a measurement, it writes the program a line and reads
  the line that comes back.
  """
  use GenServer
  @behaviour OneCore.Service

  @impl OneCore.Service
  def needs, do: "Rust's compiler, rustc, on the machine, to build the program that measures it. Linux or Mac."

  @doc "This machine's processors, its load over the last 1, 5 and 15 minutes, and since when it's measured."
  def now(service), do: GenServer.call(service, :now)

  @impl GenServer
  def init(_), do: {:ok, start(OneCore.Service.folder(self(), "meter"))}

  @impl GenServer
  def handle_call(:now, _from, %{program: program} = state) do
    Port.command(program, "now\n")

    receive do
      {^program, {:data, {:eol, line}}} -> {:reply, JSON.decode!(line), state}
      {^program, {:exit_status, status}} -> raise "Its program stopped, with status #{status}"
    end
  end

  # The program stopped by itself, so the service does too, and its server starts both again.
  @impl GenServer
  def handle_info({program, {:exit_status, status}}, %{program: program}) do
    raise "Its program stopped, with status #{status}"
  end

  # A new version of the plugin. The program carries on through it unless its own source changed, which gives
  # it a new folder: then it's built and started again.
  @impl GenServer
  def code_change(_version, state, _extra) do
    case OneCore.Service.folder(self(), "meter") do
      folder when folder == state.folder ->
        {:ok, state}

      folder ->
        Port.close(state.program)
        {:ok, start(folder)}
    end
  end

  # The program reads lines until its input closes, which is when this process ends or closes it. So nothing
  # more is needed to stop it.
  defp start(folder) do
    built = Path.join(folder, "meter")
    if not File.exists?(built), do: build(folder, built)
    %{folder: folder, program: Port.open({:spawn_executable, built}, [:binary, :exit_status, line: 4096])}
  end

  # Built under another name and then renamed, so a program that's there is a whole one.
  defp build(folder, built) do
    rustc = System.find_executable("rustc") || raise "This machine has no rustc to build the meter with"
    building = ["--edition", "2021", "-O", "-o", built <> ".building", "main.rs"]

    case System.cmd(rustc, building, cd: folder, stderr_to_stdout: true) do
      {_, 0} -> File.rename!(built <> ".building", built)
      {said, _} -> raise "The meter didn't build:\n" <> said
    end
  end
end
