defmodule OneCore.RelationsTest do
  @moduledoc "Who may do what, as the relation tuples plugin decides it from its tuples."
  use ExUnit.Case
  alias OneCore.{Plugins, Server, Services, User}

  # Notes, each an artifact made for whoever wrote it. `Peek` is the same plugin under another name, which made
  # none of them.
  @notes """
  defmodule Notes do
    @behaviour OneCore.Plugin
    alias OneCore.{Artifact, Call}

    @impl true
    def tools do
      for name <- ~w(write read edit),
          do: %{name: name, by_link: true, description: name, inputs: %{type: "object"}}
    end

    @impl true
    def call("write", %{"id" => id}, call) do
      note = Call.make(call, id)
      Artifact.write(note, "text", id)
      Artifact.name(note)
    end

    def call("read", %{"note" => note}, call) do
      {:ok, text, _tag} = Artifact.read(Call.reach(call, note), "text")
      text
    end

    def call("edit", %{"note" => note}, call), do: Artifact.write(Call.reach(call, note, "editor"), "edit", "!")
  end
  """

  @alice "users/alice"
  @bob "users/bob"
  @carol "users/carol"

  defp shipped(name) do
    folder = Path.expand("../../plugins/#{name}", __DIR__)
    files = Enum.filter(Path.wildcard(folder <> "/**"), &File.regular?/1)
    Map.new(files, &{Path.relative_to(&1, folder), File.read!(&1)})
  end

  defp call(name, inputs, who) do
    [plugin, tool] = String.split(name, "_", parts: 2)
    Plugins.call(plugin, tool, inputs, who)
  end

  defp write(id, who), do: elem({:ok, _name} = call("notes_write", %{"id" => id}, who), 1)
  defp reads?(note, who, by \\ "notes"), do: match?({:ok, _text}, call("#{by}_read", %{"note" => note}, who))
  defp edits?(note, who), do: match?({:ok, _}, call("notes_edit", %{"note" => note}, who))
  defp tuple(artifact, relation, who), do: %{"artifact" => artifact, "relation" => relation, "who" => who}
  defp share(artifact, relation, who, by), do: call("relations_share", tuple(artifact, relation, who), by)
  defp unshare(artifact, relation, who, by), do: call("relations_unshare", tuple(artifact, relation, who), by)

  setup %{tmp_dir: folder} do
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.Folder, Path.join(folder, "bucket")})
    Application.put_env(:one_core, :home, Path.join(folder, "server"))
    {:ok, key} = OneCore.Iroh.key(Server.key_file())
    assert OneCore.Servers.joined?(key)
    for child <- [Plugins, Server, Services], do: start_supervised!(child)
    for name <- ["database", "relations"], do: assert({:ok, 1, ""} = Plugins.deploy(name, shipped(name)))
    assert {:ok, 1, ""} = Plugins.deploy("notes", %{"notes.ex" => @notes})
    assert {:ok, 1, ""} = Plugins.deploy("peek", %{"peek.ex" => String.replace(@notes, "Notes", "Peek")})
    :ok
  end

  @tag :tmp_dir
  test "what someone made is theirs alone until a tuple says who else may reach it, and as what" do
    note = write("1", @alice)
    assert note == "users/alice/made/notes/1"
    assert reads?(note, @alice) and edits?(note, @alice)

    assert call("notes_read", %{"note" => note}, @bob) ==
             {:error, "users/bob isn't a viewer of #{note}. Whoever owns it shares it, with relations_share."}

    # A viewer reads, an editor is a viewer who changes it too, and each tuple is one relation of one who.
    assert {:ok, _} = share(note, "viewer", @bob, @alice)
    assert reads?(note, @bob) and not edits?(note, @bob) and not reads?(note, @carol)
    assert {:ok, _} = share(note, "editor", @carol, @alice)
    assert reads?(note, @carol) and edits?(note, @carol)
    assert {:ok, _} = unshare(note, "viewer", @bob, @alice)
    assert not reads?(note, @bob) and reads?(note, @carol)

    # An editor shares up to their own relation, a viewer shares nothing, and only an owner adds an owner.
    assert {:ok, _} = share(note, "viewer", @bob, @carol)
    assert {:error, "users/bob isn't an editor of" <> _} = share(note, "viewer", "users/dave", @bob)
    assert {:error, "users/carol isn't an owner of" <> _} = share(note, "owner", @carol, @carol)
    assert {:error, "users/carol isn't an owner of" <> _} = unshare(note, "owner", @alice, @carol)
    assert {:ok, _} = share(note, "owner", @carol, @alice)
    assert {:ok, _} = share(note, "owner", "users/dave", @carol)

    # The network's owner owns everything in it, and everyone is every user.
    other = write("2", @alice)
    assert reads?(other, User.anyone()) and edits?(other, User.anyone()) and not reads?(other, @bob)
    assert {:ok, _} = share(other, "viewer", "everyone", @alice)
    assert reads?(other, @bob) and not edits?(other, @bob)
  end

  @tag :tmp_dir
  test "a group's members, a parent's relations and a sharing link's holder reach what a tuple names them for" do
    note = write("1", @alice)

    # Being in a group is a tuple, and a group in a group brings its members.
    assert {:ok, "groups/family"} = call("relations_group", %{"name" => "family"}, @alice)
    assert {:error, "There's a group named family already."} = call("relations_group", %{"name" => "family"}, @bob)
    assert {:ok, _} = share(note, "viewer", "groups/family", @alice)
    assert not reads?(note, @bob)
    assert {:error, "users/bob isn't an editor of groups/family."} = share("groups/family", "member", @bob, @bob)
    assert {:ok, _} = share("groups/family", "member", @bob, @alice)
    assert reads?(note, @bob) and not edits?(note, @bob)
    assert {:ok, "groups/cousins"} = call("relations_group", %{"name" => "cousins"}, @carol)
    assert {:ok, _} = share("groups/family", "member", "groups/cousins", @alice)
    assert reads?(note, @carol)
    assert {:ok, _} = unshare("groups/family", "member", @bob, @alice)
    assert not reads?(note, @bob) and reads?(note, @carol)

    # What's shared on a parent is shared on what names it as one, which only an owner does.
    folder = write("folder", @alice)
    inside = write("inside", @alice)
    assert {:ok, _} = share(folder, "editor", @bob, @alice)
    assert not reads?(inside, @bob)
    assert {:error, "users/bob isn't an owner of" <> _} = share(inside, "parent", folder, @bob)
    assert {:ok, _} = share(inside, "parent", folder, @alice)
    assert reads?(inside, @bob) and edits?(inside, @bob) and not reads?(inside, "users/dave")

    # Whoever holds a sharing link gets its relation, until its tuple is removed. The tuples don't hold the link.
    assert {:error, "users/bob isn't an editor of" <> _} = call("relations_link", %{"artifact" => note}, @bob)
    assert {:ok, %{link: link, who: who}} = call("relations_link", %{"artifact" => note}, @alice)
    assert who == User.link(link)
    assert reads?(note, who) and not edits?(note, who) and not reads?(folder, who)
    assert not reads?(note, User.link("a guess"))
    assert {:ok, shared} = call("relations_shared", %{"artifact" => note}, @alice)
    assert %{"relation" => "viewer", "who" => who} in shared
    refute inspect(shared) =~ link
    assert {:ok, _} = unshare(note, "viewer", who, @alice)
    assert not reads?(note, who)
  end

  @tag :tmp_dir
  test "a plugin reaches what another made only where both it and whoever it runs for have a relation to it" do
    note = write("1", @alice)
    peek = Plugins.who("peek")

    # alice may, and the plugin may not: it's hers, and peek didn't make it.
    assert call("peek_read", %{"note" => note}, @alice) ==
             {:error, "#{peek} isn't a viewer of #{note}. Whoever owns it shares it, with relations_share."}

    assert {:ok, _} = share(note, "viewer", peek, @alice)
    assert reads?(note, @alice, "peek")
    # The plugin may, and bob may not, through either plugin.
    assert not reads?(note, @bob, "peek") and not reads?(note, @bob)
    assert {:ok, _} = share(note, "viewer", @bob, @alice)
    assert reads?(note, @bob, "peek")

    # Without the relation tuples plugin, everything in the network is shared again.
    assert :ok = Plugins.remove("relations")
    assert reads?(note, @carol, "peek")
  end

  @tag :tmp_dir
  test "Claims' approver is whoever a tuple says is an editor of its claims, and everyone else sees only their own" do
    assert {:ok, 1, ""} = Plugins.deploy("claims", shipped("claims"))
    claims = Plugins.own("claims")
    assert {:ok, %{"id" => expense}} = call("claims_record", %{"amount" => 12.5}, @alice)
    assert {:ok, %{"id" => claim}} = call("claims_gather", %{"expenses" => [expense], "title" => "Taxi"}, @alice)
    assert {:ok, %{"status" => "sent"}} = call("claims_send", %{"claim" => claim}, @alice)

    # Neither whoever made the claim nor anyone else approves it, or sees it wait.
    for who <- [@alice, @bob] do
      assert call("claims_approve", %{"claim" => claim}, who) ==
               {:error, "Only an approver does that: an editor of #{claims}, which #{who} isn't."}

      assert call("claims_waiting", %{}, who) == {:ok, []}
    end

    assert {:error, "There's no claim" <> _} = call("claims_show", %{"claim" => claim}, @bob)

    # Only an owner of the claims says who approves.
    assert {:error, "users/alice isn't an editor of" <> _} = share(claims, "editor", @alice, @alice)
    assert {:ok, _} = share(claims, "editor", @bob, User.anyone())
    assert {:ok, [%{"id" => ^claim}]} = call("claims_waiting", %{}, @bob)
    assert {:ok, %{"status" => "approved"}} = call("claims_approve", %{"claim" => claim}, @bob)
    assert {:ok, _} = unshare(claims, "editor", @bob, User.anyone())
    assert {:error, "Only an approver" <> _} = call("claims_pay", %{"claim" => claim}, @bob)
  end
end
