defmodule OneCore.Edge.Unreachable do
  @moduledoc "What's raised when the network's edge, or what it hands out a link to, couldn't be reached or didn't answer."
  defexception [:message]
end
