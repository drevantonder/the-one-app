defmodule OneCore.User do
  @moduledoc """
  Who a request is from. Signing in is a plugin, which tells One Core who each request is from, and relation tuples
  say what each user may reach. With neither, everyone who reaches a network is the same one user: every plugin is
  theirs and runs for them, and every server on their list acts for them.

  Nothing else says so. Signing in and relation tuples replace this module.
  """

  @doc "The one user: their prefix in the bucket without its last `/`, which is how a call names who it's for."
  def anyone, do: "users/anyone"
end
