defmodule OneCore.Bucket do
  @moduledoc """
  All that's asked of a network's bucket: a blob is read, a few at once too, written only if absent, replaced only if unchanged,
  listed and removed. A blob's key is a path, and its first parts say whose it is. For a blob that's large, the
  bucket gives a short-lived link, which a server hands to a client to read it or write it with.

  Which bucket a server uses is its `:bucket` setting, `{module, where}`.
  """

  alias OneCore.Edge

  @type key :: String.t()
  @typedoc "What `read` gives so that `replace` can tell whether the blob changed since."
  @type tag :: term
  @typedoc "A web request that reads or writes one blob, for a minute: whoever is handed it sends it as it says."
  @type link :: %{method: String.t(), url: String.t(), headers: %{optional(String.t()) => String.t()}}

  @callback read(where :: term, key) :: {:ok, binary, tag} | :absent
  @callback read_all(where :: term, [key]) :: [{:ok, binary, tag} | :absent]
  @callback write(where :: term, key, binary) :: :ok | :exists
  @callback replace(where :: term, key, binary, tag) :: :ok | :changed
  @callback list(where :: term, prefix :: String.t()) :: [key]
  @callback remove(where :: term, key) :: :ok
  @callback size(where :: term, key) :: {:ok, non_neg_integer} | :absent
  @callback link(where :: term, key, :read | :write) :: link

  defmodule Refused do
    @moduledoc "What a bucket raises when it won't do what was asked for the server asking: it's on no list that may."
    defexception [:message]
  end

  @doc "A blob and its tag."
  def read(key), do: bucket(:read, [key])

  @doc "Each of several blobs, as `read/1` gives it, in order. A bucket that's far away reads them all at once."
  def read_all(keys), do: bucket(:read_all, [keys])

  @doc "Writes a blob only if there's none at `key`."
  def write(key, data), do: sent(&conditionally(:write, [key, data], &1))

  @doc "Replaces a blob only if it's still the one `tag` was read with."
  def replace(key, data, tag), do: sent(&conditionally(:replace, [key, data, tag], &1))

  @doc "The keys starting with `prefix`, in order."
  def list(prefix), do: bucket(:list, [prefix])

  @doc "Removes a blob. Nothing brings it back, so only what nothing reads any more is removed."
  def remove(key), do: sent(fn _tried -> bucket(:remove, [key]) end)

  @doc "How many bytes a blob is, or `:absent`."
  def size(key), do: bucket(:size, [key])

  @doc """
  A short-lived link to a blob, for something large: to read it, or to write it only if there's none at `key`.
  A server hands it to a client, which reads or writes the bucket with it and nothing else.
  """
  def link(key, mode) when mode in [:read, :write], do: bucket(:link, [key, mode])

  # A bucket that didn't answer a write may still have made it: R2 says so of a conditional write whose answer was
  # lost. Sending one again is safe, since the bucket refuses it if anything got there first, its own earlier try
  # too. So a write is sent again, and one refused after a try that wasn't answered is read back: it was made if
  # what's there is what it wrote. A removal is the same however often it's sent. Each is sent at most this often.
  @tries 3

  defp sent(tried \\ 1, send) do
    send.(tried)
  rescue
    unreachable in Edge.Unreachable ->
      if tried == @tries, do: reraise(unreachable, __STACKTRACE__), else: sent(tried + 1, send)
  end

  defp conditionally(function, [key, data | _] = arguments, tried) do
    with refused when refused != :ok <- bucket(function, arguments) do
      if tried > 1 and match?({:ok, ^data, _tag}, read(key)), do: :ok, else: refused
    end
  end

  # A bucket that couldn't be reached raises `OneCore.Edge.Unreachable`, since what isn't on this machine is reached
  # through the network's edge. A call that did melts `bucket_fuse` (`OneCore.Bucket.Fuse`), and while it's blown
  # every call fails at once rather than each waiting through its own retries and timeouts.
  defp bucket(function, arguments) do
    {module, where} = Application.fetch_env!(:one_core, :bucket)

    if :fuse.ask(:bucket_fuse, :async_dirty) == :blown do
      raise Edge.Unreachable,
            "The bucket can't be reached just now, so this server couldn't do what was asked. Try again in a moment."
    end

    try do
      apply(module, function, [where | arguments])
    rescue
      unreachable in Edge.Unreachable ->
        :fuse.melt(:bucket_fuse)
        reraise unreachable, __STACKTRACE__
    end
  end

  defmodule Fuse do
    @moduledoc """
    `bucket_fuse`, `:fuse`'s circuit breaker for the bucket: `ok`, and `blown` once a call fails, after which it's
    `ok` again for the next try once `@reset` has passed. A server that couldn't reach the bucket may have missed being
    told something meanwhile, so each time it goes from `blown` back to `ok` it looks again
    (`OneCore.Server.reconnected/0`), once: `:fuse` tells this handler of each, one at a time. A look that can't
    reach the bucket blows the fuse again, so it's looked again after the next reset.
    """
    @behaviour :gen_event

    # A call that fails has already been sent again: by Req up to three more times, and a write by
    # `OneCore.Bucket` too, so a call failing at all is too many failures. One whose bucket couldn't be reached took
    # 7 to 27 seconds to say so, so one every 15 seconds is all that waits while it's away.
    @reset 15_000

    @doc "Installs `bucket_fuse`, and this handler of what it does."
    def install do
      :ok = :fuse.install(:bucket_fuse, {{:standard, 0, @reset}, {:reset, @reset}})
      :ok = :fuse_event.add_handler(__MODULE__, :ok)
    end

    @impl true
    def init(state), do: {:ok, state}

    @impl true
    def handle_event({:bucket_fuse, :blown}, _state), do: {:ok, :blown}

    def handle_event({:bucket_fuse, :ok}, :blown) do
      Task.start(&OneCore.Server.reconnected/0)
      {:ok, :ok}
    end

    def handle_event(_event, state), do: {:ok, state}

    @impl true
    def handle_call(_request, state), do: {:ok, :ok, state}
  end
end
