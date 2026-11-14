defmodule OneCore.Bucket.Folder do
  @moduledoc """
  A folder as a bucket, for a network on one machine. A blob is a file at its key. No part of a key starts with a
  dot, which keeps keys inside the folder and leaves dot names for this module's own files.
  """
  @behaviour OneCore.Bucket

  @impl true
  def read(folder, key) do
    case File.read(file(folder, key)) do
      {:ok, data} -> {:ok, data, :crypto.hash(:sha256, data)}
      {:error, :enoent} -> :absent
    end
  end

  # A hard link is made only if nothing has its name, and then holds the whole blob at once.
  @impl true
  def write(folder, key, data) do
    file = file(folder, key)
    File.mkdir_p!(Path.dirname(file))
    written = written(folder, data)

    try do
      case File.ln(written, file) do
        :ok -> :ok
        {:error, :eexist} -> :exists
      end
    after
      File.rm!(written)
    end
  end

  # One replace of a blob at a time, by a folder only one of them can make. A second one meanwhile is told the blob
  # changed, as a bucket in the cloud tells two conditional writes that meet.
  @impl true
  def replace(folder, key, data, tag) do
    file = file(folder, key)
    replacing = Path.join([folder, ".replacing", Base.encode16(:crypto.hash(:sha256, key))])
    File.mkdir_p!(Path.dirname(replacing))

    case File.mkdir(replacing) do
      :ok ->
        try do
          case read(folder, key) do
            {:ok, _, ^tag} -> File.rename!(written(folder, data), file)
            _ -> :changed
          end
        after
          File.rmdir!(replacing)
        end

      {:error, :eexist} ->
        :changed
    end
  end

  @impl true
  def list(folder, prefix) do
    within = if String.ends_with?(prefix, "/") or prefix == "", do: prefix, else: Path.dirname(prefix) <> "/"
    within = String.trim_leading(within, "./")
    Enum.sort(for key <- keys(folder, within), String.starts_with?(key, prefix), do: key)
  end

  @impl true
  def remove(folder, key) do
    File.rm(file(folder, key))
    :ok
  end

  defp keys(folder, within) do
    case File.ls(Path.join(folder, within)) do
      {:ok, names} ->
        for name <- names, not String.starts_with?(name, "."), key <- under(folder, within <> name), do: key

      {:error, :enoent} ->
        []
    end
  end

  defp under(folder, key) do
    if File.dir?(Path.join(folder, key)), do: keys(folder, key <> "/"), else: [key]
  end

  defp file(folder, key) do
    for part <- String.split(key, "/"), part == "" or String.starts_with?(part, ".") do
      raise ArgumentError, "a blob's key is parts joined by /, none empty or starting with a dot: #{inspect(key)}"
    end

    Path.join(folder, key)
  end

  defp written(folder, data) do
    file = Path.join([folder, ".writing", Integer.to_string(System.unique_integer([:positive])) <> "-" <> System.pid()])
    File.mkdir_p!(Path.dirname(file))
    File.write!(file, data)
    file
  end
end
