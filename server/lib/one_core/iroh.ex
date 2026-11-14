defmodule OneCore.Iroh do
  @moduledoc "The server's Rust add-on for being reached by key (`native/one_core_iroh`)."
  use Rustler, otp_app: :one_core, crate: "one_core_iroh"

  @doc """
  Opens the endpoint whose key is kept in `key_file`, made on first use, and sends each message a client asks to
  `asked` as `{:ask, ask, message}`. Gives the endpoint, open while it's held, and its address as the JSON a client
  is told it in.
  """
  def start(_key_file, _asked), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Sends `answer` back to whoever asked `ask`."
  def answer(_ask, _answer), do: :erlang.nif_error(:nif_not_loaded)
end
