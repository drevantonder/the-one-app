defmodule OneCore.ServicesTest do
  use ExUnit.Case
  alias OneCore.{Mcp, Plugins, Server, Services, User}

  # A plugin whose service keeps a count. Its tool asks the service to add one, and the service tells the plugin
  # each count, which the plugin sends to the test.
  defp counter(number, test) do
    """
    defmodule Counter do
      @behaviour OneCore.Plugin

      @impl true
      def tools, do: [%{name: "add", description: "Adds one, and gives the count and its version", inputs: %{}}]

      @impl true
      def call("add", _inputs, call), do: OneCore.Service.call(call, Counter.Kept, :add)

      def counted(%OneCore.Call{for: who}, count) do
        send(:erlang.list_to_pid(~c"#{:erlang.pid_to_list(test)}"), {:counted, who, count})
        :ok
      end
    end

    defmodule Counter.Kept do
      use GenServer
      @behaviour OneCore.Service

      @impl OneCore.Service
      def needs, do: "Nothing"

      def add(service), do: GenServer.call(service, :add)

      @impl GenServer
      def init(_), do: {:ok, 0}

      @impl GenServer
      def handle_call(:add, _from, count) do
        for who <- OneCore.Service.acts_for(), do: OneCore.Service.plugin(self(), who, :counted, [count + 1])
        {:reply, [count + 1, #{number}], count + 1}
      end

      @impl GenServer
      def code_change(_version, count, _extra), do: {:ok, count + #{number} * 100}
    end
    """
  end

  defp loaded, do: for({module, _} <- :code.all_loaded(), String.ends_with?("#{module}", ".Counter.Kept"), do: module)

  defp ask(method, params, from) do
    JSON.decode!(Mcp.answer(JSON.encode!(%{jsonrpc: "2.0", id: 1, method: method, params: params}), from))
  end

  setup %{tmp_dir: folder} do
    OneCore.Test.server(folder)
    # What an earlier test loaded stays loaded, since nothing unloads a server's plugins as it stops.
    earlier = loaded()
    assert {:ok, 1, ""} = Plugins.deploy("network", "counter", %{"counter.ex" => counter(1, self())})
    assert :ok = Services.install("network/plugins/counter", "Counter.Kept")
    %{earlier: earlier}
  end

  @tag :tmp_dir
  test "a service keeps what it holds through its plugin's updates, in the newest code", %{earlier: earlier} do
    assert Plugins.call("counter", "add", %{}, User.anyone()) == {:ok, [1, 1]}
    # The plugin was added for the network, so that's who its service has it run for.
    assert_receive {:counted, "network", 1}
    service = Plugins.Process.whereis("network/plugins/counter", "Counter.Kept")

    # More updates than the two versions of a module the VM keeps.
    for number <- 2..4,
        do:
          assert({:ok, ^number, ""} = Plugins.deploy("network", "counter", %{"counter.ex" => counter(number, self())}))

    assert Plugins.call("counter", "add", %{}, User.anyone()) == {:ok, [902, 4]}
    assert Plugins.Process.whereis("network/plugins/counter", "Counter.Kept") == service
    assert length(loaded() -- earlier) == 1

    # A version without the service stops it, and one with it starts it afresh.
    assert {:ok, 5, ""} =
             Plugins.deploy("network", "counter", %{
               "counter.ex" => "defmodule Counter do\n  @behaviour OneCore.Plugin\nend"
             })

    assert Plugins.Process.whereis("network/plugins/counter", "Counter.Kept") == nil
    assert loaded() -- earlier == []
    assert {:ok, 6, ""} = Plugins.deploy("network", "counter", %{"counter.ex" => counter(6, self())})
    assert Plugins.call("counter", "add", %{}, User.anyone()) == {:ok, [1, 6]}
  end

  @tag :tmp_dir
  test "only a server acting for whoever it asks for calls a service's functions and the functions it calls" do
    stranger = String.duplicate("ab", 32)
    said = %{who: User.anyone(), for: User.anyone(), passed_on: true}
    service = %{plugin: "network/plugins/counter", service: "Counter.Kept", function: "add", arguments: [], _meta: said}
    plugin = %{added: "network", plugin: "counter", function: "counted", arguments: [7], _meta: said}

    # A client asks neither, whoever it says it is.
    assert %{"error" => %{"message" => "Only a server" <> _}} = ask("services/call", Map.delete(service, :_meta), nil)
    assert %{"error" => %{"message" => "Only a server" <> _}} = ask("plugins/call", Map.delete(plugin, :_meta), nil)

    assert %{"error" => %{"message" => "Only a server" <> _}} = ask("services/call", service, stranger)
    assert %{"error" => %{"message" => "Only a server" <> _}} = ask("plugins/call", plugin, stranger)
    refute_received {:counted, _, _}

    assert %{"result" => %{"answer" => [1, 1]}} = ask("services/call", service, Server.key())
    assert %{"result" => %{"answer" => "ok"}} = ask("plugins/call", plugin, Server.key())
    assert_received {:counted, _who, 7}

    # Neither is a tool, so nothing else reaches them.
    assert %{"result" => %{"isError" => true}} = ask("tools/call", %{name: "counter_counted", arguments: %{}}, stranger)
  end
end
