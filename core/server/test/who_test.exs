defmodule OneCore.WhoTest do
  @moduledoc "Who may do what: who a request is from, and what One Core lets them reach by itself."
  use ExUnit.Case
  alias OneCore.{Mcp, Plugins, Server, User}
  alias OneCore.Plugins.{Added, Calls, Deploy}

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

  # A plugin that decides who reaches what, by a rule of its own, and says what it was asked. Its tool changes
  # nothing, which only it knows.
  defp decides(name, refused) do
    """
    defmodule #{name} do
      @behaviour OneCore.Plugin

      @impl true
      def tools, do: for(name <- ["change", "look"], do: %{name: name, read_only: name == "look", description: name, inputs: %{}})

      @impl true
      def call(_tool, _inputs, _call), do: "Done."

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

  defp deploy(name, source),
    do: assert({:ok, _number, ""} = Deploy.deploy("network", name, %{"server/#{name}.ex" => source}))

  defp call(tool, inputs, who),
    do: Calls.call(hd(String.split(tool, "_")), tl(String.split(tool, "_", parts: 2)) |> hd(), inputs, who)

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
    OneCore.Test.server(folder)
    assert {:ok, 1, ""} = Deploy.deploy("network", "asked", %{"server/asked.ex" => @plugin})
    :ok
  end

  @tag :tmp_dir
  test "a request is from whoever's device it came from, and from nobody where nothing places it, unless a link is shown, a server acting here says who, or the network is being built" do
    Process.register(self(), :who_test)
    {stranger, other} = {String.duplicate("ab", 32), String.duplicate("cd", 32)}
    deploy("door", decides("Door", "users/bob"))

    list = fn method, said, from ->
      JSON.decode!(Mcp.answer(JSON.encode!(%{jsonrpc: "2.0", id: 1, method: method, params: %{_meta: said}}), from))
    end

    # With nothing signing anyone in, a request is from nobody, and nobody gets nothing: whatever key its
    # connection proved, and whatever it says of itself.
    nobody = {:refused, "This device is nobody's here. Sign in on it, or let it in from a device that's yours."}
    assert ask(%{}) == nobody
    assert ask(%{}, stranger) == nobody
    assert ask(%{device: stranger}) == nobody
    assert ask(%{for: "network"}, stranger) == nobody
    assert %{"error" => %{"message" => "This device is nobody's here" <> _}} = list.("tools/list", %{}, stranger)
    assert %{"error" => %{"message" => "This device is nobody's here" <> _}} = list.("resources/list", %{}, stranger)
    assert %{"error" => %{"message" => "This device is nobody's here" <> _}} = list.("initialize", %{}, nil)

    # A plugin that signs users in says whose a device is: the key a connection proved, or the device the
    # network's edge says a request over the web is from. A device doesn't say it's another.
    deploy("placer", OneCore.Test.placer())
    assert {:ok, :ok} = call("placer_place", %{"device" => stranger, "who" => "users/alice"}, OneCore.Test.owner())
    assert ask(%{}, stranger) == "users/alice"
    assert ask(%{device: stranger}) == "users/alice"
    assert ask(%{device: stranger}, other) == nobody
    assert ask(%{}, other) == nobody
    assert %{"result" => %{"tools" => [_, _ | _]}} = list.("tools/list", %{}, stranger)

    # A run of a client has a key of its own, and is from the device that signed it, and only that one.
    {device, signing} = :crypto.generate_key(:eddsa, :ed25519)
    {device, run} = {Base.encode16(device, case: :lower), String.duplicate("ef", 32)}
    attestation = &Base.encode16(:crypto.sign(:eddsa, :none, "one/run/1\n" <> &1, [signing, :ed25519]), case: :lower)
    assert {:ok, :ok} = call("placer_place", %{"device" => device, "who" => "users/alice"}, OneCore.Test.owner())
    assert ask(%{device: device, attestation: attestation.(run)}, run) == "users/alice"
    assert ask(%{device: device, attestation: attestation.(other)}, run) == nobody
    assert ask(%{device: device, attestation: String.duplicate("00", 64)}, run) == nobody
    assert ask(%{device: stranger, attestation: attestation.(run)}, run) == nobody
    assert ask(%{device: device}, run) == nobody

    # Whose it is is kept, so a call asks nothing, until a tool of that plugin is called that doesn't only read:
    # a device that's removed is nobody's from the next call on.
    assert {:ok, :ok} = call("placer_remove", %{"device" => stranger}, OneCore.Test.owner())
    assert ask(%{}, stranger) == nobody
    # Only a plugin added for the network says whose a device is, and only a user's.
    assert {:ok, 1, ""} =
             Deploy.deploy("users/bob", "mine", %{
               "server/mine.ex" => String.replace(OneCore.Test.placer(), "Placer", "Mine")
             })

    assert {:ok, :ok} =
             Calls.call(
               "mine",
               "place",
               %{"device" => other, "who" => "users/alice"},
               {OneCore.Test.owner(), "users/bob"}
             )

    assert ask(%{}, other) == nobody
    assert {:ok, :ok} = call("placer_place", %{"device" => other, "who" => "network"}, OneCore.Test.owner())
    assert ask(%{}, other) == nobody

    # Whoever shows a sharing link is its holder, named without the link.
    assert "links/" <> name = ask(%{link: "a-secret"})
    refute name =~ "a-secret"
    assert ask(%{link: "another"}) != "links/" <> name
    # A plugin is added for users, so a link's holder calls only the tools that say they may, and none of One Core's.
    assert {:refused, "asked_members isn't a tool that a sharing link's holder calls"} =
             ask(%{link: "a-secret"}, nil, "asked_members")

    assert {:refused, "services isn't a tool" <> _} = ask(%{link: "a-secret"}, nil, "services")

    # A link starts with whose it leads to, and its holder is named after them: they're answered by the servers
    # acting for them, with the plugins added for them, and told of no tool but the ones a holder may call.
    assert "users/alice/links/" <> _ = holder = ask(%{link: "users/alice/a-secret"})
    assert User.owner(holder) == "users/alice" and User.link?(holder) and not User.link?("users/alice")

    assert %{"result" => %{"tools" => [%{"name" => "asked_who"}]}} =
             list.("tools/list", %{link: "users/alice/a-secret"}, nil)

    assert %{"error" => %{"message" => "A sharing link's holder reads nothing" <> _}} =
             list.("resources/list", %{link: "users/alice/a-secret"}, nil)

    # Only a server acting here is believed about who it asks for.
    assert ask(%{who: "users/alice", for: "users/alice"}, Server.key()) == "users/alice"
    assert ask(%{who: nil, for: "groups/family"}, Server.key()) == " for groups/family"

    assert {:refused, "Only a server acting for users/alice says it asks for them"} =
             ask(%{who: "users/alice", for: "users/alice"}, stranger)

    assert {:refused, "Only a server" <> _} = ask(%{who: "users/alice", for: "users/alice"})
    # What it passes on it has decided, so a group's member isn't asked about again, nor a link's holder refused.
    assert ask(%{who: "users/bob", for: "groups/family", forwarded: true}, Server.key()) ==
             "users/bob for groups/family"

    assert ask(%{who: User.link("a-secret"), for: User.link("a-secret")}, Server.key()) == User.link("a-secret")

    # Acting as a user is refused, unless the network is one being built.
    assert {:refused, "Acting as a user" <> _} = ask(%{as: "alice"})
    OneCore.Test.being_built()
    assert ask(%{as: "alice"}) == "users/alice"
    assert {:refused, "A user's name" <> _} = ask(%{as: "../network"})
    # Even there, a request that names nobody is from nobody.
    assert ask(%{}) == nobody

    # A request is for whoever it's from, or for the group or the network it names, where the plugins that decide
    # say they're its member. A link's holder asks for nobody else.
    asked()
    assert ask(%{as: "alice", for: "groups/family"}) == "users/alice for groups/family"
    assert ask(%{as: "alice", for: "network"}) == "users/alice for network"
    assert {:refused, "A request is for" <> _} = ask(%{as: "alice", for: "groups/../network"})
    assert {:refused, "A sharing link's holder asks for nobody else"} = ask(%{link: "a-secret", for: "groups/family"})
    asked()
    assert {:refused, "Door keeps users/bob out."} = ask(%{as: "bob", for: "groups/family"})
    assert %{"who" => "users/bob", "relation" => "member", "artifact" => "groups/family"} = List.last(asked())
  end

  @tag :tmp_dir
  test "with nothing deciding only the first owner does anything, unless the network is being built, and what decides is asked about both the user and the plugin" do
    Process.register(self(), :who_test)
    deploy("notes", @notes)
    deploy("peek", String.replace(@notes, "Notes", "Peek"))
    alice = "users/alice"

    # One Core alone: nobody calls a tool or reaches anything but the network's first owner, who is how a
    # network with no plugin deciding is put right.
    undecided = {:error, "Nothing here decides who may do what, so only the network's first owner may do anything"}
    assert call("notes_write", %{"id" => "0", "text" => "Mine"}, alice) == undecided

    assert {:ok, "users/owner/made/notes/0" = owners} =
             call("notes_write", %{"id" => "0", "text" => "Mine"}, OneCore.Test.owner())

    assert call("notes_read", %{"note" => owners}, alice) == undecided
    assert call("peek_read", %{"note" => owners}, OneCore.Test.owner()) == {:ok, "Mine"}
    assert OneCore.Tools.call("services", %{}, {alice, alice}) == undecided
    assert {:ok, _} = OneCore.Tools.call("services", %{}, {OneCore.Test.owner(), "network"})
    assert_receive {:changed, "peek", %{"artifact" => ^owners}, _}

    # A network being built: everything in it is shared, and a plugin hears of whatever changes.
    OneCore.Test.being_built()
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
    asked()
    assert call("notes_read", %{"note" => note}, "users/dave") == {:ok, "Buy milk"}
    assert [%{"who" => "users/dave"}, %{"who" => "users/dave"}] = asked()

    # That someone may reach something is kept, so it's asked once: until a tool that doesn't only read is called
    # of a plugin that decides it, which may have changed what it decides from. A refusal is never kept.
    assert call("notes_read", %{"note" => note}, "users/dave") == {:ok, "Buy milk"}
    assert asked() == []
    assert {:ok, "Done."} = call("lock_look", %{}, alice)
    assert call("notes_read", %{"note" => note}, "users/dave") == {:ok, "Buy milk"}
    assert asked() == []
    assert {:ok, "Done."} = call("lock_change", %{}, alice)
    assert call("notes_read", %{"note" => note}, "users/dave") == {:ok, "Buy milk"}
    assert [_door, _lock] = asked()

    for _ <- 1..2 do
      assert call("notes_read", %{"note" => note}, "users/carol") == {:error, "Lock keeps users/carol out."}
      assert [_ | _] = asked()
    end
  end

  @tag :tmp_dir
  test "a plugin is whoever's deployed it, and runs for whoever it's added for, with an artifact of its own for each" do
    Process.register(self(), :who_test)
    OneCore.Test.being_built()
    {alice, bob, family} = {"users/alice", "users/bob", "groups/family"}
    assert {:ok, 1, _} = Deploy.deploy("network", "database", OneCore.Test.shipped("database"))
    assert {:ok, 1, ""} = Deploy.deploy(alice, "counts", %{"server/counts.ex" => @counts})
    assert Plugins.plugin(alice, "counts") == "users/alice/plugins/counts"
    assert call("counts_count", %{}, alice) == {:ok, "users/alice/own/counts: 1 for users/alice"}

    # Nobody else has it until it's added for them: the same plugin, with another artifact of its own.
    assert call("counts_count", %{}, bob) == {:error, "There's no plugin named counts"}
    assert :ok = Added.add(bob, "counts", "users/alice/plugins/counts")
    assert :ok = Added.add(family, "counts", "users/alice/plugins/counts")
    assert call("counts_count", %{}, bob) == {:ok, "users/bob/own/counts: 1 for users/bob"}
    assert call("counts_count", %{}, alice) == {:ok, "users/alice/own/counts: 2 for users/alice"}
    assert call("counts_count", %{}, {alice, family}) == {:ok, "groups/family/own/counts: 1 for groups/family"}

    # What's added for the network is everyone's, with one artifact of its own.
    assert :ok = Added.add("network", "tally", "users/alice/plugins/counts")
    assert call("tally_count", %{}, "users/carol") == {:ok, "network/own/tally: 1 for users/carol"}
    assert call("tally_count", %{}, bob) == {:ok, "network/own/tally: 2 for users/bob"}

    assert Added.add("network", "tally", "users/bob/plugins/nothing") ==
             {:error, "There's no plugin users/bob/plugins/nothing"}

    # Another plugin's own artifact is never reached, whoever asks, even with nothing deciding who reaches what.
    assert {:error, "users/alice/own/counts is a plugin's own artifact, which only that plugin reaches"} =
             call("tally_peek", %{"artifact" => "users/alice/own/counts"}, alice)

    # Everyone using a plugin runs its newest version.
    newer = String.replace(@counts, "for \#{call.for}", "now \#{call.for}")
    assert {:ok, 2, ""} = Deploy.deploy(alice, "counts", %{"server/counts.ex" => newer})
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
    assert {:ok, 1, ""} = Deploy.deploy(bob, "shut", %{"server/shut.ex" => @shut})
    assert call("counts_count", %{}, bob) == {:error, "Shut to users/bob."}
    assert call("tally_count", %{}, bob) == {:error, "Shut to users/bob."}
    assert {:ok, _} = call("tally_count", %{}, alice)
    assert :ok = Added.add("network", "shut", "users/bob/plugins/shut")
    assert call("tally_count", %{}, alice) == {:error, "Shut to users/alice."}
    assert :ok = Added.remove("network", "shut")
    assert {:ok, _} = call("tally_count", %{}, alice)

    # Removing it for someone leaves it whoever's it is, with what it kept.
    assert :ok = Added.remove(bob, "shut")
    assert :ok = Added.remove(bob, "counts")
    assert call("counts_count", %{}, bob) == {:error, "There's no plugin named counts"}
    assert {:ok, _} = call("counts_count", %{}, alice)
    assert ["users/bob/own/counts/0", "users/bob/own/counts/1" | _] = OneCore.Bucket.list("users/bob/own/counts/")
    assert Added.remove(bob, "counts") == {:error, "No plugin named counts is added for users/bob"}
  end
end
