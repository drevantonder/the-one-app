defmodule Backups.Destination do
  @moduledoc """
  Where backups go, reached by name: a folder on a machine of the network's, through the service `Backups.Disk`
  there, or a bucket of S3-compatible storage, signed with the keys in the network's Vault item that it names.
  """
  alias Backups.Disk
  alias OneCore.Service

  @doc "A destination ready to reach: a folder's call to its machine, or a bucket's keys, read from the Vault once."
  def open(call, %{"kind" => "folder"} = destination), do: %{destination: destination, call: call}

  def open(call, %{"kind" => "s3", "item" => item} = destination) do
    vault = OneCore.Call.app(call, "vault")

    sign = [
      access_key_id: vault.value(call, item, "access_key_id"),
      secret_access_key: vault.value(call, item, "secret_access_key"),
      service: :s3,
      region: destination["region"]
    ]

    %{destination: destination, call: call, sign: sign}
  end

  @doc "Tries a destination before it's added: writes something there, reads it back and removes it."
  def try(call, destination) do
    opened = open(call, destination)
    tried = "tried-" <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
    put(opened, tried, "tried")
    if get(opened, tried) != "tried", do: raise("What was written to #{Backups.where(destination)} didn't read back.")
    remove(opened, tried)
  rescue
    failed -> reraise "#{Backups.where(destination)} didn't work: #{Exception.message(failed)}", __STACKTRACE__
  end

  def put(%{destination: %{"kind" => "folder"} = destination, call: call}, name, data),
    do: Service.call(call, Disk, :put, [destination["folder"], name, Base.encode64(data)], destination["machine"])

  def put(opened, name, data), do: s3(opened, :put, name, body: data) |> ok(name)

  @doc "What's at `name`, or `nil`."
  def get(%{destination: %{"kind" => "folder"} = destination, call: call}, name) do
    with data when is_binary(data) <- Service.call(call, Disk, :get, [destination["folder"], name], destination["machine"]),
         do: Base.decode64!(data)
  end

  def get(opened, name) do
    case s3(opened, :get, name) do
      %{status: 200, body: data} -> data
      %{status: 404} -> nil
      result -> ok(result, name)
    end
  end

  def remove(%{destination: %{"kind" => "folder"} = destination, call: call}, name),
    do: Service.call(call, Disk, :remove, [destination["folder"], name], destination["machine"])

  def remove(opened, name), do: s3(opened, :delete, name) |> ok(name)

  @doc "The names starting with `prefix`, in order."
  def list(%{destination: %{"kind" => "folder"} = destination, call: call}, prefix),
    do: Service.call(call, Disk, :list, [destination["folder"], prefix], destination["machine"])

  def list(opened, prefix, next \\ nil) do
    params =
      [{"list-type", "2"}, {"prefix", folder(opened) <> prefix}] ++
        if(next, do: [{"continuation-token", next}], else: [])

    case s3(opened, :get, nil, params: params) do
      %{status: 200, body: listed} ->
        {listed, _} = :xmerl_scan.string(:binary.bin_to_list(listed))

        names =
          for name <- texts(listed, ~c"//*[local-name()='Contents']/*[local-name()='Key']"),
              do: String.replace_prefix(name, folder(opened), "")

        case texts(listed, ~c"//*[local-name()='NextContinuationToken']") do
          [next] -> names ++ list(opened, prefix, next)
          [] -> names
        end

      result ->
        ok(result, prefix)
    end
  end

  defp texts(xml, path) do
    for element <- :xmerl_xpath.string(path, xml) do
      {:xmlElement, _, _, _, _, _, _, _, content, _, _, _} = element
      for {:xmlText, _, _, _, text, _} <- content, into: "", do: List.to_string(text)
    end
  end

  # A request to the bucket, for one of its objects or, with no name, the bucket itself.
  defp s3(%{destination: destination, sign: sign} = opened, method, name, options \\ []) do
    [bucket | _] = String.split(destination["bucket"], "/")

    path =
      if name,
        do: "/" <> Enum.map_join(String.split(folder(opened) <> name, "/"), "/", &URI.encode_www_form/1),
        else: ""

    # A big object sent while the machine's upload is busy takes longer than Req's 15 s to be answered.
    Req.request!(
      [method: method, url: "#{destination["endpoint"]}/#{bucket}#{path}", aws_sigv4: sign, decode_body: false, receive_timeout: 120_000] ++
        options
    )
  end

  # The folder in the bucket the destination is, as the start of its objects' names.
  defp folder(%{destination: destination}) do
    case String.split(destination["bucket"], "/", parts: 2) do
      [_bucket, folder] -> String.trim(folder, "/") <> "/"
      [_bucket] -> ""
    end
  end

  defp ok(%{status: status}, _name) when status in 200..299, do: :ok

  defp ok(%{status: status, body: body}, name),
    do: raise("the storage answered #{status} for #{name}: #{String.slice(body, 0, 200)}")
end
