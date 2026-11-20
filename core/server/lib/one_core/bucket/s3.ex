defmodule OneCore.Bucket.S3 do
  @moduledoc """
  A bucket that speaks S3, such as Cloudflare R2, used with no credentials of its own. For each request the server
  asks the presigner at the network's address, which signs that one request if this server is on the list of whoever
  the blob belongs to, and the server sends what was signed. A read or a list the presigner sends itself, and answers
  with what the bucket answered, so it's one trip across the internet rather than two. The address is `where`.

  The presigner signs a write only with its condition, so the bucket itself refuses one that would overwrite. A blob's
  tag is its ETag. What the presigner won't sign for this server raises `OneCore.Bucket.Refused`, and a presigner or
  bucket that can't be reached `OneCore.Edge.Unreachable`.
  """
  @behaviour OneCore.Bucket
  import SweetXml, only: [sigil_x: 2, parse: 1, xpath: 2, xpath: 3]
  alias OneCore.Edge

  @impl true
  def read(address, key) do
    case send_request(address, %{read: key}) do
      %{status: 200, body: data} = got -> {:ok, data, tag(got)}
      %{status: 404} -> :absent
      refused -> raise refusal(refused)
    end
  end

  # At most this many requests to the presigner, and as many to the bucket, are sent at once, and the rest wait their
  # turn, however long. A server that starts asks for hundreds at once, and an uplink at home that's sent them all
  # stalls every one of them past its timeout. Each one's timeout runs from when it's sent.
  @at_once 16

  # Each read is a trip across the internet, so they're made together, as many at once as the pool has connections
  # (`@at_once`): a day of Telemetry is some 700 blobs, and a process each, all asking at once, only waited on the pool
  # while holding everything they'd read. One that couldn't be made is raised as it would be alone.
  @impl true
  def read_all(address, keys) do
    read = fn key ->
      try do
        {:read, read(address, key)}
      rescue
        error -> {:raised, error, __STACKTRACE__}
      end
    end

    for {:ok, outcome} <- Task.async_stream(keys, read, max_concurrency: @at_once, timeout: :infinity) do
      case outcome do
        {:read, read} -> read
        {:raised, error, stacktrace} -> reraise error, stacktrace
      end
    end
  end

  @impl true
  def write(address, key, data) do
    case request(address, %{write: key}, data) do
      %{status: 200} -> :ok
      %{status: 412} -> :exists
      refused -> raise refusal(refused)
    end
  end

  @impl true
  def replace(address, key, data, tag) do
    case request(address, %{replace: key, tag: tag}, data) do
      %{status: 200} -> :ok
      %{status: status} when status in [404, 412] -> :changed
      refused -> raise refusal(refused)
    end
  end

  @impl true
  def list(address, prefix), do: for({key, _tag} <- list_tagged(address, prefix), do: key)

  # A bucket lists a thousand keys at a time, each with its ETag, as a read gives it, and says where the next ones
  # start.
  @impl true
  def list_tagged(address, prefix, next \\ nil) do
    case send_request(address, %{list: prefix, after: next}) do
      %{status: 200, body: listed} ->
        listed = parse(listed)
        here = xpath(listed, ~x"/ListBucketResult/Contents"l, key: ~x"./Key/text()"s, tag: ~x"./ETag/text()"s)
        here = for %{key: key, tag: tag} <- here, do: {key, tag}

        case xpath(listed, ~x"/ListBucketResult/NextContinuationToken/text()"s) do
          "" -> here
          next -> here ++ list_tagged(address, prefix, next)
        end

      refused ->
        raise refusal(refused)
    end
  end

  @impl true
  def remove(address, key) do
    case request(address, %{remove: key}) do
      %{status: status} when status in [200, 204] -> :ok
      refused -> raise refusal(refused)
    end
  end

  # A blob's size is listed with its key.
  @impl true
  def size(address, key) do
    case send_request(address, %{list: key}) do
      %{status: 200, body: listed} ->
        case xpath(listed, ~x"/ListBucketResult/Contents[Key='#{key}']/Size/text()"io) do
          nil -> :absent
          size -> {:ok, size}
        end

      refused ->
        raise refusal(refused)
    end
  end

  # A link is one request to the bucket as the presigner signs it, which works for a minute.
  @impl true
  def link(address, key, mode) do
    %{"method" => method, "url" => url, "headers" => headers} = signed(address, %{mode => key}, :transient)
    %{method: method, url: url, headers: headers}
  end

  # A read changes nothing, so one the internet lost or held up is asked for again. The presigner and the bucket start
  # answering a read well within a second, so one they've said nothing of for this many milliseconds is taken as lost.
  @reads_within 5_000

  # Has the presigner send `ask`, a read or a list, and gives what the bucket answered. What the presigner says
  # itself, why it won't, is JSON, and is raised.
  defp send_request(address, ask) do
    ask = JSON.encode!(ask)

    response =
      sent(
        [
          method: :post,
          url: address <> "/presigner/read",
          body: ask,
          retry: :transient,
          decode_body: false,
          compressed: false,
          receive_timeout: @reads_within
        ],
        "/presigner/read"
      )

    case Req.Response.get_header(response, "content-type") do
      ["application/json" <> _] -> raise refused_by(address, response.status, JSON.decode!(response.body)["error"])
      _ -> response
    end
  end

  # The bucket answers a write once it has all of it, and starts well within a second, so one it's said nothing of for
  # this many milliseconds, and this many more for each kilobyte sent, is taken as lost. `OneCore.Bucket` sends a
  # write that was lost again, so a write's own tries are three of these, not more.
  @writes_within 9_000

  # Has the presigner sign `ask`, a write or a removal, and sends what it signed. `OneCore.Bucket` asks again for a
  # write or a removal that was lost, signing it anew, so a sign that was lost isn't asked for again here.
  defp request(address, ask, data \\ nil) do
    %{"method" => method, "url" => url, "headers" => headers} = signed(address, ask, false)

    sent(
      method: method |> String.downcase() |> String.to_existing_atom(),
      url: url,
      headers: headers,
      body: data,
      decode_body: false,
      compressed: false,
      receive_timeout: @writes_within + div(byte_size(data || ""), 1000)
    )
  end

  # A sign changes nothing, so one the internet lost or held up is asked for again where `retry` says so, as Req asks a
  # read again. The presigner answers well within a second, so one that hasn't come in this many milliseconds is
  # taken as lost.
  @signs_within 5_000

  defp signed(address, ask, retry) do
    ask = JSON.encode!(ask)
    request = [method: :post, url: address <> "/presigner/sign", body: ask, retry: retry, receive_timeout: @signs_within]

    case sent(request, "/presigner/sign") do
      %{status: 200, body: %{"method" => _method} = signed} -> signed
      %{status: status, body: %{"error" => why}} -> raise refused_by(address, status, why)
      refused -> raise refusal(refused)
    end
  end

  # What's raised when the presigner at `address` won't: `OneCore.Bucket.Refused` for a server on no list that may.
  defp refused_by(address, 403, why),
    do: %OneCore.Bucket.Refused{message: "The presigner at #{address} refused: #{why}"}

  defp refused_by(address, _status, why), do: %RuntimeError{message: "The presigner at #{address} refused: #{why}"}

  @pool [finch: [size: @at_once, pool_timeout: :infinity]]

  defp sent(request), do: Edge.send!(request ++ @pool)

  # An ask of the presigner at `path`, signed (`OneCore.Edge.ask!/2`). It's signed before it waits its turn, which
  # under heavy load is longer than a minute: the edge takes a signature that's 15 minutes old.
  defp sent(request, path), do: Edge.ask!(request ++ @pool, path)

  defp tag(got), do: got |> Req.Response.get_header("etag") |> hd()

  # What's raised for an answer that isn't what was asked: `OneCore.Edge.Unreachable` for a bucket or presigner that
  # failed itself (a 5xx, as R2's `ServiceUnavailable` while it's busy), which `OneCore.Bucket` sends again, and the
  # bucket's own word for anything else. An S3 error's body is XML, of which its `Code` says what it was.
  defp refusal(%{status: status, body: body}) when status >= 500,
    do: %OneCore.Edge.Unreachable{message: "#{answered(status, body)}, so this server couldn't do what was asked. Try again in a moment."}

  defp refusal(%{status: status, body: body}), do: %RuntimeError{message: answered(status, body)}

  defp answered(status, body) do
    case is_binary(body) && Regex.run(~r{<Code>([^<]+)</Code>}, body, capture: :all_but_first) do
      [code] -> "The bucket answered #{status} #{code}"
      _ -> "The bucket answered #{status}: #{inspect(body)}"
    end
  end
end
