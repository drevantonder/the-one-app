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

  @doc """
  Whether `key` is a blob's key, or a part of one, as an artifact's name or a blob's name in it: `:ok`, or `{:error,
  why}`. It's parts joined by /, none empty or starting with a dot, so it can't step outside where it's put or name
  what's hidden there.
  """
  def check_key(key) do
    if is_binary(key) and Enum.all?(String.split(key, "/"), &(&1 =~ ~r/^[^.]/)),
      do: :ok,
      else: {:error, "#{inspect(key)} isn't parts joined by /, none empty or starting with a dot"}
  end

  @doc "`key`, where it's a blob's key (`check_key/1`), or else raises why not."
  def key!(key) do
    with {:error, why} <- check_key(key), do: raise(ArgumentError, why)
    key
  end

  @doc "A blob and its tag."
  def read(key), do: bucket(:read, [key])

  @doc "Each of several blobs, as `read/1` gives it, in order. A bucket that's far away reads them all at once."
  def read_all(keys), do: bucket(:read_all, [keys])

  @doc "Writes a blob only if there's none at `key`."
  def write(key, data), do: fused(fn -> sent(&conditionally(:write, [key, data], &1)) end)

  @doc "Replaces a blob only if it's still the one `tag` was read with."
  def replace(key, data, tag), do: fused(fn -> sent(&conditionally(:replace, [key, data, tag], &1)) end)

  @doc "The keys starting with `prefix`, in order."
  def list(prefix), do: bucket(:list, [prefix])

  @doc "Removes a blob. Nothing brings it back, so only what nothing reads any more is removed."
  def remove(key), do: fused(fn -> sent(fn _tried -> backend(:remove, [key]) end) end)

  @doc "How many bytes a blob is, or `:absent`."
  def size(key), do: bucket(:size, [key])

  @doc """
  A short-lived link to a blob, for something large: to read it, or to write it only if there's none at `key`.
  A server hands it to a client, which reads or writes the bucket with it and nothing else.
  """
  def link(key, mode) when mode in [:read, :write], do: bucket(:link, [key, mode])

  @doc """
  Changes a blob in one step: `change` is given what's there, or `nil` for nothing, and gives what's to be there,
  what's there already to write nothing, or `{:error, why}` to change nothing. If someone else wrote it meanwhile,
  `change` is given theirs and asked again, so neither change is lost: a bucket offers only writes on a condition
  (ETags, as S3 and R2 do), so that's how a change to what's there is made. Gives `:ok` or `{:error, why}`.
  """
  def update(key, change) do
    {was, tag} = with {:ok, was, tag} <- read(key), do: {was, tag}, else: (:absent -> {nil, nil})

    case change.(was) do
      {:error, why} -> {:error, why}
      ^was -> :ok
      now -> if(tag, do: replace(key, now, tag), else: write(key, now)) |> updated(key, change)
    end
  end

  defp updated(:ok, _key, _change), do: :ok
  defp updated(_changed, key, change), do: update(key, change)

  # A bucket that didn't answer a write may still have made it: R2 says so of a conditional write whose answer was
  # lost. Sending one again is safe, since the bucket refuses it if anything got there first, its own earlier try
  # too. So a write is sent again, and one refused after a try that wasn't answered is read back: it was made if
  # what's there is what it wrote. A removal is the same however often it's sent. Each is sent at most this often,
  # as one call: the fuse hears only how the last try went (`fused/1`).
  @tries 3

  # R2 answers 503 ServiceUnavailable for a while when it's busy, and asks for such a request to be sent again after
  # a wait that grows each time, by a random part of it so that servers that failed together don't all try again
  # together (exponential backoff with full jitter). So the wait before try `n` is up to `@backoff` times 2^(n-2) ms.
  @backoff 1_000

  defp sent(tried \\ 1, send) do
    send.(tried)
  rescue
    unreachable in Edge.Unreachable ->
      if tried == @tries, do: reraise(unreachable, __STACKTRACE__)
      Process.sleep(:rand.uniform(@backoff * 2 ** (tried - 1)))
      sent(tried + 1, send)
  end

  defp conditionally(function, [key, data | _] = arguments, tried) do
    with refused when refused != :ok <- backend(function, arguments) do
      if tried > 1 and match?({:ok, ^data, _tag}, backend(:read, [key])), do: :ok, else: refused
    end
  end

  defp bucket(function, arguments), do: fused(fn -> backend(function, arguments) end)

  defp backend(function, arguments) do
    {module, where} = Application.fetch_env!(:one_core, :bucket)
    apply(module, function, [where | arguments])
  end

  # A bucket that couldn't be reached raises `OneCore.Edge.Unreachable`, since what isn't on this machine is reached
  # through the network's edge. A call that did, after its own tries, melts `bucket_fuse` (`OneCore.Bucket.Fuse`),
  # and while it's blown every call fails at once rather than each waiting through its own retries and timeouts.
  defp fused(call) do
    if :fuse.ask(:bucket_fuse, :async_dirty) == :blown do
      raise Edge.Unreachable,
            "The bucket can't be reached just now, so this server couldn't do what was asked. Try again in a moment."
    end

    try do
      call.()
    rescue
      unreachable in Edge.Unreachable ->
        :fuse.melt(:bucket_fuse)
        reraise unreachable, __STACKTRACE__
    end
  end

  defmodule Fuse do
    @moduledoc """
    `bucket_fuse`, `:fuse`'s circuit breaker for the bucket: `ok`, and `blown` once a call fails, after which it's
    `ok` again for the next try once `@reset` has passed. Whoever installs it says what's done each time it goes from
    `blown` back to `ok` (`install/1`), once: `:fuse` tells this handler of each, one at a time. A server that
    couldn't reach the bucket may have missed being told something meanwhile, so it looks again then.
    """
    @behaviour :gen_event

    # A call that fails has already been sent again: a read by Req up to three more times, a write by `OneCore.Bucket`
    # twice, so a call failing at all is too many failures. One whose bucket couldn't be reached took 7 to 30 seconds
    # to say so, so one every 15 seconds is all that waits while it's away.
    @reset 15_000

    @doc "Installs `bucket_fuse`, and this handler of what it does, which runs `back` in a process of its own each time it's `ok` again."
    def install(back) do
      :ok = :fuse.install(:bucket_fuse, {{:standard, 0, @reset}, {:reset, @reset}})
      :ok = :fuse_event.add_handler(__MODULE__, {:ok, back})
    end

    @impl true
    def init(state), do: {:ok, state}

    @impl true
    def handle_event({:bucket_fuse, :blown}, {_was, back}), do: {:ok, {:blown, back}}

    def handle_event({:bucket_fuse, :ok}, {:blown, back}) do
      Task.start(back)
      {:ok, {:ok, back}}
    end

    def handle_event(_event, state), do: {:ok, state}

    @impl true
    def handle_call(_request, state), do: {:ok, :ok, state}
  end
end
