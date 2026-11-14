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
        %{name: "add", description: "Adds a note", inputs: %{type: "object"}}
      ]
    end

    @impl true
    def apps, do: [%{name: "Notes", page: "app.html", icon: "icon.svg"}]

    @impl true
    def call(_tool, _inputs, _call), do: []
  end
  """

  defp ask(method, params) do
    JSON.decode!(Mcp.answer(JSON.encode!(%{jsonrpc: "2.0", id: 1, method: method, params: params}), nil))["result"]
  end

  @tag :tmp_dir
  test "a plugin's app and its other client files are resources, and its tools say which only read", %{tmp_dir: folder} do
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.Folder, Path.join(folder, "bucket")})
    Application.put_env(:one_core, :home, Path.join(folder, "server"))
    start_supervised!(Plugins)
    start_supervised!(OneCore.Services)
    files = %{"notes.ex" => @plugin, "app.html" => "<p>Notes</p>", "app.js" => "1", "icon.svg" => "<svg/>"}
    assert {:ok, 1, ""} = Plugins.deploy("notes", files)

    assert [app, script, icon] = ask("resources/list", %{})["resources"]
    assert %{"name" => "Notes", "mimeType" => "text/html;profile=mcp-app", "icons" => [%{"src" => src}]} = app
    assert src == "data:image/svg+xml;base64," <> Base.encode64("<svg/>")
    assert ["ui:", "", "notes", version, "app.html"] = String.split(app["uri"], "/")
    assert script["uri"] == "ui://notes/#{version}/app.js"
    assert %{"mimeType" => "text/javascript"} = script
    assert %{"mimeType" => "image/svg+xml"} = icon

    assert %{"contents" => [%{"text" => "<p>Notes</p>", "mimeType" => "text/html;profile=mcp-app"}]} =
             ask("resources/read", %{uri: app["uri"]})

    # A version that's been replaced is still read by a client that's showing it.
    assert {:ok, 2, ""} = Plugins.deploy("notes", %{files | "app.js" => "2"})
    assert %{"contents" => [%{"text" => "1"}]} = ask("resources/read", %{uri: script["uri"]})
    assert Enum.all?(ask("resources/list", %{})["resources"], &(not String.contains?(&1["uri"], version)))

    reads =
      for tool <- ask("tools/list", %{})["tools"], into: %{}, do: {tool["name"], tool["annotations"]["readOnlyHint"]}

    assert %{"notes_list" => true, "notes_add" => false} = reads
  end
end
