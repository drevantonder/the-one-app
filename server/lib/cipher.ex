defmodule Cipher do
  @moduledoc """
  Encryption, for plugins to import: a server's Rust add-on (`native/encryption`). It is what plugins run in giving
  them cryptography, and no part of One Core, which uses none of it. Which artifacts are encrypted, and who holds
  their keys, is the plugin `encryption`'s, whose skill says how a plugin keeps something encrypted.

  A machine is encrypted to by its key, the one it's reached by. An artifact's keys are handed to a server
  encrypted to its key, and stay in the add-on: `keys/3` gives a reference to them, so nothing here can log or
  answer a key. A blob is encrypted with them under its whole name in the bucket, and opens under no other.

  What each of these is made of is in `client/src/cipher.rs`, which a device runs too.
  """
  use Rustler, otp_app: :one_core, crate: "encryption"

  @doc "The keys of the artifact named `artifact` that `slot` hands the server whose key is kept in `key_file`."
  def keys(_key_file, _artifact, _slot), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Lets go of an artifact's keys: whatever holds them opens nothing with them from now on."
  def forget(_keys), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Encrypts a blob under its artifact's newest key: `{:ok, blob}`. `name` is its whole name in the bucket."
  def wrap(_keys, _name, _data), do: :erlang.nif_error(:nif_not_loaded)

  @doc "A blob as it was before `wrap/3`: `{:ok, data}`, or `{:error, why}` if these keys don't open it under that name."
  def unwrap(_keys, _name, _blob), do: :erlang.nif_error(:nif_not_loaded)

  @doc "What a person typed on their device, which encrypted it to the key kept in `key_file`."
  def secret(_key_file, _sealed), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  The backup key that a device handed the server whose key is kept in `key_file`, for one restore: `{:ok, {key,
  public}}`, where `key` is a reference to it and `public` its public half, which is what backups are encrypted to.
  """
  def backup_key(_key_file, _sealed), do: :erlang.nif_error(:nif_not_loaded)

  @doc "What a backup holds, encrypted to the backup key for what `about` says: `{:ok, data}` or `{:error, why}`."
  def backup_open(_key, _about, _sealed), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Encrypts a notification, `data`, to a device's notification key `to`, as the device opens it
  (`client/src/cipher.rs`): `{:ok, sealed}`.
  """
  def notification(_to, _data), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Encrypts `data` to the machine whose key is `to`, for what `about` says, which is all it then opens for."
  def seal(_to, _about, _data), do: :erlang.nif_error(:nif_not_loaded)
end
