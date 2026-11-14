defmodule Database.Log do
  @moduledoc """
  A database's blobs: its log and its snapshots.

  `log/<number>` holds the pages of the database file that one write changed, and the server that wrote it. The
  numbers have no gaps, and each blob is written only if absent, so no two servers write the same number. Writing
  the next number is the lease: the server named in the newest blob holds it, and a server takes it by writing a
  blob with no pages.

  `snapshot/<number>` is the whole database file as it was after that log blob, which any SQLite opens. The newest
  two are kept and the ones before them removed, since the log after a snapshot is all that's read with it. The
  log itself is kept whole: no number is ever given up, which is what makes writing the next one the lease, and
  it's what a database is put back to an earlier moment from.

  Nothing here needs more of the bucket than writing a blob only if it's absent.
  """
  alias OneCore.Artifact

  @doc "The newest snapshot's number, or 0 with none."
  def snapshot(artifact) do
    case List.last(Artifact.list(artifact, "snapshot/")) do
      nil -> 0
      name -> String.to_integer(Path.basename(name))
    end
  end

  @doc "The log's entries after `number`, in order."
  def since(artifact, number) do
    case entry(artifact, number + 1) do
      nil -> []
      entry -> [entry | since(artifact, number + 1)]
    end
  end

  @doc "A log entry: its `server`, and for a write its `pages` by their numbers and the file's `size` in pages."
  def entry(artifact, number) do
    with {:ok, entry, _tag} <- Artifact.read(artifact, name("log", number)) do
      :erlang.binary_to_term(entry, [:safe])
    else
      :absent -> nil
    end
  end

  @doc "Writes the log's entry `number`, unless a server already has."
  def write(artifact, number, entry) do
    Artifact.write(artifact, name("log", number), :erlang.term_to_binary(entry, [:compressed]))
  end

  @doc """
  Keeps the database `file` as the snapshot after the log's entry `number`, and removes the snapshots before the
  one it follows. That one stays for a server that is about to read it.
  """
  def keep(artifact, number, file) do
    Artifact.write(artifact, name("snapshot", number), File.read!(file))
    for old <- Enum.drop(Enum.reverse(Artifact.list(artifact, "snapshot/")), 2), do: Artifact.remove(artifact, old)
    :ok
  end

  @doc """
  Makes `file` the database as it was after the snapshot `number` and then `entries`. `:gone` if that snapshot has
  been removed since it was listed, which means there are newer ones.
  """
  def restore(artifact, number, entries, file) do
    case if(number > 0, do: Artifact.read(artifact, name("snapshot", number)), else: {:ok, "", nil}) do
      {:ok, snapshot, _tag} -> restore(snapshot, entries, file)
      :absent -> :gone
    end
  end

  defp restore(snapshot, entries, file) do
    File.write!(file, snapshot)
    {:ok, writing} = :file.open(file, [:read, :write, :raw, :binary])

    for %{pages: [{_number, first} | _] = pages, size: size} <- entries do
      :ok = :file.pwrite(writing, for({number, page} <- pages, do: {(number - 1) * byte_size(first), page}))
      {:ok, _} = :file.position(writing, size * byte_size(first))
      :ok = :file.truncate(writing)
    end

    :ok = :file.close(writing)
  end

  @doc """
  What the transaction just committed to the database `file` changed: the file's size in pages and the pages
  written, by their numbers. `nil` if it changed nothing.

  SQLite wrote them to its write-ahead log, which holds that one transaction: a header of 32 bytes, then frames
  of a page's number, the file's size where the transaction ends, 16 bytes more, and the page.
  """
  def changed(file) do
    case File.read(file <> "-wal") do
      {:ok, <<_::binary-8, page::32, _::binary-20, frames::binary>>} -> frames(frames, page, %{}, 0)
      {:ok, ""} -> nil
      {:error, :enoent} -> nil
    end
  end

  defp frames(<<number::32, size::32, _::binary-16, rest::binary>>, page, pages, _size) do
    <<data::binary-size(^page), rest::binary>> = rest
    frames(rest, page, Map.put(pages, number, data), size)
  end

  defp frames(<<>>, _page, pages, size) when size > 0, do: {size, Enum.sort(pages)}

  defp name(kind, number), do: "#{kind}/" <> String.pad_leading("#{number}", 16, "0")
end
