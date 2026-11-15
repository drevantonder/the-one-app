defmodule Backups.Place do
  @moduledoc """
  Where backups go, reached by name: a folder on a machine of the network's, through the service `Backups.Disk`
  there, or a bucket of S3-compatible storage, signed with the keys in the network's Vault item that it names.
  """
  alias Backups.Disk
  alias OneCore.Service

  @doc "A place ready to reach: a folder's call to its machine, or a bucket's keys, read from the Vault once."
  def open(call, %{"kind" => "folder"} = place), do: %{place: place, call: call}

  def open(call, %{"kind" => "s3", "item" => item} = place) do
    vault = OneCore.Call.plugin(call, "vault")

    sign = [
      access_key_id: vault.value(call, item, "access_key_id"),
      secret_access_key: vault.value(call, item, "secret_access_key"),
      service: :s3,
      region: place["region"]
    ]

    %{place: place, call: call, sign: sign}
  end

  @doc "Tries a place before it's added: writes something there, reads it back and removes it."
  def try(call, place) do
    opened = open(call, place)
    tried = "tried-" <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
    put(opened, tried, "tried")
    if get(opened, tried) != "tried", do: raise("What was written to #{Backups.where(place)} didn't read back.")
    remove(opened, tried)
  rescue
    failed -> reraise "#{Backups.where(place)} didn't work: #{Exception.message(failed)}", __STACKTRACE__
  end

  def put(%{place: %{"kind" => "folder"} = place, call: call}, name, data),
    do: Service.call(call, Disk, :put, [place["folder"], name, Base.encode64(data)], place["machine"])

  def put(opened, name, data), do: s3(opened, :put, name, body: data) |> ok(name)

  @doc "What's at `name`, or `nil`."
  def get(%{place: %{"kind" => "folder"} = place, call: call}, name) do
    with data when is_binary(data) <- Service.call(call, Disk, :get, [place["folder"], name], place["machine"]),
         do: Base.decode64!(data)
  end

  def get(opened, name) do
    case s3(opened, :get, name) do
      %{status: 200, body: data} -> data
      %{status: 404} -> nil
      answered -> ok(answered, name)
    end
  end

  def remove(%{place: %{"kind" => "folder"} = place, call: call}, name),
    do: Service.call(call, Disk, :remove, [place["folder"], name], place["machine"])

  def remove(opened, name), do: s3(opened, :delete, name) |> ok(name)

  @doc "The names starting with `prefix`, in order."
  def list(%{place: %{"kind" => "folder"} = place, call: call}, prefix),
    do: Service.call(call, Disk, :list, [place["folder"], prefix], place["machine"])

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

      answered ->
        ok(answered, prefix)
    end
  end

  defp texts(xml, path) do
    for element <- :xmerl_xpath.string(path, xml) do
      {:xmlElement, _, _, _, _, _, _, _, content, _, _, _} = element
      for {:xmlText, _, _, _, text, _} <- content, into: "", do: List.to_string(text)
    end
  end

  # A request to the bucket, for one of its objects or, with no name, the bucket itself.
  defp s3(%{place: place, sign: sign} = opened, method, name, options \\ []) do
    [bucket | _] = String.split(place["bucket"], "/")

    path =
      if name,
        do: "/" <> Enum.map_join(String.split(folder(opened) <> name, "/"), "/", &URI.encode_www_form/1),
        else: ""

    Req.request!(
      [method: method, url: "#{place["endpoint"]}/#{bucket}#{path}", aws_sigv4: sign, decode_body: false] ++ options
    )
  end

  # The folder in the bucket the place is, as the start of its objects' names.
  defp folder(%{place: place}) do
    case String.split(place["bucket"], "/", parts: 2) do
      [_bucket, folder] -> String.trim(folder, "/") <> "/"
      [_bucket] -> ""
    end
  end

  defp ok(%{status: status}, _name) when status in 200..299, do: :ok

  defp ok(%{status: status, body: body}, name),
    do: raise("the storage answered #{status} for #{name}: #{String.slice(body, 0, 200)}")
end
