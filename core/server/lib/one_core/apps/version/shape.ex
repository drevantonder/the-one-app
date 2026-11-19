defmodule OneCore.Apps.Version.Shape do
  @moduledoc """
  The shape of each list an app says of itself (`OneCore.App`): `tools/0`, `entrypoints/0`, `widgets/0`, `offers/0` and
  `jobs/0`, each item a JSON Schema, and the problems `problems/3` finds in one, each saying where
  (`Glance.widgets()[0].sizes[0]`), what was given and what's expected, as a schema validator does.
  """
  alias ExJsonSchema.Validator.Error

  # The sizes a widget comes in, which are a phone's own: columns by rows of a Home Screen page's grid.
  @sizes ~w(2x2 4x2 4x4)
  @name %{"type" => "string", "pattern" => "^[a-z][a-z0-9_]*$", "description" => "lowercase letters, digits and _"}
  @text %{"type" => "string"}

  @items %{
    tools: %{
      "type" => "object",
      "required" => ["name", "description", "inputs"],
      "properties" => %{
        "name" => @name,
        "description" => %{"type" => "string", "description" => "a sentence saying what the tool does"},
        "inputs" => %{"type" => "object", "description" => "a JSON Schema of an object: %{type: \"object\", ...}"},
        "outputs" => %{
          "type" => "object",
          "required" => ["type"],
          "properties" => %{
            "type" => %{"enum" => ["object"], "description" => "outputs are a JSON Schema of an object"}
          },
          "description" => "a JSON Schema of the object the tool answers: %{type: \"object\", properties: ...}"
        },
        "read_only" => %{"type" => "boolean", "description" => "true for a tool that only reads"},
        "by_link" => %{"type" => "boolean"},
        "ui" => %{"type" => "string", "description" => "a page of client/"},
        "runtime_version" => %{
          "type" => "integer",
          "minimum" => 1,
          "description" => "the runtime version its page (ui) needs of a native host (core/ui/src/drawing-primitives/renderer.ts)"
        },
        "task_support" => %{"enum" => ["optional", "required"]}
      }
    },
    entrypoints: %{
      "type" => "object",
      "required" => ["page"],
      "properties" => %{
        "page" => %{"type" => "string", "description" => "an .html page of client/, such as \"client/index.html\""},
        "name" => @text,
        "icon" => %{"type" => "string", "description" => "a file of the app"},
        "category" => @text,
        "subject" => @text,
        "badge" => %{"type" => "string", "description" => "the name of one of this app's tools that only reads"},
        "runtime_version" => %{
          "type" => "integer",
          "minimum" => 1,
          "description" => "the runtime version its native page needs of a native host (core/ui/src/drawing-primitives/renderer.ts)"
        }
      }
    },
    widgets: %{
      "type" => "object",
      "required" => ["tool", "sizes"],
      "properties" => %{
        "tool" => %{"type" => "string", "description" => "the name of one of this app's tools that only reads"},
        "sizes" => %{
          "type" => "array",
          "minItems" => 1,
          "items" => %{"enum" => @sizes},
          "description" => "the sizes it comes in, of #{Enum.join(@sizes, ", ")}"
        },
        "name" => %{"type" => "string", "description" => "its name on the card, where it isn't its app's title"}
      }
    },
    offers: %{
      "type" => "object",
      "required" => ["name", "kind", "description"],
      "properties" => %{
        "name" => @name,
        "kind" => %{"enum" => ["observe", "mutate", "validate", "collect"]},
        "description" => @text,
        "untrusted" => %{"type" => "boolean", "description" => "true for a collect an untrusted app may answer too"}
      }
    },
    jobs: %{
      "type" => "object",
      "required" => ["tool", "every"],
      "properties" => %{
        "tool" => %{"type" => "string", "description" => "the name of one of this app's tools"},
        "every" => %{"type" => "integer", "minimum" => 1, "description" => "how many seconds apart"},
        "inputs" => %{"type" => "object", "description" => "what the tool takes, if it takes any"}
      }
    }
  }

  @doc "The sizes a widget comes in."
  def sizes, do: @sizes

  @doc """
  What's wrong with `list`, what an app's function `name` (`:widgets`) gave, as sentences: each says where in it
  (`prefix`, the app's module and function, then the item and its field), what was given and what's expected.
  `[]` if nothing is.
  """
  def problems(name, list, prefix) do
    where = "#{prefix}#{name}()"

    with {:list, true} <- {:list, is_list(list)},
         {:ok, json} <- json(list) do
      schema = ExJsonSchema.Schema.resolve(%{"type" => "array", "items" => Map.fetch!(@items, name)})

      case ExJsonSchema.Validator.validate(schema, json, error_formatter: false) do
        :ok -> []
        {:error, wrong} -> Enum.map(wrong, &sentence(&1, where, json, schema.schema))
      end
    else
      {:list, false} ->
        ["#{where}: got #{given(list)}, expected a list of #{keys(name)}"]

      :error ->
        [
          "#{where}: has a value JSON can't hold (a tuple, a function, a pid): give maps, lists, text, numbers, true, false and nil"
        ]
    end
  end

  defp keys(name) do
    %{"properties" => properties, "required" => required} = @items[name]
    "%{" <> Enum.map_join(Map.keys(properties), ", ", &"#{&1}: ...#{if &1 in required, do: "", else: "?"}") <> "}"
  end

  defp json(value) do
    {:ok, OneCore.JSON.normalize(value)}
  rescue
    _ -> :error
  end

  defp sentence(%Error{error: %Error.Required{missing: missing}, path: "#" <> path}, where, _json, schema) do
    at = String.split(path, "/", trim: true)
    Enum.map_join(missing, "\n", &"#{where}#{route(at ++ [&1])}: missing#{expects(schema, at ++ [&1])}")
  end

  defp sentence(%Error{error: error, path: "#" <> path}, where, json, schema) do
    at = String.split(path, "/", trim: true)
    "#{where}#{route(at)}: got #{given(value(json, at))}, expected #{expected(error, schema, at)}#{hint(schema, at)}"
  end

  defp expected(%Error.Type{expected: kinds}, _schema, _at), do: Enum.map_join(kinds, " or ", &kind/1)

  defp expected(%Error.Enum{}, schema, at),
    do: "one of " <> Enum.map_join(sub(schema, at)["enum"], ", ", &inspect/1)

  defp expected(%Error.Pattern{expected: pattern}, _schema, _at), do: "text matching #{pattern}"
  defp expected(%Error.MinItems{expected: least}, _schema, _at), do: "at least #{least} item#{if least > 1, do: "s"}"
  defp expected(%Error.Minimum{expected: least}, _schema, _at), do: "#{least} or more"
  defp expected(other, _schema, _at), do: to_string(other)

  defp expects(schema, at) do
    case sub(schema, at)["description"] do
      nil -> ""
      description -> ", expected #{description}"
    end
  end

  # What the schema says of the field, after what's expected.
  defp hint(schema, at) do
    case sub(schema, at)["description"] do
      nil -> ""
      description -> " (#{description})"
    end
  end

  # The schema of what's at `at`: an item's, or a field's by its name.
  defp sub(schema, at) do
    Enum.reduce(at, schema, fn part, schema ->
      cond do
        not is_map(schema) -> nil
        part =~ ~r/^\d+$/ -> schema["items"]
        true -> schema["properties"][part]
      end
    end) || %{}
  end

  defp value(json, at) do
    Enum.reduce(at, json, fn part, value ->
      case value do
        list when is_list(list) -> Enum.at(list, String.to_integer(part))
        %{} = map -> map[part]
        _ -> nil
      end
    end)
  end

  # Where, as Elixir would reach it: `[0].sizes[0]`.
  defp route(at), do: Enum.map_join(at, &if(&1 =~ ~r/^\d+$/, do: "[#{&1}]", else: ".#{&1}"))

  @doc "A value as it's said in a problem: short, as the app would write it."
  def given(value), do: inspect(value, limit: 4, printable_limit: 40)

  defp kind("string"), do: "text"
  defp kind("integer"), do: "a whole number"
  defp kind("number"), do: "a number"
  defp kind("boolean"), do: "true or false"
  defp kind("array"), do: "a list"
  defp kind("object"), do: "a map"
  defp kind("null"), do: "nil"
end
