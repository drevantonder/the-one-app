defmodule OneCore.Iroh do
  @moduledoc "The server's Rust add-on for being reached by key (`native/one_core_iroh`)."
  use Rustler, otp_app: :one_core, crate: "one_core_iroh"

  @doc """
  Opens the endpoint whose key is kept in `key_file`, made on first use, and sends each message a client asks to
  `asked` as `{:ask, ask, message}`. Gives the endpoint, open while it's held, and its address as the JSON a client
  is told it in.
  """
  def start(_key_file, _asked), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Sends `message` to every client connected to `endpoint`, each on a stream of its own."
  def tell(_endpoint, _message), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Sends `answer` back to whoever asked `ask`."
  def answer(_ask, _answer), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Connects to the server at `address`, and sends `to` the message `{reference, :ok}`, or `{reference, :unreachable}`
  if it doesn't answer in time.
  """
  def connect(_endpoint, _address, _to, _reference), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Asks the server whose key is `server` one message over its open connection, and sends `to` the message
  `{reference, outcome}`: `{:ok, answer}`; `:closed` if no connection to it is open, so it was asked nothing; or
  `:stopped` if it stopped answering after it was asked.
  """
  def ask_server(_endpoint, _server, _message, _to, _reference), do: :erlang.nif_error(:nif_not_loaded)
end
