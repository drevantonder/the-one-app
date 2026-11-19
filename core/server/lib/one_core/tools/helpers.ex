defmodule OneCore.Tools.Helpers do
  @moduledoc "What One Core's own tools share (`OneCore.Tools`): where a call runs, and whether whoever asked may."
  alias OneCore.{Call, Hooks, Server, Servers}

  # Has a server acting for `owner` run this call, if this one doesn't act for them: `forward` ends it here.
  def at(owner, forward), do: Servers.here?(owner) || forward.(owner)

  # Whose this server is: who it was started for.
  def owner, do: hd(Servers.members(Server.key()))

  # Raises unless whoever asked may reach the artifact as `relation`, which the apps that validate `reach` say.
  # Nothing is asked about what's their own: themselves, and what their name starts.
  def reaches!(principal, artifact, relation) do
    with {:refuse, why} <- reaches(principal, artifact, relation), do: raise(why)
  end

  def reaches({actor, _subject} = principal, artifact, relation) do
    if artifact == actor or String.starts_with?(artifact, actor <> "/"),
      do: :ok,
      else: Hooks.validate("reach", %{"who" => actor, "relation" => relation, "artifact" => artifact}, principal)
  end

  # Runs what a tool does on the server it names: here, or by asking that server to, for whoever asked, which
  # ends the call here with its answer. One that isn't ready is asked too, since only it can answer: a build starting
  # is asked its build this way before it's ready. One that's starting or stopping is waited for.
  def on(server, tool, inputs, principal, here) do
    if server in [nil, Server.key()],
      do: here.(),
      else: Call.run_on({"tools/call", %{name: tool, arguments: inputs}}, principal, server, ready: :any)
  end
end
