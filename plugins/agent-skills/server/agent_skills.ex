defmodule AgentSkills do
  @moduledoc """
  Agent Skills: the network's skills, where agent tools read them. Its service, `AgentSkills.Writer`, on each device
  it's installed on, writes them there.
  """
  @behaviour OneCore.Plugin

  @impl true
  def title, do: "Agent Skills"
end
