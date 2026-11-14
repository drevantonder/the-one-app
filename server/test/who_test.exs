defmodule OneCore.WhoTest do
  @moduledoc "Who may do what: who a request is from, and what One Core lets them reach by itself."
  use ExUnit.Case
  alias OneCore.{Mcp, Plugins, Server, Services, User}

  # A plugin whose tool says who it runs for.
  @plugin """
  defmodule Asked do
    @behaviour OneCore.Plugin

    @impl true
    def tools, do: [%{name: "who", read_only: true, description: "Says who asked", inputs: %{type: "object"}}]

    @impl true
    def call("who", _inputs, call), do: call.for
  end
  """

  defp ask(said, from \\ nil) do
    params = %{name: "asked_who", arguments: %{}, _meta: said}
    request = JSON.encode!(%{jsonrpc: "2.0", id: 1, method: "tools/call", params: params})

    case JSON.decode!(Mcp.answer(request, from)) do
      %{"result" => %{"content" => [%{"text" => who}]}} -> who
      %{"error" => %{"message" => why}} -> {:refused, why}
    end
  end

  setup %{tmp_dir: folder} do
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.Folder, Path.join(folder, "bucket")})
    Application.put_env(:one_core, :home, Path.join(folder, "server"))
    Application.put_env(:one_core, :development, false)
    on_exit(fn -> Application.delete_env(:one_core, :development) end)
    {:ok, key} = OneCore.Iroh.key(Server.key_file())
    assert OneCore.Servers.joined?(key)
    for child <- [Plugins, Server, Services], do: start_supervised!(child)
    assert {:ok, 1, ""} = Plugins.deploy("asked", %{"asked.ex" => @plugin})
    :ok
  end

  @tag :tmp_dir
  test "a request is from the one user, unless a link is shown, a server acting here says who, or the network is being built" do
    stranger = String.duplicate("ab", 32)
    assert ask(%{}) == User.anyone()

    # Whoever shows a sharing link is its holder, named without the link.
    assert "links/" <> name = ask(%{link: "a-secret"})
    refute name =~ "a-secret"
    assert ask(%{link: "another"}) != "links/" <> name

    # Only a server acting here is believed about who it asks for.
    assert ask(%{who: "users/alice"}, Server.key()) == "users/alice"
    assert {:refused, "Only a server" <> _} = ask(%{who: "users/alice"}, stranger)
    assert {:refused, "Only a server" <> _} = ask(%{who: "users/alice"})

    # Acting as another user is refused, unless the network is one being built.
    assert {:refused, "Acting as another user" <> _} = ask(%{as: "alice"})
    Application.put_env(:one_core, :development, true)
    assert ask(%{as: "alice"}) == "users/alice"
    assert {:refused, "A user's name" <> _} = ask(%{as: "../network"})
  end
end
