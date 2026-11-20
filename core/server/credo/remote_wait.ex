defmodule OneCore.Credo.RemoteWait do
  use Credo.Check,
    id: "ONE001",
    category: :warning,
    base_priority: :high,
    run_on_all: true,
    explanations: [
      check: """
      A wait on another machine has a deadline: a `Task.async_stream` (or `Task.Supervisor`'s) with `timeout:
      :infinity` whose function asks another server or a device waits for as long as one that's connected but stuck
      takes, and a raise in one of them takes down the caller. Ask several at once through `OneCore.Asks.each/3`,
      which gives each a deadline and names the ones that didn't answer.

      A call leaves the machine where it reaches, through any functions of One Core and the apps, a call of the iroh
      NIF's `ask_server` or `ask_client` (`OneCore.Server`): the only way a server asks another server or a device,
      and one that waits until the other end answers or goes. Everything else is local: a process of this server,
      or the bucket and other HTTP, whose requests have their own receive timeout (`Req`). Functions are followed by
      name, through aliases, so a call through a function value (`apply`, a module in a variable) isn't seen.
      """
    ]

  # The calls of the iroh NIF that go to another machine and wait for its answer (`one_core_iroh`).
  @asks [:ask_server, :ask_client]

  @impl true
  def run_on_all_source_files(exec, source_files, params) do
    asts = for file <- source_files, {:ok, ast} = Credo.Code.ast(file), do: {file, ast}
    defined = Enum.reduce(asts, %{}, fn {_file, ast}, defined -> definitions(ast, nil, %{}, defined) end)
    remote = remote(defined, MapSet.new())

    for {file, ast} <- asts, line <- waits(ast, nil, %{}, remote) do
      issue = format_issue(IssueMeta.for(file, params), message: "This waits without end on another machine: ask through `OneCore.Asks.each/3`, which gives each a deadline.", line_no: line)
      Credo.Execution.ExecutionIssues.append(exec, file, issue)
    end

    :ok
  end

  # Each function's calls, by `{module, name}`, of every module in `ast`.
  defp definitions({:defmodule, _, [{:__aliases__, _, parts}, [do: body]]}, module, aliases, defined) do
    {module, aliases} = entered(parts, module, aliases, body)
    definitions(body, module, aliases, defined)
  end

  defp definitions({kind, _, [head, body]}, module, aliases, defined) when kind in [:def, :defp] do
    Map.update(defined, {module, name(head)}, calls(body, module, aliases), &(&1 ++ calls(body, module, aliases)))
  end

  defp definitions({_, _, children}, module, aliases, defined) when is_list(children),
    do: Enum.reduce(children, defined, &definitions(&1, module, aliases, &2))

  defp definitions(children, module, aliases, defined) when is_list(children),
    do: Enum.reduce(children, defined, &definitions(&1, module, aliases, &2))

  defp definitions({left, right}, module, aliases, defined),
    do: definitions(right, module, aliases, definitions(left, module, aliases, defined))

  defp definitions(_leaf, _module, _aliases, defined), do: defined

  # The functions that reach one of `@asks`, by following each function's calls until nothing more is found.
  defp remote(defined, remote) do
    found =
      for {function, calls} <- defined, function not in remote, Enum.any?(calls, &(&1 == :asks or &1 in remote)),
        into: remote,
        do: function

    if MapSet.size(found) == MapSet.size(remote), do: remote, else: remote(defined, found)
  end

  # The lines of each stream of tasks with no deadline whose function reaches another machine.
  defp waits({:defmodule, _, [{:__aliases__, _, parts}, [do: body]]}, module, aliases, remote) do
    {module, aliases} = entered(parts, module, aliases, body)
    waits(body, module, aliases, remote)
  end

  defp waits({{:., meta, [{:__aliases__, _, task}, stream]}, _, arguments}, module, aliases, remote)
       when task in [[:Task], [:Task, :Supervisor]] and stream in [:async_stream, :async_stream_nolink] do
    {function, options} = {Enum.at(arguments, -2), List.last(arguments)}
    within = waits(arguments, module, aliases, remote)

    if Keyword.keyword?(options) and options[:timeout] == :infinity and
         Enum.any?(calls(function, module, aliases), &(&1 == :asks or &1 in remote)),
       do: [meta[:line] | within],
       else: within
  end

  defp waits({_, _, children}, module, aliases, remote) when is_list(children), do: waits(children, module, aliases, remote)
  defp waits(children, module, aliases, remote) when is_list(children), do: Enum.flat_map(children, &waits(&1, module, aliases, remote))
  defp waits({left, right}, module, aliases, remote), do: waits(left, module, aliases, remote) ++ waits(right, module, aliases, remote)
  defp waits(_leaf, _module, _aliases, _remote), do: []

  # What `ast` calls: `{module, name}` of each function, and `:asks` for a call of the NIF that asks another machine.
  defp calls(ast, module, aliases) do
    {_ast, calls} =
      Macro.prewalk(ast, [], fn
        {{:., _, [{:__aliases__, _, parts}, name]}, _, _} = node, calls when is_atom(name) ->
          {node, [{resolve(parts, aliases), name} | calls]}

        {{:., _, [{:__MODULE__, _, _}, name]}, _, _} = node, calls ->
          {node, [{module, name} | calls]}

        {{:., _, [{variable, _, context}, name]}, _, _} = node, calls when is_atom(variable) and is_atom(context) and name in @asks ->
          {node, [:asks | calls]}

        {:/, _, [{name, _, context}, arity]} = node, calls when is_atom(name) and is_atom(context) and is_integer(arity) ->
          {node, [{module, name} | calls]}

        {name, _, arguments} = node, calls when is_atom(name) and is_list(arguments) ->
          {node, [{module, name} | calls]}

        node, calls ->
          {node, calls}
      end)

    calls
  end

  # The module `parts` names inside `module`, and the aliases its body makes, its own name's last part among them.
  defp entered(parts, module, aliases, body) do
    module = if module, do: Module.concat([module | parts]), else: resolve(parts, aliases)
    aliases = aliases |> Map.put(List.last(parts), module) |> Map.put(:__MODULE__, module)

    {_body, aliases} =
      Macro.prewalk(body, aliases, fn
        {:alias, _, [{:__aliases__, _, named} | options]} = node, aliases ->
          as = with [[as: {:__aliases__, _, [as]}]] <- options, do: as, else: (_ -> List.last(named))
          {node, Map.put(aliases, as, resolve(named, aliases))}

        {:alias, _, [{{:., _, [{:__aliases__, _, base}, :{}]}, _, children}]} = node, aliases ->
          {node, Enum.reduce(children, aliases, fn {:__aliases__, _, named}, aliases -> Map.put(aliases, List.last(named), resolve(base ++ named, aliases)) end)}

        node, aliases ->
          {node, aliases}
      end)

    {module, aliases}
  end

  # The module an alias names: `__MODULE__.Name` inside the module, and `nil` for one made at run time.
  defp resolve([{:__MODULE__, _, _} | rest], aliases), do: Module.concat([aliases[:__MODULE__] | rest])
  defp resolve([first | rest], aliases) when is_atom(first), do: Module.concat([Map.get(aliases, first, first) | rest])
  defp resolve(_parts, _aliases), do: nil

  defp name({:when, _, [head | _]}), do: name(head)
  defp name({name, _, _}), do: name
end
