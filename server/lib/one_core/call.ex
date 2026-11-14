defmodule OneCore.Call do
  @moduledoc """
  What a tool is called with besides its inputs.

  - `own`: the plugin's own `OneCore.Artifact` for whoever it runs for. Only this plugin reaches it.
  """
  @enforce_keys [:own]
  defstruct [:own]

  @type t :: %__MODULE__{own: OneCore.Artifact.t()}
end
