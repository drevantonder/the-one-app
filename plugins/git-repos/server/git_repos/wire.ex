defmodule GitRepos.Wire do
  @moduledoc """
  Git's smart HTTP protocol, as git's own HTTP transport speaks it (gitprotocol-http): each of its requests is one
  call of the tool `git`, which `git-remote-one` makes for it on a person's device.

  A fetch is git's own: `git http-backend` answers it from the mirror, protocol v2 included. A push is answered
  here, since what it changes is decided against the bucket and not the mirror: its commands are read, its pack is
  indexed into the mirror by `git index-pack`, and each ref moves only from where the pusher saw it, as
  report-status says back.
  """
  alias GitRepos.Mirror

  @zero Mirror.zero()
  @caps "report-status delete-refs ofs-delta"

  @doc "What `git http-backend` answers for the request at `path` (and query), from the mirror at `folder`."
  def upload_pack(folder, path, body, protocol, encoding) do
    {path, query} =
      case String.split(path, "?", parts: 2) do
        [path, query] -> {path, query}
        [path] -> {path, ""}
      end

    env = [
      {"GIT_PROJECT_ROOT", Path.dirname(folder)},
      {"GIT_HTTP_EXPORT_ALL", "1"},
      {"PATH_INFO", "/" <> Path.basename(folder) <> "/" <> path},
      {"QUERY_STRING", query},
      {"REQUEST_METHOD", if(body, do: "POST", else: "GET")},
      {"CONTENT_TYPE", if(body, do: "application/x-git-upload-pack-request", else: "")},
      {"HTTP_CONTENT_ENCODING", encoding || ""},
      {"GIT_PROTOCOL", protocol || ""}
    ]

    {out, 0} = Mirror.git(Path.dirname(folder), ["http-backend"], env: env, input: body || "", binary: true)
    [head, body] = String.split(out, ~r/\r?\n\r?\n/, parts: 2)
    type = Regex.run(~r/^Content-Type: *(.*?)\r?$/mi, head, capture: :all_but_first) || ["text/plain"]

    case Regex.run(~r/^Status: *(\d+) *(.*?)\r?$/mi, head, capture: :all_but_first) do
      [code, _] when code != "200" -> raise "git refused: #{String.trim(body)}"
      _ -> {hd(type), body}
    end
  end

  @doc "What a push is told first: the refs, and what this end of it can do."
  def advertise(state) do
    lines =
      case Enum.sort(state["refs"]) do
        [] -> ["#{@zero} capabilities^{}\0#{@caps}\n"]
        [{ref, sha} | rest] -> ["#{sha} #{ref}\0#{@caps}\n" | for({ref, sha} <- rest, do: "#{sha} #{ref}\n")]
      end

    {"application/x-git-receive-pack-advertisement",
     IO.iodata_to_binary([pkt("# service=git-receive-pack\n"), "0000", Enum.map(lines, &pkt/1), "0000"])}
  end

  @doc "A push's request: its commands, each `{old, new, ref}`, and its pack, which may be empty."
  def commands(body), do: commands(body, [])

  defp commands("0000" <> pack, read), do: {Enum.reverse(read), pack}
  defp commands("", read), do: {Enum.reverse(read), ""}

  defp commands(<<size::binary-4, rest::binary>>, read) do
    size = String.to_integer(size, 16) - 4
    <<line::binary-size(^size), rest::binary>> = rest
    [line | _caps] = String.split(line, "\0", parts: 2)
    [old, new, ref] = line |> String.trim_trailing("\n") |> String.split(" ", parts: 3)
    commands(rest, [{old, new, ref} | read])
  end

  @doc "What a push is told back: each ref `:ok` or `{:no, why}`."
  def report(results) do
    lines =
      for {ref, result} <- results do
        case result do
          :ok -> pkt("ok #{ref}\n")
          {:no, why} -> pkt("ng #{ref} #{why}\n")
        end
      end

    {"application/x-git-receive-pack-result", IO.iodata_to_binary([pkt("unpack ok\n"), lines, "0000"])}
  end

  defp pkt(line),
    do: [String.pad_leading(Integer.to_string(byte_size(line) + 4, 16), 4, "0") |> String.downcase(), line]
end
