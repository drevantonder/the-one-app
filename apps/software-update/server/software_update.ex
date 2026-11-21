defmodule SoftwareUpdate do
  @moduledoc """
  Software Update: deploys the network's edge once its servers are on a newer build (One Core's hook `edge`), as
  one server is told to by the network's update (`core/edge/updater/updater.ts`). It deploys that server's build of
  the edge (`OneCore.Release.edge/0`) with `deploy.js` beside it, which checks the edge answers as that build and
  rolls it back if not, and says how it went (`OneCore.Release.edge_deployed/2`).

  It deploys with the network's Cloudflare API token, `api_token` of the item `@item` in the network's Vault, switched
  on for this app, and only on a server that opens the Vault: the token never leaves the server that reads it, so
  one that doesn't says so, and the update has the next deploy it.
  """
  @behaviour OneCore.App
  require Logger

  @item "Cloudflare edge deploy"

  @impl true
  def icon, do: "icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.34.290", "vault" => "to deploy your network's edge with its Cloudflare token"}

  @impl true
  def hooks, do: ["edge"]

  @impl true
  def hook("edge", %{"build" => build}, call) do
    case token(call) do
      {:ok, token} ->
        # A deploy takes minutes, and a hook ten seconds at most.
        Task.start(fn -> OneCore.Release.edge_deployed(build, deploy(build, token)) end)

      {:error, why} ->
        OneCore.Release.edge_deployed(build, {:error, why})
    end

    :ok
  end

  # The token, read on this server, which is never passed on to another that opens the Vault.
  defp token(call) do
    {:ok, OneCore.Call.app(call, "vault").value(%{call | forwarded: true}, @item, "api_token")}
  rescue
    error -> {:error, Exception.message(error)}
  catch
    :throw, {OneCore.Call, :forwarding_loop} -> {:error, "#{OneCore.Servers.name(call.server) || "This server"} doesn't open the network's Vault"}
  end

  # Deploys this server's build of the edge from a copy of its own, where Bun installs Wrangler, to the network's
  # address: `:ok`, or `{:error, why}`, the end of what it said.
  defp deploy(build, token) do
    with ^build <- OneCore.Builds.build(),
         from when is_binary(from) <- OneCore.Release.edge(),
         address when is_binary(address) <- Application.get_env(:one_core, :address),
         {:ok, bun} <- OneCore.Programs.path(:bun) do
      folder = Path.join([OneCore.Server.home(), "edge", build])
      File.rm_rf!(folder)
      File.cp_r!(from, folder)
      env = [{"CLOUDFLARE_API_TOKEN", token}]

      with {_, 0} <- System.cmd(bun, ["install", "--frozen-lockfile"], cd: folder, stderr_to_stdout: true),
           {said, 0} <- System.cmd(bun, ["deploy.js", address], cd: folder, env: env, stderr_to_stdout: true) do
        Logger.info(String.trim(said))
        File.rm_rf(folder)
        :ok
      else
        {said, _status} ->
          Logger.error("The edge wasn't deployed: #{said}")
          {:error, said |> String.trim() |> String.split("\n") |> List.last()}
      end
    else
      {:error, why} -> {:error, why}
      nil -> {:error, "This server has no edge to deploy, or no address to deploy it to"}
      other -> {:error, "This server is on #{other}, not #{build}"}
    end
  end
end
