defmodule OneCore.Address do
  @moduledoc """
  A server at its network's address: it signs what it asks there with its key, and shows a pass that says why it's
  one of the network's own. The pieces at the address check both (`address/ask.js`, `address/signer/signer.js`).
  """
  alias OneCore.{Iroh, Server}

  # The list this server is on. With no plugin that signs users in there is one user, whom every server acts for.
  @list "users/anyone/"

  @doc "The headers of an ask of `path` there, with `body`: this server's signature of it, and its pass."
  def signed(path, body) do
    at = Integer.to_string(System.system_time(:millisecond))
    signature = sign("one/ask/1\n#{path}\n#{at}\n#{body}")
    [{"one-key", key()}, {"one-at", at}, {"one-signature", signature}, {"one-pass", pass()}]
  end

  @doc "This server's pass: the list of servers it's on."
  def pass, do: encode(%{list: @list})

  @doc "A pass for the client whose key is `client`, until `until` in milliseconds: this server's word for it."
  def pass(client, until) do
    encode(%{list: @list, by: key(), until: until, signature: sign("one/pass/1\n#{client}\n#{@list}\n#{until}")})
  end

  defp encode(pass), do: Base.url_encode64(JSON.encode!(pass), padding: false)

  defp key do
    {:ok, key} = Iroh.key(Server.key_file())
    key
  end

  defp sign(message) do
    {:ok, signature} = Iroh.sign(Server.key_file(), message)
    signature
  end
end
