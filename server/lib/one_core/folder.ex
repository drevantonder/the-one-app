defmodule OneCore.Folder do
  @moduledoc "Reading a folder as a client deploys one: a server's Rust add-on (`native/folder`)."
  use Rustler, otp_app: :one_core, crate: "folder"

  @doc """
  The files of `folder`, by their paths in it: `{:ok, paths}`, or `{:error, why}`. Names starting with a dot are
  left out, and so is whatever the `.gitignore` files in it and above it leave out.
  """
  def files(_folder), do: :erlang.nif_error(:nif_not_loaded)
end
