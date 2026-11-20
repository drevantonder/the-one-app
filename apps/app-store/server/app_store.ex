defmodule AppStore do
  @moduledoc """
  App Store: the app a person installs and removes apps with, in the category `app-store`, which a client opens itself
  where there's no launcher yet. Its Marketplace is `marketplace`; installing, updating, uninstalling and restoring
  are One Core's own tools, which its app calls as they are.
  """
  @behaviour OneCore.App

  alias OneCore.Call

  # The app directory's list (www/app-directory/README.md): every app anyone has installed from a git repo.
  @directory "https://one.ptcau.dev/apps/api/apps"

  @impl true
  def icon, do: "client/icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.32.107"}

  @impl true
  def entrypoints, do: [%{page: "client/index.html", category: "app-store", subject: "user", runtime_version: 4}]

  @impl true
  def tools do
    requirement = %{
      type: "object",
      properties: %{
        name: %{type: "string", description: "The app it requires, by name"},
        title: %{type: "string", description: "Its name, as a person reads it"},
        why: %{type: ["string", "null"], description: "Why it's required: to reach your mail"}
      },
      required: ["name", "title", "why"]
    }

    [
      %{
        name: "marketplace",
        read_only: true,
        description: """
        Lists every app the network can install, as its Marketplace shows them: each one anyone in the network
        deployed (`app`, its artifact, which `install` takes), once, as its broadest copy, the one installed for the
        most, with who it's installed for (`installed_for`), then each the app directory lists that isn't
        among them (`app` null, `source` its git repo, which `one install` takes once the skill installing-apps
        says it's checked). Each with what a person reads of it, its `category`, one of `categories`, and what it
        `requires`, each with why.
        """,
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            categories: %{type: "array", items: %{type: "string"}, description: "What an app may be for, in order"},
            apps: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string", description: "Its own name"},
                  app: %{
                    type: ["string", "null"],
                    description: "Its artifact, or null for one only the directory has"
                  },
                  source: %{type: ["string", "null"], description: "The git repo the directory has it from: owner/repo"},
                  title: %{type: "string"},
                  icon: %{type: ["string", "null"], description: "A data: address"},
                  subtitle: %{type: ["string", "null"]},
                  about: %{type: ["string", "null"]},
                  category: %{type: ["string", "null"]},
                  entrypoints: %{type: "array", description: "Each one's name, icon and App Library category"},
                  widgets: %{type: "array", description: "Each one's name and sizes"},
                  tools: %{type: "array", items: %{type: "string"}, description: "Its tools' names"},
                  trusted: %{type: ["boolean", "null"], description: "Whether it's trusted, or runs in a sandbox"},
                  installed_for: %{
                    type: "array",
                    items: %{type: "string"},
                    description: "Who it's installed for: users, groups and the network"
                  },
                  requires: %{type: "array", items: requirement, description: "The apps it requires, each with why"}
                },
                required:
                  ~w(name app source title icon subtitle about category entrypoints widgets tools trusted installed_for requires)
              }
            }
          },
          required: ["categories", "apps"]
        }
      }
    ]
  end

  @impl true
  def call("marketplace", _inputs, call) do
    # Who each is installed for, as its artifact says, read for every app at once.
    deployed =
      Call.deployed(call)
      |> Enum.filter(& &1.icon)
      |> Task.async_stream(&Map.put(&1, :installed_for, OneCore.Apps.Installed.in_use_for(&1.app)), max_concurrency: 16)
      |> Enum.map(fn {:ok, app} -> app end)
      # Each app once: copies of the same one (`OneCore.Apps.Store.same?/2`) show as the broadest.
      |> Enum.group_by(&{Path.basename(&1.app), &1[:repo] || &1.app})
      |> Enum.map(fn {_app, copies} -> Enum.max_by(copies, &breadth/1) end)

    titles = Map.new(deployed, &{Path.basename(&1.app), &1.title})
    title = &(titles[&1] || &1)

    here =
      for app <- deployed do
        app
        |> Map.take([:app, :title, :icon, :subtitle, :about, :category, :entrypoints, :widgets, :tools, :trusted, :installed_for])
        |> Map.merge(%{
          name: Path.basename(app.app),
          source: nil,
          requires:
            for(
              {name, why} <- Enum.sort(app.requires),
              name != "core",
              do: %{name: name, title: title.(name), why: why}
            )
        })
      end

    names = MapSet.new(here, & &1.name)

    listed =
      for app <- directory(call), app["icon"] != nil, app["name"] not in names do
        %{
          name: app["name"],
          app: nil,
          source: app["source"],
          title: app["title"] || app["name"],
          icon: app["icon"],
          subtitle: app["subtitle"],
          about: app["about"],
          category: app["category"],
          entrypoints: app["entrypoints"] || [],
          widgets: app["widgets"] || [],
          tools: [],
          trusted: nil,
          installed_for: [],
          requires: for(name <- app["requires"] || [], do: %{name: name, title: title.(name), why: nil})
        }
      end

    %{
      categories: OneCore.Apps.Deploy.categories(),
      apps: Enum.sort_by(here ++ listed, &String.downcase(&1.title))
    }
  end

  # How far a copy of an app reaches: the broadest who it's installed for, then whose it is, the network before a group
  # and a group before a user.
  defp breadth(app) do
    whose = OneCore.Apps.Store.whose(app.app)
    {Enum.max(Enum.map([whose | app.installed_for], &reach/1)), reach(whose)}
  end

  defp reach("network"), do: 2
  defp reach("groups/" <> _), do: 1
  defp reach(_user), do: 0

  # The directory's apps. It's a website outside the network, which may not answer: then the Marketplace lists only
  # the network's own.
  defp directory(call) do
    case Call.fetch(call, %{url: @directory}) do
      %{status: 200, body: body} -> JSON.decode!(body)["apps"]
      _ -> []
    end
  rescue
    _ -> []
  end
end
