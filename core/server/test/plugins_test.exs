defmodule OneCore.PluginsTest do
  use ExUnit.Case
  alias OneCore.Plugins
  alias OneCore.Plugins.{Installed, Calls, Deploy, Store, Version}

  # A plugin of two modules. Its tool waits to be told to finish, then asks its other module which version it is.
  defp slow(number) do
    """
    defmodule Slow do
      @behaviour OneCore.Plugin

      @impl true
      def tools, do: [%{name: "wait", description: "Says its version once it's told to finish", inputs: %{}}]

      @impl true
      def call("wait", %{"test" => test}, _call) do
        send(test, {:waiting, self()})

        receive do
          :finish -> Slow.Version.number()
        end
      end
    end

    defmodule Slow.Version do
      def number, do: #{number}
    end
    """
  end

  defp loaded, do: for({module, _} <- :code.all_loaded(), String.ends_with?("#{module}", ".Slow"), do: module)

  @tag :tmp_dir
  test "a call running through updates finishes on its own version, and the next gets the newest", %{tmp_dir: folder} do
    OneCore.Test.server(folder)
    test = self()
    wait = fn -> Task.async(fn -> Calls.call("slow", "wait", %{"test" => test}, OneCore.Test.owner()) end) end

    assert {:ok, 1, ""} = Deploy.deploy("network", "slow", %{"server/slow.ex" => slow(1)})
    first = wait.()
    assert_receive {:waiting, first_call}

    # More updates than the two versions of a module the VM keeps.
    assert {:ok, 2, ""} = Deploy.deploy("network", "slow", %{"server/slow.ex" => slow(2)})
    assert {:ok, 3, ""} = Deploy.deploy("network", "slow", %{"server/slow.ex" => slow(3)})

    assert {:error, why} =
             Deploy.deploy("network", "slow", %{"server/slow.ex" => "defmodule Slow do\n  def broken(, do: 1\nend"})

    assert why =~ "slow.ex:2"

    next = wait.()
    assert_receive {:waiting, next_call}
    send(next_call, :finish)
    assert Task.await(next) == {:ok, 3}
    assert length(loaded()) == 2

    send(first_call, :finish)
    assert Task.await(first) == {:ok, 1}

    # A plugin is installed for someone once the plugins it uses are there for them, as when it's deployed.
    uses = ~s(defmodule Uses do\n  @behaviour OneCore.Plugin\n  @impl true\n  def uses, do: ["thing"]\nend)
    assert {:ok, 1, ""} = Deploy.deploy("users/alice", "thing", %{"client/thing.js" => "1"})
    assert {:ok, 1, ""} = Deploy.deploy("users/alice", "uses", %{"server/uses.ex" => uses})

    assert Installed.install("users/bob", "uses", "users/alice/plugins/uses") ==
             {:error,
              "uses uses thing, which isn't installed for users/bob or the network. Add it for them first, or deploy it."}

    assert :ok = Installed.install("users/bob", "thing", "users/alice/plugins/thing")
    assert :ok = Installed.install("users/bob", "uses", "users/alice/plugins/uses")
    assert length(loaded()) == 1

    # A server starting on the same bucket runs what was deployed.
    stop_supervised!(Plugins)
    start_supervised!(Plugins)
    last = wait.()
    assert_receive {:waiting, last_call}
    send(last_call, :finish)
    assert Task.await(last) == {:ok, 3}
  end

  @tag :tmp_dir
  test "an app is built from its source as it's deployed, gives its source back, and is built again when the UI library changes",
       %{tmp_dir: folder} do
    OneCore.Test.server(folder)
    assert {:ok, 1, ""} = Deploy.deploy("network", "ui", OneCore.Test.ui("export const ui = 1"))

    page = ~s(<!doctype html><title>Notes</title><script type="module" src="./app.jsx"></script>)

    app = """
    import { render } from "@solidjs/web"
    import { Screen } from "ui"
    render(() => <Screen heading="Notes" />, document.body)
    """

    source = %{"client/index.html" => page, "client/app.jsx" => app, "server/notes.txt" => "kept on servers"}
    assert {:ok, 1, ""} = Deploy.deploy("network", "notes", source)

    version = Plugins.running("network")["notes"]
    # A client is listed what a page loads, and nothing else of the plugin.
    assert "client/index.html" in Version.files(version) and
             Enum.any?(Version.files(version), &(&1 =~ ~r{^client/assets/.*\.js$}))

    assert Enum.all?(Version.files(version), &(Path.extname(&1) in ~w(.html .js .css .svg)))
    refute "server/notes.txt" in Version.files(version)
    {:ok, built} = Store.file(version.artifact, version.code, "client/index.html")
    assert built =~ "/ui/client/ui.css"
    assert Store.source(version.artifact, version.code) == source

    broken =
      Map.put(
        source,
        "client/app.jsx",
        String.replace(app, "<Screen heading=\"Notes\" />", "<Screen heading=\"Notes\">")
      )

    assert {:error, why} = Deploy.deploy("network", "notes", broken)
    assert why =~ "app.jsx:3:"

    # A new UI library: the app is built again against it, from the source it was deployed with.
    assert {:ok, 2, ""} = Deploy.deploy("network", "ui", OneCore.Test.ui("export const ui = 2"))
    ui = Plugins.running("network")["ui"].code
    rebuilt = Enum.find_value(1..100, fn _ -> Process.sleep(50) && Plugins.running("network")["notes"].ui == ui end)
    assert rebuilt
    assert Store.source(version.artifact, Plugins.running("network")["notes"].code) == source
  end

  # A plugin the server ships, kept in a folder of its own, whose tool says which version of the folder it is.
  defp ships(folder, number) do
    File.mkdir_p!(Path.join(folder, "server"))

    File.write!(Path.join(folder, "server/said.ex"), """
    defmodule Said do
      @behaviour OneCore.Plugin
      @impl true
      def tools, do: [%{name: "which", description: "Says which it is", inputs: %{}}]
      @impl true
      def call("which", _inputs, _call), do: #{number}
    end
    """)
  end

  @tag :tmp_dir
  test "a server brings along the plugins it ships when they've changed, but not over one deployed on purpose", %{
    tmp_dir: folder
  } do
    shipped = Path.join(folder, "shipped/said")
    Application.put_env(:one_core, :shipped, %{"said" => shipped})
    on_exit(fn -> Application.delete_env(:one_core, :shipped) end)
    OneCore.Test.server(folder)
    which = fn -> Calls.call("said", "which", %{}, OneCore.Test.owner()) end

    ships(shipped, 1)
    assert [{"said", {:ok, 1, _}}] = Deploy.ship()
    assert Deploy.ship() == []
    assert which.() == {:ok, 1}

    # An updated server ships other files, and the network takes them.
    ships(shipped, 2)
    assert [{"said", {:ok, 2, _}}] = Deploy.ship()
    assert which.() == {:ok, 2}

    # Someone deploys their own, and a server that ships other files again leaves it.
    {:ok, 3, _} =
      Deploy.deploy("network", "said", %{
        "server/said.ex" => File.read!(Path.join(shipped, "server/said.ex")) |> String.replace("2", "30")
      })

    ships(shipped, 4)
    assert Deploy.ship() == []
    assert which.() == {:ok, 30}

    # Someone deploys the very files a server shipped once, and an updated server takes it over again.
    ships(shipped, 1)

    {:ok, 4, _} =
      Deploy.deploy("network", "said", %{"server/said.ex" => File.read!(Path.join(shipped, "server/said.ex"))})

    ships(shipped, 4)
    assert [{"said", {:ok, 5, _}}] = Deploy.ship()
    assert which.() == {:ok, 4}
    assert Deploy.shipping() == %{}

    # One that fails to ship says why.
    File.write!(Path.join(shipped, "server/said.ex"), "defmodule Said do")
    assert [{"said", {:error, why}}] = Deploy.ship()
    assert %{"said" => ^why} = Deploy.shipping()
  end

  @tag :tmp_dir
  test "a file that isn't text is kept as it is, read by the plugin's own code, and given back as deploy takes it",
       %{tmp_dir: folder} do
    OneCore.Test.server(folder)
    form = :crypto.strong_rand_bytes(4096) <> <<0xFF, 0xFE>>

    plugin = """
    defmodule Forms do
      @behaviour OneCore.Plugin
      @impl true
      def tools, do: [%{name: "blank", description: "The blank form's size", inputs: %{}}]
      @impl true
      def call("blank", _inputs, _call), do: byte_size(OneCore.Plugin.file(__MODULE__, "templates/form.xlsx"))
    end
    """

    deployed = Store.sent(%{"server/forms.ex" => plugin, "templates/form.xlsx" => form})
    assert %{"base64" => _} = deployed["templates/form.xlsx"]
    assert {:ok, 1, ""} = Deploy.deploy("network", "forms", Store.received(JSON.decode!(JSON.encode!(deployed))))

    # A server starting on the same bucket reads the same bytes.
    stop_supervised!(Plugins)
    start_supervised!(Plugins)
    assert Calls.call("forms", "blank", %{}, OneCore.Test.owner()) == {:ok, byte_size(form)}
    version = Plugins.running("network")["forms"]
    assert Store.sent(Store.source(version.artifact, version.code)) == deployed
  end
end
