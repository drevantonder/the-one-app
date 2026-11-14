defmodule OneCore.Bucket.S3 do
  @moduledoc """
  A bucket that speaks S3, such as Cloudflare R2, used with no credentials of its own. For each request the server
  asks the signer at the network's address, which signs that one request if this server is on the list of whoever
  the blob belongs to, and the server sends what was signed. The address is `where`.

  The signer signs a write only with its condition, so the bucket itself refuses one that would overwrite. A blob's
  tag is its ETag.
  """
  @behaviour OneCore.Bucket
  require Record
  alias OneCore.Address

  Record.defrecordp(:xml_element, :xmlElement, Record.extract(:xmlElement, from_lib: "xmerl/include/xmerl.hrl"))
  Record.defrecordp(:xml_text, :xmlText, Record.extract(:xmlText, from_lib: "xmerl/include/xmerl.hrl"))

  @impl true
  def read(address, key) do
    case request(address, %{read: key}) do
      %{status: 200, body: data} = got -> {:ok, data, tag(got)}
      %{status: 404} -> :absent
      refused -> raise refusal(refused)
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
    case request(address, %{list: prefix, after: next}) do
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

  defp texts(xml, path) do
    for element <- :xmerl_xpath.string(path, xml) do
      for xml_text(value: text) <- xml_element(element, :content), into: "", do: List.to_string(text)
    end
  end

  # Has the signer sign `ask`, and sends what it signed.
  defp request(address, ask, data \\ nil) do
    ask = JSON.encode!(ask)
    signed = Req.post!(address <> "/signer/sign", body: ask, headers: Address.signed("/signer/sign", ask))

    case signed do
      %{status: 200, body: %{"method" => method, "url" => url, "headers" => headers}} ->
        Req.request!(method: method, url: url, headers: headers, body: data, decode_body: false, compressed: false)

      %{body: %{"error" => why}} ->
        raise "The signer at #{address} refused: #{why}"

      refused ->
        raise refusal(refused)
    end
  end

  defp tag(got), do: got |> Req.Response.get_header("etag") |> hd()

  defp refusal(%{status: status, body: body}), do: "The bucket answered #{status}: #{inspect(body)}"
end
