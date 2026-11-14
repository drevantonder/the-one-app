defmodule OneCore.Artifact do
  @moduledoc """
  A couple of blobs together in the bucket. A plugin reads a blob, writes one only if it's absent, replaces one only
  if it's unchanged, lists them and removes one, by their names in the artifact.

      case OneCore.Artifact.read(artifact, "notes.json") do
        {:ok, data, tag} -> OneCore.Artifact.replace(artifact, "notes.json", change(data), tag)
        :absent -> OneCore.Artifact.write(artifact, "notes.json", first())
      end

  `write` answers `:exists` and `replace` answers `:changed` when someone else got there first: read again and
  decide again. `remove` is for a blob nothing reads any more, since nothing brings it back.
  """
  alias OneCore.Bucket

  @enforce_keys [:prefix]
  defstruct [:prefix]

  @type t :: %__MODULE__{prefix: String.t()}

  def read(%__MODULE__{prefix: prefix}, name), do: Bucket.read(prefix <> name)
  def write(%__MODULE__{prefix: prefix}, name, data), do: Bucket.write(prefix <> name, data)
  def replace(%__MODULE__{prefix: prefix}, name, data, tag), do: Bucket.replace(prefix <> name, data, tag)
  def remove(%__MODULE__{prefix: prefix}, name), do: Bucket.remove(prefix <> name)

  @doc "The names of its blobs that start with `starting`, in order."
  def list(%__MODULE__{prefix: prefix}, starting \\ "") do
    for key <- Bucket.list(prefix <> starting), do: String.replace_prefix(key, prefix, "")
  end
end
