defmodule AgentSkills.Writer do
  @moduledoc """
  Writes the skills its person sees (`OneCore.Skills`) where each agent tool on its device reads skills: as it starts
  and every minute, so an app installed, updated or removed, or an agent tool installed later, is there within the
  minute. Its person is whoever its server is for, or else the first it acts for: the first of
  `OneCore.Service.acts_for()`.

  Each skill with its own folder in `skill://` gets one here, all its files and the skills beneath it, named
  `<network>-<its first part>`: an app's by the name it's installed by, One Core's by its own. The network's name is
  its address in kebab case, so two networks' skills sit side by side. One named for the network alone lists them.
  """
  use GenServer
  require Logger
  @behaviour OneCore.Service

  @every 60_000
  @marker ".agent-skills"

  @impl OneCore.Service
  def called, do: "Agent Skills' writer on this machine"

  @impl OneCore.Service
  def os, do: ["linux", "darwin"]

  @impl OneCore.Service
  def needs do
    "A Linux or Mac machine its person uses agent tools on (Claude Code, Codex, OpenCode, Amp). It writes the skills of their apps into the home folder of the user this server runs as."
  end

  @impl GenServer
  def init(_) do
    send(self(), :write)
    {:ok, %{listed: nil, skills: %{}, logged: MapSet.new()}}
  end

  @impl GenServer
  def handle_info(:write, state) do
    Process.send_after(self(), :write, @every)
    {:noreply, sync(state)}
  end

  defp sync(state) do
    with [who | _] <- OneCore.Service.acts_for(),
         {:ok, network} <- network() do
      state = skills(who, network, state)
      Enum.reduce(roots(System.user_home!()), state, &write(&1, network, &2))
    else
      [] -> state
      {:error, why} -> log_once(state, why)
    end
  rescue
    problem -> log_once(state, "couldn't write them: " <> Exception.message(problem))
  end

  defp network do
    case Application.get_env(:one_core, :address) do
      nil ->
        {:error, "this network has no address, which its skills are named by"}

      address ->
        host = URI.parse(address).host
        {:ok, %{address: host, name: kebab(host)}}
    end
  end

  # Each skill's folder name and its files by their paths in it, read again only when what's listed changes.
  defp skills(who, network, state) do
    case OneCore.Skills.list(who) do
      listed when listed == state.listed ->
        state

      listed ->
        paths = for %{uri: uri} <- listed, do: path(uri)

        skills =
          for %{uri: skill, resources: resources} <- listed, %{uri: uri} <- resources, reduce: %{} do
            skills ->
              # A skill beneath another goes in that one's folder, where its files say it is.
              path = path(skill)

              top =
                Enum.min_by(
                  for(p <- paths, p == path or String.starts_with?(path, p <> "/skills/"), do: p),
                  &byte_size/1
                )

              folder = network.name <> "-" <> kebab(hd(String.split(top, "/")))
              file = String.replace_prefix(uri, "skill://#{top}/", "")

              case OneCore.Skills.text(who, uri) do
                {:ok, text} when is_binary(text) ->
                  Map.update(skills, folder, %{file => text}, &Map.put(&1, file, text))

                _absent ->
                  skills
              end
          end

        skills = for {folder, files} <- skills, into: %{}, do: {folder, named(files, folder)}
        %{state | listed: listed, skills: Map.put(skills, network.name, root(network, skills))}
    end
  end

  # A skill's path in `skill://`, from its SKILL.md's address.
  defp path(uri), do: uri |> String.replace_prefix("skill://", "") |> String.replace_suffix("/SKILL.md", "")

  # An agent tool finds a skill by its folder, whose name its SKILL.md's frontmatter says.
  defp named(%{"SKILL.md" => text} = files, folder),
    do: %{files | "SKILL.md" => Regex.replace(~r/\A(---\r?\n(?:.*\r?\n)*?)name:[^\r\n]*/, text, "\\1name: #{folder}")}

  defp named(files, _folder), do: files

  defp root(network, skills) do
    listed =
      for {folder, %{"SKILL.md" => text}} <- Enum.sort(skills),
          do: "- `#{folder}`: #{description(text)}"

    %{
      "SKILL.md" => """
      ---
      name: #{network.name}
      description: #{JSON.encode!("Read before using the network of the One App at #{network.address}, by its MCP tools (<app>_<tool>) or the `one` command, and whenever the user mentions the One App or wants something done with what they keep there: their email, messages, calendar, files, devices, keys and more. It lists a skill for each of its apps.")}
      ---

      # #{network.address}

      This is a network of the One App: what one user, family or workplace keeps and makes, which you use as whoever you
      act for. Its tools are its apps', each the MCP tool `<app>_<tool>` and the command `one <app> <tool>`
      (`--help` says what it takes), and One Core's own, which add and remove apps, servers and services.

      Read an app's skill before using its tools. Each is a skill here:

      #{Enum.join(listed, "\n")}
      """
    }
  end

  defp description(text) do
    with [_, yaml] <- Regex.run(~r/\A---\r?\n(.*?)\r?\n---/s, text),
         {:ok, %{"description" => description}} <- YamlElixir.read_from_string(yaml),
         do: description,
         else: (_ -> "")
  end

  # Claude Code's, and the one every other agent tool reads. A tool that isn't on the device has no folder, and gets
  # nothing.
  defp roots(home) do
    for root <- [
          Path.join(System.get_env("CLAUDE_CONFIG_DIR") || Path.join(home, ".claude"), "skills"),
          Path.join(home, ".agents/skills")
        ],
        File.dir?(Path.dirname(root)),
        do: root
  end

  defp write(root, network, state) do
    File.mkdir_p!(root)
    state = Enum.reduce(state.skills, state, fn {name, files}, state -> skill(root, name, files, network, state) end)

    for name <- File.ls!(root),
        not Map.has_key?(state.skills, name),
        ours?(Path.join(root, name), network),
        reduce: state do
      state ->
        File.rm_rf!(Path.join(root, name))
        Logger.info("Agent Skills removed #{Path.join(root, name)}")
        state
    end
  end

  # A skill's folder is written whole beside it and moved in, so an agent tool never reads half of one.
  defp skill(root, name, files, network, state) do
    folder = Path.join(root, name)
    ours = ours?(folder, network)

    cond do
      File.exists?(folder) and not ours ->
        log_once(state, "left #{folder} alone: it isn't a skill Agent Skills wrote")

      ours and same?(folder, files) ->
        state

      true ->
        writing = folder <> ".writing"
        File.rm_rf!(writing)

        for {path, text} <- files do
          File.mkdir_p!(Path.dirname(Path.join(writing, path)))
          File.write!(Path.join(writing, path), text)
        end

        File.write!(Path.join(writing, @marker), marker(network))
        File.rm_rf!(folder)
        File.rename!(writing, folder)
        state
    end
  end

  defp same?(folder, files) do
    on_disk =
      for file <- Path.wildcard(Path.join(folder, "**"), match_dot: true),
          File.regular?(file),
          do: Path.relative_to(file, folder)

    Enum.sort(on_disk) == Enum.sort([@marker | Map.keys(files)]) and
      Enum.all?(files, fn {path, text} -> File.read(Path.join(folder, path)) == {:ok, text} end)
  end

  defp ours?(folder, network), do: File.read(Path.join(folder, @marker)) == {:ok, marker(network)}

  defp marker(network) do
    "Agent Skills writes this skill for #{network.address}, from what its app says, and keeps it current. Delete this file to take the folder over.\n"
  end

  defp kebab(text), do: text |> String.downcase() |> String.replace(~r/[^a-z0-9]+/, "-") |> String.trim("-")

  # Says each problem once, not every minute.
  defp log_once(state, message) do
    if not MapSet.member?(state.logged, message), do: Logger.warning("Agent Skills: " <> message)
    %{state | logged: MapSet.put(state.logged, message)}
  end
end
