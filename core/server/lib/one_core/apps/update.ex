defmodule OneCore.Apps.Update do
  @moduledoc """
  An app's updates. An app is one artifact with one version, the same everywhere it's installed, and a newer one
  is a new commit at the git source it was deployed from. So its updates are the app's, not any install's:

      trusted:    up to date ─newer version─► updating ─► up to date
                                                 └─fails, or its task ends first─► old version keeps running (why · Try Again)
      untrusted:  up to date ─newer version─► update available ─Update─► updating ─► up to date
                                                 (an even newer one ─► update available, the newest)
                                                                                └─fails, or its task ends first─► old version keeps running (why · Try Again)

  Its task ending first, as its server stopping ends it, is done where the version it took went live before, and fails
  where it didn't. A trusted app updates by itself; an untrusted one waits for Update, once an agent has read what
  changed. Each is one blob in the app's artifact, `update`: none while it's up to date, then its `state`,
  `available`, `updating` (at a `stage`) or `failed` (`why`), with the `commit` it offers or takes, the git `source`
  and `ref` it's at, the `commits` since the version running, and while it's updating, the server taking it (`by`)
  and the `task` that does, the MCP task whose end, however it ends, ends updating (`OneCore.Apps.Lifecycle`). Only
  servers acting for whoever the app is write it, as any of its blobs; `move/2` is the transition table, and each move is made with `OneCore.Artifact.update/3`, so two
  servers finding the same commit take it once.
  """
  alias OneCore.Artifact

  @doc "Where the app `artifact`'s updates are: its record, or `nil` while it's up to date."
  def read(artifact) do
    case Artifact.read(%Artifact{prefix: "#{artifact}/"}, "update") do
      {:ok, "", _tag} -> nil
      {:ok, json, _tag} -> JSON.decode!(json)
      :absent -> nil
    end
  end

  @doc "Moves it by `event` (`move/2`): the record it's at, `nil` for up to date, or `{:error, why}`."
  def change(artifact, event) do
    changed =
      Artifact.update(%Artifact{prefix: "#{artifact}/"}, "update", fn was ->
        record = if was in [nil, ""], do: nil, else: JSON.decode!(was)

        case move(record, event) do
          {:ok, nil} -> ""
          {:ok, record} -> JSON.encode!(record)
          {:error, why} -> throw({__MODULE__, why})
        end
      end)

    if changed in [nil, ""], do: nil, else: JSON.decode!(changed)
  catch
    {__MODULE__, why} -> {:error, why}
  end

  @doc """
  The transition table, from up to date (`nil`): `{:newer, offer, trusted, by, task}`, a newer commit found, which a
  trusted app takes at once and an untrusted one offers; `:current`, nothing newer than what runs, which drops an offer or a failure left behind; `{:update, by, task}` (Update, and Try Again); `{:stage,
  stage}`; `:done`; and `{:fail, why}`. `by` is the server that takes it, which alone runs it, as `task`.
  """
  def move(record, event)

  # A newer one than what's offered or failed is the one there is now; one being taken is finished first.
  def move(%{"state" => "updating"} = record, {:newer, _offer, _trusted, _by, _task}), do: {:ok, record}

  def move(record, {:newer, offer, trusted, by, task}) do
    cond do
      record && record["commit"] == offer["commit"] -> {:ok, record}
      trusted -> {:ok, offer |> Map.merge(%{"by" => by, "task" => task}) |> moved("updating", "fetching")}
      true -> {:ok, moved(offer, "available")}
    end
  end

  # What runs is as new as the source: an offer or a failure still kept is of a commit it has gone past.
  def move(%{"state" => state}, :current) when state in ~w(available failed), do: {:ok, nil}

  def move(%{"state" => state} = record, {:update, by, task}) when state in ~w(available failed),
    do: {:ok, record |> Map.merge(%{"by" => by, "task" => task}) |> moved("updating", "fetching")}

  def move(%{"state" => "updating"} = record, {:stage, stage}) when stage in ~w(fetching building starting),
    do: {:ok, moved(record, "updating", stage)}

  def move(%{"state" => "updating"}, :done), do: {:ok, nil}
  def move(%{"state" => "updating"} = record, {:fail, why}), do: {:ok, record |> moved("failed") |> Map.put("why", why)}

  def move(nil, {:update, _by, _task}), do: {:error, "It's up to date."}
  def move(%{"state" => "updating", "stage" => stage}, _event), do: {:error, "It's updating (#{stage}). Wait until it's done."}
  def move(%{"state" => state}, event), do: {:error, "An update that's #{state} can't be moved by #{inspect(event)}."}

  defp moved(record, state, stage \\ nil) do
    record
    |> Map.merge(%{"state" => state, "at" => System.system_time(:millisecond)})
    |> Map.drop(["stage", "why"])
    |> then(&if(stage, do: Map.put(&1, "stage", stage), else: &1))
  end
end
