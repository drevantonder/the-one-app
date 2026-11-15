defmodule Cipher do
  @moduledoc """
  Encryption for a server's plugins: the encryption plugin's NIF (`nif/encryption`), which a plugin that uses
  `encryption` calls. It names no plugin: each says what it encrypts something for, `about`, which is all that then
  opens it. Which artifacts are encrypted, and who holds their keys, is the plugin `encryption`'s, whose skill says
  how a plugin keeps something encrypted.

  A machine is encrypted to by its key, the one it's reached by. Keys stay in the NIF: `key/1` and `open_keys/3`
  give a reference to them, so nothing here can log or answer a key. A blob is encrypted with an artifact's keys
  under its whole name in the bucket, and opens under no other.

  What each of these is made of is in `plugins/encryption/client/native/src/cipher.rs`, which a device runs too.
  """
  @on_load :load

  @doc false
  def load, do: OneCore.Plugin.nif(__MODULE__, &:erlang.load_nif(&1, 0))

  @doc "The key kept in the file `file`, as a server keeps its own (`OneCore.Server.key_file/0`): `{:ok, keys}`."
  def key(_file), do: :erlang.nif_error(:nif_not_loaded)

  @doc "The newest of `keys` as a machine's key is named, which is what's encrypted to them: `{:ok, key}`."
  def public(_keys), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Lets go of keys: whatever holds them opens nothing with them from now on."
  def forget(_keys), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Encrypts `data` to the machine whose key is `to`, for what `about` says: `{:ok, sealed}`."
  def seal(_to, _about, _data), do: :erlang.nif_error(:nif_not_loaded)

  @doc "What was encrypted to the newest of `keys` for what `about` says: `{:ok, data}`, or `{:error, why}`."
  def open(_keys, _about, _sealed), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  What `open/3` would give, kept as keys rather than given, 32 bytes each: an artifact's for each epoch, the first
  first, or a key to open more with. `{:ok, keys}`, or `{:error, why}`.
  """
  def open_keys(_keys, _about, _sealed), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Encrypts a blob under the newest of an artifact's `keys`: `{:ok, blob}`. `name` is its whole name in the bucket."
  def wrap(_keys, _name, _data), do: :erlang.nif_error(:nif_not_loaded)

  @doc "A blob as it was before `wrap/3`: `{:ok, data}`, or `{:error, why}` if these keys don't open it under that name."
  def unwrap(_keys, _name, _blob), do: :erlang.nif_error(:nif_not_loaded)
end
