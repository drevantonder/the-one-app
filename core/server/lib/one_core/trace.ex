defmodule OneCore.Trace do
  @moduledoc """
  Which trace a tool call is in, as W3C's Trace Context names it: a call is a span of a trace, a call it makes is
  a span under it, and a call passed to another server carries its span there (`traceparent`), so one trace spans
  both. The apps that observe `called` are told it; with none observing, no call is given a span.
  """

  @doc """
  The span of the tool call running here: its `traceId`, its `spanId` and the `parentSpanId` of the call it's
  under, if any. `nil` outside one.
  """
  def current do
    with {trace, span, parent} <- Process.get(__MODULE__),
         do: %{"traceId" => trace, "spanId" => span, "parentSpanId" => parent}
  end

  @doc "Runs `fun` as a span of its own, under the one running here or in a trace of its own, and gives it the span."
  def span(fun) do
    outer = Process.get(__MODULE__)
    {trace, parent} = if outer, do: {elem(outer, 0), elem(outer, 1)}, else: {hex(16), nil}
    Process.put(__MODULE__, {trace, hex(8), parent})

    try do
      fun.(current())
    after
      if outer, do: Process.put(__MODULE__, outer), else: Process.delete(__MODULE__)
    end
  end

  @doc "The `traceparent` that carries the span running here to another server, or `nil` with none."
  def traceparent do
    with {trace, span, _parent} <- Process.get(__MODULE__), do: "00-#{trace}-#{span}-01"
  end

  @doc "Carries on the trace that `traceparent`, from another server, is in: a call run here next is its child."
  def follow(<<"00-", trace::binary-32, "-", span::binary-16, "-", _flags::binary-2>>),
    do: Process.put(__MODULE__, {trace, span, nil})

  def follow(_none), do: Process.delete(__MODULE__)

  defp hex(bytes), do: Base.encode16(:crypto.strong_rand_bytes(bytes), case: :lower)
end
