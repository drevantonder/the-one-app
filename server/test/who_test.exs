defmodule OneCore.WhoTest do
  @moduledoc "Who may do what: who a request is from, and what One Core lets them reach by itself."
  use ExUnit.Case
  alias OneCore.{Mcp, Plugins, Server, Services, User}

  # A plugin whose tool says who it runs for.
  @plugin """
  defmodule Asked do
    @behaviour OneCore.Plugin

    @impl true
    def tools do
      [
        %{name: "who", read_only: true, by_link: true, description: "Says who asked", inputs: %{type: "object"}},
        %{name: "members", read_only: true, description: "Says who asked, to a user", inputs: %{type: "object"}}
      ]
    end

    @impl true
    def call(_tool, _inputs, %{for: who, by: who}), do: who
    def call(_tool, _inputs, call), do: "\#{call.by} for \#{call.for}"
  end
  """

  # Notes, each an artifact made for whoever wrote it, and a plugin that reads what another made. Both hear
  # artifacts change.
  @notes """
  defmodule Notes do
    @behaviour OneCore.Plugin
    alias OneCore.{Artifact, Call}

    @impl true
    def tools do
      for name <- ~w(write read scribble edit),
          do: %{name: name, by_link: true, description: name, inputs: %{type: "object"}}
    end

    @impl true
    def call("write", %{"id" => id, "text" => text}, call) do
      note = Call.make(call, id)
      Artifact.write(note, "text", text)
      Artifact.name(note)
    end

    def call("read", %{"note" => note}, call) do
      {:ok, text, _tag} = Artifact.read(Call.reach(call, note), "text")
      text
    end

    def call("scribble", %{"note" => note}, call), do: Artifact.write(Call.reach(call, note), "scribble", "!")
    def call("edit", %{"note" => note}, call), do: Artifact.write(Call.reach(call, note, "editor"), "edit", "!")

    @impl true
    def hooks, do: ["changed"]

    @impl true
    def hook("changed", about, call), do: send(:who_test, {:changed, call.plugin, about, call.for})
  end
  """

  # A plugin that decides who reaches what, by a rule of its own, and says what it was asked.
  defp decides(name, refused) do
    """
    defmodule #{name} do
      @behaviour OneCore.Plugin

      @impl true
      def hooks, do: ["reach"]

      @impl true
      def hook("reach", %{"who" => who} = about, _call) do
        send(:who_test, {:asked, "#{name}", about})
        if who == "#{refused}", do: {:refuse, "#{name} keeps \#{who} out."}, else: :ok
      end
    end
    """
  end

  defp deploy(name, source), do: assert({:ok, _number, ""} = Plugins.deploy(name, %{"#{name}.ex" => source}))

  defp call(tool, inputs, who),
    do: Plugins.call(hd(String.split(tool, "_")), tl(String.split(tool, "_", parts: 2)) |> hd(), inputs, who)

  defp asked do
    receive do
      {:asked, _by, about} -> [about | asked()]
    after
      50 -> []
    end
  end

  defp ask(said, from \\ nil, tool \\ "asked_who") do
    params = %{name: tool, arguments: %{}, _meta: said}
    request = JSON.encode!(%{jsonrpc: "2.0", id: 1, method: "tools/call", params: params})

    case JSON.decode!(Mcp.answer(request, from)) do
      %{"result" => %{"content" => [%{"text" => why}], "isError" => true}} -> {:refused, why}
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
    # A plugin is added for users, so a link's holder calls only the tools that say they may, and none of One Core's.
    assert {:refused, "asked_members isn't a tool that a sharing link's holder calls"} =
             ask(%{link: "a-secret"}, nil, "asked_members")

    assert {:refused, "services isn't a tool" <> _} = ask(%{link: "a-secret"}, nil, "services")

    # Only a server acting here is believed about who it asks for.
    assert ask(%{who: "users/alice", for: "users/alice"}, Server.key()) == "users/alice"
    assert ask(%{who: nil, for: "groups/family"}, Server.key()) == " for groups/family"
    assert {:refused, "Only a server" <> _} = ask(%{who: "users/alice", for: "users/alice"}, stranger)
    assert {:refused, "Only a server" <> _} = ask(%{who: "users/alice", for: "users/alice"})

    # Acting as another user is refused, unless the network is one being built.
    assert {:refused, "Acting as another user" <> _} = ask(%{as: "alice"})
    Application.put_env(:one_core, :development, true)
    assert ask(%{as: "alice"}) == "users/alice"
    assert {:refused, "A user's name" <> _} = ask(%{as: "../network"})

    # A request is for whoever it's from, or for the group or the network it names. With nothing deciding who
    # is whose member, everything is shared. A link's holder asks for nobody else.
    assert ask(%{as: "alice", for: "groups/family"}) == "users/alice for groups/family"
    assert ask(%{as: "alice", for: "network"}) == "users/alice for network"
    assert {:refused, "A request is for" <> _} = ask(%{as: "alice", for: "groups/../network"})
    assert {:refused, "A sharing link's holder asks for nobody else"} = ask(%{link: "a-secret", for: "groups/family"})
    Process.register(self(), :who_test)
    deploy("door", decides("Door", "users/bob"))
    assert ask(%{as: "alice", for: "groups/family"}) == "users/alice for groups/family"
    assert {:refused, "Door keeps users/bob out."} = ask(%{as: "bob", for: "groups/family"})
    assert %{"who" => "users/bob", "relation" => "member", "artifact" => "groups/family"} = List.last(asked())
  end

  @tag :tmp_dir
  test "with nothing deciding everything is shared, and what decides is asked about both the user and the plugin" do
    Process.register(self(), :who_test)
    deploy("notes", @notes)
    deploy("peek", String.replace(@notes, "Notes", "Peek"))
    alice = "users/alice"

    # One Core alone: everything in a network is shared, and a plugin hears of whatever changes.
    assert {:ok, "users/alice/made/notes/1" = note} = call("notes_write", %{"id" => "1", "text" => "Buy milk"}, alice)
    assert call("notes_read", %{"note" => note}, "users/bob") == {:ok, "Buy milk"}
    assert call("peek_read", %{"note" => note}, alice) == {:ok, "Buy milk"}
    assert_receive {:changed, "peek", %{"artifact" => ^note, "blob" => "text", "by" => "notes"}, ^alice}
    # What was reached to read is only read, and a link's holder makes nothing.
    assert {:error, "users/alice/made/notes/1 was reached as a viewer" <> _} =
             call("notes_scribble", %{"note" => note}, "users/bob")

    assert {:error, "A sharing link's holder makes nothing" <> _} =
             call("notes_write", %{"id" => "2", "text" => "x"}, User.link("a-secret"))

    # A plugin that decides is asked nothing about what a plugin made for whoever it runs for.
    deploy("door", decides("Door", "users/bob"))
    assert {:ok, _name} = call("notes_write", %{"id" => "3", "text" => "x"}, alice)
    assert call("notes_read", %{"note" => note}, alice) == {:ok, "Buy milk"}
    # peek hears of the change only if it could reach the note for alice, which is what was asked here.
    assert [%{"who" => ^alice}, %{"who" => "users/anyone/plugins/peek"}] = asked()

    # For someone else it's asked about them, and for another plugin about the plugin too: both need the relation.
    assert call("notes_read", %{"note" => note}, "users/bob") == {:error, "Door keeps users/bob out."}
    assert asked() == [%{"who" => "users/bob", "relation" => "viewer", "artifact" => note}]
    assert call("notes_edit", %{"note" => note}, "users/carol") == {:ok, :ok}
    assert [%{"who" => "users/carol", "relation" => "editor"} | _hearing] = asked()

    deploy("door", decides("Door", "users/anyone/plugins/peek"))
    assert call("peek_read", %{"note" => note}, alice) == {:error, "Door keeps users/anyone/plugins/peek out."}
    assert [%{"who" => ^alice}, %{"who" => "users/anyone/plugins/peek"}] = asked()

    # A plugin kept out hears nothing of the artifact, and none hears what it changed itself.
    assert {:ok, _name} = call("notes_write", %{"id" => "4", "text" => "x"}, alice)
    asked()
    refute_received {:changed, _plugin, %{"artifact" => "users/alice/made/notes/4"}, _who}

    # More than one plugin decides, and any of them says no.
    deploy("lock", decides("Lock", "users/carol"))
    assert call("notes_read", %{"note" => note}, "users/carol") == {:error, "Lock keeps users/carol out."}
    assert call("notes_read", %{"note" => note}, "users/dave") == {:ok, "Buy milk"}
  end
end
