defmodule OneCore.Apps.Version.Checks do
  @moduledoc """
  Whether what an app says of itself is what an app says (`OneCore.App`), both for a version compiled here
  (`OneCore.Apps.Version.Compile`) and for an untrusted one, whose code said it in its sandbox
  (`OneCore.Apps.Version.Description`).
  """
  alias OneCore.App.Manifest
  alias OneCore.Apps.Version

  # The sizes a widget comes in, which are a phone's own: columns by rows of a Home Screen page's grid.
  @sizes ~w(2x2 4x2 4x4)

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
      title: title,
      icon: icon,
      widgets: widgets,
      exports: exports,
      hooks: hooks
    } =
      description

    %{offers: offers, jobs: jobs, requires: requires, own: own} = description

    cond do
      not (is_list(tools) and Enum.all?(tools, &tool?(&1, rest))) ->
        {:error,
         "tools/0 gives a list of %{name: ..., description: ..., inputs: ..., outputs: ...}, where outputs is a JSON " <>
           "Schema of an object, ui, if a tool has one, is a page of client/, and task_support, if it has one, is " <>
           ":optional or :required"}

      not ((title == nil or is_binary(title)) and (icon == nil or is_binary(icon))) ->
        {:error, "title/0 gives text, and icon/0 a file's path"}

      not (is_list(entrypoints) and Enum.all?(entrypoints, &entrypoint?(&1, rest))) ->
        {:error,
         "entrypoints/0 gives a list of %{page: ..., icon: ...}, where page is a page of client/ and icon a file " <>
           "here, the app's icon/0 where an entrypoint has none, and name, category and subject, if it has them, are " <>
           "text: an entrypoint's name is its app's title where it's that app, and its own only where it's something else"}

      not (is_list(widgets) and Enum.all?(widgets, &widget?(&1, tools))) ->
        {:error,
         "widgets/0 gives a list of %{tool: ..., sizes: [...]}: one of this app's tools that only reads, and the " <>
           "sizes it comes in, of #{Enum.join(@sizes, ", ")}, and a name of its own only where it isn't its app's title"}

      Version.title(skills) == nil and title == nil and Enum.any?(entrypoints ++ widgets, &(&1[:name] == nil)) ->
        {:error,
         "An entrypoint or widget with no name of its own is called its app's title, which its skill's metadata says"}

      not (is_list(exports) and Enum.all?(exports, &is_binary/1)) ->
        {:error, "Its module's functions are said as a list of names"}

      not (is_list(hooks) and Enum.all?(hooks, &Manifest.hook?/1) and
               (hooks == [] or app == nil or function_exported?(app, :hook, 3))) ->
        {:error,
         "hooks/0 gives a list of hooks, and hook/3 is called at each. The hooks are One Core's, " <>
           Enum.join(Enum.sort(Map.keys(Manifest.kinds())), ", ") <>
           ", and the ones apps offer, each as <app>.<hook>: soft-delete.moved"}

      not (is_list(offers) and Enum.all?(offers, &offer?/1)) ->
        {:error,
         "offers/0 gives a list of %{name: ..., kind: ..., description: ...}: a name of lowercase letters, " <>
           "digits and _, and a kind of :observe, :mutate, :validate or :collect"}

      not (is_map(requires) and Enum.all?(requires, &requirement?/1)) ->
        {:error,
         "requires/0 gives the other apps it needs, each by its name with why, as a person reads it after " <>
           "\"Gmail uses Google Account\", and \"core\" the lowest version of the platform it runs on, as an Elixir " <>
           "requirement: %{\"core\" => \">= 0.15.0\", \"google-account\" => \"to reach your mail\"}"}

      own not in [:subject, :installed] ->
        {:error,
         "own/0 gives whose its own artifact is: :subject, whoever each call runs for (the default), or :installed, " <>
           "whoever it was installed for, everyone's together"}

      true ->
        tools = Enum.map(tools, &Version.schemas/1)

        title = Version.title(skills) || title || Enum.find_value(entrypoints, & &1[:name])
        version = version(under, app, tools, entrypoints, widgets, Enum.sort(services), rest, icon, title)

        with {:ok, jobs} <- jobs(jobs, tools),
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
    end
  end

  # The lowest version of the platform it runs on, as an Elixir requirement, or another app it requires, by its name,
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

  # A scheduled job calls one of the app's tools with inputs the tool takes, so one that could never run is
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
         "jobs/0 gives a list of %{tool: ..., every: ...}: one of this app's tools, how many seconds apart, and the inputs it takes, if any"}
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
            subject: entrypoint[:subject]
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

  defp entrypoint?(%{page: page} = entrypoint, rest),
    do:
      page?(page, rest) and (entrypoint[:name] == nil or is_binary(entrypoint[:name])) and
        (entrypoint[:icon] == nil or is_map_key(rest, entrypoint[:icon])) and
        (entrypoint[:category] == nil or is_binary(entrypoint[:category])) and
        (entrypoint[:subject] == nil or is_binary(entrypoint[:subject]))

  defp entrypoint?(_, _), do: false

  # A widget is drawn from what one of the app's tools answers, which a Home Screen page asks again and again.
  defp widget?(%{tool: tool, sizes: [_ | _] = sizes} = widget, tools),
    do:
      (widget[:name] == nil or is_binary(widget[:name])) and
        Enum.any?(tools, &(&1.name == tool and &1[:read_only] == true)) and sizes -- @sizes == []

  defp widget?(_, _), do: false

  defp page?(page, rest),
    do: is_binary(page) and Version.client?(page) and Path.extname(page) == ".html" and is_map_key(rest, page)

  # A tool may have a page of its own, its UI as MCP Apps has it, which a host shows for it. What it answers is an
  # object, which is what every version of MCP a client speaks takes as a tool's `outputSchema`. A version deployed
  # before tools said what they answer says nothing of it, and still loads (`OneCore.Apps.Deploy` refuses a new one).
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
