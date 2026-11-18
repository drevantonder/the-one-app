defmodule OneCore.Apps.Version.Checks do
  @moduledoc """
  Whether what an app says of itself is what an app says (`OneCore.App`), both for a version compiled here
  (`OneCore.Apps.Version.Compile`) and for an untrusted one, whose code said it in its sandbox
  (`OneCore.Apps.Version.Description`).
  """
  alias OneCore.App.Manifest
  alias OneCore.Apps.Version
  alias OneCore.Apps.Version.Shape

  @doc """
  A version from what its app said of itself, `description`, if that's what an app says: `{:ok, version}` or
  `{:error, why}`. `app` is its module, or nil for an untrusted one, whose module is in its sandbox's VM, `rest` its
  files that aren't Elixir or skills, and `skills` its skills, whose metadata says its `title`
  (`OneCore.Apps.Version.title/1`).
  """
  def checked(under, app, description, services, rest, skills) do
    %{
      tools: tools,
      entrypoints: entrypoints,
      icon: icon,
      widgets: widgets,
      exports: exports,
      hooks: hooks
    } =
      description

    %{offers: offers, jobs: jobs, requires: requires, own: own} = description

    prefix = if app, do: Version.own_name(app) <> ".", else: ""

    problems =
      Enum.flat_map([:tools, :entrypoints, :widgets, :offers], &Shape.problems(&1, description[&1], prefix)) ++
        tool_problems(tools, rest, prefix) ++
        entrypoint_problems(entrypoints, rest, prefix) ++
        widget_problems(widgets, tools, prefix) ++
        badge_problems(entrypoints, tools, prefix) ++
        other_problems(description, app, prefix)

    case problems do
      [] ->
        tools = Enum.map(tools, &Version.schemas/1)

        title = Version.title(skills)
        version = version(under, app, tools, entrypoints, widgets, Enum.sort(services), rest, icon, title)

        with {:ok, jobs} <- jobs(jobs, tools, prefix),
             do:
               {:ok,
                %{
                  version
                  | exports: exports,
                    hooks: Enum.uniq(hooks),
                    offers: offers,
                    jobs: jobs,
                    own: own,
                    requires: Map.delete(requires, "core"),
                    core: requires["core"]
                }}

      problems ->
        {:error, Enum.join(Enum.take(problems, 20), "\n")}
    end
  end

  # The items of a list an app gave that are maps, with where each is in it. What isn't one, `Shape` says.
  defp indexed(list),
    do: if(is_list(list), do: for({item, at} <- Enum.with_index(list), is_map(item), do: {item, at}), else: [])

  # A tool may have a page of its own, its UI as MCP Apps has it, which a host shows for it.
  defp tool_problems(tools, rest, prefix) do
    for {%{ui: ui}, at} <- indexed(tools), is_binary(ui), why = page(ui, rest) do
      "#{prefix}tools()[#{at}].ui: got #{Shape.given(ui)}, #{why}"
    end
  end

  defp entrypoint_problems(entrypoints, rest, prefix) do
    for {entrypoint, at} <- indexed(entrypoints),
        {field, why} <- [
          {:page, is_binary(entrypoint[:page]) && page(entrypoint[:page], rest)},
          {:icon,
           is_binary(entrypoint[:icon]) && not is_map_key(rest, entrypoint[:icon]) && "expected a file of the app"}
        ],
        why do
      "#{prefix}entrypoints()[#{at}].#{field}: got #{Shape.given(entrypoint[field])}, #{why}"
    end
  end

  # A widget is drawn from what one of the app's tools answers, which a Home Screen page asks again and again.
  defp widget_problems(widgets, tools, prefix) do
    tools = for {tool, _at} <- indexed(tools), do: tool

    for {%{tool: name}, at} <- indexed(widgets), is_binary(name), why = reading_tool(name, tools) do
      "#{prefix}widgets()[#{at}].tool: got #{Shape.given(name)}, #{why}"
    end
  end

  # An entrypoint's badge is a count one of the app's tools answers, which a Home Screen asks again and again.
  defp badge_problems(entrypoints, tools, prefix) do
    tools = for {tool, _at} <- indexed(tools), do: tool

    for {%{badge: name}, at} <- indexed(entrypoints), is_binary(name), why = reading_tool(name, tools) do
      "#{prefix}entrypoints()[#{at}].badge: got #{Shape.given(name)}, #{why}"
    end
  end

  defp reading_tool(name, tools) do
    case Enum.find(tools, &(&1[:name] == name)) do
      nil -> "expected one of this app's tools that only read: #{Enum.join(Enum.map(tools, & &1[:name]), ", ")}"
      %{read_only: true} -> nil
      _ -> "which doesn't say read_only: true in tools/0, as a tool that only reads does and a Home Screen calls"
    end
  end

  # Why `page` isn't a page of the app's client, or nil.
  defp page(page, rest) do
    cond do
      not Version.client?(page) -> "expected an .html page of client/, such as \"client/index.html\""
      Path.extname(page) != ".html" -> "expected an .html page of client/"
      not is_map_key(rest, page) -> "expected a page the app has, and it has no #{page}"
      true -> nil
    end
  end

  defp other_problems(description, app, prefix) do
    %{icon: icon, exports: exports, hooks: hooks, requires: requires, own: own} = description

    List.flatten([
      if(not (icon == nil or is_binary(icon)), do: "#{prefix}icon(): got #{Shape.given(icon)}, expected a file's path"),
      if(not (is_list(exports) and Enum.all?(exports, &is_binary/1)),
        do: "Its module's functions are said as a list of names"
      ),
      hook_problems(hooks, app, prefix),
      requires_problems(requires, prefix),
      if(own not in [:subject, :installed],
        do:
          "#{prefix}own(): got #{Shape.given(own)}, expected :subject, whoever each call runs for (the default), or " <>
            ":installed, whoever it was installed for, everyone's together"
      )
    ])
    |> Enum.reject(&is_nil/1)
  end

  defp hook_problems(hooks, app, prefix) do
    if is_list(hooks) do
      one_of = Enum.join(Enum.sort(Map.keys(Manifest.kinds())), ", ")

      for {hook, at} <- Enum.with_index(hooks), not Manifest.hook?(hook) do
        "#{prefix}hooks()[#{at}]: got #{Shape.given(hook)}, expected one of One Core's hooks (#{one_of}), or " <>
          "another app's, as <app>.<hook>: soft-delete.moved"
      end ++
        if(hooks != [] and app != nil and not function_exported?(app, :hook, 3),
          do: ["#{prefix}hooks(): hook/3 isn't defined, and it's called at each hook"],
          else: []
        )
    else
      "#{prefix}hooks(): got #{Shape.given(hooks)}, expected a list of hooks"
    end
  end

  defp requires_problems(requires, prefix) when is_map(requires) do
    for {name, why} <- requires, not requirement?({name, why}) do
      "#{prefix}requires()[#{Shape.given(name)}]: got #{Shape.given(why)}, " <>
        if(name == "core",
          do: "expected the lowest version of the platform it runs on, as an Elixir requirement: \">= 0.15.0\"",
          else:
            "expected why it needs the app, as a person reads it after \"Gmail uses Google Account\": " <>
              "\"to reach your mail\" (and the app's name is lowercase letters, digits and -)"
        )
    end
  end

  defp requires_problems(requires, prefix),
    do:
      "#{prefix}requires(): got #{Shape.given(requires)}, expected a map of the other apps it needs, each by its name with why"

  # The lowest version of the platform it runs on, as an Elixir requirement, or another app it requires, by its name,
  # and why, which isn't left out.
  defp requirement?({"core", requirement}),
    do: is_binary(requirement) and match?({:ok, _}, Elixir.Version.parse_requirement(requirement))

  defp requirement?({name, why}),
    do: OneCore.Apps.Installed.named(name) == :ok and is_binary(why) and String.trim(why) != ""

  # A scheduled job calls one of the app's tools with inputs the tool takes, so one that could never run is
  # found out when it's deployed.
  defp jobs(jobs, tools, prefix) do
    with [] <- Shape.problems(:jobs, jobs, prefix),
         [] <-
           for({job, at} <- indexed(jobs), problem <- job_problems(job, tools), do: "#{prefix}jobs()[#{at}]#{problem}") do
      {:ok,
       for(job <- jobs, do: %{tool: job.tool, every: job.every, inputs: OneCore.JSON.normalize(job[:inputs] || %{})})}
    else
      problems -> {:error, Enum.join(problems, "\n")}
    end
  end

  defp job_problems(%{tool: name} = job, tools) do
    case Enum.find(tools, &(&1.name == name)) do
      nil ->
        [".tool: got #{Shape.given(name)}, expected one of this app's tools: #{Enum.map_join(tools, ", ", & &1.name)}"]

      %{takes: takes} ->
        case ExJsonSchema.Validator.validate(takes, OneCore.JSON.normalize(job[:inputs] || %{})) do
          :ok -> []
          {:error, wrong} -> for {why, "#" <> at} <- wrong, do: ".inputs#{at}: #{why}, for the tool #{name}"
        end
    end
  end

  # An icon goes to clients as a `data:` address, so a home screen asks for nothing more. The app's is the file it
  # names, nil where that isn't one of its files (which deploying refuses), and an entrypoint's its own or else the app's.
  defp version(under, app, tools, entrypoints, widgets, services, rest, icon, title) do
    entrypoints =
      for %{page: page} = entrypoint <- entrypoints,
          do: %{
            name: entrypoint[:name] || title,
            page: page,
            icon: if(is_map_key(rest, entrypoint[:icon] || icon), do: data(entrypoint[:icon] || icon, rest)),
            category: entrypoint[:category],
            subject: entrypoint[:subject],
            badge: entrypoint[:badge],
            host: entrypoint[:host]
          }

    widgets =
      for %{tool: tool, sizes: sizes} = widget <- widgets, do: %{name: widget[:name] || title, tool: tool, sizes: sizes}

    %Version{
      under: under,
      app: app,
      tools: tools,
      entrypoints: entrypoints,
      title: title,
      icon: if(is_map_key(rest, icon), do: data(icon, rest)),
      widgets: widgets,
      exports: [],
      services: services,
      hooks: [],
      offers: [],
      jobs: [],
      code: nil
    }
  end

  defp data(path, rest), do: "data:#{Version.kind(path)};base64,#{Base.encode64(rest[path])}"
end
