defmodule OneCore.Artifact do
  @moduledoc """
  A couple of blobs together in the bucket. A plugin reads a blob, writes one only if it's absent, replaces one only
  if it's unchanged, lists them and removes one, by their names in the artifact.

      case OneCore.Artifact.read(artifact, "notes.json") do
        {:ok, data, tag} -> OneCore.Artifact.replace(artifact, "notes.json", change(data), tag)
        :absent -> OneCore.Artifact.write(artifact, "notes.json", first())
      end

  `write` answers `:exists` and `replace` answers `:changed` when someone else got there first: read again and
  decide again. `remove` is for a blob nothing reads any more, since nothing brings it back. `size` gives
  `{:ok, bytes}` or `:absent` without reading the blob.

  A blob that's large doesn't pass through the server. A tool answers with one as `{:file, "image/jpeg", artifact,
  name}`, and takes one a client sent as an upload: the server hands the client a short-lived link to the bucket
  for each.

  An artifact's name is its prefix in the bucket without the last `/`, which is how relation tuples name it, and
  its first parts say whose it is. A plugin has its own artifact (a call's `own`), makes others for whoever it
  runs for (`OneCore.Call.make/2`) and reaches the ones they may (`OneCore.Call.reach/3`). One that was made or
  reached knows as what: reached as a `viewer`, it's only read, and a change to it is told to the plugins that
  hear `changed`.
  """
  alias OneCore.{Bucket, Hooks}

  @enforce_keys [:prefix]
  defstruct [:prefix, :reached]

  @typedoc "`reached` is the plugin that reached it, who for and as what, or `nil` for a plugin's own."
  @type t :: %__MODULE__{prefix: String.t(), reached: {String.t(), String.t(), String.t()} | nil}

  @doc "An artifact's name."
  def name(%__MODULE__{prefix: prefix}), do: String.trim_trailing(prefix, "/")

  def read(%__MODULE__{prefix: prefix}, name), do: Bucket.read(prefix <> name)
  def write(artifact, name, data), do: changed(artifact, name, Bucket.write(writes(artifact) <> name, data))

  def replace(artifact, name, data, tag),
    do: changed(artifact, name, Bucket.replace(writes(artifact) <> name, data, tag))

  def remove(artifact, name), do: changed(artifact, name, Bucket.remove(writes(artifact) <> name))
  def size(%__MODULE__{prefix: prefix}, name), do: Bucket.size(prefix <> name)

  @doc "A short-lived link to one of its blobs, for a client to read it with, or to write it if there's none."
  def link(%__MODULE__{prefix: prefix}, name, :read), do: Bucket.link(prefix <> name, :read)
  def link(artifact, name, :write), do: Bucket.link(writes(artifact) <> name, :write)

  @doc "The names of its blobs that start with `starting`, in order."
  def list(%__MODULE__{prefix: prefix}, starting \\ "") do
    for key <- Bucket.list(prefix <> starting), do: String.replace_prefix(key, prefix, "")
  end

  defp writes(%__MODULE__{reached: {_plugin, _who, "viewer"}} = artifact),
    do: raise("#{name(artifact)} was reached as a viewer, which only reads it")

  defp writes(%__MODULE__{prefix: prefix}), do: prefix

  defp changed(%__MODULE__{reached: {plugin, who, _as}} = artifact, blob, :ok) do
    Hooks.heard("changed", %{"artifact" => name(artifact), "blob" => blob, "by" => plugin}, who)
  end

  defp changed(_artifact, _blob, outcome), do: outcome
end
