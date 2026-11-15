defmodule Fake do
  @moduledoc """
  An app that signs its person in to an outside service, for the Vault's proof (`plugins/vault/test/vault.sh`): all it
  says of the service is the service's own, its addresses, scopes and sites, and the Vault does the rest. The
  service is the fake one the proof runs on this machine (`vault/service.js`), at the port written here as PORT.
  """
  @behaviour OneCore.Plugin

  @service %{
    name: "Fake",
    authorize: "http://127.0.0.1:PORT/authorize",
    token: "http://127.0.0.1:PORT/token",
    scopes: ["me.read"],
    hosts: ["127.0.0.1:PORT"],
    client: "Fake client"
  }

  @impl true
  def uses, do: ["vault"]

  @impl true
  def tools do
    item = %{
      type: "object",
      properties: %{item: %{type: "string"}, field: %{type: "string"}},
      required: ["item", "field"]
    }

    [
      %{name: "sign_in", description: "Gives the link you sign in to Fake at", inputs: %{type: "object"}},
      %{name: "me", description: "Asks Fake who you are there", inputs: %{type: "object"}},
      %{
        name: "keep",
        description: "Makes a secret of its own and keeps it in an item, and gives its SHA-256",
        inputs: %{type: "object", properties: %{item: %{type: "string"}}, required: ["item"]}
      },
      %{
        name: "forget",
        description: "Removes an item it keeps",
        inputs: %{type: "object", properties: %{item: %{type: "string"}}, required: ["item"]}
      },
      %{
        name: "digest",
        description: "Hands an item's field to a program, and gives the program's answer: its SHA-256",
        inputs: item
      }
    ]
  end

  @impl true
  def call("sign_in", _inputs, call), do: OneCore.Call.plugin(call, "vault").sign_in(call, @service)

  def call("me", _inputs, call) do
    request = %{url: "http://127.0.0.1:PORT/me", headers: %{"authorization" => "Bearer {{access_token}}"}}
    answer = OneCore.Call.plugin(call, "vault").fetch(call, "Fake", request)
    if answer.status != 200, do: raise("Fake answered #{answer.status}")
    "Fake says you're #{JSON.decode!(answer.body)["account"]}."
  end

  def call("keep", %{"item" => item}, call) do
    secret = Base.encode64(:crypto.strong_rand_bytes(32))
    OneCore.Call.plugin(call, "vault").keep(call, item, %{"key" => secret})
    Base.encode16(:crypto.hash(:sha256, secret), case: :lower)
  end

  def call("forget", %{"item" => item}, call) do
    OneCore.Call.plugin(call, "vault").remove(call, item)
    "Removed."
  end

  # The secret goes to the program on its input, which is how a service hands one to a program it brings. The
  # program is told how much to read, since a port's input can't be closed to say it's done.
  def call("digest", %{"item" => item, "field" => field}, call) do
    secret = OneCore.Call.plugin(call, "vault").value(call, item, field)
    program = Port.open({:spawn, "sh -c 'head -c #{byte_size(secret)} | sha256sum'"}, [:binary])
    Port.command(program, secret)

    receive do
      {^program, {:data, digest}} -> hd(String.split(digest))
    after
      5_000 -> raise "The program didn't answer."
    end
  end
end
