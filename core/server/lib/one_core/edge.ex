defmodule OneCore.Edge do
  @moduledoc """
  A server at its network's edge: it signs what it asks there with its key, and shows a token that says why it's
  one of the network's own. The pieces of the edge check both (`core/edge/ask.ts`, `core/edge/presigner/presigner.ts`).
  """
  alias OneCore.{Cache, Iroh, Server}
  alias OneCore.Edge.Unreachable

  # The list this server is on: its owner's, whom it was started for.
  defp list, do: Application.get_env(:one_core, :owner, "network") <> "/"

  # A connection kept open with nothing sent on it for this many milliseconds is closed rather than used again
  # (Finch's `conn_max_idle_time`), as AWS's SDKs close theirs after a minute. Cloudflare, the edge's and R2's, closes
  # one that's idle for somewhere between 200 and 400 seconds (18 Nov), and a request sent as it does meets the close
  # ("closed the connection partway"). What's between a server and Cloudflare may forget an idle connection without
  # saying so, as a home router's NAT, or a laptop that slept or changed networks does, and a request sent on it is
  # answered by nothing ("didn't answer in time"). A new connection would have been answered at once, so one idle
  # this long costs a handshake rather than a failed try.
  @pool [conn_max_idle_time: 60_000]

  @doc """
  Sends `request`, as `Req.request!/1` takes it, to the network's edge or to what it hands out a link to, and
  gives the answer. Raises `Unreachable` with a sentence for whoever asked if it couldn't be reached
  or didn't answer, which isn't the fault of whatever asked. A request Req sends again (`retry`) says so only in
  this server's own log, at `info`: one answered at a later try worked, and one that failed every time raises,
  which is the error.
  """
  def send!(request) do
    request
    |> Keyword.update(:finch, @pool, &Keyword.merge(@pool, &1))
    |> Keyword.put(:retry_log_level, :info)
    |> Req.request!()
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

  @doc """
  Sends `request` to `path` at the network's edge, signed by this server (`signed/2`), as `send!/1` does. One the
  edge refuses for this server's clock (`RequestTimeTooSkewed`, as S3 says) is signed again by the edge's clock, its
  `Date`, and sent once more, as AWS's SDKs do: the server keeps how far its clock is from the edge's for every ask
  after.
  """
  def ask!(request, path) do
    ask = fn -> send!(Keyword.put(request, :headers, signed(path, request[:body] || ""))) end
    response = ask.()
    if skewed?(response), do: ask.(), else: response
  end

  # Seconds from year 0 to 1970, by which Erlang's calendar counts.
  @unix_epoch 62_167_219_200

  # Whether the edge refused `response`'s ask for this server's clock, and if so, keeps how far that is from the
  # edge's. Its `Date` is to the second, so the edge's time is taken as the middle of that second.
  defp skewed?(%{status: 401} = response) do
    with "RequestTimeTooSkewed" <- code(response.body),
         [date] <- Req.Response.get_header(response, "date"),
         {{_, _, _}, {_, _, _}} = time <- :httpd_util.convert_request_date(String.to_charlist(date)) do
      edge = (:calendar.datetime_to_gregorian_seconds(time) - @unix_epoch) * 1000 + 500
      :persistent_term.put({__MODULE__, :skew}, edge - System.system_time(:millisecond))
      true
    else
      _ -> false
    end
  end

  defp skewed?(_response), do: false

  defp code(%{"code" => code}), do: code

  defp code(body) when is_binary(body) do
    case JSON.decode(body) do
      {:ok, %{"code" => code}} -> code
      _ -> nil
    end
  end

  defp code(_body), do: nil

  # The time by the edge's clock, in milliseconds: this server's, as far from it as the edge last said (`ask!/2`).
  defp now, do: System.system_time(:millisecond) + :persistent_term.get({__MODULE__, :skew}, 0)

  @doc """
  The headers of an ask of `path` there, with `body`: this server's signature of it, its token, and the tokens it
  keeps of other servers' word that it acts for someone too (`token/2`).
  """
  def signed(path, body) do
    at = Integer.to_string(now())
    signature = sign("one/ask/1\n#{path}\n#{at}\n#{body}")
    tokens = for {:token, _who} = kept <- Cache.keys(__MODULE__), token = Cache.fetch(__MODULE__, kept, fn -> {:ignore, nil} end), do: token
    [{"one-key", key()}, {"one-at", at}, {"one-signature", signature}, {"one-token", token()}, {"one-tokens", Enum.join(Enum.reject(tokens, &is_nil/1), " ")}]
  end

  @doc """
  This server's token: the list of servers it's on. A network's first server shows the secret its edge was
  deployed with too, the server's `:first` setting, which is what lets it write the network's first list.
  """
  def token do
    case Application.get_env(:one_core, :first) do
      nil -> encode(%{list: list()})
      first -> encode(%{list: list(), first: first})
    end
  end

  # How long a client's token lasts, in milliseconds. A client asks a server for the next half an hour before it
  # ends (`RENEW` in `core/client/src/lib.rs`), and one that's taken out of its network is given none: so it uses the
  # relay for an hour at most after that.
  @lasts :timer.hours(1)

  @doc "A token for the client whose key is `client`: this server's word for it, for an hour."
  def token(client) do
    until = now() + @lasts
    encode(%{list: list(), by: key(), until: until, signature: sign("one/token/1\n#{client}\n#{list()}\n#{until}")})
  end

  # How long one server's word that another acts for someone lasts, in milliseconds: so whoever's taken out of a
  # group is acted for by its servers for that long at most after.
  @vouched :timer.minutes(5)

  @doc """
  A token that the server whose key is `server` acts for `who`, a user, too: this server's word, which it gives as one
  of the network's, which act for everyone, to a group's server for each of the group's members
  (`OneCore.Apps.acts_for?/2`), as it gives a client its token, for five minutes.
  """
  def token(server, who) do
    until = now() + @vouched
    list = who <> "/"
    encode(%{list: list, by: key(), until: until, signature: sign("one/token/1\n#{server}\n#{list}\n#{until}")})
  end

  @doc """
  The token another server gave that this one acts for `who` too (`token/2`), as `fetch` gets it, `{:ok, token}`,
  kept until a minute before it ends, and shown with every ask meanwhile (`signed/2`). `nil` where it was refused.
  """
  def vouched(who, fetch) do
    Cache.fetch(__MODULE__, {:token, who}, fn ->
      case fetch.() do
        {:ok, token} -> {:commit, token, expire: @vouched - :timer.minutes(1)}
        _refused -> {:ignore, nil}
      end
    end)
  end

  defp encode(token), do: Base.url_encode64(JSON.encode!(token), padding: false)

  defp key do
    {:ok, key} = Iroh.key(Server.key_file())
    key
  end

  defp sign(message) do
    {:ok, signature} = Iroh.sign(Server.key_file(), message)
    signature
  end
end
