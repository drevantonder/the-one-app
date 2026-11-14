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

  # A plugin that counts in its own artifact, says where that is and who it ran for, and has a job that says the
  # same to the test.
  @counts """
  defmodule Counts do
    @behaviour OneCore.Plugin
    alias OneCore.Artifact

    @impl true
    def uses, do: ["database"]

    @impl true
    def jobs, do: [%{tool: "tick", every: 1}]

    @impl true
    def tools, do: for(name <- ~w(count tick peek), do: %{name: name, description: name, inputs: %{type: "object"}})

    @impl true
    def call("count", _inputs, %{own: own} = call) do
      Artifact.write(own, "\#{length(Artifact.list(own))}", "")
      "\#{Artifact.name(own)}: \#{length(Artifact.list(own))} for \#{call.for}"
    end

    def call("tick", _inputs, call), do: send(:who_test, {:ticked, call.for, call.by, Artifact.name(call.own)})
    def call("peek", %{"artifact" => name}, call), do: Artifact.list(OneCore.Call.reach(call, name))
  end
  """

  # A plugin that decides every call, and refuses it.
  @shut """
  defmodule Shut do
    @behaviour OneCore.Plugin

    @impl true
    def hooks, do: ["call"]

    @impl true
    def hook("call", _about, call), do: {:refuse, "Shut to \#{call.for}."}
  end
  """

  defp shipped(name) do
    folder = Path.expand("../../plugins/#{name}", __DIR__)
    files = Enum.filter(Path.wildcard(folder <> "/**"), &File.regular?/1)
    Map.new(files, &{Path.relative_to(&1, folder), File.read!(&1)})
  end

  defp deploy(name, source), do: assert({:ok, _number, ""} = Plugins.deploy("network", name, %{"#{name}.ex" => source}))

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
    assert {:ok, 1, ""} = Plugins.deploy("network", "asked", %{"asked.ex" => @plugin})
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
    # What it passes on it has decided, so a group's member isn't asked about again, nor a link's holder refused.
    assert ask(%{who: "users/bob", for: "groups/family", passed_on: true}, Server.key()) ==
             "users/bob for groups/family"

    assert ask(%{who: User.link("a-secret"), for: User.link("a-secret")}, Server.key()) == User.link("a-secret")

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
    assert_receive {:changed, "peek", %{"artifact" => ^note, "blob" => "text", "by" => "network/plugins/notes"}, ^alice}
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
    assert [%{"who" => ^alice}, %{"who" => "network/plugins/peek"}] = asked()

    # For someone else it's asked about them, and for another plugin about the plugin too: both need the relation.
    assert call("notes_read", %{"note" => note}, "users/bob") == {:error, "Door keeps users/bob out."}
    assert asked() == [%{"who" => "users/bob", "relation" => "viewer", "artifact" => note}]
    assert call("notes_edit", %{"note" => note}, "users/carol") == {:ok, :ok}
    assert [%{"who" => "users/carol", "relation" => "editor"} | _hearing] = asked()

    deploy("door", decides("Door", "network/plugins/peek"))
    assert call("peek_read", %{"note" => note}, alice) == {:error, "Door keeps network/plugins/peek out."}
    assert [%{"who" => ^alice}, %{"who" => "network/plugins/peek"}] = asked()

    # A plugin kept out hears nothing of the artifact, and none hears what it changed itself.
    assert {:ok, _name} = call("notes_write", %{"id" => "4", "text" => "x"}, alice)
    asked()
    refute_received {:changed, _plugin, %{"artifact" => "users/alice/made/notes/4"}, _who}

    # More than one plugin decides, and any of them says no.
    deploy("lock", decides("Lock", "users/carol"))
    assert call("notes_read", %{"note" => note}, "users/carol") == {:error, "Lock keeps users/carol out."}
    assert call("notes_read", %{"note" => note}, "users/dave") == {:ok, "Buy milk"}
  end

  @tag :tmp_dir
  test "a plugin is whoever's deployed it, and runs for whoever it's added for, with an artifact of its own for each" do
    Process.register(self(), :who_test)
    {alice, bob, family} = {"users/alice", "users/bob", "groups/family"}
    assert {:ok, 1, _} = Plugins.deploy("network", "database", shipped("database"))
    assert {:ok, 1, ""} = Plugins.deploy(alice, "counts", %{"counts.ex" => @counts})
    assert Plugins.plugin(alice, "counts") == "users/alice/plugins/counts"
    assert call("counts_count", %{}, alice) == {:ok, "users/alice/own/counts: 1 for users/alice"}

    # Nobody else has it until it's added for them: the same plugin, with another artifact of its own.
    assert call("counts_count", %{}, bob) == {:error, "There's no plugin named counts"}
    assert :ok = Plugins.add(bob, "counts", "users/alice/plugins/counts")
    assert :ok = Plugins.add(family, "counts", "users/alice/plugins/counts")
    assert call("counts_count", %{}, bob) == {:ok, "users/bob/own/counts: 1 for users/bob"}
    assert call("counts_count", %{}, alice) == {:ok, "users/alice/own/counts: 2 for users/alice"}
    assert call("counts_count", %{}, {alice, family}) == {:ok, "groups/family/own/counts: 1 for groups/family"}

    # What's added for the network is everyone's, with one artifact of its own.
    assert :ok = Plugins.add("network", "tally", "users/alice/plugins/counts")
    assert call("tally_count", %{}, "users/carol") == {:ok, "network/own/tally: 1 for users/carol"}
    assert call("tally_count", %{}, bob) == {:ok, "network/own/tally: 2 for users/bob"}

    assert Plugins.add("network", "tally", "users/bob/plugins/nothing") ==
             {:error, "There's no plugin users/bob/plugins/nothing"}

    # Another plugin's own artifact is never reached, whoever asks, even with nothing deciding who reaches what.
    assert {:error, "users/alice/own/counts is a plugin's own artifact, which only that plugin reaches"} =
             call("tally_peek", %{"artifact" => "users/alice/own/counts"}, alice)

    # Everyone using a plugin runs its newest version.
    newer = String.replace(@counts, "for \#{call.for}", "now \#{call.for}")
    assert {:ok, 2, ""} = Plugins.deploy(alice, "counts", %{"counts.ex" => newer})
    assert call("counts_count", %{}, bob) == {:ok, "users/bob/own/counts: 2 now users/bob"}
    assert call("tally_count", %{}, bob) == {:ok, "network/own/tally: 3 now users/bob"}

    # A scheduled job runs for each one its plugin was added for, asked by nobody.
    start_supervised!({Task.Supervisor, name: OneCore.Asks})
    start_supervised!(OneCore.Jobs)

    for {who, own} <- [
          {alice, "users/alice/own/counts"},
          {family, "groups/family/own/counts"},
          {"network", "network/own/tally"}
        ],
        do: assert_receive({:ticked, ^who, nil, ^own}, 5_000)

    stop_supervised!(OneCore.Jobs)

    # A plugin hooks into the moments of whoever it was added for, and of nobody else.
    assert {:ok, 1, ""} = Plugins.deploy(bob, "shut", %{"shut.ex" => @shut})
    assert call("counts_count", %{}, bob) == {:error, "Shut to users/bob."}
    assert call("tally_count", %{}, bob) == {:error, "Shut to users/bob."}
    assert {:ok, _} = call("tally_count", %{}, alice)
    assert :ok = Plugins.add("network", "shut", "users/bob/plugins/shut")
    assert call("tally_count", %{}, alice) == {:error, "Shut to users/alice."}
    assert :ok = Plugins.remove("network", "shut")
    assert {:ok, _} = call("tally_count", %{}, alice)

    # Removing it for someone leaves it whoever's it is, with what it kept.
    assert :ok = Plugins.remove(bob, "shut")
    assert :ok = Plugins.remove(bob, "counts")
    assert call("counts_count", %{}, bob) == {:error, "There's no plugin named counts"}
    assert {:ok, _} = call("counts_count", %{}, alice)
    assert ["users/bob/own/counts/0", "users/bob/own/counts/1" | _] = OneCore.Bucket.list("users/bob/own/counts/")
    assert Plugins.remove(bob, "counts") == {:error, "No plugin named counts is added for users/bob"}
  end
end
