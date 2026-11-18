defmodule OneCore.JSON do
  @moduledoc """
  What JSON holds of a term: maps with string keys, atoms as strings and tuples refused, as a term is once it's been
  encoded and decoded, which is how it's passed between servers, apps and clients.
  """

  @doc "`term` as JSON holds it: encoded and decoded again. Raises where JSON can't hold it."
  def normalize(term), do: JSON.decode!(JSON.encode!(term))
end
