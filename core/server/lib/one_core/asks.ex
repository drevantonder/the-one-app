defmodule OneCore.Asks do
  @moduledoc """
  What this server asks of others, in processes of their own under the `Task.Supervisor` of this name
  (`OneCore.Application`), and the one way to ask several at once: `each/3`.
  """
  require Logger

  # How long each has to answer `each/3`, in milliseconds, unless it says otherwise: short of the ten seconds an app
  # has to answer a hook (`OneCore.Hooks.within/3`), so a hook that asks several still has what the others said when
  # one doesn't.
  @deadline 8_000

  @doc """
  Asks each of `targets` (servers, devices, apps) at once, by calling `ask` with it, each under its own deadline of
  `deadline` milliseconds, eight seconds unless given, as `Promise.allSettled` does, in this call's trace. Gives
  `{answers, failed}`: what each that answered gave, as `{target, answer}` in the order of `targets`, and why each
  that didn't couldn't, by target: it raised or exited, or didn't answer in time and was stopped. One that doesn't
  answer takes away only its own part, and none keeps the caller past its deadline.
  """
  def each(targets, ask, deadline \\ @deadline) do
    {trace, call} = {OpenTelemetry.Ctx.get_current(), OneCore.Call.State.id()}

    asked = fn target ->
      OpenTelemetry.Ctx.attach(trace)
      OneCore.Call.State.follow(call)

      try do
        {:ok, ask.(target)}
      rescue
        problem -> {:error, Exception.message(problem)}
      catch
        kind, reason -> {:error, Exception.format_banner(kind, reason)}
      end
    end

    results =
      __MODULE__
      |> Task.Supervisor.async_stream_nolink(targets, asked,
        max_concurrency: max(length(targets), 1),
        timeout: deadline,
        on_timeout: :kill_task
      )
      |> Enum.zip(targets)

    answers = for {{:ok, {:ok, answer}}, target} <- results, do: {target, answer}
    failed = for {result, target} <- results, why = why(result, deadline), into: %{}, do: {target, why}
    for {target, why} <- failed, do: Logger.warning("Asking #{inspect(target)} failed: #{why}")
    {answers, failed}
  end

  defp why({:ok, {:ok, _answer}}, _deadline), do: nil
  defp why({:ok, {:error, why}}, _deadline), do: why
  defp why({:exit, :timeout}, deadline), do: "didn't answer within #{div(deadline, 1000)} seconds"
  defp why({:exit, reason}, _deadline), do: Exception.format_exit(reason)
end
