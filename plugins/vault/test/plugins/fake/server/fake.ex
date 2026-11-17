defmodule Fake do
  @moduledoc """
  An app that signs its person in to an outside service, for the Vault's test (`plugins/vault/test/vault.sh`): all it
  says of the service is the service's own, its addresses, scopes and sites, and the Vault does the rest. The
  service is the fake one the test runs on this machine (`vault/service.js`), at the port written here as PORT. It
  signs in and uses its account through the Vault's tools, as an untrusted plugin does, and is handed secrets
  themselves through its functions, as one with a service is.
  """
  @behaviour OneCore.Plugin

  @service %{
    "name" => "Fake",
    "authorize" => "http://127.0.0.1:PORT/authorize",
    "token" => "http://127.0.0.1:PORT/token",
    "scopes" => ["me.read"],
    "hosts" => ["127.0.0.1:PORT"],
    "client" => "Fake client"
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

    sha256 = %{type: "object", properties: %{sha256: %{type: "string"}}, required: ["sha256"]}

    [
      # What the Vault's sign_in answers, as its own tool says.
      %{name: "sign_in", description: "Gives the link you sign in to Fake at", inputs: %{type: "object"}, outputs: %{type: "object"}},
      %{
        name: "me",
        description: "Asks Fake who you are there",
        inputs: %{type: "object"},
        outputs: %{type: "object", properties: %{account: %{type: "string"}}, required: ["account"]}
      },
      %{
        name: "keep",
        description: "Makes a secret of its own and keeps it in an item, and gives its SHA-256",
        inputs: %{type: "object", properties: %{item: %{type: "string"}}, required: ["item"]},
        outputs: sha256
      },
      %{
        name: "forget",
        description: "Removes an item it keeps",
        inputs: %{type: "object", properties: %{item: %{type: "string"}}, required: ["item"]},
        outputs: %{type: "object"}
      },
      %{
        name: "digest",
        description: "Hands an item's field to a program, and gives the program's answer: its SHA-256",
        inputs: item,
        outputs: sha256
      }
    ]
  end

  @impl true
  def call("sign_in", _inputs, call), do: OneCore.Call.tool(call, "vault", "sign_in", %{"service" => @service})

  def call("me", _inputs, call) do
    request = %{"item" => "Fake", "url" => "http://127.0.0.1:PORT/me", "headers" => %{"authorization" => "Bearer {{access_token}}"}}
    response = OneCore.Call.tool(call, "vault", "fetch", request)
    if response["status"] != 200, do: raise("Fake answered #{response["status"]}")
    %{account: JSON.decode!(response["body"])["account"]}
  end

  def call("keep", %{"item" => item}, call) do
    secret = Base.encode64(:crypto.strong_rand_bytes(32))
    OneCore.Call.plugin(call, "vault").keep(call, item, %{"key" => secret})
    %{sha256: Base.encode16(:crypto.hash(:sha256, secret), case: :lower)}
  end

  def call("forget", %{"item" => item}, call) do
    OneCore.Call.plugin(call, "vault").remove(call, item)
    %{}
  end

  # The secret goes to the program on its input, which is how a service hands one to a program it brings. The
  # program is told how much to read, since a port's input can't be closed to say it's done.
  def call("digest", %{"item" => item, "field" => field}, call) do
    secret = OneCore.Call.plugin(call, "vault").value(call, item, field)
    program = Port.open({:spawn, "sh -c 'head -c #{byte_size(secret)} | sha256sum'"}, [:binary])
    Port.command(program, secret)

    receive do
      {^program, {:data, digest}} -> %{sha256: hd(String.split(digest))}
    after
      5_000 -> raise "The program didn't answer."
    end
  end
end
