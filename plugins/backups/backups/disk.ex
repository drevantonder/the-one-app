defmodule Backups.Disk do
  @moduledoc """
  A folder on the machine it runs on, as on a disk plugged in there, that backups go to. It holds nothing between
  calls: each is told the folder. What's written is written beside its name and then given it, so a name holds a
  whole object or nothing.
  """
  use GenServer
  @behaviour OneCore.Service

  @impl OneCore.Service
  def called, do: "Backups to a disk"

  @impl OneCore.Service
  def needs,
    do: "A folder on this machine to back up to, as on a disk plugged into it, which whoever runs the server may write"

  @impl GenServer
  def init(_), do: {:ok, nil}

  def put(_service, folder, name, data) do
    file = file(folder, name)
    File.mkdir_p!(Path.dirname(file))
    writing = file <> ".writing"
    File.write!(writing, Base.decode64!(data))
    File.rename!(writing, file)
    "ok"
  end

  def get(_service, folder, name) do
    case File.read(file(folder, name)) do
      {:ok, data} -> Base.encode64(data)
      {:error, :enoent} -> nil
    end
  end

  def remove(_service, folder, name) do
    File.rm(file(folder, name))
    "ok"
  end

  def list(_service, folder, prefix) do
    Path.wildcard(Path.join(folder, "**"))
    |> Enum.filter(&File.regular?/1)
    |> Enum.map(&Path.relative_to(&1, folder))
    |> Enum.filter(&(String.starts_with?(&1, prefix) and not String.ends_with?(&1, ".writing")))
    |> Enum.sort()
  end

  # A name is the backup's own, so it stays inside the folder.
  defp file(folder, name) do
    if Path.type(folder) != :absolute or ".." in Path.split(name), do: raise("#{name} isn't in #{folder}")
    Path.join(folder, name)
  end
end
