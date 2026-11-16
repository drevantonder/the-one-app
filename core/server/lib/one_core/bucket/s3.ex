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
  require Record
  alias OneCore.Edge

  Record.defrecordp(:xml_element, :xmlElement, Record.extract(:xmlElement, from_lib: "xmerl/include/xmerl.hrl"))
  Record.defrecordp(:xml_text, :xmlText, Record.extract(:xmlText, from_lib: "xmerl/include/xmerl.hrl"))

  @impl true
  def read(address, key) do
    case answered(address, %{read: key}) do
      %{status: 200, body: data} = got -> {:ok, data, tag(got)}
      %{status: 404} -> :absent
      refused -> raise refusal(refused)
    end
  end

  # Each read is a trip across the internet, so they're made at once. One that couldn't be made is raised as it would
  # be alone.
  @impl true
  def read_all(address, keys) do
    read = fn key ->
      try do
        {:read, read(address, key)}
      rescue
        error -> {:raised, error, __STACKTRACE__}
      end
    end

    for {:ok, outcome} <- Task.async_stream(keys, read, max_concurrency: max(length(keys), 1), timeout: :infinity) do
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

  # A bucket lists a thousand keys at a time, and says where the next ones start.
  @impl true
  def list(address, prefix, next \\ nil) do
    case answered(address, %{list: prefix, after: next}) do
      %{status: 200, body: listed} ->
        {listed, _} = :xmerl_scan.string(:binary.bin_to_list(listed))

        case texts(listed, ~c"/ListBucketResult/NextContinuationToken") do
          [] -> texts(listed, ~c"/ListBucketResult/Contents/Key")
          [next] -> texts(listed, ~c"/ListBucketResult/Contents/Key") ++ list(address, prefix, next)
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

  defp texts(xml, path) do
    for element <- :xmerl_xpath.string(path, xml) do
      for xml_text(value: text) <- xml_element(element, :content), into: "", do: List.to_string(text)
    end
  end

  # A blob's size is listed with its key.
  @impl true
  def size(address, key) do
    case answered(address, %{list: key}) do
      %{status: 200, body: listed} ->
        {listed, _} = :xmerl_scan.string(:binary.bin_to_list(listed))

        case texts(listed, ~c"/ListBucketResult/Contents[Key='#{key}']/Size") do
          [size] -> {:ok, String.to_integer(size)}
          [] -> :absent
        end

      refused ->
        raise refusal(refused)
    end
  end

  # A link is one request to the bucket as the presigner signs it, which works for a minute.
  @impl true
  def link(address, key, mode) do
    %{"method" => method, "url" => url, "headers" => headers} = signed(address, %{mode => key})
    %{method: method, url: url, headers: headers}
  end

  # A read changes nothing, so one the internet lost or held up is asked for again. The presigner and the bucket start
  # answering a read well within a second, so one they've said nothing of for this many milliseconds is taken as lost.
  @reads_within 5_000

  # Has the presigner send `ask`, a read or a list, and gives what the bucket answered. What the presigner says
  # itself, why it won't, is JSON, and is raised.
  defp answered(address, ask) do
    ask = JSON.encode!(ask)

    answer =
      sent(
        method: :post,
        url: address <> "/presigner/read",
        body: ask,
        headers: Edge.signed("/presigner/read", ask),
        retry: :transient,
        decode_body: false,
        compressed: false,
        receive_timeout: @reads_within
      )

    case Req.Response.get_header(answer, "content-type") do
      ["application/json" <> _] -> raise refused_by(address, answer.status, JSON.decode!(answer.body)["error"])
      _ -> answer
    end
  end

  # Has the presigner sign `ask`, a write or a removal, and sends what it signed.
  defp request(address, ask, data \\ nil) do
    %{"method" => method, "url" => url, "headers" => headers} = signed(address, ask)

    sent(
      method: method |> String.downcase() |> String.to_existing_atom(),
      url: url,
      headers: headers,
      body: data,
      decode_body: false,
      compressed: false,
      receive_timeout: :timer.seconds(15)
    )
  end

  # A sign changes nothing, so one the internet lost or held up is asked for again, as Req asks a read again. The
  # presigner answers well within a second, so one that hasn't come in this many milliseconds is taken as lost.
  @signs_within 5_000

  defp signed(address, ask) do
    ask = JSON.encode!(ask)
    url = address <> "/presigner/sign"
    headers = Edge.signed("/presigner/sign", ask)

    case sent(method: :post, url: url, body: ask, headers: headers, retry: :transient, receive_timeout: @signs_within) do
      %{status: 200, body: %{"method" => _method} = signed} -> signed
      %{status: status, body: %{"error" => why}} -> raise refused_by(address, status, why)
      refused -> raise refusal(refused)
    end
  end

  # What's raised when the presigner at `address` won't: `OneCore.Bucket.Refused` for a server on no list that may.
  defp refused_by(address, 403, why),
    do: %OneCore.Bucket.Refused{message: "The presigner at #{address} refused: #{why}"}

  defp refused_by(address, _status, why), do: %RuntimeError{message: "The presigner at #{address} refused: #{why}"}

  # At most this many requests to the presigner, and as many to the bucket, are sent at once, and the rest wait their
  # turn, however long. A server that starts asks for hundreds at once, and an uplink at home that's sent them all
  # stalls every one of them past its timeout. Each one's timeout runs from when it's sent.
  @at_once 16

  defp sent(request), do: Edge.send!(request ++ [finch: [size: @at_once, pool_timeout: :infinity]])

  defp tag(got), do: got |> Req.Response.get_header("etag") |> hd()

  defp refusal(%{status: status, body: body}), do: "The bucket answered #{status}: #{inspect(body)}"
end
