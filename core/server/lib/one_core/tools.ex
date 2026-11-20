defmodule OneCore.Tools do
  @moduledoc """
  One Core's own tools: what installs, updates and removes apps, servers and services.

  They're called as an app's tools are, through the same hooks (`OneCore.Apps.Calls.tool/5`): the apps that
  mutate a call's inputs mutate theirs, the ones that validate a call validate theirs, and the ones that observe
  are told. One that says `exempts_app: true` removes or deploys over the app its input `name` names, which has no
  say in it (`OneCore.Hooks`): that's how an app that breaks is undone. What each changes it reaches as an app reaches an artifact, so the apps that validate `reach` say
  whether whoever asked may: an `editor` of an app deploys over it, a `viewer` reads its code and installs it, and
  so does a `member` of the group or the network it's whoever's or installed for, since what's in use is open to
  everyone it's in use for. An `installer` of a user, a group or the network adds apps for them, which an `owner`
  is, and only an `owner` removes them and says which servers act for them. An app is added only where a server
  would run it (`OneCore.Apps.runs?/3`): for a user or a group, one of their own. A server is its owner's: who it was started for, and only their `owner` says
  who else it acts for and what it runs. Nothing is asked about what's whoever's asked, which is what their name
  starts. With no app validating `reach`, they're the network's first owner's alone.

  What a tool changes is whoever's it is, so a server acting for them runs it: the one asked, or one it passes
  the call on to.

  What each says of itself is `OneCore.Tools.Definitions`, and what runs it the module of what it does:
  `OneCore.Tools.Agents`, `OneCore.Tools.Apps`, `OneCore.Tools.Servers` and `OneCore.Tools.Builds`.
  """
  alias OneCore.{Call, User}
  alias OneCore.Call.State
  alias OneCore.Apps.Calls
  alias __MODULE__.{Agents, Apps, Builds, Definitions, Servers}

  # What each takes and gives is read once here, as the schemas its inputs and answers are checked against on every
  # call.
  @taking Enum.map(Definitions.tools(), &OneCore.Apps.Version.schemas/1)
  @agents for tool <- Definitions.agents(), do: tool.name
  @apps for tool <- Definitions.apps(), do: tool.name
  @servers for tool <- Definitions.servers(), do: tool.name

  @doc """
  One Core's own tools, each as an app says one of its own. Code mode lists only `search`, `execute`, `call` and
  `skills`, and lists every tool but those three otherwise (`listed`): a client that runs scripts finds the rest by
  searching.
  """
  def tools, do: @taking

  @doc """
  Calls one of One Core's own tools for `asked`, who asked and who for, as an app's tool is called
  (`OneCore.Apps.Calls.tool/5`), hooks and all. `request` is how it was asked: its `mode` (`:mcp`, `:code` or
  `:cli`), `tools`, a function giving the tools a script has, as MCP lists them, and `call`, a function that calls
  one of them by its name and inputs as MCP would, for code mode's own. Gives what `OneCore.Apps.Calls.call/5` does,
  or `:none` if `name` isn't one.
  """
  def call(name, inputs, principal, forwarded, request) do
    case Enum.find(@taking, &(&1.name == name)) do
      nil ->
        :none

      tool ->
        # What's passed on is what was asked, so the server it's passed to hooks into the same thing.
        ask = {"tools/call", %{name: name, arguments: inputs}}
        # To a server acting for someone, or with `{:running, owner}` to one that runs their apps.
        forward = fn
          {:running, owner} ->
            Call.run_elsewhere(ask, principal, forwarded, {owner, OneCore.Apps.servers(owner)}, "An app for #{owner}")
          owner -> Call.forward(ask, principal, forwarded, owner)
        end

        about = %{"app" => nil, "tool" => name, "inputs" => inputs}
        whose = User.owner(elem(principal, 1)) || "network"

        # A task is kept under whoever it's for (`OneCore.Tasks.made/1`), so a server that doesn't act for them passes
        # a call that may be one on before it's accepted, as an app's call is passed on before it's checked
        # (`OneCore.Apps.Calls`).
        State.run(fn ->
          if tool[:task_support] != nil and not OneCore.Servers.here?(whose),
            do: OneCore.Apps.held([], fn -> forward.(whose) end),
            else: Calls.tool(tool, about, principal, [], &run(name, &1, principal, forward, request))
        end)
    end
  end

  defp run(name, inputs, principal, forward, request) when name in @agents,
    do: Agents.run(name, inputs, principal, forward, request)

  defp run(name, inputs, principal, forward, request) when name in @apps,
    do: Apps.run(name, inputs, principal, forward, request)

  defp run(name, inputs, principal, forward, request) when name in @servers,
    do: Servers.run(name, inputs, principal, forward, request)

  defp run(name, inputs, principal, forward, request), do: Builds.run(name, inputs, principal, forward, request)
end
