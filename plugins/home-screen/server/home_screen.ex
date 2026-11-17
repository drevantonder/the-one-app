defmodule HomeScreen do
  @moduledoc """
  Home Screen: where apps and widgets sit. A user has one, and so does each group and the network: pages, and an App
  Library after them.

  Its pages hold what its owner put on them, apps and widgets, page by page in the order they arranged them, each page
  shown or hidden: an artifact made for them.
  The App Library holds every app that runs for whoever it's for, by category: what's installed for them and for each
  broader owner they're within (a user's has their groups' and the network's too, a group's the network's), each app
  once. An app installed since goes there, not on a page. An app about whoever opens it (`subject: "user"`, as Vault
  and Devices say) is only in a user's, and is opened for them.

  Every tool here is asked for whoever's Home Screen it is, as an app opened from it is. What draws them is this
  plugin's `client/`: its app in the category `launcher`, which a client shows first.
  """
  @behaviour OneCore.Plugin

  # Every client opens on its Home Screen.
  @impl true
  def icon, do: "client/home.svg"

  alias OneCore.{Artifact, Call}

  # The categories an app says it's in, as freedesktop's menus have theirs, in the order an App Library shows them.
  # An app that says none of these is in Other, last, as such a menu puts what's left (SKILL.md).
  @categories [
    {"coding", "Coding"},
    {"device-automation", "Device Automation"},
    {"admin", "Admin"},
    {"appearance", "Appearance"},
    {"agents", "Agents"},
    {"devices", "Devices"},
    {"access", "Access"}
  ]

  @impl true
  def apps do
    [
      %{name: "Home Screen", page: "client/index.html", category: "launcher"},
      %{name: "App Store", page: "client/store.html", icon: "client/store.svg", category: "admin", subject: "user"},
      %{
        name: "Colour Mode",
        page: "client/colour-mode.html",
        icon: "client/colour-mode.svg",
        category: "appearance",
        subject: "user"
      }
    ]
  end

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}

    app = %{
      type: "object",
      properties: %{
        app: %{type: "string", description: "Its plugin's name and its page: notes/client/index.html"},
        plugin: %{type: "string"},
        name: %{type: "string"},
        icon: %{type: "string", description: "A data: address"},
        category: %{type: ["string", "null"], description: "Its category, as the App Library has them"},
        subject: %{type: "string", description: "`user` for an app opened for whoever opens it"}
      },
      required: ["app", "plugin", "name", "icon", "category"]
    }

    widget = %{
      type: "object",
      properties: %{
        widget: %{type: "string", description: "Its plugin's name and its tool: notes/glance"},
        name: %{type: "string"},
        tool: %{type: "string"},
        sizes: %{type: "array", items: %{type: "string", enum: ~w(2x2 4x2 4x4)}},
        size: %{type: "string", enum: ~w(2x2 4x2 4x4), description: "Its size on the page"},
        opens: %{anyOf: [app, %{type: "null"}], description: "The app pressing it opens"}
      },
      required: ["widget", "name", "tool", "sizes", "opens"]
    }

    arranged = %{
      type: "object",
      properties: %{
        app: %{type: "string", description: "An app, as its plugin's name and its page: notes/client/index.html"},
        widget: %{type: "string", description: "A widget, as its plugin's name and its tool: notes/glance"},
        size: %{type: "string", enum: ~w(2x2 4x2 4x4), description: "The widget's size"}
      }
    }

    adding = %{
      type: "object",
      properties: %{
        app: %{
          type: "string",
          description:
            "An app, as `pages` gives it (notes/client/index.html), or a plugin, by its name or as `store` lists it, for its first app"
        }
      },
      required: ["app"]
    }

    [
      %{
        name: "pages",
        read_only: true,
        description: """
        Gives your Home Screen: its pages, in order, each what's on it and whether it's hidden (`pages`), its App
        Library, every app that runs for you by category, yours, your groups' and everyone's (`library`), the widgets
        that could be put on a page (`widgets`), and whether you may arrange it. Asked for a group or for the network, it's theirs.
        """,
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            who: %{type: "string", description: "Whose Home Screen"},
            arranges: %{type: "boolean", description: "Whether you may arrange it"},
            pages: %{
              type: "array",
              description: "Its pages, in order",
              items: %{
                type: "object",
                properties: %{
                  items: %{
                    type: "array",
                    description: "What's on it, in order: each an app or a widget",
                    items: %{anyOf: [app, Map.put(widget, :required, ["widget", "name", "tool", "sizes", "opens", "size"])]}
                  },
                  hidden: %{type: "boolean", description: "Whether it's hidden, kept but not shown"}
                },
                required: ["items", "hidden"]
              }
            },
            library: %{
              type: "array",
              description: "Its App Library: each category with an app, in order, and its apps by name",
              items: %{
                type: "object",
                properties: %{
                  category: %{type: ["string", "null"], description: "As an app says it, or null for Other"},
                  name: %{type: "string", description: "As a person reads it"},
                  apps: %{type: "array", items: app}
                },
                required: ["category", "name", "apps"]
              }
            },
            widgets: %{type: "array", items: widget, description: "The widgets that could be put on a page"}
          },
          required: ["who", "arranges", "pages", "library", "widgets"]
        }
      },
      %{
        name: "arrange",
        description: """
        Arranges your Home Screen's pages: each page's apps and widgets, in order, and whether it's hidden (`pages`),
        or the apps and widgets alone (`items`), which fill each page before the next. One not given is taken off, and
        stays in the App Library; a page with nothing on it goes. Asked for a group or for the network, it arranges
        theirs, for everyone who sees it, which their owners do.
        """,
        inputs: %{
          type: "object",
          properties: %{
            pages: %{
              type: "array",
              description: "Each page, in order",
              items: %{
                type: "object",
                properties: %{
                  items: %{type: "array", items: arranged, description: "Each an app or a widget, as `pages` gives them"},
                  hidden: %{type: "boolean", description: "Whether it's hidden, kept but not shown"}
                },
                required: ["items"]
              }
            },
            items: %{type: "array", items: arranged, description: "Each an app or a widget, as `pages` gives them"}
          }
        },
        outputs: %{
          type: "object",
          properties: %{
            pages: %{
              type: "array",
              items: %{type: "object"},
              description: "The pages now, each its `items` and, if it's hidden, `hidden`"
            }
          },
          required: ["pages"]
        }
      },
      %{
        name: "store",
        read_only: true,
        description:
          "Lists every plugin anyone has deployed that a person could add: one with an icon that not every network has. Each as it's listed: what a person reads of it (`subtitle`, `about`), what it's for (`category`), and its apps and widgets",
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            plugins: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  plugin: %{type: "string", description: "Its artifact: users/alice/plugins/notes"},
                  title: %{type: "string", description: "Its name, as a person reads it"},
                  icon: %{type: "string", description: "A data: address"},
                  subtitle: %{type: ["string", "null"]},
                  about: %{type: ["string", "null"]},
                  category: %{type: ["string", "null"], description: "What it's for: Communication, Productivity, ..."},
                  apps: %{type: "array", description: "Each one's name, icon and App Library category"},
                  widgets: %{type: "array", description: "Each one's name and sizes"}
                },
                required: ["plugin", "title", "icon", "subtitle", "about", "category", "apps", "widgets"]
              }
            }
          },
          required: ["plugins"]
        }
      },
      %{
        name: "add",
        description: """
        Puts an app on your Home Screen, last on its last page: one installed for you, which is in your App Library,
        or one of a plugin you can already use, in use for the network, which installs nothing. Asked for a group or
        for the network, on theirs.
        """,
        inputs: adding,
        outputs: %{
          type: "object",
          properties: %{
            app: %{type: "string", description: "The app, on a page now"},
            installed: %{type: "string", description: "Who its plugin is in use for"}
          },
          required: ["app", "installed"]
        }
      },
      %{
        name: "remove",
        description: "Takes an app off your Home Screen's pages. It stays in the App Library, and `uninstall` removes it.",
        inputs: adding,
        outputs: %{
          type: "object",
          properties: %{app: %{type: "string", description: "The app, off the pages"}},
          required: ["app"]
        }
      }
    ]
  end

  @impl true
  def call("pages", _inputs, call) do
    plugins = Call.plugins(call)
    stored = stored(call)
    openable = openable(plugins)
    library = library(call)
    in_library = for %{apps: apps} <- library, app <- apps, into: MapSet.new(), do: app.plugin
    put = for page <- stored, %{"app" => id} <- page["items"], app = openable[id], into: MapSet.new(), do: app.plugin

    # The widgets that could go on a page: those of the plugins installed for whoever's it is, in its App Library,
    # or with an app on its pages.
    widgets =
      for plugin <- plugins,
          plugin.installed == call.subject or plugin.plugin in in_library or plugin.plugin in put,
          widget <- plugin.widgets do
        opens = Enum.find_value(plugin.apps, &(&1.category != "launcher" && entry(plugin, &1)))
        Map.merge(widget, %{widget: "#{plugin.name}/#{widget.tool}", opens: opens})
      end

    pages =
      for page <- stored do
        items =
          Enum.flat_map(page["items"], fn
            %{"app" => id} -> List.wrap(openable[id])
            %{"widget" => id} = item -> for w <- widgets, w.widget == id, do: Map.put(w, :size, size(w, item))
            _ -> []
          end)

        %{items: items, hidden: page["hidden"] == true}
      end

    %{
      who: call.subject,
      arranges: Call.owns?(call),
      pages: pages,
      library: library,
      widgets: widgets
    }
  end

  def call("arrange", inputs, call) do
    owner!(call)

    pages =
      case inputs do
        %{"pages" => pages} when is_list(pages) ->
          for %{"items" => items} = page <- pages, is_list(items), do: paged(items, page["hidden"] == true)

        %{"items" => items} when is_list(items) ->
          [paged(items, false)]

        _ ->
          raise "Arranging takes the pages, each with its items (pages), or the items alone (items)."
      end

    # A page with nothing on it goes, as a phone's does.
    pages = Enum.reject(pages, &(&1["items"] == []))
    store(call, fn _ -> pages end)
    %{pages: pages}
  end

  # Every plugin anyone deployed that a person could add, whether or not anyone has it now.
  def call("store", _inputs, call) do
    plugins =
      for plugin <- Call.deployed(call),
          plugin.icon != nil,
          do: Map.take(plugin, [:plugin, :title, :icon, :subtitle, :about, :category, :apps, :widgets])

    %{plugins: Enum.sort_by(plugins, &String.downcase(&1.title))}
  end

  def call("add", %{"app" => app}, call) do
    owner!(call)
    plugins = Call.plugins(call)

    case named(plugins, app) do
      nil ->
        raise "#{app} isn't installed for #{call.subject}, nor in use for the network, so it can't go on a page. " <>
                "Install it first."

      {plugin, found} ->
        id = "#{plugin.name}/#{found.page}"
        store(call, &last(&1, %{"app" => id}))
        %{app: id, installed: plugin.installed}
    end
  end

  def call("remove", %{"app" => app}, call) do
    owner!(call)

    case named(Call.plugins(call), app) do
      nil ->
        raise "#{app} isn't on #{call.subject}'s pages."

      {plugin, found} ->
        id = "#{plugin.name}/#{found.page}"

        store(call, fn pages ->
          for page <- pages, items = Enum.reject(page["items"], &(&1["app"] == id)), items != [],
              do: %{page | "items" => items}
        end)

        %{app: id}
    end
  end

  # Every app whoever this is for can open, by its plugin's name and its page: theirs and the network's.
  defp openable(plugins) do
    for plugin <- plugins, app <- plugin.apps, app.category != "launcher", into: %{} do
      {"#{plugin.name}/#{app.page}", entry(plugin, app)}
    end
  end

  # An app as `pages` gives it, on a page or in the App Library: what it's opened as, and who it's opened for, if not whoever's
  # Home Screen it's on.
  defp entry(plugin, app) do
    about = if app[:subject], do: %{subject: app[:subject]}, else: %{}

    Map.merge(
      %{app: "#{plugin.name}/#{app.page}", plugin: plugin.plugin, name: app.name, icon: app.icon, category: app.category},
      about
    )
  end

  # The App Library: every app that runs for whoever it's for, by category, in the order of `@categories`, then Other.
  # That's what's installed for them and for each broader owner they're within: a user's has theirs, their groups'
  # and the network's, a group's its own and the network's. A plugin installed for more than one of them is there
  # once, as the narrowest's. An app about whoever opens it is only in a user's.
  defp library(call) do
    apps =
      for scope <- scopes(call),
          plugin <- Call.plugins(%{call | subject: scope}),
          plugin.installed == scope,
          app <- plugin.apps,
          app.category != "launcher",
          match?("users/" <> _, call.subject) or app[:subject] != "user" do
        {{plugin.plugin, app.page}, opened(entry(plugin, app), call.subject, scope)}
      end
      |> Enum.uniq_by(&elem(&1, 0))
      |> Enum.map(&elem(&1, 1))

    known = for {category, name} <- @categories, do: {category, name, Enum.filter(apps, &(&1.category == category))}
    other = {nil, "Other", Enum.reject(apps, &List.keymember?(@categories, &1.category, 0))}

    for {category, name, apps} <- known ++ [other], apps != [],
        do: %{category: category, name: name, apps: Enum.sort_by(apps, &String.downcase(&1.name))}
  end

  # Whoever an App Library is for, then each broader owner they're within, narrowest first.
  defp scopes(%{subject: "network"}), do: ["network"]
  defp scopes(%{subject: "users/" <> _ = user} = call), do: [user | groups(call)] ++ ["network"]
  defp scopes(%{subject: group}), do: [group, "network"]

  # The groups a user is in, as `permissions` keeps them. A network without it has none.
  defp groups(call) do
    for %{"group" => group} <- Call.tool(call, "permissions", "groups", %{})["groups"], do: group
  rescue
    _ -> []
  end

  # A group's app in a user's App Library opens for the group, whose install it is.
  defp opened(entry, "users/" <> _, "groups/" <> _ = group) when not is_map_key(entry, :subject),
    do: Map.put(entry, :subject, group)

  defp opened(entry, _subject, _scope), do: entry

  # A plugin's app by what `add` and `remove` take: the app as `pages` gives it, or the plugin by its name or its
  # artifact, for its first.
  defp named(plugins, app) do
    Enum.find_value(plugins, fn plugin ->
      apps = Enum.reject(plugin.apps, &(&1.category == "launcher"))

      cond do
        found = Enum.find(apps, &("#{plugin.name}/#{&1.page}" == app)) -> {plugin, found}
        app in [plugin.name, plugin.plugin] and apps != [] -> {plugin, hd(apps)}
        true -> nil
      end
    end)
  end

  defp owner!(call),
    do: if(not Call.owns?(call), do: raise("#{call.subject}'s Home Screen is arranged by their owners."))

  # The size a widget is drawn at: the one it was put on the page at, if it still comes in it.
  defp size(widget, item), do: if(item["size"] in widget.sizes, do: item["size"], else: hd(widget.sizes))

  # A page as it's kept: its apps and widgets, as arranging names them, and `hidden` only where it is.
  defp paged(items, hidden) do
    items =
      for item <- items, is_binary(item["app"]) or is_binary(item["widget"]), do: Map.take(item, ~w(app widget size))

    if hidden, do: %{"items" => items, "hidden" => true}, else: %{"items" => items}
  end

  # The pages with `item` last on the last one shown, or on a new one, unless it's on one already.
  defp last(pages, item) do
    shown = pages |> Enum.with_index() |> Enum.reject(fn {page, _} -> page["hidden"] end) |> List.last()

    cond do
      Enum.any?(pages, &(item in &1["items"])) -> pages
      shown -> List.update_at(pages, elem(shown, 1), &%{&1 | "items" => &1["items"] ++ [item]})
      true -> pages ++ [%{"items" => [item]}]
    end
  end

  # How a Home Screen's pages are arranged is its owner's: an artifact made for them, which holds the one blob, every
  # page with what's on it.
  defp stored(call) do
    with {:ok, pages, _tag} <- Artifact.read(Call.make(call, "page"), "pages"),
         do: JSON.decode!(pages),
         else: (:absent -> [])
  end

  # Changes the pages as `change` says of them, in one step, so two changes at once both land.
  defp store(call, change) do
    Artifact.update(Call.make(call, "page"), "pages", fn pages ->
      JSON.encode!(change.(if pages, do: JSON.decode!(pages), else: []))
    end)
  end
end
