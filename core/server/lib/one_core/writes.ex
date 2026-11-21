defmodule OneCore.Writes do
  @moduledoc """
  A write asked more than once is done once, by its idempotency key, as Stripe's API and the IETF's
  `Idempotency-Key` header have one. A client says a key in the `_meta` of a `tools/call` of a tool that doesn't only
  read, `idempotencyKey`, and says the same key each time it asks that call again, of this server or another. The
  server that runs it claims the key first, in the bucket, `<whoever it runs for>/writes/<key>`, with a write that's
  made only where there's nothing (`OneCore.Bucket.write/2`), just before its tool runs, so a server stuck before then
  holds nothing. Another server asked it again answers what the first answered, kept for a day, or says it's still
  running and where.

  A server passing a call on passes its key on (`OneCore.Call`). One that holds its claim names the server it passes
  it to (`passing/1`), which takes the claim over before it runs it, and takes it back once it gives up on that server
  (`kept/0`). Each is a replace on the condition that nothing changed it meanwhile, so of the two, only one runs it:
  so a write passed to a server that's stuck is safe to run elsewhere, as one that only reads is.

  What it answered can hold secrets, as a Vault call's answer does, so it's kept encrypted to the device that asked
  (`client::sealed`), whose key is in every request it makes: any server hands it back, and only that device opens it.
  A call from no device, as one over the web, has only that it's done kept, and its client reads again what it did.

  A key is `<milliseconds since 1970 when it was made>-<random>`, so what's past its day goes by its name alone.
  A server that claims a write and then gets stuck still holds it, as Stripe's does: it's said to be running there
  until that server comes back.

  A client asks a write again (`resent`) only of a server that speaks the protocol from which servers keep to keys,
  by saying it speaks only that one, and a server runs it only where every server that could have been asked it
  first speaks it too: one from before would have run it without claiming it.
  """
  alias OneCore.{Bucket, Cache, Iroh, Protocol, Server, Servers, User}
  alias OneCore.Call.State

  # The protocols that keep to idempotency keys (`core/protocol.json`).
  @keys %{"oldest" => 2, "newest" => 2}

  # How long what a write answered is kept, in milliseconds.
  @kept :timer.hours(24)

  @doc """
  The JSON-RPC error a write asked again is answered where it isn't run there: it's running elsewhere, or isn't to be
  asked again. A client or a server that asked it of another too waits for the first (`core/client/src/rpc.rs`).
  """
  def not_run, do: -32004

  @doc "Whether the server whose key is `server` keeps to idempotency keys, so a write is safe to ask of it again."
  def keeps?(server), do: Protocol.overlap?(Servers.protocol(server), @keys)

  @doc """
  The write a `tools/call` for `subject` is, from what its request says (`meta`), the machine that asked (`from`,
  `nil` for the edge) and the device it was for (`client`, `OneCore.Call.client/1`): `nil` where it says no key.
  `claim` names this request's claim, which is the one it answers.
  """
  def asked(%{"idempotencyKey" => key} = meta, from, client, subject) do
    %{
      key: key,
      subject: subject,
      resent: meta["resent"] == true,
      from: from,
      to: if(client["web"] != true, do: client["device"]),
      claim: Base.url_encode64(:crypto.strong_rand_bytes(12), padding: false)
    }
  end

  def asked(_meta, _from, _client, _subject), do: nil

  @doc """
  Claims the write this process's call is, where its request says a key and its tool `tool` doesn't only read, just
  before it runs, as `name` with `inputs` (as asked, before any hook changed them) for `{actor, subject}`. Gives `:run`
  where it runs here, `{:done, result}` with what it answered where it ran already, or `{:not_run, why}` where it
  isn't to run here: it's running elsewhere, or the key isn't one, or was used for another call. That's a JSON-RPC
  error, which a client that asked it again tells from an answer (`OneCore.Mcp`).
  """
  def claim(tool, name, inputs, {actor, subject}) do
    with {_id, %{write: %{} = write}} <- State.current(),
         false <- tool[:read_only] == true,
         path when is_binary(path) <- path(subject, write.key) do
      claimed(write, path, asked(name, inputs), actor, subject)
    else
      {:error, why} -> {:not_run, why}
      _ -> :run
    end
  end

  defp claimed(write, path, asked, actor, subject) do
    record = %{"server" => Server.key(), "claim" => write.claim, "actor" => actor, "asked" => asked, "status" => "running"}

    swept(subject)

    with :ok <- resendable(write, actor, subject),
         :exists <- Bucket.write(path, JSON.encode!(record)) do
      existing(path, write, asked, actor)
    else
      :ok -> :run
      {:error, why} -> {:not_run, why}
    end
  end

  defp existing(path, write, asked, actor) do
    with {:ok, data, tag} <- Bucket.read(path),
         %{"actor" => ^actor, "asked" => ^asked} = record <- JSON.decode!(data) do
      me = Server.key()

      case record do
        %{"status" => "done"} ->
          {:done, done(record["sealed"])}

        # Passed here by the server that holds it, as one whose call ends where what it needs is: taken over.
        %{"server" => server, "to" => ^me} when server == write.from ->
          case Bucket.replace(path, JSON.encode!(%{record | "server" => me, "claim" => write.claim, "to" => nil}), tag) do
            :ok -> :run
            :changed -> existing(path, write, asked, actor)
          end

        %{"server" => server} ->
          {:not_run, running(server)}
      end
    else
      :absent -> {:not_run, "That idempotency key is past its day: a client gives each write a key of its own"}
      _other -> {:not_run, "That idempotency key was used for another call: a client gives each write a key of its own"}
    end
  end

  defp running(server) do
    "This is still running on #{Servers.name(server) || "the server"} that was asked it first, so it isn't run here " <>
      "too. Its answer comes from there."
  end

  # What a write that ran answered, as MCP's `tools/call` answers it: encrypted to the device that asked, in `_meta`,
  # which that device opens in its place, or for one from no device, that it was done.
  defp done(sealed) when is_binary(sealed),
    do: %{content: [%{type: "text", text: "This was done when it was first asked."}], _meta: %{sealed: sealed}}

  defp done(nil),
    do: %{content: [%{type: "text", text: "This was done when it was first asked. Read again what it changed to see it."}]}

  # A write asked again is run only where every server that could have been asked it first keeps to keys.
  defp resendable(%{resent: false}, _actor, _subject), do: :ok

  defp resendable(%{resent: true}, actor, subject) do
    older =
      [actor, subject]
      |> Enum.map(&User.owner/1)
      |> Enum.reject(&is_nil/1)
      |> Enum.flat_map(&Servers.covering/1)
      |> Enum.uniq()
      |> Enum.reject(&keeps?/1)

    if older == [],
      do: :ok,
      else: {:error, "#{Enum.map_join(older, ", ", &(Servers.name(&1) || &1))} would run a write asked again without " <> "knowing it was asked before, until it's updated, so this isn't asked again here"}
  end

  @doc """
  Names `server` as the one this process's write is passed to, where this server holds its claim, so that one takes
  it over (`claim/4`), before it's asked it.
  """
  def passing(server), do: held(&Map.put(&1, "to", server))

  @doc """
  Takes back this process's write from the server it was passed to, which didn't answer: `:ok` where this server holds
  it, or none does, or where that server took it over meanwhile, `{:done, result}` with what it answered or
  `{:not_run, why}` while it runs it, which ends the call.
  """
  def kept do
    me = Server.key()

    case held(&Map.put(&1, "to", nil)) do
      {:ok, %{"status" => "done"} = record} -> {:done, done(record["sealed"])}
      {:ok, %{"server" => server}} when server != me -> {:not_run, running(server)}
      _ours_or_none -> :ok
    end
  end

  # Changes the claim this process's write holds, by `change`, where it's this server's: gives `{:ok, record}` with it
  # as it is after, or `nil` where there's none.
  defp held(change) do
    with {_id, %{write: %{} = write}} <- State.current(),
         path when is_binary(path) <- path(write.subject, write.key),
         {:ok, data, tag} <- Bucket.read(path) do
      record = JSON.decode!(data)
      changed = change.(record)

      cond do
        record["server"] != Server.key() or record["status"] != "running" -> {:ok, record}
        Bucket.replace(path, JSON.encode!(changed), tag) == :ok -> {:ok, changed}
        true -> held(change)
      end
    else
      _ -> nil
    end
  end

  @doc """
  Keeps what this request's write answered, `outcome` as MCP's `tools/call` ends (`{:ok, result}`, or a JSON-RPC
  error), where it claimed it, for a day, in the background: so the answer goes at once, and one asked again meanwhile
  is told it's still running.
  """
  def answered(nil, _outcome), do: :ok

  def answered(write, outcome) do
    with path when is_binary(path) <- path(write.subject, write.key) do
      Task.Supervisor.start_child(OneCore.Asks, fn ->
        claim = write.claim

        Bucket.update(path, fn
          nil ->
            nil

          was ->
            case JSON.decode!(was) do
              %{"claim" => ^claim, "status" => "running"} = record ->
                JSON.encode!(Map.merge(record, %{"status" => "done", "sealed" => sealed(write, outcome)}))

              _theirs ->
                was
            end
        end)
      end)
    end

    :ok
  end

  defp sealed(%{to: nil}, _outcome), do: nil

  defp sealed(%{to: to, key: key}, outcome) do
    answer =
      case outcome do
        {:ok, result} -> %{result: result}
        {:error, why} -> %{error: %{code: -32000, message: why}}
        {:error, code, why} -> %{error: %{code: code, message: why}}
        {:error, code, why, data} -> %{error: %{code: code, message: why, data: data}}
      end

    {:ok, sealed} = Iroh.seal(to, about(key), JSON.encode!(answer))
    Base.encode64(sealed)
  end

  # What an answer is encrypted for, which the device that opens it says too (`core/client/src/rpc.rs`).
  defp about(key), do: "one/writes/1/answer\n" <> key

  # Where a write's key is kept for whoever it's for, on a server that acts for them, or `nil` on one that passes it on.
  defp path(subject, key) do
    owner = User.owner(subject)

    cond do
      not is_binary(key) or not (key =~ ~r/^[0-9]{13}-[A-Za-z0-9_-]{16,64}\z/) ->
        {:error, "An idempotency key is <milliseconds since 1970>-<16 to 64 random letters, digits, - or _>"}

      expired?(key) ->
        {:error, "That idempotency key is past its day: a client gives each write a key of its own"}

      owner != nil and Servers.here?(owner) ->
        "#{subject}/writes/#{key}"

      true ->
        nil
    end
  end

  # Removes what's kept of `subject`'s writes past their day, in the background, as a write is claimed, at most once an
  # hour on each server: so none is kept much past its day, however few are written, as `OneCore.Tasks` removes tasks.
  # A key says when it was made, so nothing is read to tell.
  defp swept(subject) do
    Cache.fetch(__MODULE__, {:swept, subject}, fn ->
      Task.Supervisor.start_child(OneCore.Asks, fn ->
        for path <- Bucket.list("#{subject}/writes/"), expired?(Path.basename(path)), do: Bucket.remove(path)
      end)

      {:commit, true, expire: :timer.hours(1)}
    end)
  end

  defp expired?(key) do
    [made | _] = String.split(key, "-", parts: 2)
    String.to_integer(made) < System.system_time(:millisecond) - @kept
  end

  # What a call asked, so a key said again for another is found out.
  defp asked(name, inputs),
    do: Base.url_encode64(:crypto.hash(:sha256, :erlang.term_to_binary({name, inputs}, [:deterministic])), padding: false)
end
