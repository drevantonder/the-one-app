defmodule Home do
  @moduledoc """
  Home pages: where apps and widgets sit. A user has one, and so does each group and the network.

  A page is what's added for whoever it belongs to: each plugin's apps, and the widgets its plugins have. So
  adding an app to a page is adding its plugin for them (One Core's `add`), which is what puts it on the clients
  of everyone who sees that page, and nothing here keeps which apps a page has. What this keeps is how a page is
  arranged: the order of its apps, and the widgets put on it, each at a size. That's the page's owner's, an
  artifact made for them.

  A group has a page once it's made, so the groups someone is in, and who's in each, are a part of Settings
  here, over the tools of whatever keeps them (`relations`).

  Every tool here is asked for whoever's page it is, as an app opened from that page is. What draws the pages is
  this plugin's `app/`, which a client shows first.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @impl true
  def apps do
    [
      %{name: "Apps", page: "app/apps.html", icon: "apps.svg", settings: true},
      %{name: "Groups", page: "app/groups.html", icon: "groups.svg", settings: true},
      %{name: "Appearance", page: "app/appearance.html", icon: "appearance.svg", settings: true}
    ]
  end

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}

    [
      %{
        name: "page",
        read_only: true,
        description: """
        Gives your home page: its apps and widgets in order (`items`), the widgets that could be put on it
        (`widgets`), and whether you may arrange it. Asked for a group or for the network, it's theirs.
        """,
        inputs: none
      },
      %{
        name: "arrange",
        description: """
        Arranges your home page: its apps and widgets, in the order given. Asked for a group or for the network,
        it arranges theirs, which their owners do. A widget not given is taken off the page, and an app not given
        goes after the rest: `remove` is what takes an app off a page, and `add` puts one on.
        """,
        inputs: %{
          type: "object",
          properties: %{
            items: %{
              type: "array",
              description: "Each an app or a widget, as `page` gives them",
              items: %{
                type: "object",
                properties: %{
                  app: %{type: "string", description: "An app, as its plugin's name and its page: notes/app/index.html"},
                  widget: %{type: "string", description: "A widget, as its plugin's name and its tool: notes/glance"},
                  size: %{type: "string", enum: ~w(2x2 4x2 4x4), description: "The widget's size"}
                }
              }
            }
          },
          required: ["items"]
        }
      },
      %{
        name: "settings",
        read_only: true,
        description: "Lists the parts of Settings: each an app of a plugin that says it's one",
        inputs: none
      },
      %{
        name: "store",
        read_only: true,
        description: "Lists every app anyone has deployed, which could be added to a page, each with its plugin",
        inputs: none
      }
    ]
  end

  @impl true
  def call("page", _inputs, call) do
    here = for plugin <- Call.plugins(call), plugin.added == call.for, do: plugin

    apps =
      for plugin <- here, app <- plugin.apps, not app.settings do
        %{app: "#{plugin.name}/#{app.page}", plugin: plugin.plugin, name: app.name, icon: app.icon}
      end

    widgets =
      for plugin <- here, widget <- plugin.widgets do
        opens = Enum.find(apps, &String.starts_with?(&1.app, plugin.name <> "/"))
        Map.merge(widget, %{widget: "#{plugin.name}/#{widget.tool}", opens: opens && opens.app})
      end

    placed =
      Enum.flat_map(kept(call), fn
        %{"app" => app} -> Enum.filter(apps, &(&1.app == app))
        %{"widget" => widget} = item -> for w <- widgets, w.widget == widget, do: Map.put(w, :size, size(w, item))
        _ -> []
      end)

    # An app added since the page was last arranged goes after the rest.
    items = placed ++ Enum.sort_by(apps -- placed, &String.downcase(&1.name))
    %{who: call.for, arranges: Call.owns?(call), items: items, widgets: widgets}
  end

  def call("arrange", %{"items" => items}, call) do
    if not Call.owns?(call), do: raise("#{call.for}'s home page is arranged by their owners.")

    items =
      for item <- items, is_binary(item["app"]) or is_binary(item["widget"]), do: Map.take(item, ~w(app widget size))

    keep(call, JSON.encode!(items))
    "The page is arranged."
  end

  def call("settings", _inputs, call) do
    parts =
      for plugin <- Call.plugins(call), app <- plugin.apps, app.settings do
        %{app: "#{plugin.name}/#{app.page}", name: app.name, icon: app.icon}
      end

    Enum.sort_by(parts, &String.downcase(&1.name))
  end

  # A plugin's deployed code is open to everyone in the network, so every plugin anyone deployed is one anyone may
  # add, whether or not it's on a page now.
  def call("store", _inputs, call) do
    apps =
      for %{plugin: plugin, apps: apps} <- Call.deployed(call), app <- apps, not app.settings do
        %{plugin: plugin, name: app.name, icon: app.icon}
      end

    Enum.sort_by(apps, &String.downcase(&1.name))
  end

  # The size a widget is drawn at: the one it was put on the page at, if it still comes in it.
  defp size(widget, item), do: if(item["size"] in widget.sizes, do: item["size"], else: hd(widget.sizes))

  # How a page is arranged is its owner's: an artifact made for them, which holds the one blob.
  defp kept(call) do
    with {:ok, items, _tag} <- Artifact.read(Call.make(call, "page"), "items"),
         do: JSON.decode!(items),
         else: (:absent -> [])
  end

  defp keep(call, items) do
    page = Call.make(call, "page")

    wrote =
      case Artifact.read(page, "items") do
        {:ok, _was, tag} -> Artifact.replace(page, "items", items, tag)
        :absent -> Artifact.write(page, "items", items)
      end

    if wrote != :ok, do: raise("Someone else arranged this page just now. Look at it, and do this again.")
  end
end
