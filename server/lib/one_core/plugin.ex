defmodule OneCore.Plugin do
  @guide """
  A plugin is a folder of Elixir files, and nothing is needed to write one but a text editor. Deploying it (the
  `deploy` tool) hands the server those files: it compiles them, says what's wrong if they don't compile, and
  otherwise runs them at once, with no restart. A plugin is updated by deploying it again. Calls already running
  finish on the version they started on, and the next call gets the new one.

  One module in the plugin says `@behaviour OneCore.Plugin`, and gives the plugin's tools:

      defmodule Hello do
        @behaviour OneCore.Plugin

        @impl true
        def tools do
          [
            %{
              name: "greet",
              description: "Greets someone, and says how many it has greeted",
              inputs: %{
                type: "object",
                properties: %{name: %{type: "string", description: "Who to greet"}},
                required: ["name"]
              }
            }
          ]
        end

        @impl true
        def guide, do: "Use greet when someone arrives."

        @impl true
        def call("greet", %{"name" => name}, %OneCore.Call{own: own}) do
          OneCore.Artifact.write(own, "greeted/" <> name, "")
          "Hello, \#{name}! That's \#{length(OneCore.Artifact.list(own, "greeted/"))} greeted."
        end
      end

  - `tools/0` lists each tool's `name` (lowercase letters, digits and `_`), its `description`, and its `inputs` as a
    JSON Schema object. A tool of the plugin `hello` named `greet` is the MCP tool `hello_greet` and the command
    `one hello greet`.
  - `call/3` runs a tool with its inputs, a map with string keys. It returns the answer: text, or anything JSON can
    hold. To refuse, raise, and the message is what the caller is told.
  - `guide/0`, if the plugin has one, is text for agents on when and how to use its tools.
  - The third argument to `call/3` is an `OneCore.Call`. Its `own` is the plugin's own artifact, where it keeps
    what it needs as blobs through `OneCore.Artifact`: `read/2`, `write/3` (only if absent), `replace/4` (only if
    unchanged) and `list/2`.
  - Every module is a `defmodule` at the top of a file, and a plugin can have as many files and modules as it
    likes. The code is free to use the machine it runs on.
  """
  @moduledoc "The interface a plugin's server part is written against.\n\n" <> @guide

  @doc "How to write a plugin, for whoever is about to."
  def guide, do: @guide

  @callback tools() :: [%{name: String.t(), description: String.t(), inputs: map}]
  @callback call(tool :: String.t(), inputs :: map, OneCore.Call.t()) :: term
  @callback guide() :: String.t()
  @optional_callbacks guide: 0
end
