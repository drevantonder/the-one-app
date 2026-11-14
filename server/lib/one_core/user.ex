defmodule OneCore.User do
  @moduledoc """
  Who a request is from: one value, which is how relation tuples name whoever asks. A user is `users/<name>`, their
  prefix in the bucket without its last `/`. The holder of a sharing link is `links/<the link's name>`, and is
  nobody else.

  Signing in is a plugin, which will tell One Core who each request is from: `asking/2` is the one place that's
  decided. With none, everyone who reaches a network is the same one user, `anyone/0`. Every plugin is added for
  them, and every server on their list acts for them.
  """
  alias OneCore.Services

  @doc "The one user of a network that signs nobody in."
  def anyone, do: "users/anyone"

  @doc """
  Who a request is from: `{:ok, who}`, or `{:error, why}`. `from` is the key of whatever sent it, and `said` is
  what the request says of itself, its `_meta`:

  - `who`, from a server acting here, is who that server was asked by: it passes on what it was asked.
  - `link` is a sharing link, and whoever shows one is its holder.
  - `as` names another user to act as. It's for a network being built, to show who may do what before anyone
    signs in, and is refused unless the server's `:development` setting is on.
  """
  def asking(from, said) do
    case said do
      %{"who" => who} when is_binary(who) ->
        if Services.acting?(from), do: {:ok, who}, else: {:error, "Only a server acting here says who it asks for"}

      %{"link" => link} when is_binary(link) ->
        {:ok, link(link)}

      %{"as" => name} when is_binary(name) ->
        cond do
          not Application.get_env(:one_core, :development, false) ->
            {:error, "Acting as another user is for a network being built, and this one wasn't started as one"}

          name =~ ~r/^[a-z][a-z0-9-]*$/ ->
            {:ok, "users/" <> name}

          true ->
            {:error, "A user's name is lowercase letters, digits and hyphens, starting with a letter"}
        end

      _ ->
        {:ok, anyone()}
    end
  end

  @doc """
  The who that is the holder of a sharing link. It's named by the link's hash, so a relation tuple that names it
  doesn't hold the link.
  """
  def link(link), do: "links/" <> Base.encode16(:crypto.hash(:sha256, link), case: :lower)

  @doc "Whether `who` is the holder of a sharing link, who is in no network: nothing is theirs, and nothing is added for them."
  def link?(who), do: String.starts_with?(who, "links/")
end
