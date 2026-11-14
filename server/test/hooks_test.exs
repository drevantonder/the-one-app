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
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.Folder, Path.join(folder, "bucket")})
    Application.put_env(:one_core, :home, Path.join(folder, "server"))
    start_supervised!(Plugins)
    start_supervised!(OneCore.Services)
    Process.register(self(), :hooks_test)
    anyone = User.anyone()

    for {name, source} <- [echo: @echo, upper: changes("Upper", "upcase"), gate: @gate, ears: @ears],
        do: assert({:ok, 1, ""} = Plugins.deploy("#{name}", %{"#{name}.ex" => source}))

    assert {:error, "hooks/0 gives" <> _} =
             Plugins.deploy("wrong", %{"wrong.ex" => String.replace(@ears, ~s(["called"]), ~s(["calling"]))})

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
    assert {:ok, 1, ""} = Plugins.deploy("lower", %{"lower.ex" => changes("Lower", "downcase")})
    assert say("42") == {:ok, "42"}
    assert say("hello") == {:error, "lower and upper each change word to something else. One has to give way."}
  end
end
