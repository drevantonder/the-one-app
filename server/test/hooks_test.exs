defmodule OneCore.HooksTest do
  use ExUnit.Case
  alias OneCore.{Plugins, User}

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

  defp say(word), do: Plugins.call("echo", "say", %{"word" => word}, User.anyone())

  @tag :tmp_dir
  test "plugins change, decide and hear a call, and neither their order nor a forgotten answer lets one through", %{
    tmp_dir: folder
  } do
    OneCore.Test.server(folder)
    Process.register(self(), :hooks_test)
    anyone = User.anyone()

    for {name, source} <- [echo: @echo, upper: changes("Upper", "upcase"), gate: @gate, ears: @ears],
        do: assert({:ok, 1, ""} = Plugins.deploy("network", "#{name}", %{"#{name}.ex" => source}))

    assert {:error, "hooks/0 gives" <> _} =
             Plugins.deploy("network", "wrong", %{"wrong.ex" => String.replace(@ears, ~s(["called"]), ~s(["calling"]))})

    # The tool is called with what was changed, and those that hear are told who changed what.
    assert say("hello") == {:ok, "HELLO"}

    assert_receive {:called, %{"plugin" => "echo", "tool" => "say", "inputs" => %{"word" => "HELLO"}} = about, ^anyone}
    assert %{"changed" => %{"word" => "upper"}, "error" => nil, "took" => took} = about
    assert is_integer(took)

    # One that decides refuses with its reason, and so does one that forgot to answer. The tool isn't called.
    assert say("no") == {:error, "The gate says no to #{anyone}."}
    assert {:error, "gate couldn't decide it: It answered nil" <> _} = say("forgot")
    # A change the tool doesn't take doesn't reach it.
    assert {:error, "Its inputs were changed (word by upper)" <> _} = say("seven")
    refute_receive {:called, _about, _who}

    # Two that change the same thing agree, or the call is refused: neither is the one that came last.
    assert {:ok, 1, ""} = Plugins.deploy("network", "lower", %{"lower.ex" => changes("Lower", "downcase")})
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

    for name <- ["upper", "lower"], do: assert(:ok = Plugins.remove("network", name))
    assert {:ok, 1, ""} = Plugins.deploy("network", "refused", %{"refused.ex" => refused})

    assert OneCore.Tools.call("deploy", %{"name" => "more", "files" => %{}}, {anyone, anyone}) ==
             {:error, "Nothing is deployed for #{anyone}."}

    assert OneCore.Tools.call("deploy", %{"name" => 7}, {anyone, anyone}) ==
             {:error, "name is text, not a whole number.\nfiles is needed."}

    assert OneCore.Tools.call("remove", %{"name" => "ECHO"}, {anyone, "network"}) ==
             {:ok, "echo is removed from what's added for network."}

    assert_receive {:called,
                    %{"plugin" => nil, "tool" => "remove", "changed" => %{"name" => "refused"}, "error" => nil}, _}

    assert {:error, "There's no plugin named echo"} = say("hello")

    assert {:error, "services isn't a tool that a sharing link's holder calls"} =
             OneCore.Tools.call("services", %{}, {User.link("x"), User.link("x")})

    assert OneCore.Tools.call("nothing", %{}, {anyone, anyone}) == :none
  end
end
