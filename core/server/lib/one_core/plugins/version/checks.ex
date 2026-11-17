defmodule OneCore.Plugins.Version.Checks do
  @moduledoc """
  Whether what a plugin says of itself is what a plugin says (`OneCore.Plugin`), both for a version compiled here
  (`OneCore.Plugins.Version.Compile`) and for an untrusted one, whose code said it in its sandbox
  (`OneCore.Plugins.Version.Description`).
  """
  alias OneCore.Hooks
  alias OneCore.Plugins.Version

  # The sizes a widget comes in, which are a phone's own: columns by rows of a Home Screen page's grid.
  @sizes ~w(2x2 4x2 4x4)

  @doc """
  A version from what its plugin said of itself, `description`, if that's what a plugin says: `{:ok, version}` or
  `{:error, why}`. `plugin` is its module, or nil for an untrusted one, whose module is in its sandbox's VM, and
  `rest` its files that aren't Elixir or skills.
  """
  def checked(under, plugin, description, services, rest) do
    %{tools: tools, apps: apps, title: title, icon: icon, widgets: widgets, exports: exports, hooks: hooks} =
      description

    %{offers: offers, jobs: jobs, requires: requires} = description

    cond do
      not (is_list(tools) and Enum.all?(tools, &tool?(&1, rest))) ->
        {:error,
         "tools/0 gives a list of %{name: ..., description: ..., inputs: ..., outputs: ...}, where outputs is a JSON " <>
           "Schema of an object, ui, if a tool has one, is a page of client/, and task_support, if it has one, is " <>
           ":optional or :required"}

      not ((title == nil or is_binary(title)) and (icon == nil or is_binary(icon))) ->
        {:error, "title/0 gives text, and icon/0 a file's path"}

      not (is_list(apps) and Enum.all?(apps, &app?(&1, rest))) ->
        {:error,
         "apps/0 gives a list of %{name: ..., page: ..., icon: ...}, where page is a page of client/ and icon a file " <>
           "here, the plugin's icon/0 where an app has none, and category and subject, if it has them, are text"}

      not (is_list(widgets) and Enum.all?(widgets, &widget?(&1, tools))) ->
        {:error,
         "widgets/0 gives a list of %{name: ..., tool: ..., sizes: [...]}: one of this plugin's tools that only " <>
           "reads, and the sizes it comes in, of #{Enum.join(@sizes, ", ")}"}

      not (is_list(exports) and Enum.all?(exports, &is_binary/1)) ->
        {:error, "Its module's functions are said as a list of names"}

      not (is_list(hooks) and Enum.all?(hooks, &Hooks.hook?/1) and
               (hooks == [] or plugin == nil or function_exported?(plugin, :hook, 3))) ->
        {:error,
         "hooks/0 gives a list of hooks, and hook/3 is called at each. The hooks are One Core's, " <>
           Enum.join(Enum.sort(Map.keys(Hooks.kinds())), ", ") <>
           ", and the ones plugins offer, each as <plugin>.<hook>: soft-delete.moved"}

      not (is_list(offers) and Enum.all?(offers, &offer?/1)) ->
        {:error,
         "offers/0 gives a list of %{name: ..., kind: ..., description: ...}: a name of lowercase letters, " <>
           "digits and _, and a kind of :observe, :mutate, :validate or :collect"}

      not (is_map(requires) and Enum.all?(requires, &requirement?/1)) ->
        {:error,
         "requires/0 gives the other plugins it needs, each by its name with why, as a person reads it after " <>
           "\"Gmail uses Google Account\", and \"core\" the versions of the platform it runs on, as an Elixir " <>
           "requirement: %{\"core\" => \"~> 0.15.0\", \"google-account\" => \"to reach your mail\"}"}


      true ->
        tools = Enum.map(tools, &Version.schemas/1)

        version = version(under, plugin, tools, apps, widgets, Enum.sort(services), rest, icon)
        version = %{version | title: title || version.title}

        with {:ok, jobs} <- jobs(jobs, tools),
             do:
               {:ok,
                %{
                  version
                  | exports: exports,
                    hooks: Enum.uniq(hooks),
                    offers: offers,
                    jobs: jobs,
                    requires: Map.delete(requires, "core"),
                    core: requires["core"]
                }}
    end
  end

  # The versions of the platform it runs on, as an Elixir requirement, or another plugin it requires, by its name,
  # and why, which isn't left out.
  defp requirement?({"core", requirement}),
    do: is_binary(requirement) and match?({:ok, _}, Elixir.Version.parse_requirement(requirement))

  defp requirement?({name, why}),
    do: is_binary(name) and name =~ ~r/^[a-z][a-z0-9-]*$/ and is_binary(why) and String.trim(why) != ""

  defp requirement?(_requirement), do: false


  defp offer?(%{name: name, kind: kind, description: description}),
    do:
      is_binary(name) and name =~ ~r/^[a-z][a-z0-9_]*$/ and kind in [:observe, :mutate, :validate, :collect] and
        is_binary(description)

  defp offer?(_offer), do: false

  # A scheduled job calls one of the plugin's tools with inputs the tool takes, so one that could never run is
  # found out when it's deployed.
  defp jobs(jobs, tools) do
    checked =
      for job <- List.wrap(jobs) do
        with %{tool: name, every: every} when is_integer(every) and every > 0 <- job,
             inputs = JSON.decode!(JSON.encode!(Map.get(job, :inputs, %{}))),
             %{takes: takes} <- Enum.find(tools, &(&1.name == name)),
             :ok <- ExJsonSchema.Validator.validate(takes, inputs) do
          %{tool: name, every: every, inputs: inputs}
        else
          _ -> nil
        end
      end

    if is_list(jobs) and nil not in checked,
      do: {:ok, checked},
      else:
        {:error,
         "jobs/0 gives a list of %{tool: ..., every: ...}: one of this plugin's tools, how many seconds apart, and the inputs it takes, if any"}
  end

  # An icon goes to clients as a `data:` address, so a home screen asks for nothing more. The plugin's is the file it
  # names, nil where that isn't one of its files (which deploying refuses), and an app's its own or else the plugin's.
  defp version(under, plugin, tools, apps, widgets, services, rest, icon) do
    apps =
      for %{name: name, page: page} = app <- apps,
          do: %{
            name: name,
            page: page,
            icon: if(is_map_key(rest, app[:icon] || icon), do: data(app[:icon] || icon, rest)),
            category: app[:category],
            subject: app[:subject]
          }

    widgets = for %{name: name, tool: tool, sizes: sizes} <- widgets, do: %{name: name, tool: tool, sizes: sizes}

    %Version{
      under: under,
      plugin: plugin,
      tools: tools,
      apps: apps,
      title: List.first(for(app <- apps, do: app.name)),
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

  defp app?(%{name: name, page: page} = app, rest) when is_binary(name),
    do:
      page?(page, rest) and (app[:icon] == nil or is_map_key(rest, app[:icon])) and
        (app[:category] == nil or is_binary(app[:category])) and (app[:subject] == nil or is_binary(app[:subject]))

  defp app?(_, _), do: false

  # A widget is drawn from what one of the plugin's tools answers, which a Home Screen page asks again and again.
  defp widget?(%{name: name, tool: tool, sizes: [_ | _] = sizes}, tools) when is_binary(name),
    do: Enum.any?(tools, &(&1.name == tool and &1[:read_only] == true)) and sizes -- @sizes == []

  defp widget?(_, _), do: false

  defp page?(page, rest),
    do: is_binary(page) and Version.client?(page) and Path.extname(page) == ".html" and is_map_key(rest, page)

  # A tool may have a page of its own, its UI as MCP Apps has it, which a host shows for it. What it answers is an
  # object, which is what every version of MCP a client speaks takes as a tool's `outputSchema`. A version deployed
  # before tools said what they answer says nothing of it, and still loads (`OneCore.Plugins.Deploy` refuses a new one).
  defp tool?(%{name: name, description: description, inputs: %{}} = tool, rest)
       when is_binary(name) and is_binary(description),
       do:
         name =~ ~r/^[a-z][a-z0-9_]*$/ and (tool[:ui] == nil or page?(tool[:ui], rest)) and outputs?(tool[:outputs]) and
           tool[:task_support] in [nil, :optional, :required]

  defp tool?(_, _), do: false

  defp outputs?(nil), do: true
  defp outputs?(%{} = outputs), do: JSON.decode!(JSON.encode!(outputs))["type"] == "object"
  defp outputs?(_outputs), do: false
end
