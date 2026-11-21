defmodule OneCore.Iroh do
  @moduledoc "The server's Rust NIF for being reached by key (`nif/one_core_iroh`)."
  use OneCore.Nif, library: "one_core_iroh"

  @doc "The key kept in `key_file`, made on first use, as others know it."
  def key(_key_file), do: :erlang.nif_error(:nif_not_loaded)

  @doc "The signature of `message` by the key kept in `key_file`."
  def sign(_key_file, _message), do: :erlang.nif_error(:nif_not_loaded)

  @doc "`data` encrypted to the machine whose key is `to`, for what `about` says, which it's bound to (`client::sealed`)."
  def seal(_to, _about, _data), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  What the program keeping this server running told it in `line` (`client::serve::Told`, which says each message),
  as the JSON it's read as: `{:ok, json}`, or `{:error, why}` for a line that isn't one.
  """
  def told(_line), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  The line this server says `message` in to the program keeping it running, `message` being the JSON of one of
  `client::serve::Said`: `{:ok, line}`, or `{:error, why}` for one that isn't.
  """
  def said(_message), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Opens the endpoint whose key is kept in `key_file`, and sends each message asked to `asked` as
  `{:ask, ask, from, message}`, where `from` is the key of whoever asked. With a `network`, `{address, token}`, it's
  reached through the relay at that address too, and hears what outsiders ask there, whose `from` is `nil`, and it
  returns once it's connected there, so a server that says it's running is one the others know is on. Gives the
  endpoint, open while it's held, and its address as the JSON a client is told it in.
  """
  def start(_key_file, _network, _handler), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Keeps where the client whose key is `client` says a server reaches it while it wants to hear what changes:
  `address`, the JSON a client is told one in, and `device`, the device it proved it runs for, if it did. With `nil`,
  forgets it. It's kept in the file `watching` beside the server's key, which the endpoint starts with. A device
  watches from its latest run, so a run of the same device from before is forgotten.
  """
  def watch(_endpoint, _client, _address, _device), do: :erlang.nif_error(:nif_not_loaded)

  @doc "The devices watching, by their keys."
  def devices(_endpoint), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Asks the device whose key is `device` one message, the run of its client that's watching, and sends `to` the
  message `{reference, outcome}`: `{:ok, answer}`; `:unreachable` if no run of it is watching or it doesn't answer, so
  it was asked nothing; or `:stopped` if it stopped answering after it was asked.
  """
  def ask_client(_endpoint, _device, _message, _to, _reference), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Sends `message` to every client that's watching, each on a stream of its own: over the connection it has open
  to `endpoint`, or else over one made to it for this.
  """
  def notify(_endpoint, _message), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Stops taking what's asked: whoever asks or connects from now on is refused, so they ask another server. Once
  every message taken is answered, or `within` milliseconds have passed, the endpoint closes.
  """
  def close(_endpoint, _within), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Sends `answer` back to whoever asked `ask`."
  def reply(_ask, _response), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Says what the call `ask` asked for is doing, which the progress notifications it's sent carry from now on."
  def stage(_ask, _doing), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Connects to the server at `address`, and sends `to` the message `{reference, :ok}`, or `{reference, :unreachable}`
  if it doesn't answer in time.
  """
  def connect(_endpoint, _address, _to, _reference), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Asks the server whose key is `server` one message over its open connection, and sends `to` the message
  `{reference, outcome}`: `{:ok, answer}`; `:closed` if no connection to it is open, or `{:gone, address}` if the
  one that was open, made to `address`, no longer answers, so either way it was asked nothing; or `:stopped` if it
  stopped answering after it was asked. What that server says the call is doing, in its progress notifications, is
  said of `relay` too (`stage/2`), the message this server is answering by asking it, if it is.
  """
  def ask_server(_endpoint, _server, _message, _to, _reference, _relay), do: :erlang.nif_error(:nif_not_loaded)
end
