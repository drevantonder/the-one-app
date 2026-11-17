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
  app's `client/`: its app in the category `launcher`, which a client shows first.
  """
  @behaviour OneCore.App

  # Every client opens on its Home Screen.
  @impl true
  def icon, do: "client/home.svg"

  alias OneCore.{Artifact, Call}

  @impl true
  def entrypoints do
    [
      %{name: "Home Screen", page: "client/index.html", category: "launcher"},
      %{
        name: "Colour Mode",
        page: "client/colour-mode.html",
        icon: "client/colour-mode.svg",
        subject: "user"
      }
    ]
  end

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}

    entrypoint = %{
      type: "object",
      properties: %{
        entrypoint: %{type: "string", description: "Its app's name and its page: notes/client/index.html"},
        app: %{type: "string"},
        name: %{type: "string"},
        icon: %{type: "string", description: "A data: address"},
        category: %{type: "string", description: "Its category in the App Library: its own, or else its app's"},
        subject: %{type: "string", description: "`user` for an app opened for whoever opens it"}
      },
      required: ["entrypoint", "app", "name", "icon", "category"]
    }

    widget = %{
      type: "object",
      properties: %{
        widget: %{type: "string", description: "Its app's name and its tool: notes/glance"},
        name: %{type: "string"},
        tool: %{type: "string"},
        sizes: %{type: "array", items: %{type: "string", enum: ~w(2x2 4x2 4x4)}},
        size: %{type: "string", enum: ~w(2x2 4x2 4x4), description: "Its size on the page"},
        opens: %{anyOf: [entrypoint, %{type: "null"}], description: "The app pressing it opens"}
      },
      required: ["widget", "name", "tool", "sizes", "opens"]
    }

    arranged = %{
      type: "object",
      properties: %{
        entrypoint: %{type: "string", description: "An app, as its app's name and its page: notes/client/index.html"},
        widget: %{type: "string", description: "A widget, as its app's name and its tool: notes/glance"},
        size: %{type: "string", enum: ~w(2x2 4x2 4x4), description: "The widget's size"}
      }
    }

    adding = %{
      type: "object",
      properties: %{
        entrypoint: %{
          type: "string",
          description:
            "An app, as `pages` gives it (notes/client/index.html), or an app, by its name or as `app-store_marketplace` lists it, for its first app"
        }
      },
      required: ["entrypoint"]
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
                    items: %{
                      anyOf: [entrypoint, Map.put(widget, :required, ["widget", "name", "tool", "sizes", "opens", "size"])]
                    }
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
                  category: %{type: "string", description: "One of an app's categories (the skill building-apps)"},
                  entrypoints: %{type: "array", items: entrypoint}
                },
                required: ["category", "entrypoints"]
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
                  items: %{
                    type: "array",
                    items: arranged,
                    description: "Each an app or a widget, as `pages` gives them"
                  },
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
        name: "add",
        description: """
        Puts an app on your Home Screen, last on its last page: one installed for you, which is in your App Library,
        or one of an app you can already use, in use for the network, which installs nothing. Asked for a group or
        for the network, on theirs.
        """,
        inputs: adding,
        outputs: %{
          type: "object",
          properties: %{
            entrypoint: %{type: "string", description: "The app, on a page now"},
            installed: %{type: "string", description: "Who its app is in use for"}
          },
          required: ["entrypoint", "installed"]
        }
      },
      %{
        name: "remove",
        description:
          "Takes an app off your Home Screen's pages. It stays in the App Library, and `uninstall` removes it.",
        inputs: adding,
        outputs: %{
          type: "object",
          properties: %{entrypoint: %{type: "string", description: "The app, off the pages"}},
          required: ["entrypoint"]
        }
      }
    ]
  end

  @impl true
  def call("pages", _inputs, call) do
    apps = Call.apps(call)
    stored = stored(call)
    openable = openable(apps)
    library = library(call)
    in_library = for %{entrypoints: entrypoints} <- library, entrypoint <- entrypoints, into: MapSet.new(), do: entrypoint.app
    put = for page <- stored, %{"entrypoint" => id} <- page["items"], entrypoint = openable[id], into: MapSet.new(), do: entrypoint.app

    # The widgets that could go on a page: those of the apps installed for whoever's it is, in its App Library,
    # or with an app on its pages.
    widgets =
      for app <- apps,
          app.installed == call.subject or app.app in in_library or app.app in put,
          widget <- app.widgets do
        opens = Enum.find_value(app.entrypoints, &(&1.category != "launcher" && entry(app, &1)))
        Map.merge(widget, %{widget: "#{app.name}/#{widget.tool}", opens: opens})
      end

    # What's gone since it was put there, as an app that no longer has that entrypoint, isn't there, and a page left
    # with nothing goes, as one emptied by hand does.
    pages =
      for page <- stored,
          items =
            Enum.flat_map(page["items"], fn
              %{"entrypoint" => id} -> List.wrap(openable[id])
              %{"widget" => id} = item -> for w <- widgets, w.widget == id, do: Map.put(w, :size, size(w, item))
              _ -> []
            end),
          items != [],
          do: %{items: items, hidden: page["hidden"] == true}

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

  def call("add", %{"entrypoint" => entrypoint}, call) do
    owner!(call)
    apps = Call.apps(call)

    case named(apps, entrypoint) do
      nil ->
        raise "#{entrypoint} isn't installed for #{call.subject}, nor in use for the network, so it can't go on a page. " <>
                "Install it first."

      {app, found} ->
        id = "#{app.name}/#{found.page}"
        store(call, &last(&1, %{"entrypoint" => id}))
        %{entrypoint: id, installed: app.installed}
    end
  end

  def call("remove", %{"entrypoint" => entrypoint}, call) do
    owner!(call)

    case named(Call.apps(call), entrypoint) do
      nil ->
        raise "#{entrypoint} isn't on #{call.subject}'s pages."

      {app, found} ->
        id = "#{app.name}/#{found.page}"

        store(call, fn pages ->
          for page <- pages,
              items = Enum.reject(page["items"], &(&1["entrypoint"] == id)),
              items != [],
              do: %{page | "items" => items}
        end)

        %{entrypoint: id}
    end
  end

  # Every app whoever this is for can open, by its app's name and its page: theirs and the network's.
  defp openable(apps) do
    for app <- apps, entrypoint <- app.entrypoints, entrypoint.category != "launcher", into: %{} do
      {"#{app.name}/#{entrypoint.page}", entry(app, entrypoint)}
    end
  end

  # An app as `pages` gives it, on a page or in the App Library: what it's opened as, its category, and who it's opened
  # for, if not whoever's Home Screen it's on. It's in its own category where it says one of an app's, and else in its
  # app's.
  defp entry(app, entrypoint) do
    about = if entrypoint[:subject], do: %{subject: entrypoint[:subject]}, else: %{}
    category = if entrypoint.category in OneCore.Apps.Deploy.categories(), do: entrypoint.category, else: app.category

    Map.merge(
      %{
        entrypoint: "#{app.name}/#{entrypoint.page}",
        app: app.app,
        name: entrypoint.name,
        icon: entrypoint.icon,
        category: category
      },
      about
    )
  end

  # The App Library: every app that runs for whoever it's for, by category, in the order an app's categories are
  # listed (`OneCore.Apps.Deploy.categories/0`), Admin only for the network's owners. That's what's installed for them
  # and for each broader owner they're within: a user's has theirs, their groups' and the network's, a group's its own
  # and the network's. An app installed for more than one of them is there once, as the narrowest's. An app about
  # whoever opens it is only in a user's.
  defp library(call) do
    entrypoints =
      for scope <- scopes(call),
          app <- Call.apps(%{call | subject: scope}),
          app.installed == scope,
          entrypoint <- app.entrypoints,
          entrypoint.category != "launcher",
          match?("users/" <> _, call.subject) or entrypoint[:subject] != "user" do
        {{app.app, entrypoint.page}, opened(entry(app, entrypoint), call.subject, scope)}
      end
      |> Enum.uniq_by(&elem(&1, 0))
      |> Enum.map(&elem(&1, 1))

    admins = Call.owns?(%{call | subject: "network"})

    for category <- OneCore.Apps.Deploy.categories(),
        category != "Admin" or admins,
        theirs = Enum.filter(entrypoints, &(&1.category == category)),
        theirs != [],
        do: %{category: category, entrypoints: Enum.sort_by(theirs, &String.downcase(&1.name))}
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

  # An app's entrypoint by what `add` and `remove` take: the app as `pages` gives it, or the app by its name or its
  # artifact, for its first.
  defp named(apps, entrypoint) do
    Enum.find_value(apps, fn app ->
      entrypoints = Enum.reject(app.entrypoints, &(&1.category == "launcher"))

      cond do
        found = Enum.find(entrypoints, &("#{app.name}/#{&1.page}" == entrypoint)) -> {app, found}
        entrypoint in [app.name, app.app] and entrypoints != [] -> {app, hd(entrypoints)}
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
      for item <- items, is_binary(item["entrypoint"]) or is_binary(item["widget"]), do: Map.take(item, ~w(entrypoint widget size))

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
    with {:ok, pages, _tag} <- Artifact.read(Call.make(call, "home"), "pages"),
         do: JSON.decode!(pages),
         else: (:absent -> [])
  end

  # Changes the pages as `change` says of them, in one step, so two changes at once both land.
  defp store(call, change) do
    Artifact.update(Call.make(call, "home"), "pages", fn pages ->
      JSON.encode!(change.(if pages, do: JSON.decode!(pages), else: []))
    end)
  end
end
