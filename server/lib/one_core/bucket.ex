defmodule OneCore.Bucket do
  @moduledoc """
  All that's asked of a network's bucket: a blob is read, written only if absent, replaced only if unchanged,
  listed and removed. A blob's key is a path, and its first parts say whose it is.

  Which bucket a server uses is its `:bucket` setting, `{module, where}`.
  """

  @type key :: String.t()
  @typedoc "What `read` gives so that `replace` can tell whether the blob changed since."
  @type tag :: term

  @callback read(where :: term, key) :: {:ok, binary, tag} | :absent
  @callback write(where :: term, key, binary) :: :ok | :exists
  @callback replace(where :: term, key, binary, tag) :: :ok | :changed
  @callback list(where :: term, prefix :: String.t()) :: [key]
  @callback remove(where :: term, key) :: :ok

  defmodule Refused do
    @moduledoc "What a bucket raises when it won't do what was asked for the server asking: it's on no list that may."
    defexception [:message]
  end

  @doc "A blob and its tag."
  def read(key), do: bucket(:read, [key])

  @doc "Writes a blob only if there's none at `key`."
  def write(key, data), do: bucket(:write, [key, data])

  @doc "Replaces a blob only if it's still the one `tag` was read with."
  def replace(key, data, tag), do: bucket(:replace, [key, data, tag])

  @doc "The keys starting with `prefix`, in order."
  def list(prefix), do: bucket(:list, [prefix])

  @doc "Removes a blob. Nothing brings it back, so only what nothing reads any more is removed."
  def remove(key), do: bucket(:remove, [key])

  defp bucket(function, arguments) do
    {module, where} = Application.fetch_env!(:one_core, :bucket)
    apply(module, function, [where | arguments])
  end
end
