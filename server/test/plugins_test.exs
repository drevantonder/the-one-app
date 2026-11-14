defmodule OneCore.PluginsTest do
  use ExUnit.Case
  alias OneCore.Plugins

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
    wait = fn -> Task.async(fn -> Plugins.call("slow", "wait", %{"test" => test}, OneCore.User.anyone()) end) end

    assert {:ok, 1, ""} = Plugins.deploy("network", "slow", %{"slow.ex" => slow(1)})
    first = wait.()
    assert_receive {:waiting, first_call}

    # More updates than the two versions of a module the VM keeps.
    assert {:ok, 2, ""} = Plugins.deploy("network", "slow", %{"slow.ex" => slow(2)})
    assert {:ok, 3, ""} = Plugins.deploy("network", "slow", %{"slow.ex" => slow(3)})

    assert {:error, why} =
             Plugins.deploy("network", "slow", %{"slow.ex" => "defmodule Slow do\n  def broken(, do: 1\nend"})

    assert why =~ "slow.ex:2"

    next = wait.()
    assert_receive {:waiting, next_call}
    send(next_call, :finish)
    assert Task.await(next) == {:ok, 3}
    assert length(loaded()) == 2

    send(first_call, :finish)
    assert Task.await(first) == {:ok, 1}
    assert length(loaded()) == 1

    # A server starting on the same bucket runs what was deployed.
    stop_supervised!(Plugins)
    start_supervised!(Plugins)
    last = wait.()
    assert_receive {:waiting, last_call}
    send(last_call, :finish)
    assert Task.await(last) == {:ok, 3}
  end
end
