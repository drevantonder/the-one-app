defmodule OneCore.User do
  @moduledoc """
  Who a request is from and who it's for, each one value, which is how relation tuples name them. A user is
  `users/<name>`, a group is `groups/<name>` and the network is `network`: each is their prefix in the bucket
  without its last `/`. The holder of a sharing link is `links/<the link's name>`, and is nobody else.

  Signing in is a plugin, which will tell One Core who each request is from: `asking/2` is the one place that's
  decided. With none, everyone who reaches a network is the same one user, `anyone/0`.

  A request is for whoever it's from, unless it says who else: a group, or the network. It then runs for them,
  with their artifacts, once the plugins that decide `reach` say whoever asked is their `member`.
  """
  alias OneCore.{Hooks, Services}

  @doc "The one user of a network that signs nobody in."
  def anyone, do: "users/anyone"

  @doc """
  Who a request is by and who it's for: `{:ok, {by, for}, passed_on}`, or `{:error, why}`. `from` is the key of
  whatever sent it, and `said` is what the request says of itself, its `_meta`:

  - `who` and `for`, from a server acting here, are who that server was asked by and for: it passes on what it
    was asked, and `passed_on` says so. A scheduled job is asked by nobody, so its `who` is `nil`.
  - `link` is a sharing link, and whoever shows one is its holder.
  - `as` names another user to act as. It's for a network being built, to show who may do what before anyone
    signs in, and is refused unless the server's `:development` setting is on.
  - `for` names who else the request is for: a group, or the network.
  """
  def asking(from, %{"who" => by} = said) when is_binary(by) or is_nil(by) do
    cond do
      not Services.acting?(from) -> {:error, "Only a server acting here says who it asks for"}
      owner?(said["for"]) -> with :ok <- acts(by, said["for"]), do: {:ok, {by, said["for"]}, said["passed_on"] == true}
      true -> {:error, "A server says who it asks for: a user, a group or the network"}
    end
  end

  def asking(_from, said) do
    with {:ok, by} <- by(said),
         asked = Map.get(said, "for", by),
         :ok <- acts(by, asked),
         do: {:ok, {by, asked}, false}
  end

  defp by(%{"link" => link}) when is_binary(link), do: {:ok, link(link)}

  defp by(%{"as" => name}) when is_binary(name) do
    cond do
      not Application.get_env(:one_core, :development, false) ->
        {:error, "Acting as another user is for a network being built, and this one wasn't started as one"}

      name =~ ~r/^[a-z][a-z0-9-]*$/ ->
        {:ok, "users/" <> name}

      true ->
        {:error, "A user's name is lowercase letters, digits and hyphens, starting with a letter"}
    end
  end

  defp by(_said), do: {:ok, anyone()}

  # Whether a request by `by` runs for `asked`: themselves, or whoever the plugins that decide say they're a
  # member of. With none deciding, everything in a network is shared.
  defp acts(by, by), do: :ok
  defp acts(nil, _asked), do: :ok

  defp acts(by, asked) do
    cond do
      not owner?(asked) ->
        {:error, "A request is for a user, a group or the network: users/<name>, groups/<name> or network"}

      link?(by) ->
        {:error, "A sharing link's holder asks for nobody else"}

      true ->
        with {:refuse, why} <-
               Hooks.decide("reach", %{"who" => by, "relation" => "member", "artifact" => asked}, {by, asked}),
             do: {:error, why}
    end
  end

  @doc "Whether `who` names a user, a group or the network: someone things belong to."
  def owner?(who), do: is_binary(who) and who =~ ~r/^((users|groups)\/[a-z][a-z0-9-]*|network)$/

  @doc """
  The who that is the holder of a sharing link. It's named by the link's hash, so a relation tuple that names it
  doesn't hold the link.
  """
  def link(link), do: "links/" <> Base.encode16(:crypto.hash(:sha256, link), case: :lower)

  @doc "Whether `who` is the holder of a sharing link, who is in no network: nothing is theirs, and nothing is added for them."
  def link?(who), do: String.starts_with?(who, "links/")
end
