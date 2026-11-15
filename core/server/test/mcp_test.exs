defmodule OneCore.McpTest do
  use ExUnit.Case
  alias OneCore.{Mcp, Plugins}
  alias OneCore.Plugins.{Added, Deploy, Version}

  # What a client's Rust and an app's JavaScript read of a plugin, as the server's Elixir says it.
  @plugin """
  defmodule Notes do
    @behaviour OneCore.Plugin

    @impl true
    def tools do
      [
        %{name: "list", read_only: true, description: "Lists the notes", inputs: %{type: "object"}},
        %{name: "add", description: "Adds a note", inputs: %{type: "object"}},
        %{name: "photo", read_only: true, description: "A note's photo", inputs: %{type: "object"}},
        %{name: "scan", read_only: true, description: "A note's scan", inputs: %{type: "object"}}
      ]
    end

    @impl true
    def apps, do: [%{name: "Notes", page: "client/index.html", icon: "client/icon.svg"}]

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
  test "what a plugin's app loads is listed, the rest of client/ is read by name, and its tools say which only read", %{
    tmp_dir: folder
  } do
    OneCore.Test.server(folder)
    OneCore.Test.being_built()
    assert {:ok, 1, ""} = Deploy.deploy("network", "ui", OneCore.Test.kit("export const kit = 1"))

    page = ~s(<!doctype html><title>Notes</title><script type="module" src="./app.js"></script>)

    files = %{
      "server/notes.ex" => @plugin,
      "client/index.html" => page,
      "client/app.js" => "document.title = 1",
      "client/icon.svg" => "<svg/>"
    }

    assert {:ok, 1, ""} = Deploy.deploy("network", "notes", files)

    resources = for %{"uri" => "ui://notes/" <> _} = resource <- ask("resources/list", %{})["resources"], do: resource
    assert [script, icon, app] = resources
    assert %{"name" => "Notes", "mimeType" => "text/html;profile=mcp-app", "icons" => [%{"src" => src}]} = app
    assert src == "data:image/svg+xml;base64," <> Base.encode64("<svg/>")
    assert ["ui:", "", "notes", version, "client", "index.html"] = String.split(app["uri"], "/")
    assert %{"mimeType" => "text/javascript", "uri" => "ui://notes/" <> _} = script
    assert %{"mimeType" => "image/svg+xml"} = icon

    assert %{"contents" => [%{"text" => "<!doctype html>" <> _, "mimeType" => "text/html;profile=mcp-app"}]} =
             ask("resources/read", %{uri: app["uri"]})

    # A sourcemap, which no page loads, is read by name, and nothing outside client/ is a resource.
    assert %{"contents" => [%{"mimeType" => "application/json"}]} =
             ask("resources/read", %{uri: script["uri"] <> ".map"})

    assert %{"error" => %{"code" => -32002}} =
             answer("resources/read", %{uri: "ui://notes/#{version}/server/notes.ex"}, %{as: "owner"})

    # A version that's been replaced is still read by a client that's showing it.
    assert {:ok, 2, ""} = Deploy.deploy("network", "notes", %{files | "client/app.js" => "document.title = 2"})
    assert %{"contents" => [%{"text" => text}]} = ask("resources/read", %{uri: script["uri"]})
    assert text =~ "document.title=1" or text =~ "document.title = 1"
    assert Enum.all?(ask("resources/list", %{})["resources"], &(not String.contains?(&1["uri"], version)))

    tools = ask("tools/list", %{})["tools"]
    reads = for tool <- tools, into: %{}, do: {tool["name"], tool["annotations"]["readOnlyHint"]}
    assert %{"notes_list" => true, "notes_add" => false} = reads

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
    assert {:ok, 1, ""} = Deploy.deploy("users/alice", "notes", %{"SKILL.md" => skill})
    digest = &("sha256:" <> Base.encode16(:crypto.hash(:sha256, &1), case: :lower))
    {own, uri} = {"skill://building-plugins/SKILL.md", "skill://notes/notes/SKILL.md"}

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
    deploy = &Deploy.deploy("users/alice", "notes", %{"SKILL.md" => &1})

    assert {:error,
            "SKILL.md names its skill \"diary\". A skill is named as its folder is, the plugin's at its root: `name: notes`."} =
             deploy.(String.replace(skill, "name: notes", "name: diary"))

    assert {:error, "SKILL.md starts with its skill's name" <> _} = deploy.("Use `add` to keep one.")
    assert {:error, "SKILL.md starts with its skill's name" <> _} = deploy.("---\nname: notes\n---\n")
    assert {:error, "SKILL.md's first lines" <> _} = deploy.("---\nname: notes\ndescription: Read: now\n---\n")
    assert %{"skill" => ^notes} = ask("skills/get", %{uri: uri}, alice)

    # A plugin offers more skills in folders of skills/, each with the files it points to.
    more = "---\nname: notes-by-voice\ndescription: Read before keeping a note said aloud.\n---\n\nSee WORDS.md.\n"

    files = %{
      "SKILL.md" => skill,
      "skills/notes-by-voice/SKILL.md" => more,
      "skills/notes-by-voice/WORDS.md" => "Say it."
    }

    assert {:ok, 2, ""} = Deploy.deploy("users/alice", "notes", files)
    assert %{"skills" => [^core, ^notes, voice]} = ask("skills/list", %{}, alice)
    voice_uri = "skill://notes/notes/skills/notes-by-voice/SKILL.md"
    words = "skill://notes/notes/skills/notes-by-voice/WORDS.md"

    assert %{"uri" => ^voice_uri, "resources" => [%{"uri" => ^voice_uri}, %{"uri" => ^words, "size" => 7}]} = voice
    assert %{"contents" => [%{"text" => "Say it."}]} = ask("resources/read", %{uri: words}, alice)
    refute Enum.any?(Version.files(Plugins.running("users/alice")["notes"]), &String.starts_with?(&1, "skills/"))

    # A skill is found by the name its plugin is added by, so neither another plugin by the same own name nor one
    # named as One Core's own skill takes its place.
    assert :ok = Added.add("users/bob", "journal", "users/alice/plugins/notes")
    other = String.replace(skill, "keep a note", "keep another note")
    assert {:ok, 1, ""} = Deploy.deploy("users/bob", "notes", %{"SKILL.md" => other})
    builder = String.replace(skill, "name: notes", "name: building-plugins")
    assert {:ok, 1, ""} = Deploy.deploy("users/bob", "building-plugins", %{"SKILL.md" => builder})
    bobs = for %{"uri" => uri} <- ask("skills/list", %{}, bob)["skills"], do: uri

    assert bobs == [
             own,
             "skill://building-plugins/building-plugins/SKILL.md",
             "skill://journal/notes/SKILL.md",
             "skill://journal/notes/skills/notes-by-voice/SKILL.md",
             "skill://notes/notes/SKILL.md"
           ]

    assert %{"contents" => [%{"text" => ^other}]} = ask("resources/read", %{uri: "skill://notes/notes/SKILL.md"}, bob)
    assert %{"contents" => [%{"text" => ^skill}]} = ask("resources/read", %{uri: "skill://journal/notes/SKILL.md"}, bob)
  end
end
