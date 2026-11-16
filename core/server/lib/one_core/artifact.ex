defmodule OneCore.Artifact do
  @moduledoc """
  A couple of blobs together in the bucket. A plugin reads a blob, or several at once, writes one only if it's absent, replaces one only
  if it's unchanged, updates one, lists them and removes one, by their names in the artifact.

      case OneCore.Artifact.read(artifact, "notes.json") do
        {:ok, data, tag} -> OneCore.Artifact.replace(artifact, "notes.json", change(data), tag)
        :absent -> OneCore.Artifact.write(artifact, "notes.json", first())
      end

  `write` answers `:exists` and `replace` answers `:changed` when someone else got there first: read again and
  decide again. `update` does that for a record that's kept changing, in one step:

      OneCore.Artifact.update(artifact, "notes.json", fn was -> change(was || first()) end)

  `remove` is for a blob nothing reads any more, since nothing brings it back, so a record is never removed to be
  written again: one that fails between the two is lost. `size` gives `{:ok, bytes}` or `:absent` without reading
  the blob.

  A blob that's large doesn't pass through the server. A tool answers with one as `{:file, "image/jpeg", artifact,
  name}`, and takes one a client sent as an upload: the server hands the client a short-lived link to the bucket
  for each.

  An artifact's name is its prefix in the bucket without the last `/`, which is how relation tuples name it, and
  its first parts say whose it is. A plugin has its own artifact (a call's `own`), makes others for whoever it
  runs for (`OneCore.Call.make/2`) and reaches the ones they may (`OneCore.Call.reach/3`). One that was made or
  reached knows as what: reached as a `viewer`, it's only read, and a change to it is told to the plugins that
  observe `changed`.

  A plugin gives an artifact a codec by saying what they pass through on their way in and out: `%{artifact |
  codec: {module, with}}`, where `module.encode(with, name, data)` and `module.decode(with, name, blob)` each give
  `{:ok, binary}` or `{:error, why}`, and `name` is the blob's whole name in the bucket. Everything built on blobs
  then reads and writes through it without knowing, a database too. That's how a plugin encrypts
  one. A blob with a codec isn't handed out by a link, which would give what's in the bucket.
  """
  alias OneCore.{Bucket, Hooks}

  @enforce_keys [:prefix]
  defstruct [:prefix, :reached, :codec]

  @typedoc """
  `reached` is the plugin that reached it, who asked and who for, and as what, or `nil` for a plugin's own.
  `codec` is what its blobs pass through, a module and what to call it with, or `nil`.
  """
  @type t :: %__MODULE__{
          prefix: String.t(),
          reached: {String.t(), {String.t() | nil, String.t()}, String.t()} | nil,
          codec: {module, term} | nil
        }

  @doc "An artifact's name."
  def name(%__MODULE__{prefix: prefix}), do: String.trim_trailing(prefix, "/")

  def read(%__MODULE__{prefix: prefix} = artifact, name) do
    with {:ok, blob, tag} <- Bucket.read(prefix <> name), do: {:ok, passed(artifact, :decode, name, blob), tag}
  end

  @doc "Each of several of its blobs, as `read/2` gives it, in order, read at once where the bucket is far away."
  def read_all(%__MODULE__{prefix: prefix} = artifact, names) do
    for {name, read} <- Enum.zip(names, Bucket.read_all(Enum.map(names, &(prefix <> &1)))) do
      with {:ok, blob, tag} <- read, do: {:ok, passed(artifact, :decode, name, blob), tag}
    end
  end

  def write(artifact, name, data) do
    key = writes(artifact) <> name
    changed(artifact, name, Bucket.write(key, passed(artifact, :encode, name, data)))
  end

  def replace(artifact, name, data, tag) do
    key = writes(artifact) <> name
    changed(artifact, name, Bucket.replace(key, passed(artifact, :encode, name, data), tag))
  end

  @doc """
  Changes a blob in one step, and gives what it wrote: `change` is given what's there, or `nil` for nothing, and
  gives what's to be there. If someone else wrote it meanwhile, `change` is given theirs and asked again, so neither
  change is lost: a bucket offers only writes on a condition (ETags, as S3 and R2 do), so that's how a change to
  what's there is made. One that gives what's there already writes nothing.
  """
  def update(artifact, name, change) do
    {was, tag} = with {:ok, was, tag} <- read(artifact, name), do: {was, tag}, else: (:absent -> {nil, nil})

    case change.(was) do
      ^was -> was
      now -> if write_or_replace(artifact, name, now, tag) == :ok, do: now, else: update(artifact, name, change)
    end
  end

  defp write_or_replace(artifact, name, data, nil), do: write(artifact, name, data)
  defp write_or_replace(artifact, name, data, tag), do: replace(artifact, name, data, tag)

  def remove(artifact, name), do: changed(artifact, name, Bucket.remove(writes(artifact) <> name))
  def size(%__MODULE__{prefix: prefix}, name), do: Bucket.size(prefix <> name)

  @doc "A short-lived link to one of its blobs, for a client to read it with, or to write it if there's none."
  def link(%__MODULE__{codec: {_module, _with}} = artifact, _name, _mode),
    do: raise("#{name(artifact)} has a codec, so its blobs aren't handed out by a link")

  def link(%__MODULE__{prefix: prefix}, name, :read), do: Bucket.link(prefix <> name, :read)
  def link(artifact, name, :write), do: Bucket.link(writes(artifact) <> name, :write)

  @doc "The names of its blobs that start with `starting`, in order."
  def list(%__MODULE__{prefix: prefix}, starting \\ "") do
    for key <- Bucket.list(prefix <> starting), do: String.replace_prefix(key, prefix, "")
  end

  # A blob on its way into the bucket or out of it, through its codec, if it has one.
  defp passed(%__MODULE__{codec: nil}, _way, _name, data), do: data

  defp passed(%__MODULE__{codec: {module, with}, prefix: prefix}, way, name, data) do
    case apply(module, way, [with, prefix <> name, data]) do
      {:ok, passed} -> passed
      {:error, why} -> raise why
    end
  end

  defp writes(%__MODULE__{reached: {_plugin, _principal, "viewer"}} = artifact),
    do: raise("#{name(artifact)} was reached as a viewer, which only reads it")

  defp writes(%__MODULE__{prefix: prefix}), do: prefix

  defp changed(%__MODULE__{reached: {plugin, principal, _as}} = artifact, blob, :ok) do
    Hooks.observe("changed", %{"artifact" => name(artifact), "blob" => blob, "by" => plugin}, principal)
  end

  defp changed(_artifact, _blob, outcome), do: outcome
end
