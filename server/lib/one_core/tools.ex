defmodule OneCore.Tools do
  @moduledoc """
  One Core's own tools: what adds, updates and removes plugins, servers and services.

  They're called as a plugin's tools are, through the same hooks (`OneCore.Plugins.tool/5`): the plugins that
  change a call's inputs change theirs, the ones that decide a call decide theirs, and the ones that hear are
  told. What each changes it reaches as a plugin reaches an artifact, so the plugins that decide `reach` say
  whether whoever asked may: an `editor` of a plugin deploys over it, a `viewer` adds it, and only an `owner` of
  a user, a group or the network changes what's added for them and which servers act for them. A server is its
  owner's: who it was started for, and only their `owner` says who else it acts for and what it runs. Nothing is
  asked about what's whoever's asked, which is what their name starts. With no plugin deciding, everything in a
  network is shared.

  What a tool changes is whoever's it is, so a server acting for them runs it: the one asked, or one it passes
  the call on to.
  """
  alias OneCore.{Artifact, Call, Hooks, Plugins, Server, Servers, Services, User}

  @tools [
    %{
      name: "deploy",
      description: """
      Adds a plugin for you, or updates it, from its files.
      The server compiles them and runs the plugin at once, with its tools listed and called as <name>_<tool>. If
      they don't compile, it says why, and the version already running stays. The server's instructions say how to
      write a plugin. It's yours, and is added for you: asked for a group or for the network, it's theirs.
      """,
      inputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The plugin's name: lowercase letters, digits and hyphens"},
          files: %{type: "object", description: "The plugin's files: each one's text, by its path in the plugin"}
        },
        required: ["name", "files"]
      }
    },
    %{
      name: "add",
      description: """
      Adds a plugin someone else deployed for you: asked for a group or for the network, for them.
      It's the same plugin, at its newest version, with an artifact of its own for whoever it's added for. Whoever
      it belongs to shares it with you first, as a viewer.
      """,
      inputs: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin, by its artifact's name: users/alice/plugins/notes"},
          name: %{type: "string", description: "What to call it. Its own name, if left out"}
        },
        required: ["plugin"]
      }
    },
    %{
      name: "remove",
      description: "Removes a plugin from what's added for you. It stays whoever's it is, and what it kept stays.",
      inputs: %{
        type: "object",
        properties: %{name: %{type: "string", description: "The plugin's name"}},
        required: ["name"]
      }
    },
    %{
      name: "upload",
      description: """
      Gives somewhere to send a file that's too large to go in a call: a link that takes it once, within a minute.
      Send the file as the link says, by its `method` to its `url` with its `headers`, then give `upload`, its
      name, to the plugin's tool that takes one. `one <plugin> <tool> --<input>-from <path>` does all of this.
      """,
      inputs: %{
        type: "object",
        properties: %{plugin: %{type: "string", description: "The plugin whose tool the file is for"}},
        required: ["plugin"]
      }
    },
    %{
      name: "add_server",
      description: """
      Lets a server act for you: asked for a group or for the network, for them.
      It then reads and writes what's yours, runs the plugins added for you, and takes over when another of your
      servers stops. A new server is its owner's alone, who adds it first. Someone else's acts for you once its
      owner chose you, with `add_member`, and you add it.
      """,
      inputs: %{
        type: "object",
        properties: %{key: %{type: "string", description: "The server's key, which it says when it's started"}},
        required: ["key"]
      }
    },
    %{
      name: "add_member",
      description: """
      Chooses someone else a server of yours acts for too: a user, a group or the network.
      It runs the plugins they add on its machine, once they add it with `add_server`.
      """,
      inputs: %{
        type: "object",
        properties: %{
          who: %{type: "string", description: "Who: users/<name>, groups/<name> or network"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["who"]
      }
    },
    %{
      name: "services",
      read_only: true,
      description: """
      Lists the services your plugins have, what each needs of a machine, and the servers that run it.
      A service is a part of a plugin that runs on particular machines, for what is only there.
      """,
      inputs: %{type: "object", properties: %{}}
    },
    %{
      name: "install_service",
      description: """
      Runs a plugin's service on a server from now on.
      That is the server's owner's to decide, knowing what the service needs of the machine, which `services` says.
      """,
      inputs: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["plugin", "service"]
      }
    },
    %{
      name: "remove_service",
      description: "Stops running a plugin's service on a server.",
      inputs: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["plugin", "service"]
      }
    }
  ]

  # What each takes is read once here, as the schema its inputs are checked against on every call.
  @taking Enum.map(@tools, &Map.put(&1, :takes, ExJsonSchema.Schema.resolve(JSON.decode!(JSON.encode!(&1.inputs)))))

  @doc "One Core's own tools, each as a plugin says one of its own."
  def tools, do: @taking

  @doc """
  Calls one of One Core's own tools for `asked`, who asked and who for. Gives what `OneCore.Plugins.call/5` does,
  or `:none` if `name` isn't one.
  """
  def call(name, inputs, asked, passed_on \\ false) do
    case Enum.find(@taking, &(&1.name == name)) do
      nil ->
        :none

      tool ->
        # What's passed on is what was asked, so the server it's passed to hooks into the same thing.
        pass = &Call.pass_to({"tools/call", %{name: name, arguments: inputs}}, asked, passed_on, &1)
        about = %{"plugin" => nil, "tool" => name, "inputs" => inputs}
        # A plugin has no say in its own removal, or in being deployed over: that's how one that breaks is undone.
        apart = if name in ["deploy", "remove"], do: {User.owner(elem(asked, 1)), inputs["name"]}
        Plugins.tool(tool, about, asked, nil, &run(name, &1, asked, pass), apart)
    end
  end

  # Has a server acting for `owner` run this call, if this one doesn't act for them: `pass` ends it here.
  defp at(owner, pass), do: Servers.here?(owner) || pass.(owner)

  # A plugin is whoever's deployed it. Deploying over one is changing it, and a new one is added for whoever it's
  # deployed for.
  defp run("deploy", %{"name" => name, "files" => files}, {_by, who} = asked, pass) do
    case Plugins.plugin(who, name) do
      nil -> at(who, pass) && reaches!(asked, who, "owner")
      plugin -> at(User.owner(plugin), pass) && reaches!(asked, plugin, "editor")
    end

    case Plugins.deploy(who, name, files) do
      {:ok, number, ""} -> "#{name} is running, at version #{number}."
      {:ok, number, warnings} -> "#{name} is running, at version #{number}. The compiler warns:\n#{warnings}"
      {:error, why} -> raise why
    end
  end

  defp run("add", %{"plugin" => plugin} = inputs, {_by, who} = asked, pass) do
    name = Map.get(inputs, "name", Path.basename(plugin))
    at(who, pass)
    reaches!(asked, plugin, "viewer")
    reaches!(asked, who, "owner")
    with {:error, why} <- Plugins.add(who, name, plugin), do: raise(why)
    "#{plugin} is added for #{who}, as #{name}."
  end

  defp run("remove", %{"name" => name}, {_by, who} = asked, pass) do
    at(who, pass)
    reaches!(asked, who, "owner")
    with {:error, why} <- Plugins.remove(who, name), do: raise(why)
    "#{name} is removed from what's added for #{who}."
  end

  # A file sent this way lands in the plugin's own artifact, for whoever it was added for, under a name nobody
  # could guess.
  defp run("upload", %{"plugin" => plugin}, {_by, who}, pass) do
    with %{^plugin => version} <- Plugins.running(who),
         {added, ^plugin} <- Plugins.added(who, version.artifact) do
      at(added, pass)
      name = "uploads/" <> Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
      own = %Artifact{prefix: "#{added}/own/#{plugin}/"}
      JSON.encode!(Map.put(Artifact.link(own, name, :write), :upload, name))
    else
      _ -> raise "There's no plugin named #{plugin}"
    end
  end

  # Whoever a server acts for put it on their list, and its owner chose them. A server with no members yet is a
  # new one, which whoever adds it first is bringing.
  defp run("add_server", %{"key" => key}, {_by, who} = asked, pass) do
    at(who, pass)
    reaches!(asked, who, "owner")

    Servers.forget()

    if Servers.members(key) != [] and who not in Servers.members(key),
      do:
        raise(
          "That server is #{hd(Servers.members(key))}'s, who hasn't chosen #{who}. Its owner does, with add_member."
        )

    with {:error, why} <- Servers.add(key, who), do: raise(why)
    Hooks.heard("joined", %{"server" => key, "for" => who, "by" => Server.key()}, asked)
    Server.tell_servers([who])
    "That server acts for #{who} now, once it's started for them or its owner has chosen them."
  end

  defp run("add_member", %{"who" => member} = inputs, asked, _pass) do
    on(inputs["server"], "add_member", inputs, asked, fn ->
      if not User.owner?(member),
        do: raise("A server acts for a user, a group or the network: users/<name>, groups/<name> or network")

      reaches!(asked, owner(), "owner")
      Servers.choose(member)
      Server.tell_servers([member, owner()])
      "The server #{Server.key()} acts for #{member} too, once they add it with add_server."
    end)
  end

  defp run("services", _inputs, {_by, who}, _pass) do
    owner = User.owner(who) || "network"
    JSON.encode!(%{asked: Server.key(), servers: Servers.acting(owner), services: Services.list(who)})
  end

  defp run("install_service", %{"plugin" => name, "service" => service} = inputs, {_by, who} = asked, _pass) do
    on(inputs["server"], "install_service", inputs, asked, fn ->
      reaches!(asked, owner(), "owner")
      with {:error, why} <- Services.install(plugin(who, name), service), do: raise(why)
      "#{service} is running on the server #{Server.key()}."
    end)
  end

  defp run("remove_service", %{"plugin" => name, "service" => service} = inputs, {_by, who} = asked, _pass) do
    on(inputs["server"], "remove_service", inputs, asked, fn ->
      reaches!(asked, owner(), "owner")
      with {:error, why} <- Services.remove(plugin(who, name), service), do: raise(why)
      "#{service} no longer runs on the server #{Server.key()}."
    end)
  end

  # Whose this server is: who it was started for.
  defp owner, do: hd(Servers.members(Server.key()))

  # Raises unless whoever asked may reach the artifact as `relation`, which the plugins that decide `reach` say.
  # Nothing is asked about what's their own: themselves, and what their name starts.
  defp reaches!({by, _who} = asked, artifact, relation) do
    if not (artifact == by or String.starts_with?(artifact, by <> "/")) do
      about = %{"who" => by, "relation" => relation, "artifact" => artifact}
      with {:refuse, why} <- Hooks.decide("reach", about, asked), do: raise(why)
    end
  end

  # The plugin that something running for `who` finds by `name`, as its artifact's name.
  defp plugin(who, name) do
    case Plugins.running(who) do
      %{^name => version} -> version.artifact
      _ -> raise "There's no plugin named #{name}"
    end
  end

  # Runs what a tool does on the server it names: here, or by asking that server to, for whoever asked, which
  # ends the call here with its answer.
  defp on(server, tool, inputs, {by, who}, here) do
    if server in [nil, Server.key()] do
      here.()
    else
      case Server.request(server, "tools/call", %{name: tool, arguments: inputs, _meta: %{who: by, for: who}}) do
        {:ok, result} -> throw({Call, {:passed_on, result}})
        {:error, _code, why} -> raise why
        down -> raise Server.said(down)
      end
    end
  end
end
