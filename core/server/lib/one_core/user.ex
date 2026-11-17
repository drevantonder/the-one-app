defmodule OneCore.User do
  @moduledoc """
  Who a request is from and who it's for, each one value, which is how relation tuples name them. A user is
  `users/<name>`, a group is `groups/<name>` and the network is `network`: each is their prefix in the bucket
  without its last `/`. The holder of a sharing link is `users/<name>/links/<the link's name>`, after whoever the
  link leads to, and is nobody else.

  Signing in is an app, which tells One Core whose device a request came from: `asking/3` is the one place
  that's decided, and it asks the apps that mutate `device` (`OneCore.Hooks.whose/1`). A device no app
  places is nobody's, and nobody is refused everything: so with no app signing anyone in, a network answers
  nobody.

  A request is for whoever it's from, unless it says who else: a group, or the network. It then runs for them,
  with their artifacts, once the apps that validate `reach` say whoever asked is their `member`.
  """
  alias OneCore.{Bucket, Hooks, Server, Servers}

  @nobody "This device isn't signed in here. Sign in on it, or approve it from a device of yours."

  @doc "The JSON-RPC error a device that's nobody's is answered, so whatever passed its request on can tell."
  def nobody, do: -32003

  @doc """
  Who a request is by and who it's for, its actor and its subject: `{:ok, {actor, subject}, forwarded}`, or
  `{:error, why}`. `from` is the key of whatever sent it, which its connection proved, or `nil` for what came in at
  the network's address over the web. `said` is what the request says of itself, its `_meta`. `hook` is whether it asks an app's hook: a
  moment on any of the network's servers is about whoever it's about, so each of them is believed about that,
  where only one acting for someone is believed about what they asked for.

  - `actor` and `subject`, from a server acting for whoever asked, are who that server was asked by and for: it
    passes on what it was asked, having decided it, and `forwarded` says so: `:to` from a server that couldn't run
    it, and `true` from one that handed it to this one for what it holds (`OneCore.Call`). A scheduled job is asked
    by nobody, so its `actor` is `nil`.
  - `link` is a sharing link, and whoever shows one is its holder.
  - `as` names a user to act as. It's for a network being built, to show who may do what with nobody signing
    in, and is refused unless the server's `:development` setting is on.
  - `device`, in what came in at the network's address, is the device the edge says it's from: it proved
    that there, as a connection proves a key here. Over a connection, it's the device a run of a client runs for,
    with `attestation` (`device/2`).
  - `subject` names who else the request is for: a group, or the network.

  Otherwise it's from whoever's device `from` is.
  """
  def asking(from, meta, hook \\ false)

  def asking(from, %{"actor" => actor} = meta, hook) when is_binary(actor) or is_nil(actor) do
    whose = owner(actor || meta["subject"] || "") || "network"

    cond do
      not (Servers.acts_for?(from, whose) or (hook and is_binary(from) and Servers.acts_for(from) != [])) ->
        {:error, "Only a server acting for #{whose} says it asks for them"}

      is_binary(meta["subject"]) ->
        {:ok, {actor, meta["subject"]}, if(meta["forwarded"] == "to", do: :to, else: meta["forwarded"] == true)}

      true ->
        {:error, "A server says who it asks for too"}
    end
  end

  def asking(from, meta, _hook) do
    with {:ok, actor} <- by(from, meta),
         :ok <- here(actor),
         subject = Map.get(meta, "subject", actor),
         :ok <- acts(actor, subject),
         do: {:ok, {actor, subject}, false}
  end

  defp by(_from, %{"link" => link}) when is_binary(link), do: {:ok, link(link)}

  defp by(_from, %{"as" => name}) when is_binary(name) do
    cond do
      not Application.get_env(:one_core, :development, false) ->
        {:error, "Acting as a user is for a network being built, and this one wasn't started as one"}

      name =~ ~r/^[a-z][a-z0-9-]*$/ ->
        {:ok, "users/" <> name}

      true ->
        {:error, "A user's name is lowercase letters, digits and hyphens, starting with a letter"}
    end
  end

  defp by(from, meta) do
    case device(from, meta) do
      nil -> {:error, nobody(), @nobody}
      device -> whose(device)
    end
  end

  @doc """
  The key of the device a request is from, or `nil`: the device the network's edge says, for what came in
  there over the web, and otherwise the key the connection proved. A run of a client has a key of its own, since
  a device runs many at once and each is reached by its key, and a request from one names the device it runs for
  with that device's signature of `one/run/1`, a line, and the run's key (`core/client/src/lib.rs`). A run whose device
  doesn't attestation for it is nobody's.
  """
  def device(nil, %{"device" => device}) when is_binary(device), do: device

  def device(from, %{"device" => device, "attestation" => attestation}) when is_binary(from) do
    with true <- is_binary(device) and is_binary(attestation),
         {:ok, key} <- Base.decode16(device, case: :lower),
         {:ok, signature} <- Base.decode16(attestation, case: :lower),
         true <- byte_size(key) == 32 and byte_size(signature) == 64,
         true <- :crypto.verify(:eddsa, :none, "one/run/1\n" <> from, signature, [key, :ed25519]) do
      device
    else
      _ -> nil
    end
  end

  def device(from, _meta) when is_binary(from), do: from
  def device(_from, _meta), do: nil

  defp whose(device) do
    case Hooks.whose(device) do
      nil -> {:error, nobody(), @nobody}
      {:error, why} -> {:error, "Couldn't check whose this device is, so nothing was done: #{why}. Try again."}
      who -> {:ok, who}
    end
  end

  # A server answers whoever it acts for, and the holder of a link that leads to them.
  defp here(by) do
    owner = owner(by)

    if owner == nil or Servers.here?(owner),
      do: :ok,
      else: {:error, "This server doesn't act for #{owner}. The ones that do are the ones to ask."}
  end

  # Whether a request by `by` runs for `asked`: themselves, or whoever the apps that validate `reach` say they're a
  # member of.
  defp acts(by, by), do: :ok

  defp acts(by, subject) do
    cond do
      not owner?(subject) ->
        {:error, "A request is for a user, a group or the network: users/<name>, groups/<name> or network"}

      link?(by) ->
        {:error, "A sharing link's holder asks for nobody else"}

      true ->
        with {:refuse, why} <-
               Hooks.validate("reach", %{"who" => by, "relation" => "member", "artifact" => subject}, {by, subject}),
             do: {:error, why}
    end
  end

  @doc """
  The network's first owner, or `nil` while it has none. One Core keeps who that is itself, in the blob
  `network/owner`, since it's who may do anything where no app decides who may (`OneCore.Hooks.validate/4`).
  """
  def first_owner do
    Servers.cached(:first_owner, fn ->
      with {:ok, who, _tag} <- Bucket.read("network/owner"), do: who, else: (:absent -> nil)
    end)
  end

  # The apps a network starts with, which whoever makes its first owner installs for it, each after the ones it
  # uses: database and encryption keep what the others keep, permissions decides who may do what, sign-in says whose
  # each device is, sandbox runs untrusted apps, vault keeps accounts and secrets, soft-delete keeps what's
  # deleted, notifications tells people things, and home-screen is the launcher every client opens first.
  @starting ~w(database encryption permissions sign-in sandbox vault soft-delete notifications home-screen)

  @doc """
  Makes the user `who` the network's first owner, which a network gets once: `:ok`, or `{:error, why}`. Then the
  apps a network starts with are installed for it as the first owner installs any, from what the network has
  deployed of them (`network/apps/<name>`), and only then are the apps that validate `owner` asked, so the one
  that keeps who may do what is there to write it down, and the one that signs people in to say where they join.
  Doing it again for the same user finishes what an earlier try left undone.
  """
  def make_first_owner(who) do
    if is_binary(who) and who =~ ~r/^users\/[a-z][a-z0-9-]*$/ do
      wrote = Bucket.write("network/owner", who)
      Servers.invalidate()

      if wrote == :ok or first_owner() == who do
        Server.notify_servers(["network"])
        failed = for name <- @starting, why = starting(name, who), do: "#{name}: #{why}"

        case Hooks.validate("owner", %{"who" => who}, {nil, "network"}) do
          {:refuse, why} ->
            {:error, why}

          :ok when failed != [] ->
            {:error,
             "#{who} is the network's first owner, but these weren't installed for it: " <> Enum.join(failed, "; ")}

          :ok ->
            :ok
        end
      else
        {:error, "The network's first owner is #{first_owner()}"}
      end
    else
      {:error, "The network's first owner is a user: users/<name>, in lowercase letters, digits and hyphens"}
    end
  end

  # Installs one of the apps a network starts with for it, unless it's there: `nil`, or why it wasn't.
  defp starting(name, who) do
    case OneCore.Apps.Installed.read("network")[name] do
      %{"state" => "installed"} ->
        nil

      _ ->
        case OneCore.Apps.Lifecycle.install("network", name, "network/apps/#{name}", who) do
          %{"state" => "installed"} -> nil
          %{"state" => state} = record -> record["why"] || "it's #{state}"
          {:error, why} -> why
        end
    end
  end

  @doc """
  Whose `who` is, or who they are: the user, the group or the network that an artifact's name, or a who's, starts
  with. A sharing link's holder is whoever's the link leads to: their servers answer the holder, with the apps
  installed for them. `nil` for a name that starts with nobody's.
  """
  def owner(who) do
    case String.split(who, "/") do
      [kind, name | _] when kind in ["users", "groups"] -> kind <> "/" <> name
      ["network" | _] -> "network"
      _ -> nil
    end
  end

  @doc "Whether `who` names a user, a group or the network: someone things belong to."
  def owner?(who), do: is_binary(who) and who =~ ~r/^((users|groups)\/[a-z][a-z0-9-]*|network)$/

  @doc """
  The who that is the holder of a sharing link. A link starts with whose it leads to, as their name: that's how
  the network's edge finds a server for it. Its holder is named after them and by the link's hash, so a
  relation tuple that names the holder doesn't hold the link. A link that starts with nobody's is nobody's.
  """
  def link(link) do
    Enum.join(List.wrap(owner(link)) ++ ["links", Base.encode16(:crypto.hash(:sha256, link), case: :lower)], "/")
  end

  @doc "Whether `who` is the holder of a sharing link, who is in no network: nothing is theirs, and nothing is installed for them."
  def link?(who), do: match?(["links", _name], Enum.take(String.split(who, "/"), -2))
end
