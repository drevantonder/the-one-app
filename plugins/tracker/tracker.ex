defmodule Tracker do
  @moduledoc "Trackers for anything counted over time. Each tracker is one blob in the plugin's own artifact."
  @behaviour OneCore.Plugin
  alias OneCore.Artifact

  @impl true
  def guide do
    """
    Trackers count things over time: glasses of water, push-ups, hours slept. Add an entry when someone says they
    did something they track, and make a new tracker just by adding to it. An entry is stamped with the time it's
    added.
    """
  end

  @impl true
  def apps, do: [%{name: "Tracker", page: "app.html", icon: "icon.svg"}]

  @impl true
  def tools do
    [
      %{
        name: "add",
        description: "Adds an entry to a tracker, making the tracker if it's new",
        inputs: %{
          type: "object",
          properties: %{
            tracker: %{type: "string", description: "The tracker's name, such as water"},
            amount: %{type: "number", description: "How much. 1 if left out"},
            note: %{type: "string", description: "Anything worth remembering about this entry"}
          },
          required: ["tracker"]
        }
      },
      %{
        name: "list",
        read_only: true,
        description: "Lists the trackers with their totals, or one tracker's entries",
        inputs: %{
          type: "object",
          properties: %{
            tracker: %{type: "string", description: "The tracker whose entries to list. All trackers if left out"}
          }
        }
      }
    ]
  end

  @impl true
  def call("add", %{"tracker" => tracker} = inputs, %OneCore.Call{own: own}) do
    entry =
      Map.merge(
        %{"at" => DateTime.to_iso8601(DateTime.utc_now(:second)), "amount" => 1},
        Map.take(inputs, ["amount", "note"])
      )

    entries = add(own, tracker, entry)
    "Added. #{tracker} is at #{total(entries)} over #{length(entries)} entries."
  end

  def call("list", %{"tracker" => tracker}, %OneCore.Call{own: own}) do
    case Artifact.read(own, tracker <> ".json") do
      {:ok, entries, _tag} -> JSON.decode!(entries)
      :absent -> raise "There's no tracker named #{tracker}"
    end
  end

  def call("list", _inputs, %OneCore.Call{own: own}) do
    for blob <- Artifact.list(own) do
      {:ok, entries, _tag} = Artifact.read(own, blob)
      entries = JSON.decode!(entries)
      %{tracker: String.replace_suffix(blob, ".json", ""), total: total(entries), entries: length(entries)}
    end
  end

  # Two entries can be added at the same moment. The blob is written only if it's still as it was read, and
  # otherwise read again.
  defp add(own, tracker, entry) do
    blob = tracker <> ".json"

    {entries, written} =
      case Artifact.read(own, blob) do
        {:ok, entries, tag} ->
          entries = JSON.decode!(entries) ++ [entry]
          {entries, Artifact.replace(own, blob, JSON.encode!(entries), tag)}

        :absent ->
          {[entry], Artifact.write(own, blob, JSON.encode!([entry]))}
      end

    if written == :ok, do: entries, else: add(own, tracker, entry)
  end

  defp total(entries), do: Enum.sum_by(entries, & &1["amount"])
end
