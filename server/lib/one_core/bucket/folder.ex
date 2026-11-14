defmodule OneCore.Bucket.Folder do
  @moduledoc """
  A folder as a bucket, for a network on one machine. A blob is a file at its key. No part of a key starts with a
  dot, which keeps keys inside the folder and leaves dot names for this module's own files.

  A folder has no signer to make links to its blobs, so the server answers them itself, to this machine alone,
  which is where a network kept in a folder is. A link says what it's to and when it ends, signed with a secret
  made when the server first gives one.
  """
  @behaviour OneCore.Bucket
  require Record

  Record.defrecordp(:asked, :mod, Record.extract(:mod, from_lib: "inets/include/httpd.hrl"))

  # How long a link works for, in seconds, as the signer's does.
  @linked_for 60

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

  @impl true
  def size(folder, key) do
    case File.stat(file(folder, key)) do
      {:ok, %{size: size}} -> {:ok, size}
      {:error, :enoent} -> :absent
    end
  end

  @impl true
  def link(folder, key, mode) do
    file(folder, key)
    {port, secret} = links()
    to = Base.url_encode64(:erlang.term_to_binary({folder, key, mode, System.system_time(:second) + @linked_for}))
    method = if mode == :read, do: "GET", else: "PUT"
    %{method: method, url: "http://127.0.0.1:#{port}/#{to}.#{signature(secret, to)}", headers: %{}}
  end

  defp signature(secret, to), do: Base.url_encode64(:crypto.mac(:hmac, :sha256, secret, to))

  # Where links are answered, and the secret they're signed with: made when the first link is.
  defp links do
    with nil <- :persistent_term.get(__MODULE__, nil) do
      :global.trans({__MODULE__, self()}, fn ->
        with nil <- :persistent_term.get(__MODULE__, nil) do
          {:ok, _} = Application.ensure_all_started(:inets)

          here = [
            bind_address: {127, 0, 0, 1},
            port: 0,
            server_name: ~c"bucket",
            server_root: ~c"/",
            document_root: ~c"/"
          ]

          {:ok, listener} = :inets.start(:httpd, [modules: [__MODULE__]] ++ here)
          links = {:httpd.info(listener)[:port], :crypto.strong_rand_bytes(32)}
          :persistent_term.put(__MODULE__, links)
          links
        end
      end)
    end
  end

  @doc false
  # Answers a link, as `:httpd` asks a module to: the blob read, or written only if there's none.
  def unquote(:do)(request) do
    {_port, secret} = :persistent_term.get(__MODULE__)

    {status, body} =
      with [to, signed] <- String.split(String.trim_leading(List.to_string(asked(request, :request_uri)), "/"), "."),
           true <- signed == signature(secret, to),
           {folder, key, mode, until} <- :erlang.binary_to_term(Base.url_decode64!(to)),
           true <- until > System.system_time(:second) do
        case {mode, asked(request, :method)} do
          {:read, ~c"GET"} ->
            with {:ok, data, _tag} <- read(folder, key), do: {200, data}, else: (:absent -> {404, ""})

          {:write, ~c"PUT"} ->
            with :ok <- write(folder, key, :erlang.list_to_binary(asked(request, :entity_body))),
                 do: {200, ""},
                 else: (:exists -> {412, ""})

          _ ->
            {405, ""}
        end
      else
        _ -> {403, ""}
      end

    {:proceed, [response: {:response, [code: status, content_length: Integer.to_charlist(byte_size(body))], [body]}]}
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
