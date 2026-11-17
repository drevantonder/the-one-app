defmodule Containers.Sealed do
  @moduledoc """
  A file as `upload` sends it to the bucket and `download` fetches it back: sealed with AES-256-GCM under a key made
  for that one blob, a chunk of 1 MiB at a time, so a file of any size is sealed and sent, or fetched and opened,
  without ever being whole in memory, and without passing through a tool's result.

  The blob is a version byte, 1, then each chunk sealed with its tag after it: chunk `n` under the nonce `n`, and
  the last with "last" as its associated data, so a blob that's cut short or put in another order doesn't open. An
  empty file is one empty last chunk. The key is fresh for every blob, so no nonce is ever used twice under one.
  """
  @chunk 1024 * 1024
  @tag 16

  @doc """
  Seals the file at `path` with `key` and sends it by `link`, which writes one blob (`OneCore.Artifact.link/3`).
  Gives the file's size in bytes.
  """
  def put(path, key, %{"method" => method, "url" => url, "headers" => headers}) do
    size = File.stat!(path).size
    chunks = max(1, div(size + @chunk - 1, @chunk))

    sealed =
      if size == 0,
        do: [<<1>>, seal(key, 0, true, "")],
        else:
          Stream.concat(
            [<<1>>],
            path
            |> File.stream!(@chunk)
            |> Stream.with_index()
            |> Stream.map(fn {plain, n} -> seal(key, n, n == chunks - 1, plain) end)
          )

    # A bucket takes a blob sent in parts only with its whole length said first, which a sealed file's is.
    length = 1 + size + chunks * @tag

    response =
      Req.request!(
        method: method |> String.downcase() |> String.to_existing_atom(),
        url: url,
        headers: Map.to_list(Map.put(headers, "content-length", Integer.to_string(length))),
        body: sealed,
        retry: false,
        decode_body: false,
        receive_timeout: 600_000
      )

    if response.status not in 200..299, do: raise("The bucket didn't take the file (#{response.status})")
    size
  end

  @doc """
  Fetches a blob `put/3` sent, by `link`, which reads it, and opens it with `key` into the file at `path`. Gives
  the file's size in bytes.
  """
  def get(%{"method" => method, "url" => url, "headers" => headers}, key, path) do
    sealed = path <> ".sealed"

    response =
      Req.request!(
        method: method |> String.downcase() |> String.to_existing_atom(),
        url: url,
        headers: Map.to_list(headers),
        into: File.stream!(sealed),
        retry: :transient,
        raw: true,
        receive_timeout: 600_000
      )

    if response.status not in 200..299, do: raise("The bucket didn't give the file (#{response.status})")
    open(sealed, key, path)
  after
    File.rm(path <> ".sealed")
  end

  defp open(sealed, key, path) do
    chunks = max(1, div(File.stat!(sealed).size - 1 + @chunk + @tag - 1, @chunk + @tag))
    from = File.open!(sealed, [:read, :binary, :raw, {:read_ahead, @chunk + @tag}])
    to = File.open!(path, [:write, :binary, :raw])

    try do
      <<1>> = IO.binread(from, 1)

      Enum.reduce(0..(chunks - 1), 0, fn n, size ->
        plain = unseal(key, n, n == chunks - 1, IO.binread(from, @chunk + @tag))
        :ok = IO.binwrite(to, plain)
        size + byte_size(plain)
      end)
    after
      File.close(from)
      File.close(to)
    end
  end

  defp seal(key, n, last, plain) do
    {sealed, tag} = :crypto.crypto_one_time_aead(:aes_256_gcm, key, <<n::96>>, plain, aad(last), true)
    [sealed, tag]
  end

  defp unseal(key, n, last, chunk) when is_binary(chunk) and byte_size(chunk) >= @tag do
    {sealed, tag} = :erlang.split_binary(chunk, byte_size(chunk) - @tag)

    case :crypto.crypto_one_time_aead(:aes_256_gcm, key, <<n::96>>, sealed, aad(last), tag, false) do
      :error -> raise "The file doesn't open with its key"
      plain -> plain
    end
  end

  defp unseal(_key, _n, _last, _chunk), do: raise("The file is cut short")

  defp aad(true), do: "last"
  defp aad(false), do: ""
end
