defmodule AgentSkills do
  @moduledoc """
  Agent Skills: the network's skills, where agent tools read them. Its service, `AgentSkills.Writer`, on each device
  it's installed on, writes them there.
  """
  @behaviour OneCore.App

  @impl true
  def icon, do: "icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.32.47"}
end
