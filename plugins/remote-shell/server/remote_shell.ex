defmodule RemoteShell do
  @moduledoc """
  Remote Shell: commands run on the user's own devices, by name. Its service, `RemoteShell.Runner`, on each device
  it's installed on, runs them as the user that device's server runs as, and logs each there.
  """
  @behaviour OneCore.Plugin
  alias RemoteShell.Runner

  @impl true
  def tools do
    [
      %{
        name: "run",
        description:
          "Run a shell command on one of your devices, as its owner, and wait for it to finish. Gives { exit, stdout, stderr }: exit 0 when it worked, and what it printed, the last 64 KB of each when longer. Every command is logged on the device it ran on.",
        inputs: %{
          type: "object",
          required: ["command"],
          properties: %{
            command: %{
              type: "string",
              description: ~s(the command line, run by sh -c, like "df -h" or "cd src && git status"; it reads no input)
            },
            dir: %{type: "string", description: "the folder to run it in, as an absolute path; default the home folder"},
            timeout: %{
              type: "integer",
              minimum: 1,
              maximum: 600,
              description: "seconds to wait before stopping it, 1 to 600; default 60"
            },
            device: %{
              type: "string",
              description:
                "the device to run it on, by name, as `services` names it; else the one asked if it has Remote Shell, or the first that's up"
            }
          }
        }
      }
    ]
  end

  @impl true
  def call("run", %{"command" => command} = inputs, call) do
    if String.trim(command) == "", do: raise("Give a command")
    run = [command, inputs["dir"], Map.get(inputs, "timeout", 60), call.actor]
    OneCore.Service.call(call, Runner, :run, run, device(call, inputs["device"]))
  end

  # The server that is the device called `name`, or nil for none named: whichever runs Remote Shell then answers.
  defp device(_call, nil), do: nil

  defp device(call, name) do
    servers = OneCore.Service.servers(call, Runner)
    called = &String.downcase(OneCore.Service.name(&1) || "")

    Enum.find(servers, &(called.(&1) == String.downcase(name))) ||
      raise "No device called #{name} has Remote Shell. " <>
              if(servers == [],
                do: "None does yet: `install_service` runs RemoteShell.Runner on one.",
                else: "It's on #{Enum.map_join(servers, ", ", &OneCore.Service.name/1)}."
              )
  end
end
