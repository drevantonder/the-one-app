defmodule OneCore.VaultTest do
  @moduledoc """
  What encryption and the Vault promise, as far as one server shows it: what's typed is nowhere but encrypted, a
  server opens something only while its owner has it chosen, nobody but the owner hands its keys over, and a
  plugin uses an item only while it's switched on for it. `test/vault.sh` shows the same across servers and
  devices, with a real client.

  The tests play alice's device, which is the only thing that holds her key: they make an artifact's keys
  themselves and hand them over encrypted to the server, as her device would.
  """
  use ExUnit.Case
  # The encryption plugin's NIF, which is there once the plugin is deployed.
  @compile {:no_warn_undefined, Cipher}
  import ExUnit.CaptureLog
  alias OneCore.Bucket
  alias OneCore.Plugins.{Calls, Deploy}

  @alice "users/alice"
  @vault "users/alice/made/vault/items"
  # What every secret here starts with, to search for.
  @marker "vault-test-marker"

  # A plugin that's handed an item's secret, and says only what a program would make of it.
  @reader """
  defmodule Reader do
    @behaviour OneCore.Plugin

    @impl true
    def uses, do: ["vault"]

    @impl true
    def tools do
      for {name, description} <- [digest: "The SHA-256 of an item's field", items: "Its items", stop: "Stops using one"],
          do: %{name: "\#{name}", description: description, inputs: %{type: "object"}}
    end

    @impl true
    def call("digest", %{"item" => item, "field" => field}, call) do
      secret = OneCore.Call.plugin(call, "vault").value(call, item, field)
      Base.encode16(:crypto.hash(:sha256, secret), case: :lower)
    end

    def call("items", _inputs, call), do: OneCore.Call.plugin(call, "vault").items(call)
    def call("stop", %{"item" => item}, call), do: OneCore.Call.plugin(call, "vault").stop_using(call, item)
  end
  """

  defp call(name, inputs, who) do
    [plugin, tool] = String.split(name, "_", parts: 2)
    Calls.call(plugin, tool, inputs, who)
  end

  defp sealed(server, about, data) do
    {:ok, sealed} = Cipher.seal(server, about, data)
    Base.encode64(sealed)
  end

  # What a device fills in to choose the servers that open an artifact: its keys for every epoch so far, encrypted
  # to each server chosen.
  defp chosen(servers, keys, was) do
    slots = Map.new(servers, &{&1, sealed(&1, "one/encryption/1/keys\n#{@vault}\n#{&1}", Enum.join(keys))})
    JSON.encode!(%{artifact: @vault, epoch: length(keys), was: was, slots: slots})
  end

  # What a device makes of what its person types: encrypted for the server that opens what it's kept in.
  defp typed(server, text), do: JSON.encode!(%{server => sealed(server, "one/encryption/1/secret\n#{server}", text)})

  defp grant(servers, keys, was, by \\ @alice) do
    call("vault_grant", %{"servers" => servers, "keys" => chosen(servers, keys, was)}, by)
  end

  # Everything the bucket holds of alice's Vault, as whoever runs the bucket reads it.
  defp in_bucket(folder) do
    files = Path.wildcard(Path.join(folder, "bucket/#{@vault}/**"))
    Enum.map_join(Enum.filter(files, &File.regular?/1), &File.read!/1)
  end

  setup %{tmp_dir: folder} do
    server = OneCore.Test.server(folder)
    assert {:ok, 1, ""} = Deploy.deploy("network", "ui", OneCore.Test.ui("export const ui = 1"))

    for name <- ~w(database relations encryption vault),
        do: assert({:ok, 1, ""} = Deploy.deploy("network", name, OneCore.Test.shipped(name)))

    # The network's first owner was made before anything kept who may do what, which is told of them now.
    assert :ok = OneCore.User.make_first_owner(OneCore.Test.owner())
    %{server: server, keys: [:crypto.strong_rand_bytes(32)]}
  end

  @tag :tmp_dir
  test "what's typed is nowhere but encrypted: not in the bucket, in what a tool answers, in what one raises or in the log",
       %{tmp_dir: folder, server: server, keys: keys} do
    assert {:ok, _} = grant([server], keys, 0)
    secret = &"#{@marker}-#{&1}"

    {said, logged} =
      with_log(fn ->
        [
          call(
            "vault_set",
            %{"item" => "GitHub", "field" => "token", "value" => typed(server, secret.("token"))},
            @alice
          ),
          call(
            "vault_set",
            %{"item" => "GitHub", "field" => "user", "value" => typed(server, "octocat"), "secret" => false},
            @alice
          ),
          # An assistant somewhere else has no device to encrypt on, and what it sends as it is isn't kept.
          call("vault_set", %{"item" => "GitHub", "field" => "pasted", "value" => secret.("pasted")}, @alice),
          call(
            "vault_request",
            %{"title" => "Stripe", "fields" => %{"key" => %{"secret" => true}, "account" => %{}}},
            @alice
          ),
          call("vault_list", %{}, @alice)
        ]
      end)

    assert [{:ok, _}, {:ok, _}, {:error, refused}, {:ok, %{id: request}}, {:ok, %{items: [github], requests: [_]}}] =
             said

    assert refused =~ "wasn't encrypted on a device"
    assert github.fields == [%{name: "token", secret: true}, %{name: "user", secret: false, value: "octocat"}]

    {answered, logged_more} =
      with_log(fn ->
        [
          # An answer that leaves a field out raises, and says which, not what the others were.
          call(
            "vault_answer",
            %{"id" => request, "values" => typed(server, JSON.encode!(%{key: secret.("key")}))},
            @alice
          ),
          # One that isn't the JSON a request's answer is raises too.
          call("vault_answer", %{"id" => request, "values" => typed(server, secret.("not json"))}, @alice),
          call(
            "vault_answer",
            %{
              "id" => request,
              "values" => typed(server, JSON.encode!(%{key: secret.("key"), account: secret.("account")}))
            },
            @alice
          ),
          call("vault_status", %{"id" => request}, @alice),
          call("vault_list", %{}, @alice),
          call("vault_fetch", %{"item" => "Stripe", "url" => "https://nowhere.example/{{key}}"}, @alice),
          call(
            "vault_fetch",
            %{"item" => "GitHub", "url" => "https://nowhere.example/", "headers" => %{"x" => "{{token}}"}},
            @alice
          )
        ]
      end)

    assert [{:error, _}, {:error, _}, {:ok, "Allowed."}, {:ok, %{state: "set"}}, {:ok, _}, {:error, _}, {:error, _}] =
             answered

    for what <- [said, answered, logged, logged_more, in_bucket(folder)],
        do: refute(inspect(what, limit: :infinity) =~ @marker)

    # The bucket holds none of what isn't secret either: all of the database is encrypted.
    refute in_bucket(folder) =~ "octocat"
    assert {:ok, entry, _tag} = Bucket.read(@vault <> "/log/0000000000000002")
    assert_raise ArgumentError, fn -> :erlang.binary_to_term(entry, [:safe]) end
  end

  @tag :tmp_dir
  test "a server opens it only while its owner has it chosen, and a relation tuple alone hands it no key",
       %{server: server, keys: [first] = keys} do
    refused = "no server may open it yet"
    assert {:error, why} = call("vault_list", %{}, @alice)
    assert why =~ refused

    # The network's owner writes the tuple that names the server, as its owner may write any, and hands no key
    # over: only alice hands hers, whoever asks for her.
    tuple = %{"artifact" => @vault, "relation" => "editor", "who" => "servers/" <> server}
    assert {:ok, _} = call("relations_share", tuple, OneCore.Test.owner())
    assert {:error, why} = grant([server], keys, 0, {OneCore.Test.owner(), @alice})
    assert why =~ "theirs alone"
    assert {:error, why} = call("vault_list", %{}, {OneCore.Test.owner(), @alice})
    assert why =~ "theirs alone"
    assert {:error, why} = call("vault_list", %{}, @alice)
    assert why =~ refused
    # It took no lease of the database it couldn't open: nothing was written to its log.
    assert Bucket.list(@vault <> "/log/") == []

    assert {:ok, _} = grant([server], keys, 0)

    assert {:ok, _} =
             call("vault_set", %{"item" => "GitHub", "field" => "token", "value" => typed(server, "one")}, @alice)

    assert {:ok, %{items: [%{name: "GitHub"}]}} = call("vault_list", %{}, @alice)

    # Taken off, it opens nothing more, and the tuple that named it is gone: a new epoch began without it.
    second = keys ++ [:crypto.strong_rand_bytes(32)]
    assert {:ok, "No server opens your Vault now."} = grant([], second, 1)
    assert {:error, why} = call("vault_list", %{}, @alice)
    assert why =~ refused
    assert {:ok, []} = call("relations_shared", %{"artifact" => @vault}, @alice)

    # Chosen again, it's handed both epochs' keys, and reads what was written under the first.
    assert {:ok, _} = grant([server], second, 2)
    assert {:ok, %{items: [%{name: "GitHub"}]}} = call("vault_list", %{}, @alice)
    # With keys that aren't the ones it was written under, it reads nothing.
    assert {:ok, _} = grant([], second ++ [first], 2)
    assert {:ok, _} = grant([server], [:crypto.strong_rand_bytes(32), :crypto.strong_rand_bytes(32), first], 3)
    assert {:error, _} = call("vault_list", %{}, @alice)
  end

  @tag :tmp_dir
  test "a plugin is handed an item's secret, and lists it, only while it's switched on for that item",
       %{server: server, keys: keys} do
    assert {:ok, 1, ""} = Deploy.deploy("network", "reader", %{"server/reader.ex" => @reader})
    assert {:ok, _} = grant([server], keys, 0)
    secret = "#{@marker}-token"

    for item <- ["GitHub", "Other"],
        do:
          assert(
            {:ok, _} =
              call("vault_set", %{"item" => item, "field" => "token", "value" => typed(server, secret)}, @alice)
          )

    digest = %{"item" => "GitHub", "field" => "token"}

    assert {:error, why} = call("reader_digest", digest, @alice)
    assert why =~ "isn't an editor of #{@vault}"
    assert {:ok, []} = call("reader_items", %{}, @alice)
    assert {:ok, _} = call("vault_use", %{"item" => "GitHub", "plugin" => "reader"}, @alice)
    assert {:ok, [%{name: "GitHub", sign_in: nil}]} = call("reader_items", %{}, @alice)
    assert call("reader_digest", digest, @alice) == {:ok, Base.encode16(:crypto.hash(:sha256, secret), case: :lower)}
    assert {:error, why} = call("reader_digest", %{digest | "item" => "Other"}, @alice)
    assert why =~ "Other isn't switched on for reader"

    assert {:ok, %{items: [%{plugins: ["reader"]}, %{plugins: []}], plugins: ["reader"]}} =
             call("vault_list", %{}, @alice)

    assert {:ok, _} = call("reader_stop", %{"item" => "GitHub"}, @alice)
    assert {:error, _} = call("reader_digest", digest, @alice)
    assert {:ok, []} = call("reader_items", %{}, @alice)
    assert {:ok, %{items: [%{plugins: []}, %{plugins: []}]}} = call("vault_list", %{}, @alice)
    # Agents use one only once its person said so.
    assert {:error, why} = call("vault_fetch", %{"item" => "GitHub", "url" => "https://nowhere.example/"}, @alice)
    assert why =~ "isn't one agents may use"
  end
end
