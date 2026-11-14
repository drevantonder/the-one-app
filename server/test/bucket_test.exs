defmodule OneCore.BucketTest do
  use ExUnit.Case, async: true
  alias OneCore.Bucket.Folder

  @tag :tmp_dir
  test "of many writing a blob at once exactly one does, and it's replaced only if unchanged", %{tmp_dir: folder} do
    wrote =
      1..50
      |> Task.async_stream(&Folder.write(folder, "whose/blob", "#{&1}"), max_concurrency: 50)
      |> Enum.map(fn {:ok, wrote} -> wrote end)

    assert Enum.sort(wrote) == [:exists |> List.duplicate(49), :ok] |> List.flatten()
    assert {:ok, data, tag} = Folder.read(folder, "whose/blob")
    assert data == "#{Enum.find_index(wrote, &(&1 == :ok)) + 1}"

    assert Folder.replace(folder, "whose/blob", "next", tag) == :ok
    assert Folder.replace(folder, "whose/blob", "late", tag) == :changed
    assert {:ok, "next", _} = Folder.read(folder, "whose/blob")
    assert Folder.list(folder, "whose/") == ["whose/blob"]
  end
end
