defmodule OneCore.Tools.Servers do
  @moduledoc "One Core's own tools that say which servers act for whom, and which services each runs, as `OneCore.Tools` calls them, with their definitions in `OneCore.Tools.Definitions`."
  import OneCore.Tools.Helpers
  alias OneCore.{Hooks, Apps, Server, Servers, Services, User}

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
    acting = OneCore.Apps.covering(subject)
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
    servers = OneCore.Apps.covering(User.owner(subject) || "network")
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
