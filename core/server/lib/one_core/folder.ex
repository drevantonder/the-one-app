defmodule OneCore.Folder do
  @moduledoc "Reading a folder as a client deploys one: a server's Rust NIF (`nif/folder`)."
  use OneCore.Nif, library: "folder"

  @doc """
  The files of `folder`, by their paths in it: `{:ok, paths}`, or `{:error, why}`, leaving out what's never
  deployed, as `client::files` says (`core/client/src/lib.rs`).
  """
  def files(_folder), do: :erlang.nif_error(:nif_not_loaded)
end
