defmodule OneCore.Edge do
  @moduledoc """
  A server at its network's edge: it signs what it asks there with its key, and shows a pass that says why it's
  one of the network's own. The pieces of the edge check both (`edge/ask.js`, `edge/signer/signer.js`).
  """
  alias OneCore.{Iroh, Server}

  defmodule Unreachable do
    @moduledoc "What's raised when the network's edge, or what it hands out a link to, couldn't be reached or didn't answer."
    defexception [:message]
  end

  # The list this server is on: its owner's, whom it was started for.
  defp list, do: Application.get_env(:one_core, :for, "network") <> "/"

  @doc """
  Sends `request`, as `Req.request!/1` takes it, to the network's edge or to what it hands out a link to, and
  gives the answer. Raises `Unreachable` with a sentence for whoever asked if it couldn't be reached
  or didn't answer, which isn't the fault of whatever asked.
  """
  def send!(request) do
    Req.request!(request)
  rescue
    error in Req.TransportError ->
      host = URI.parse(to_string(request[:url])).host

      how =
        case error.reason do
          :timeout -> "didn't answer in time"
          :closed -> "closed the connection partway"
          :econnrefused -> "refused the connection"
          :nxdomain -> "isn't a name the internet knows"
          _ -> "couldn't be reached (#{Exception.message(error)})"
        end

      reraise Unreachable,
              "#{host} #{how}, so this server couldn't do what was asked. Check this server is online, then try again.",
              __STACKTRACE__
  end

  @doc "The headers of an ask of `path` there, with `body`: this server's signature of it, and its pass."
  def signed(path, body) do
    at = Integer.to_string(System.system_time(:millisecond))
    signature = sign("one/ask/1\n#{path}\n#{at}\n#{body}")
    [{"one-key", key()}, {"one-at", at}, {"one-signature", signature}, {"one-pass", pass()}]
  end

  @doc """
  This server's pass: the list of servers it's on. A network's first server shows the secret its edge was
  deployed with too, the server's `:first` setting, which is what lets it write the network's first list.
  """
  def pass do
    case Application.get_env(:one_core, :first) do
      nil -> encode(%{list: list()})
      first -> encode(%{list: list(), first: first})
    end
  end

  # How long a client's pass lasts, in milliseconds. A client asks a server for the next half an hour before it
  # ends (`RENEW` in `client/src/lib.rs`), and one that's taken out of its network is given none: so it uses the
  # relay for an hour at most after that.
  @lasts :timer.hours(1)

  @doc "A pass for the client whose key is `client`: this server's word for it, for an hour."
  def pass(client) do
    until = System.system_time(:millisecond) + @lasts
    encode(%{list: list(), by: key(), until: until, signature: sign("one/pass/1\n#{client}\n#{list()}\n#{until}")})
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
