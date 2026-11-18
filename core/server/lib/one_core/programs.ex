defmodule OneCore.Programs do
  @moduledoc """
  The programs an installed server runs that its release names rather than carries, each as `{url, sha256}` of a zip
  (`tooling/release`): Bun, which builds apps, on every machine, and Git, which reads apps' sources
  (`OneCore.Apps.Git`), on Windows, as Linux and a Mac have their own and Windows doesn't. Each is downloaded the
  first time it's needed and kept, unpacked whole, in the server's home, `<name>/<sha256>`, so an update downloads
  none of its own. One started with `mix network` uses those on its PATH, as the rest of the repo does.
  """
  alias OneCore.Server

  @doc """
  The path of the program `name`, `:bun` or `:git`: `{:ok, path}`, or `{:error, why}`, saying what to do where it
  can't be found or downloaded.
  """
  def path(name) do
    case Application.get_env(:one_core, name) do
      {url, sha256} -> fetched(to_string(name), url, sha256)
      nil -> found(to_string(name))
    end
  end

  defp found(name) do
    case System.find_executable(name) do
      nil -> {:error, "This server can't find #{name}: put it on its PATH, as `mise install` does in the One App's folder."}
      path -> {:ok, path}
    end
  end

  defp fetched(name, url, sha256) do
    folder = Path.join([Server.home(), name, sha256])

    with nil <- program(folder, name),
         {:ok, %{status: 200, body: zip}} <- Req.get(url, raw: true),
         ^sha256 <- Base.encode16(:crypto.hash(:sha256, zip), case: :lower) do
      writing = folder <> "." <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
      {:ok, _} = :zip.extract(zip, cwd: to_charlist(writing))
      # A zip keeps no file's mode that Erlang reads.
      File.chmod!(program(writing, name), 0o755)
      # Another may have fetched it meanwhile, which is as good.
      with {:error, _} <- File.rename(writing, folder), do: File.rm_rf!(writing)
      {:ok, program(folder, name)}
    else
      path when is_binary(path) -> {:ok, path}
      _ -> {:error, "This server needs #{name}, which it couldn't download whole from #{url}."}
    end
  end

  # The program in a zip unpacked, at its top or in a folder there: Bun's is in a folder of its own, and Git's
  # commands are in `cmd`. Windows finds a program by its extension.
  defp program(folder, name) do
    file = if match?({:win32, _}, :os.type()), do: name <> ".exe", else: name
    List.first(Path.wildcard(Path.join(folder, file)) ++ Path.wildcard(Path.join([folder, "*", file])))
  end
end
