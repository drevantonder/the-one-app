defmodule OneCore.HooksTest do
  use ExUnit.Case
  alias OneCore.User
  alias OneCore.Plugins.{Installed, Calls, Deploy}

  @echo """
  defmodule Echo do
    @behaviour OneCore.Plugin

    @impl true
    def tools do
      inputs = %{type: "object", properties: %{word: %{type: "string"}}, required: ["word"]}
      [%{name: "say", description: "Says a word back", inputs: inputs}]
    end

    @impl true
    def call("say", %{"word" => word}, _call), do: word
  end
  """

  # A plugin that changes the word a tool is asked with: to capitals, or to small letters.
  defp changes(name, change) do
    """
    defmodule #{name} do
      @behaviour OneCore.Plugin

      @impl true
      def hooks, do: ["inputs"]

      @impl true
      def hook("inputs", %{"inputs" => %{"word" => "seven"}}, _call), do: %{"word" => 7}
      def hook("inputs", %{"inputs" => %{"word" => word}}, _call), do: %{"word" => String.#{change}(word)}
    end
    """
  end

  @gate """
  defmodule Gate do
    @behaviour OneCore.Plugin

    @impl true
    def hooks, do: ["call"]

    @impl true
    def hook("call", %{"inputs" => %{"word" => "NO"}}, call), do: {:refuse, "The gate says no to \#{call.for}."}
    def hook("call", %{"inputs" => %{"word" => "FORGOT"}}, _call), do: nil
    def hook("call", _about, _call), do: :ok
  end
  """

  @ears """
  defmodule Ears do
    @behaviour OneCore.Plugin

    @impl true
    def hooks, do: ["called"]

    @impl true
    def hook("called", about, call), do: send(:hooks_test, {:called, about, call.for})
  end
  """

  defp say(word), do: Calls.call("echo", "say", %{"word" => word}, OneCore.Test.owner())

  @tag :tmp_dir
  test "plugins change, decide and hear a call, and neither their order nor a forgotten answer lets one through", %{
    tmp_dir: folder
  } do
    OneCore.Test.server(folder)
    Process.register(self(), :hooks_test)
    anyone = OneCore.Test.owner()

    for {name, source} <- [echo: @echo, upper: changes("Upper", "upcase"), gate: @gate, ears: @ears],
        do: assert({:ok, 1, ""} = Deploy.deploy("network", "#{name}", %{"server/#{name}.ex" => source}))

    assert {:error, "hooks/0 gives" <> _} =
             Deploy.deploy("network", "wrong", %{
               "server/wrong.ex" => String.replace(@ears, ~s(["called"]), ~s(["calling"]))
             })

    # The tool is called with what was changed, and those that hear are told who changed what.
    assert say("hello") == {:ok, "HELLO"}

    assert_receive {:called, %{"plugin" => "echo", "tool" => "say", "inputs" => %{"word" => "HELLO"}} = about, ^anyone}
    assert %{"changed" => %{"word" => "upper"}, "error" => nil, "took" => took} = about
    assert is_integer(took)

    # One that decides refuses with its reason, and so does one that forgot to answer. The tool isn't called, and
    # those that hear are told it was refused, and why.
    assert say("no") == {:error, "The gate says no to #{anyone}."}
    assert_receive {:called, %{"tool" => "say", "error" => "The gate says no to " <> _}, ^anyone}
    assert {:error, "gate couldn't decide it: It answered nil" <> _} = say("forgot")
    assert_receive {:called, %{"error" => "gate couldn't decide it" <> _}, ^anyone}
    # A change the tool doesn't take doesn't reach it.
    assert {:error, "Its inputs were changed (word by upper)" <> _} = say("seven")
    assert_receive {:called, %{"error" => "Its inputs were changed" <> _}, ^anyone}
    # Nor does a tool that isn't there.
    assert {:error, "echo has no tool named shout" <> _} = Calls.call("echo", "shout", %{}, anyone)
    assert_receive {:called, %{"tool" => "shout", "error" => "echo has no tool named shout" <> _}, ^anyone}

    # Two that change the same thing agree, or the call is refused: neither is the one that came last.
    assert {:ok, 1, ""} = Deploy.deploy("network", "lower", %{"server/lower.ex" => changes("Lower", "downcase")})
    assert say("42") == {:ok, "42"}
    assert say("hello") == {:error, "lower and upper each change word to something else. One has to give way."}

    # One Core's own tools are called through the same hooks: changed, decided and heard as any plugin's.
    refused = """
    defmodule Refused do
      @behaviour OneCore.Plugin

      @impl true
      def hooks, do: ["inputs", "call"]

      @impl true
      def hook("inputs", %{"plugin" => nil, "tool" => "remove", "inputs" => %{"name" => "ECHO"}}, _call), do: %{"name" => "echo"}
      def hook("inputs", _about, _call), do: %{}
      def hook("call", %{"plugin" => nil, "tool" => "deploy"}, call), do: {:refuse, "Nothing is deployed for \#{call.for}."}
      def hook("call", _about, _call), do: :ok
    end
    """

    for name <- ["upper", "lower"], do: assert(:ok = Installed.uninstall("network", name))
    assert {:ok, 1, ""} = Deploy.deploy("network", "refused", %{"server/refused.ex" => refused})

    assert OneCore.Tools.call("deploy", %{"name" => "more", "files" => %{}}, {anyone, anyone}) ==
             {:error, "Nothing is deployed for #{anyone}."}

    assert OneCore.Tools.call("deploy", %{"name" => 7}, {anyone, anyone}) ==
             {:error, "name is text, not a whole number.\nfiles is needed."}

    assert OneCore.Tools.call("uninstall", %{"name" => "ECHO"}, {anyone, "network"}) ==
             {:ok, "echo is removed from what's installed for network."}

    assert_receive {:called,
                    %{"plugin" => nil, "tool" => "remove", "changed" => %{"name" => "refused"}, "error" => nil}, _}

    assert {:error, "There's no plugin named echo"} = say("hello")

    assert {:error, "services isn't a tool that a sharing link's holder calls"} =
             OneCore.Tools.call("services", %{}, {User.link("x"), User.link("x")})

    assert OneCore.Tools.call("nothing", %{}, {anyone, anyone}) == :none

    # A plugin that breaks has refused, for whoever it was installed for and nobody else, and One Core's own tools
    # with the rest. It has no say in its own removal or in being deployed over, which is how it's undone.
    broken = fn what ->
      """
      defmodule Refused do
        @behaviour OneCore.Plugin

        @impl true
        def hooks, do: ["call"]

        @impl true
        def hook("call", _about, _call), do: #{what}
      end
      """
    end

    assert :ok = Installed.uninstall("network", "refused")
    OneCore.Test.being_built()
    assert {:ok, 1, ""} = Deploy.deploy("users/alice", "echo", %{"server/echo.ex" => @echo})

    assert {:ok, 1, ""} =
             Deploy.deploy("users/alice", "refused", %{"server/refused.ex" => broken.("raise(\"Broken.\")")})

    alice = {"users/alice", "users/alice"}

    assert Calls.call("echo", "say", %{"word" => "hi"}, "users/alice") ==
             {:error, "refused couldn't decide it: Broken."}

    assert OneCore.Tools.call("services", %{}, alice) == {:error, "refused couldn't decide it: Broken."}
    assert OneCore.Tools.call("uninstall", %{"name" => "echo"}, alice) == {:error, "refused couldn't decide it: Broken."}
    assert {:ok, 1, ""} = Deploy.deploy("users/bob", "echo", %{"server/echo.ex" => @echo})
    assert Calls.call("echo", "say", %{"word" => "hi"}, "users/bob") == {:ok, "hi"}

    assert {:ok, "refused is running, at version 2."} =
             OneCore.Tools.call(
               "deploy",
               %{"name" => "refused", "files" => %{"server/refused.ex" => broken.("{:refuse, \"Still broken.\"}")}},
               alice
             )

    assert Calls.call("echo", "say", %{"word" => "hi"}, "users/alice") == {:error, "Still broken."}

    # One that never answers has refused once it's had ten seconds.
    hangs = %{"name" => "refused", "files" => %{"server/refused.ex" => broken.("Process.sleep(:infinity)")}}
    assert {:ok, "refused is running, at version 3."} = OneCore.Tools.call("deploy", hangs, alice)
    {took, refused} = :timer.tc(fn -> Calls.call("echo", "say", %{"word" => "hi"}, "users/alice") end, :second)
    assert refused == {:error, "refused couldn't decide it: refused didn't answer call within 10 seconds"}
    assert took == 10
    assert {:ok, _} = OneCore.Tools.call("uninstall", %{"name" => "refused"}, alice)
    assert Calls.call("echo", "say", %{"word" => "hi"}, "users/alice") == {:ok, "hi"}
  end
end
