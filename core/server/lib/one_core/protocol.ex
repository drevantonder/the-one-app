defmodule OneCore.Protocol do
  @moduledoc """
  The protocol the parts of the platform speak to each other, numbered as MCP's `protocolVersion` is, apart from the
  builds that speak it: what crosses the wire between a client, the edge and a server, and between servers. Every
  part built from a commit speaks the range `core/protocol.json` says there, `%{"oldest" => 1, "newest" => 1}`, and
  says it: as `_meta.protocol` in each request, and as a server or a device says what it is. Two parts talk where
  their ranges overlap, and are refused only where they don't, saying why, as MCP refuses a protocol version it
  doesn't speak (-32022). A release's version, and the `!`s that move it, decide nothing here.

  A change to what crosses the wire raises `newest`, the newer part speaking the older protocol beside it; `oldest`
  is raised in a change of its own, once no part seen in 90 days speaks only an older one (`tooling/release` refuses
  it before then).
  """
  alias OneCore.Builds

  @external_resource path = Path.expand("../../../protocol.json", __DIR__)
  @range path |> File.read!() |> JSON.decode!()

  @doc "The range this server speaks: `%{\"oldest\" => 1, \"newest\" => 1}`."
  def range, do: @range

  @doc """
  The range a part speaks, as it `said`, or, where it said none, as its `build` says: `nil` for one that speaks none
  this server knows.
  """
  def of(%{"oldest" => oldest, "newest" => newest}, _build)
      when is_integer(oldest) and is_integer(newest) and oldest <= newest,
      do: %{"oldest" => oldest, "newest" => newest}

  # Parts built before they said their protocol, as phones whose people haven't updated them are, speak 1 from 0.23,
  # whose wire it is unchanged, and none known before it. This goes once no part seen in 90 days says none.
  def of(_said, build) do
    version = Builds.version(build)
    if version && Version.compare(version, "0.23.0") != :lt, do: %{"oldest" => 1, "newest" => 1}
  end

  @doc "Whether a part that speaks `theirs` (`of/2`) talks with this one (`overlap?/2`)."
  def speaks?(theirs), do: overlap?(theirs, @range)

  @doc "Whether parts that speak `a` and `b` (`of/2`) talk: their ranges overlap."
  def overlap?(%{"oldest" => a_oldest, "newest" => a_newest}, %{"oldest" => b_oldest, "newest" => b_newest}),
    do: a_oldest <= b_newest and b_oldest <= a_newest

  def overlap?(_a, _b), do: false

  @doc "Whether a part that speaks `theirs`, and doesn't talk with this one, is the one behind."
  def behind?(%{"newest" => newest}), do: newest < @range["oldest"]
  def behind?(nil), do: true

  @doc "What a range is, said: `1`, `1 to 2`, or `none it knows` for `nil`."
  def name(%{"oldest" => same, "newest" => same}), do: "#{same}"
  def name(%{"oldest" => oldest, "newest" => newest}), do: "#{oldest} to #{newest}"
  def name(nil), do: "none it knows"
end
