defmodule HomeScreen do
  @moduledoc """
  Home Screen: where apps and widgets sit. A user has one, and so does each group and the network.

  A page is what's installed for whoever it belongs to, each plugin's apps and the widgets its plugins have, and the
  apps put on it of the plugins they can already use: the network's (`add`), which installs nothing. What this keeps
  is how a page is arranged: the order of its apps, those put on it, and the widgets put on it, each at a size.
  That's the page's owner's, an artifact made for them.

  Installing a plugin puts its code on a server, so for the network only its owners and installers do it. Anyone
  asks them for one (`request`), and they approve or decline (`approve`, `decline`): the requests are the
  network's, in this plugin's own artifact for it.

  A group has a page once it's made, so the groups someone is in, and who's in each, are a part of Settings
  here, over the tools of whatever keeps them (`permissions`).

  Every tool here is asked for whoever's page it is, as an app opened from that page is. What draws the pages is
  this plugin's `client/`: its app in the category `launcher`, which a client shows first.

  A plugin has accounts, outside accounts a person signs it in to, when it has an icon and the tools `accounts`,
  `sign_in` and `sign_out` (SKILL.md). Accounts, a folder at the end of the first page, lists them by plugin. Having
  accounts doesn't make a plugin's app, nor put it on a page.
  """
  @behaviour OneCore.Plugin

  # Every client opens on its Home Screen, and apps are added from its App Store.
  @impl true
  def essential, do: true

  alias OneCore.{Artifact, Call}
  alias OneCore.Plugins.Store

  @impl true
  def apps do
    [
      %{name: "Home Screen", page: "client/index.html", icon: "client/home.svg", category: "launcher"},
      %{name: "Apps", page: "client/store.html", icon: "client/store.svg", category: "settings"},
      %{name: "Groups", page: "client/groups.html", icon: "client/groups.svg", category: "settings"},
      %{name: "Appearance", page: "client/appearance.html", icon: "client/appearance.svg", category: "settings"}
    ]
  end

  # The tools a plugin with accounts has, beside its icon (SKILL.md). A plugin that only serves other plugins' code
  # (google-account) may have tools named the same, and no icon.
  @accounts ~w(accounts sign_in sign_out)
  defp accounts?(%{icon: icon}, tools), do: icon != nil and @accounts -- tools == []

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}

    plugin = %{
      type: "object",
      properties: %{plugin: %{type: "string", description: "The plugin: its name (gmail), or as `store` lists it"}},
      required: ["plugin"]
    }

    request = %{
      type: "object",
      properties: %{id: %{type: "integer", description: "The request, as `requests` lists it"}},
      required: ["id"]
    }

    app = %{
      type: "object",
      properties: %{
        app: %{type: "string", description: "Its plugin's name and its page: notes/client/index.html"},
        plugin: %{type: "string"},
        name: %{type: "string"},
        icon: %{type: "string", description: "A data: address"},
        put: %{type: "boolean", description: "Whether it was put on the page, not installed for it"}
      },
      required: ["app", "plugin", "name", "icon", "put"]
    }

    widget = %{
      type: "object",
      properties: %{
        widget: %{type: "string", description: "Its plugin's name and its tool: notes/glance"},
        name: %{type: "string"},
        tool: %{type: "string"},
        sizes: %{type: "array", items: %{type: "string", enum: ~w(2x2 4x2 4x4)}},
        size: %{type: "string", enum: ~w(2x2 4x2 4x4), description: "Its size on the page"},
        opens: %{type: ["string", "null"], description: "The app pressing it opens"}
      },
      required: ["widget", "name", "tool", "sizes"]
    }

    # A request for a plugin to be installed for everyone.
    asked = %{
      type: "object",
      properties: %{
        id: %{type: "integer"},
        by: %{type: "string", description: "Who asked"},
        reason: %{type: ["string", "null"]},
        at: %{type: "string", description: "When, ISO 8601"},
        state: %{type: "string", enum: ~w(asked approved declined)},
        answered_by: %{type: "string"},
        plugin: %{type: "string", description: "A plugin in the network"},
        source: %{type: "string", description: "Or the git repo it's in"},
        name: %{type: "string", description: "And its name there"}
      },
      required: ["id", "by", "reason", "at", "state"]
    }

    [
      %{
        name: "page",
        read_only: true,
        description: """
        Gives your Home Screen page: its apps and widgets in order (`items`), the widgets that could be put on it
        (`widgets`), the plugins with accounts (`plugins_with_accounts`), which Accounts lists, and whether you may
        arrange it. Asked for a group or for the network, it's theirs.
        """,
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            who: %{type: "string", description: "Whose page"},
            arranges: %{type: "boolean", description: "Whether you may arrange it"},
            items: %{
              type: "array",
              description: "What's on it, in order: each an app or a widget",
              items: %{anyOf: [app, Map.put(widget, :required, ["widget", "name", "tool", "sizes", "size"])]}
            },
            widgets: %{type: "array", items: widget, description: "The widgets that could be put on it"},
            plugins_with_accounts: %{
              type: "array",
              description: "The plugins with accounts, which Accounts lists. None is an app",
              items: %{
                type: "object",
                properties: %{
                  plugin: %{type: "string"},
                  name: %{type: "string", description: "What its tools start with"},
                  title: %{type: "string", description: "Its name, as a person reads it"},
                  icon: %{type: "string", description: "Its own icon, as a data: address"},
                  sign_in: %{type: "string", description: "Its sign_in tool's page, if signing in needs one"}
                },
                required: ["plugin", "name", "title", "icon"]
              }
            }
          },
          required: ["who", "arranges", "items", "widgets", "plugins_with_accounts"]
        }
      },
      %{
        name: "arrange",
        description: """
        Arranges your Home Screen page: its apps and widgets, in the order given. Asked for a group or for the network,
        it arranges theirs, which their owners do. A widget or an app put on it (`add`) not given is taken off the
        page, and an app installed for them not given goes after the rest: `uninstall` takes one of those off.
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
                  app: %{
                    type: "string",
                    description: "An app, as its plugin's name and its page: notes/client/index.html"
                  },
                  widget: %{type: "string", description: "A widget, as its plugin's name and its tool: notes/glance"},
                  size: %{type: "string", enum: ~w(2x2 4x2 4x4), description: "The widget's size"}
                }
              }
            }
          },
          required: ["items"]
        },
        outputs: %{
          type: "object",
          properties: %{
            items: %{type: "array", items: %{type: "object"}, description: "What's on the page now, as given"}
          },
          required: ["items"]
        }
      },
      %{
        name: "settings",
        read_only: true,
        description: "Lists the parts of Settings: each an app of a plugin that says it's one",
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            parts: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  app: %{type: "string", description: "Its plugin's name and its page"},
                  name: %{type: "string"},
                  icon: %{type: "string", description: "A data: address"}
                },
                required: ["app", "name", "icon"]
              }
            }
          },
          required: ["parts"]
        }
      },
      %{
        name: "store",
        read_only: true,
        description:
          "Lists every plugin anyone has deployed that a person could add: one with an icon, and an app on a page if it has apps. Each with what a person reads of it (`subtitle`, `about`) and whether it has accounts (`accounts`)",
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
                  accounts: %{type: "boolean", description: "Whether it has accounts"}
                },
                required: ["plugin", "title", "icon", "subtitle", "about", "accounts"]
              }
            }
          },
          required: ["plugins"]
        }
      },
      %{
        name: "add",
        description: """
        Puts the app of a plugin you can already use, one in use for the network, on your Home Screen page. It
        installs nothing. Asked for a group, on theirs. One not in use for the network is asked for with `request`.
        """,
        inputs: plugin,
        outputs: %{
          type: "object",
          properties: %{
            apps: %{type: "array", items: %{type: "string"}, description: "Its apps, on the page now"},
            installed: %{type: "string", description: "Who the plugin is in use for"}
          },
          required: ["apps", "installed"]
        }
      },
      %{
        name: "remove",
        description:
          "Takes an app put on your page with `add` off it. One installed for you is taken off with `uninstall`.",
        inputs: plugin,
        outputs: %{
          type: "object",
          properties: %{plugin: %{type: "string", description: "The plugin whose app is off the page"}},
          required: ["plugin"]
        }
      },
      %{
        name: "request",
        description: """
        Asks whoever may install plugins for the network to install one for everyone: one in the network, as
        `store` lists it, or one from a git repo, by the repo (owner/repo) and the plugin's name, as
        `one install <owner>/<repo> --plugin <name>` takes them. They're notified, and you are when they answer.
        """,
        inputs: %{
          type: "object",
          properties: %{
            plugin: %{type: "string", description: "A plugin in the network, as `store` lists it"},
            source: %{type: "string", description: "The git repo it's in: owner/repo, a URL or a path"},
            name: %{type: "string", description: "The plugin's name in that repo"},
            reason: %{type: "string", maxLength: 300, description: "Why, in a sentence"}
          }
        },
        outputs:
          Map.merge(asked, %{
            properties:
              Map.put(asked.properties, :told, %{
                type: "array",
                items: %{type: "string"},
                description: "Who was told: whoever may install plugins for everyone"
              }),
            required: ["told" | asked.required]
          })
      },
      %{
        name: "requests",
        read_only: true,
        description: """
        Lists the requests for plugins to be installed for the network, newest first: every one, for whoever may
        install them, and otherwise your own. Each is `asked`, `approved` or `declined`.
        """,
        inputs: none,
        outputs: %{type: "object", properties: %{requests: %{type: "array", items: asked}}, required: ["requests"]}
      },
      %{
        name: "approve",
        description: """
        Approves a request, which whoever may install plugins for the network does. One for a plugin in the network
        is installed for everyone now. One from a git repo is installed by your agent, with the command this
        answers, and approving it again once it's installed marks it approved. The asker is told.
        """,
        inputs: request,
        outputs:
          Map.put(
            asked,
            :properties,
            Map.put(asked.properties, :install, %{
              type: "string",
              description: "For one from a git repo, the command that installs it, after which approve again"
            })
          )
      },
      %{
        name: "decline",
        description: "Declines a request, which whoever may install plugins for the network does. The asker is told.",
        inputs: request,
        outputs: asked
      }
    ]
  end

  @impl true
  def hooks, do: ["renamed"]

  # The apps and widgets of a renamed plugin stay where they were on the page of whoever it's renamed for.
  @impl true
  def hook("renamed", %{"from" => from, "to" => to}, call) do
    stored = stored(call)

    renamed =
      for item <- stored do
        Map.new(item, fn
          {kind, ^from <> "/" <> part} when kind in ["app", "widget"] -> {kind, "#{to}/#{part}"}
          other -> other
        end)
      end

    if renamed != stored, do: store(call, JSON.encode!(renamed))
  end

  @impl true
  def call("page", _inputs, call) do
    stored = stored(call)
    put = for %{"app" => app} <- stored, do: app

    here =
      for plugin <- Call.plugins(call),
          plugin.installed == call.subject or Enum.any?(plugin.apps, &("#{plugin.name}/#{&1.page}" in put)),
          do: plugin

    # An app in a category is somewhere else: Settings, or this one.
    apps =
      for plugin <- here, app <- plugin.apps, app.category == nil do
        %{
          app: "#{plugin.name}/#{app.page}",
          plugin: plugin.plugin,
          name: app.name,
          icon: app.icon,
          put: plugin.installed != call.subject
        }
      end

    # A plugin with accounts is listed in Accounts, and signs one in on its `sign_in`'s page if it has one.
    plugins_with_accounts =
      for plugin <- here, names = Enum.map(plugin.tools, & &1.name), accounts?(plugin, names) do
        page = Enum.find_value(plugin.tools, &(&1.name == "sign_in" && &1[:ui]))
        signs_in = if page, do: %{sign_in: "#{plugin.name}/#{page}"}, else: %{}
        Map.merge(%{plugin: plugin.plugin, name: plugin.name, title: plugin.title, icon: plugin.icon}, signs_in)
      end

    widgets =
      for plugin <- here, widget <- plugin.widgets do
        opens = Enum.find(apps, &String.starts_with?(&1.app, plugin.name <> "/"))
        Map.merge(widget, %{widget: "#{plugin.name}/#{widget.tool}", opens: opens && opens.app})
      end

    placed =
      Enum.flat_map(stored, fn
        %{"app" => app} -> Enum.filter(apps, &(&1.app == app))
        %{"widget" => widget} = item -> for w <- widgets, w.widget == widget, do: Map.put(w, :size, size(w, item))
        _ -> []
      end)

    # An app added since the page was last arranged goes after the rest.
    items = placed ++ Enum.sort_by(apps -- placed, &String.downcase(&1.name))
    plugins_with_accounts = Enum.sort_by(plugins_with_accounts, &String.downcase(&1.title))

    %{
      who: call.subject,
      arranges: Call.owns?(call),
      items: items,
      widgets: widgets,
      plugins_with_accounts: plugins_with_accounts
    }
  end

  def call("arrange", %{"items" => items}, call) do
    if not Call.owns?(call), do: raise("#{call.subject}'s Home Screen page is arranged by their owners.")

    items =
      for item <- items, is_binary(item["app"]) or is_binary(item["widget"]), do: Map.take(item, ~w(app widget size))

    store(call, JSON.encode!(items))
    %{items: items}
  end

  def call("settings", _inputs, call) do
    parts =
      for plugin <- Call.plugins(call), app <- plugin.apps, app.category == "settings" do
        %{app: "#{plugin.name}/#{app.page}", name: app.name, icon: app.icon}
      end

    %{parts: Enum.sort_by(parts, &String.downcase(&1.name))}
  end

  # Every plugin anyone deployed, whether or not it's on a page now. One that isn't in use for a group someone's in
  # or the network, nor shared with them, is listed for them too, and adding it says why it can't be.
  def call("store", _inputs, call) do
    # One whose apps are all somewhere else (Settings, this one) isn't added from here.
    plugins =
      for plugin <- Call.deployed(call),
          plugin.icon != nil and (plugin.apps == [] or Enum.any?(plugin.apps, &(&1.category == nil))),
          do: %{
            plugin: plugin.plugin,
            title: plugin.title,
            icon: plugin.icon,
            subtitle: plugin.subtitle,
            about: plugin.about,
            accounts: accounts?(plugin, plugin.tools)
          }

    %{plugins: Enum.sort_by(plugins, &String.downcase(&1.title))}
  end

  def call("add", %{"plugin" => plugin}, call) do
    if not Call.owns?(call), do: raise("#{call.subject}'s Home Screen page is arranged by their owners.")

    case named(call, plugin) do
      nil ->
        raise "#{plugin} isn't in use for the network, so its app can't go on a page yet. Whoever may install " <>
                "plugins for everyone can, and home-screen_request asks them."

      found ->
        apps = for app <- found.apps, app.category == nil, do: "#{found.name}/#{app.page}"

        if apps == [],
          do: raise("#{plugin} has no app to put on a page. If it has accounts, Accounts lists them already.")

        # One installed for whoever's page this is is on it already.
        if found.installed != call.subject,
          do: store(call, JSON.encode!(Enum.uniq(stored(call) ++ for(app <- apps, do: %{"app" => app}))))

        %{apps: apps, installed: found.installed}
    end
  end

  def call("remove", %{"plugin" => plugin}, call) do
    if not Call.owns?(call), do: raise("#{call.subject}'s Home Screen page is arranged by their owners.")

    case named(call, plugin) do
      %{installed: at, name: name} when at == call.subject ->
        raise "#{plugin} is installed for #{at}: uninstall --name #{name} takes it off."

      %{name: name} ->
        store(call, JSON.encode!(Enum.reject(stored(call), &String.starts_with?(&1["app"] || "", name <> "/"))))
        %{plugin: plugin}

      nil ->
        raise "#{plugin} isn't on the page."
    end
  end

  # A request names a plugin in the network, or a repo and a plugin's name in it, as `one install` takes them.
  def call("request", inputs, call) do
    request =
      case inputs do
        %{"plugin" => plugin} ->
          %{"plugin" => plugin}

        %{"source" => source, "name" => name} ->
          %{"source" => source, "name" => name}

        _ ->
          raise "A request names a plugin in the network (plugin), or a repo and the plugin's name in it (source, name)."
      end

    if installed(call, request), do: raise("#{called(request)} is installed for everyone already.")
    at = DateTime.to_iso8601(DateTime.utc_now(:second))

    request =
      noted(call, fn requests ->
        id = 1 + Enum.max(Enum.map(requests, & &1["id"]), fn -> 0 end)

        request =
          Map.merge(request, %{
            "id" => id,
            "by" => call.actor,
            "reason" => inputs["reason"],
            "at" => at,
            "state" => "asked"
          })

        {[request | requests], request}
      end)

    installers = Call.tool(call, "permissions", "users", %{"artifact" => "network", "relation" => "installer"})["users"]

    text = Enum.join(List.wrap(inputs["reason"]) ++ ["home-screen_approve or home-screen_decline answers it."], "\n")

    for to <- installers,
        do:
          notify(call, if(to == "everyone", do: "network", else: to), "#{call.actor} asks for #{called(request)}", text)

    Map.put(request, "told", installers)
  end

  def call("requests", _inputs, call) do
    requests = requests(call)
    %{requests: if(installs?(call), do: requests, else: Enum.filter(requests, &(&1["by"] == call.actor)))}
  end

  def call("approve", %{"id" => id}, call) do
    request = answering!(call, id)

    cond do
      Map.has_key?(request, "plugin") ->
        # As the approver would install it themselves, which One Core checks as it does theirs.
        case OneCore.Tools.call("install", %{"plugin" => request["plugin"]}, {call.actor, "network"}) do
          {:error, why} -> raise why
          {:forwarded, %{"isError" => true, "content" => [%{"text" => why} | _]}} -> raise why
          _installed -> decided(call, request, "approved")
        end

      installed(call, request) ->
        decided(call, request, "approved")

      true ->
        Map.put(request, "install", "ONE_SUBJECT=network one install #{request["source"]} --plugin #{request["name"]}")
    end
  end

  def call("decline", %{"id" => id}, call), do: decided(call, answering!(call, id), "declined")

  # The size a widget is drawn at: the one it was put on the page at, if it still comes in it.
  # A plugin this page's owner finds, by the name it's installed by, as other tools take it, or by its artifact.
  defp named(call, plugin), do: Enum.find(Call.plugins(call), &(plugin in [&1.name, &1.plugin]))

  defp size(widget, item), do: if(item["size"] in widget.sizes, do: item["size"], else: hd(widget.sizes))

  # How a page is arranged is its owner's: an artifact made for them, which holds the one blob.
  defp stored(call) do
    with {:ok, items, _tag} <- Artifact.read(Call.make(call, "page"), "items"),
         do: JSON.decode!(items),
         else: (:absent -> [])
  end

  # Whether whoever asks may install plugins for the network, as the plugin that keeps who may do what says.
  defp installs?(call) do
    about = %{"who" => call.actor, "relation" => "installer", "artifact" => "network"}
    OneCore.Hooks.validate("reach", about, {call.actor, "network"}) == :ok
  end

  defp called(%{"plugin" => plugin}), do: plugin
  defp called(%{"source" => source, "name" => name}), do: "#{name} from #{source}"

  # Whether what's asked for is installed for the network: the plugin itself, or one installed from that repo by that name.
  defp installed(call, request) do
    Enum.any?(Call.plugins(%{call | subject: "network"}), fn plugin ->
      plugin.installed == "network" and
        case request do
          %{"plugin" => artifact} ->
            plugin.plugin == artifact

          %{"source" => source, "name" => name} ->
            plugin.name == name and Store.about(plugin.plugin)["from"]["source"] == source
        end
    end)
  end

  defp answering!(call, id) do
    if not installs?(call),
      do: raise("Only whoever may install plugins for the network answers a request. home-screen_requests lists yours.")

    case Enum.find(requests(call), &(&1["id"] == id)) do
      nil -> raise "There's no request #{id}. home-screen_requests lists them."
      %{"state" => "asked"} = request -> request
      %{"state" => state} -> raise "Request #{id} was #{state} already."
    end
  end

  defp decided(call, request, state) do
    request = Map.merge(request, %{"state" => state, "answered_by" => call.actor})
    noted(call, fn requests -> {for(r <- requests, do: if(r["id"] == request["id"], do: request, else: r)), nil} end)
    notify(call, request["by"], "Your request for #{called(request)} was #{state}", "#{call.actor} #{state} it.")
    request
  end

  defp notify(call, to, title, text),
    do: Call.tool(call, "notifications", "send", %{"to" => to, "title" => title, "text" => text, "app" => call.plugin})

  defp requests(call) do
    with {:ok, requests, _tag} <- Artifact.read(call.own, "requests"), do: JSON.decode!(requests), else: (:absent -> [])
  end

  # Changes the requests as `change` says of them, which gives them and what to answer. Two at once both land.
  defp noted(call, change) do
    {requests, tag} =
      case Artifact.read(call.own, "requests") do
        {:ok, requests, tag} -> {JSON.decode!(requests), tag}
        :absent -> {[], nil}
      end

    {next, result} = change.(requests)

    wrote =
      if tag,
        do: Artifact.replace(call.own, "requests", JSON.encode!(next), tag),
        else: Artifact.write(call.own, "requests", JSON.encode!(next))

    if wrote == :ok, do: result, else: noted(call, change)
  end

  defp store(call, items) do
    page = Call.make(call, "page")

    wrote =
      case Artifact.read(page, "items") do
        {:ok, _was, tag} -> Artifact.replace(page, "items", items, tag)
        :absent -> Artifact.write(page, "items", items)
      end

    if wrote != :ok, do: raise("Someone else arranged this page just now. Look at it, and do this again.")
  end
end
