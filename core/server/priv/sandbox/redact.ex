defmodule OneCore.Redact do
  @moduledoc false
  # What Erlang/OTP and Elixir report of a process that crashed holds whatever it was working with: a task's
  # arguments, a GenServer's last message and state, a crash's reason and the arguments in its stack trace. Any of
  # them may be a person's tokens. So every string in what they report is "…", and each stack frame's arguments are
  # their arity, which keeps the module, function, arity, the reason's type and the stack trace. What code says to
  # Logger itself, and the message it raises an exception with, are its own words, and stay.
  #
  # Both the server and an untrusted plugin's VM compile this file (`mix.exs`, `OneCore.Sandbox`).

  @doc """
  Puts the filter first among `:logger`'s primary filters, ahead of Elixir's translator, so every handler and every
  translation hears an OTP report only redacted.
  """
  def install do
    %{filters: filters} = :logger.get_primary_config()
    filter = {__MODULE__, {&__MODULE__.filter/2, nil}}
    :ok = :logger.set_primary_config(:filters, [filter | List.keydelete(filters, __MODULE__, 0)])
  end

  @doc """
  A `:logger` primary filter: a report, or a format's arguments, redacted, as OTP and the emulator say what crashed.
  Text, as Elixir's `Logger` says it, is left as it is.
  """
  def filter(%{msg: {:string, _}}, _config), do: :ignore

  def filter(%{msg: msg} = event, _config) do
    case msg do
      {:report, report} -> %{event | msg: {:report, redact(report)}}
      {format, args} -> %{event | msg: {format, redact(args)}}
    end
  rescue
    # `:logger` removes a filter that raises, which would stop it redacting anything after: the event goes instead.
    _ -> %{event | msg: {:string, "A report that couldn't be redacted"}}
  end

  @doc "`Exception.format/3` of a crash, redacted."
  def format(kind, reason, stacktrace), do: Exception.format(kind, redact(reason), redact(stacktrace))

  @doc "`term` with every string \"…\" and every stack frame's arguments their arity."
  def redact(""), do: ""
  def redact(binary) when is_binary(binary), do: "…"
  def redact(bits) when is_bitstring(bits), do: <<>>

  def redact({module, function, args, location})
      when is_atom(module) and is_atom(function) and (is_list(args) or is_integer(args)) and is_list(location),
      do:
        {module, function, if(is_list(args), do: length(args), else: args),
         for({key, _} = at <- location, key in [:file, :line], do: at)}

  def redact(tuple) when is_tuple(tuple), do: tuple |> Tuple.to_list() |> Enum.map(&redact/1) |> List.to_tuple()

  def redact(%{__exception__: true, message: message} = error) when is_binary(message),
    do: %{redact_map(error) | message: message}

  def redact(map) when is_map(map), do: redact_map(map)

  def redact([_ | _] = list) do
    if :io_lib.printable_unicode_list(list), do: ~c"…", else: redact_list(list)
  end

  def redact(term), do: term

  defp redact_map(map), do: :maps.from_list(for {key, value} <- :maps.to_list(map), do: {redact(key), redact(value)})

  defp redact_list([head | tail]), do: [redact(head) | redact_list(tail)]
  defp redact_list(tail), do: redact(tail)
end
