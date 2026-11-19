defmodule OneCore.Tools.Servers do
  @moduledoc "One Core's own tools that say which servers act for whom, and which services each runs, as `OneCore.Tools` calls them. "
  import OneCore.Tools.Helpers

  alias OneCore.{Hooks, Apps, Server, Servers, Services, User}

  @acts_for %{
    type: "object",
    properties: %{
      server: %{type: "string", description: "The server's key"},
      for: %{type: "string", description: "Who it acts for, or no longer does"}
    },
    required: ["server", "for"]
  }

  @service %{
    type: "object",
    properties: %{
      app: %{type: "string"},
      service: %{type: "string"},
      server: %{type: "string", description: "The key of the server it runs on, or no longer does"}
    },
    required: ["app", "service", "server"]
  }

  @tools [
    %{
      name: "add_server",
      description: """
      Lets a server act for you: asked for a group or for the network, for them.
      It then reads and writes what's yours, runs the apps installed for you, and takes over when another of your
      servers stops. A new server is its owner's alone, who adds it first. Someone else's acts for you once its
      owner chose you, with `add_member`, and you add it.
      """,
      inputs: %{
        type: "object",
        properties: %{key: %{type: "string", description: "The server's key, which it says when it's started"}},
        required: ["key"]
      },
      outputs: @acts_for
    },
    %{
      name: "remove_server",
      description: """
      Takes a server off your list: asked for a group or for the network, off theirs.
      It no longer acts for you, and is handed nothing of yours.
      """,
      inputs: %{
        type: "object",
        properties: %{key: %{type: "string", description: "The server's key"}},
        required: ["key"]
      },
      outputs: @acts_for
    },
    %{
      name: "add_member",
      description: """
      Chooses someone else a server of yours acts for too: a user, a group or the network.
      It runs the apps they add on its machine, once they add it with `add_server`.
      """,
      inputs: %{
        type: "object",
        properties: %{
          who: %{type: "string", description: "Who: users/<name>, groups/<name> or network"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["who"]
      },
      outputs: %{
        type: "object",
        properties: %{
          server: %{type: "string", description: "The server's key"},
          member: %{type: "string", description: "Who it acts for too"}
        },
        required: ["server", "member"]
      }
    },
    %{
      name: "services",
      read_only: true,
      description: """
      Lists the services your apps have, what each needs of a machine, and the servers that run it, with
      what each server's machine is called. A service is a part of an app that runs on particular machines,
      for what is only there.
      """,
      inputs: %{type: "object", properties: %{}},
      outputs: %{
        type: "object",
        properties: %{
          server: %{type: "string", description: "The key of the server that answered"},
          servers: %{type: "array", items: %{type: "string"}, description: "The keys of the servers acting for you"},
          names: %{
            type: "object",
            additionalProperties: %{type: ["string", "null"]},
            description: "What each server's machine is called, by its key"
          },
          services: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{
                app: %{type: "string"},
                service: %{type: "string"},
                called: %{type: "string", description: "What it's called where a person reads it"},
                needs: %{type: "string", description: "What it needs of a machine"},
                servers: %{type: "array", items: %{type: "string"}, description: "The keys of the servers running it"}
              },
              required: ["app", "service", "called", "needs", "servers"]
            }
          }
        },
        required: ["server", "servers", "names", "services"]
      }
    },
    %{
      name: "install_service",
      description: """
      Runs an app's service on a server from now on.
      That is the server's owner's to decide, knowing what the service needs of the machine, which `services` says.
      """,
      inputs: %{
        type: "object",
        properties: %{
          app: %{type: "string", description: "The app's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["app", "service"]
      },
      outputs: @service
    },
    %{
      name: "uninstall_service",
      description: "Stops running an app's service on a server.",
      inputs: %{
        type: "object",
        properties: %{
          app: %{type: "string", description: "The app's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["app", "service"]
      },
      outputs: @service
    }
  ]

  # What each takes and gives is read once here, as the schemas its inputs and answers are checked against on every
  # call.
  @taking Enum.map(@tools, &OneCore.Apps.Version.schemas/1)

  @doc "Its tools, each as an app says one of its own."
  def tools, do: @taking

  # Whoever a server acts for put it on their list, and its owner chose them. A server with no members yet is a
  # new one, which whoever adds it first is bringing.
  @doc "Runs its tool `name` with `inputs`, as `OneCore.Tools.call/5` hands it on."
  def run("add_server", %{"key" => key}, {_actor, subject} = principal, forward, _request) do
    at(subject, forward)
    reaches!(principal, subject, "owner")

    Servers.invalidate()
    members = Servers.members(key)

    if members != [] and subject not in members,
      do: raise("That server is #{hd(members)}'s, who hasn't chosen #{subject}. Its owner does, with add_member.")

    with {:error, why} <- Servers.add(key, subject), do: raise(why)
    Hooks.observe("joined", %{"server" => key, "for" => subject, "by" => Server.key()}, principal)
    Server.notify_servers([subject])
    %{server: key, for: subject}
  end

  # The server taken off is told too, so that it stops acting for them at once.
  def run("remove_server", %{"key" => key}, {_actor, subject} = principal, forward, _request) do
    at(subject, forward)
    reaches!(principal, subject, "owner")
    acting = Servers.acting(subject)
    with {:error, why} <- Servers.remove(key, subject), do: raise(why)
    Server.notify(acting, [subject])
    %{server: key, for: subject}
  end

  def run("add_member", %{"who" => member} = inputs, principal, _pass, _request) do
    on(inputs["server"], "add_member", inputs, principal, fn ->
      if not User.owner?(member),
        do: raise("A server acts for a user, a group or the network: users/<name>, groups/<name> or network")

      reaches!(principal, owner(), "owner")
      Servers.choose(member)
      Server.notify_servers([member, owner()])
      %{server: Server.key(), member: member}
    end)
  end

  def run("services", _inputs, {_actor, subject}, _pass, _request) do
    servers = Servers.acting(User.owner(subject) || "network")
    names = Map.new(servers, &{&1, Servers.name(&1)})
    %{server: Server.key(), servers: servers, names: names, services: Services.list(subject)}
  end

  def run(
        "install_service",
        %{"app" => name, "service" => service} = inputs,
        {_, subject} = principal,
        _pass,
        _request
      ) do
    on(inputs["server"], "install_service", inputs, principal, fn ->
      reaches!(principal, owner(), "owner")
      with {:error, why} <- Services.install(app(subject, name), service), do: raise(why)
      %{app: name, service: service, server: Server.key()}
    end)
  end

  def run(
        "uninstall_service",
        %{"app" => name, "service" => service} = inputs,
        {_actor, subject} = principal,
        _pass,
        _request
      ) do
    on(inputs["server"], "uninstall_service", inputs, principal, fn ->
      reaches!(principal, owner(), "owner")
      with {:error, why} <- Services.remove(app(subject, name), service), do: raise(why)
      %{app: name, service: service, server: Server.key()}
    end)
  end

  # The app that something running for `who` finds by `name`, as its artifact's name.
  defp app(who, name) do
    case Apps.running(who) do
      %{^name => version} -> version.artifact
      _ -> raise "There's no app named #{name}"
    end
  end
end
