defmodule OneCore.McpTest do
  use ExUnit.Case
  alias OneCore.{Mcp, Plugins}

  # What a client's Rust and an app's JavaScript read of a plugin, as the server's Elixir says it.
  @plugin """
  defmodule Notes do
    @behaviour OneCore.Plugin

    @impl true
    def tools do
      [
        %{name: "list", read_only: true, description: "Lists the notes", inputs: %{type: "object"}},
        %{name: "add", description: "Adds a note", inputs: %{type: "object"}},
        %{name: "photo", read_only: true, fixed: true, description: "A note's photo", inputs: %{type: "object"}},
        %{name: "scan", read_only: true, description: "A note's scan", inputs: %{type: "object"}}
      ]
    end

    @impl true
    def apps, do: [%{name: "Notes", page: "app.html", icon: "icon.svg"}]

    @impl true
    def call("photo", _inputs, _call), do: {:blob, "image/png", <<137, 80>>}
    def call("scan", _inputs, _call), do: {:blob, "application/pdf", "%PDF"}
    def call(_tool, _inputs, _call), do: []
  end
  """

  # What a request is answered, and its result. `said` is what it says of itself: who it's from.
  defp answer(method, params, said) do
    params = Map.put(params, :_meta, said)
    JSON.decode!(Mcp.answer(JSON.encode!(%{jsonrpc: "2.0", id: 1, method: method, params: params}), nil))
  end

  defp ask(method, params, said \\ %{as: "owner"}), do: answer(method, params, said)["result"]

  @tag :tmp_dir
  test "a plugin's app and its other client files are resources, and its tools say which only read", %{tmp_dir: folder} do
    OneCore.Test.server(folder)
    OneCore.Test.being_built()
    files = %{"notes.ex" => @plugin, "app.html" => "<p>Notes</p>", "app.js" => "1", "icon.svg" => "<svg/>"}
    assert {:ok, 1, ""} = Plugins.deploy("network", "notes", files)

    assert [%{"uri" => "skill://" <> _}, app, script, icon] = ask("resources/list", %{})["resources"]
    assert %{"name" => "Notes", "mimeType" => "text/html;profile=mcp-app", "icons" => [%{"src" => src}]} = app
    assert src == "data:image/svg+xml;base64," <> Base.encode64("<svg/>")
    assert ["ui:", "", "notes", version, "app.html"] = String.split(app["uri"], "/")
    assert script["uri"] == "ui://notes/#{version}/app.js"
    assert %{"mimeType" => "text/javascript"} = script
    assert %{"mimeType" => "image/svg+xml"} = icon

    assert %{"contents" => [%{"text" => "<p>Notes</p>", "mimeType" => "text/html;profile=mcp-app"}]} =
             ask("resources/read", %{uri: app["uri"]})

    # A version that's been replaced is still read by a client that's showing it.
    assert {:ok, 2, ""} = Plugins.deploy("network", "notes", %{files | "app.js" => "2"})
    assert %{"contents" => [%{"text" => "1"}]} = ask("resources/read", %{uri: script["uri"]})
    assert Enum.all?(ask("resources/list", %{})["resources"], &(not String.contains?(&1["uri"], version)))

    tools = ask("tools/list", %{})["tools"]
    reads = for tool <- tools, into: %{}, do: {tool["name"], tool["annotations"]["readOnlyHint"]}
    assert %{"notes_list" => true, "notes_add" => false} = reads
    assert ["notes_photo"] = for(%{"_meta" => %{"fixed" => true}} = tool <- tools, do: tool["name"])

    # A file a tool answers with is a picture an agent sees, or a file inside the answer.
    assert %{"content" => [%{"type" => "text", "text" => "[]"}]} = ask("tools/call", %{name: "notes_list"})

    assert %{"content" => [%{"type" => "image", "mimeType" => "image/png", "data" => "iVA="}]} =
             ask("tools/call", %{name: "notes_photo"})

    assert %{
             "content" => [
               %{"type" => "resource", "resource" => %{"mimeType" => "application/pdf", "blob" => "JVBERg=="}}
             ]
           } =
             ask("tools/call", %{name: "notes_scan"})
  end

  @tag :tmp_dir
  test "skills are listed, given and read as MCP's Skills Extension says, to whoever their plugins run for", %{
    tmp_dir: folder
  } do
    OneCore.Test.server(folder)
    Application.put_env(:one_core, :development, true)
    on_exit(fn -> Application.delete_env(:one_core, :development) end)
    {alice, bob, holder} = {%{as: "alice"}, %{as: "bob"}, %{link: "users/alice/a-secret"}}

    skill = "---\nname: notes\ndescription: Read before keeping a note.\nlicense: MIT\n---\n\nUse `add` to keep one.\n"
    assert {:ok, 1, ""} = Plugins.deploy("users/alice", "notes", %{"SKILL.md" => skill})
    digest = &("sha256:" <> Base.encode16(:crypto.hash(:sha256, &1), case: :lower))
    {own, uri} = {"skill://building-plugins/SKILL.md", "skill://notes/SKILL.md"}

    # A server says it serves skills, and points at them without saying what they say.
    assert %{"capabilities" => capabilities, "instructions" => told} = ask("initialize", %{}, alice)
    assert %{"resources" => %{}, "extensions" => %{"io.modelcontextprotocol/skills" => %{}}} = capabilities
    assert told =~ own and byte_size(told) < 1000

    # What's listed of a skill is what its file says of itself, and the file by what's in it.
    assert %{"resultType" => "complete", "skills" => [%{"uri" => ^own} = core, notes]} = ask("skills/list", %{}, alice)

    frontmatter = %{"name" => "notes", "description" => "Read before keeping a note.", "license" => "MIT"}
    file = %{"uri" => uri, "digest" => digest.(skill), "size" => byte_size(skill)}
    assert notes == %{"uri" => uri, "frontmatter" => frontmatter, "resources" => [file]}

    assert %{"resultType" => "complete", "skill" => ^notes} = ask("skills/get", %{uri: uri}, alice)

    assert %{"contents" => [%{"uri" => ^uri, "mimeType" => "text/markdown", "text" => ^skill}]} =
             ask("resources/read", %{uri: uri}, alice)

    resource = %{
      "uri" => uri,
      "name" => "notes",
      "description" => frontmatter["description"],
      "mimeType" => "text/markdown"
    }

    assert resource in ask("resources/list", %{}, alice)["resources"]

    # One Core's own is a skill like any other.
    assert %{"contents" => [%{"text" => "---\nname: building-plugins\ndescription: " <> _ = text}]} =
             ask("resources/read", %{uri: own}, bob)

    assert %{"frontmatter" => %{"name" => "building-plugins"}, "resources" => [%{"uri" => ^own} = file]} = core
    assert file == %{"uri" => own, "digest" => digest.(text), "size" => byte_size(text)}

    # A plugin's skill is seen by whoever the plugin runs for, and a sharing link's holder sees none.
    assert %{"skills" => [^core]} = ask("skills/list", %{}, bob)
    assert %{"error" => %{"code" => -32602}} = answer("skills/get", %{uri: uri}, bob)
    assert %{"error" => %{"code" => -32602}} = answer("resources/read", %{uri: uri}, bob)
    assert %{"skills" => []} = ask("skills/list", %{}, holder)
    assert %{"error" => %{"code" => -32602}} = answer("skills/get", %{uri: uri}, holder)

    assert %{"error" => %{"message" => "A sharing link's holder reads nothing" <> _}} =
             answer("resources/read", %{uri: uri}, holder)

    refute ask("initialize", %{}, holder)["instructions"] =~ "skill"

    # A SKILL.md that isn't a skill of the plugin's name fails the deploy, with how to put it right.
    deploy = &Plugins.deploy("users/alice", "notes", %{"SKILL.md" => &1})

    assert {:error, "SKILL.md names its skill \"diary\". A plugin's skill has the plugin's name: `name: notes`."} =
             deploy.(String.replace(skill, "name: notes", "name: diary"))

    assert {:error, "SKILL.md starts with its skill's name" <> _} = deploy.("Use `add` to keep one.")
    assert {:error, "SKILL.md starts with its skill's name" <> _} = deploy.("---\nname: notes\n---\n")
    assert {:error, "SKILL.md's first lines" <> _} = deploy.("---\nname: notes\ndescription: Read: now\n---\n")
    assert %{"skill" => ^notes} = ask("skills/get", %{uri: uri}, alice)
  end
end
