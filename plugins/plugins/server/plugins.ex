defmodule Plugins do
  @moduledoc """
  Plugins: the app a person installs and removes plugins with, in the category `plugins`, which a client opens itself
  where there's no launcher yet. Its Marketplace is `marketplace`; installing, updating, uninstalling and restoring
  are One Core's own tools, which its app calls as they are.
  """
  @behaviour OneCore.Plugin

  alias OneCore.Call

  # The plugin directory's list (www/plugin-directory/README.md): every plugin anyone has installed from a git repo.
  @directory "https://one.drevan.me/plugins/api/plugins"

  @impl true
  def icon, do: "client/icon.svg"

  @impl true
  def requires, do: %{"core" => "~> 0.18.0"}

  @impl true
  def apps, do: [%{name: "Plugins", page: "client/index.html", category: "plugins", subject: "user"}]

  @impl true
  def tools do
    requirement = %{
      type: "object",
      properties: %{
        name: %{type: "string", description: "The plugin it requires, by name"},
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
        Lists every plugin the network can install, as its Marketplace shows them: each one anyone in the network
        deployed (`plugin`, its artifact, which `install` takes), then each the plugin directory lists that isn't
        among them (`plugin` null, `source` its git repo, which `one install` takes once the skill installing-plugins
        says it's checked). Each with what a person reads of it, its `category`, one of `categories`, and what it
        `requires`, each with why.
        """,
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            categories: %{type: "array", items: %{type: "string"}, description: "What a plugin may be for, in order"},
            plugins: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string", description: "Its own name"},
                  plugin: %{
                    type: ["string", "null"],
                    description: "Its artifact, or null for one only the directory has"
                  },
                  source: %{type: ["string", "null"], description: "The git repo the directory has it from: owner/repo"},
                  title: %{type: "string"},
                  icon: %{type: ["string", "null"], description: "A data: address"},
                  subtitle: %{type: ["string", "null"]},
                  about: %{type: ["string", "null"]},
                  category: %{type: ["string", "null"]},
                  apps: %{type: "array", description: "Each one's name, icon and App Library category"},
                  widgets: %{type: "array", description: "Each one's name and sizes"},
                  tools: %{type: "array", items: %{type: "string"}, description: "Its tools' names"},
                  trusted: %{type: ["boolean", "null"], description: "Whether it's trusted, or runs in a sandbox"},
                  requires: %{type: "array", items: requirement, description: "The plugins it requires, each with why"}
                },
                required: ~w(name plugin source title icon subtitle about category apps widgets tools trusted requires)
              }
            }
          },
          required: ["categories", "plugins"]
        }
      }
    ]
  end

  @impl true
  def call("marketplace", _inputs, call) do
    deployed = for plugin <- Call.deployed(call), plugin.icon != nil, do: plugin
    titles = Map.new(deployed, &{Path.basename(&1.plugin), &1.title})
    title = &(titles[&1] || &1)

    here =
      for plugin <- deployed do
        plugin
        |> Map.take([:plugin, :title, :icon, :subtitle, :about, :category, :apps, :widgets, :tools, :trusted])
        |> Map.merge(%{
          name: Path.basename(plugin.plugin),
          source: nil,
          requires:
            for(
              {name, why} <- Enum.sort(plugin.requires),
              name != "core",
              do: %{name: name, title: title.(name), why: why}
            )
        })
      end

    names = MapSet.new(here, & &1.name)

    listed =
      for plugin <- directory(call), plugin["icon"] != nil, plugin["name"] not in names do
        %{
          name: plugin["name"],
          plugin: nil,
          source: plugin["source"],
          title: plugin["title"] || plugin["name"],
          icon: plugin["icon"],
          subtitle: plugin["subtitle"],
          about: plugin["about"],
          category: plugin["category"],
          apps: plugin["apps"] || [],
          widgets: plugin["widgets"] || [],
          tools: [],
          trusted: nil,
          requires: for(name <- plugin["requires"] || [], do: %{name: name, title: title.(name), why: nil})
        }
      end

    %{
      categories: OneCore.Plugins.Deploy.categories(),
      plugins: Enum.sort_by(here ++ listed, &String.downcase(&1.title))
    }
  end

  # The directory's plugins. It's a website outside the network, which may not answer: then the Marketplace lists only
  # the network's own.
  defp directory(call) do
    case Call.fetch(call, %{url: @directory}) do
      %{status: 200, body: body} -> JSON.decode!(body)["plugins"]
      _ -> []
    end
  rescue
    _ -> []
  end
end
