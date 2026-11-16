defmodule Sandbox do
  @moduledoc """
  The wall a plugin in sandbox mode runs behind: this machine's own process sandbox, around a program One Core
  starts (`OneCore.Sandbox`). On Linux it's bubblewrap, the distro's own `bwrap` (Ubuntu's AppArmor lets that one
  make the namespaces it needs), in new namespaces with no network, under an address-space limit (`prlimit`). On a
  Mac it's a Seatbelt profile (`sandbox-exec`). Windows servers have none yet.

  Microsoft's MXC was tried first and works, but a server would have to ship its executor as Microsoft's prebuilt
  binaries or build it for every kind of machine, and its Linux baseline gives the program a shell, `/usr` and
  `/etc`. These walls give it only what's asked for.
  """
  @behaviour OneCore.Plugin

  # Without it no plugin runs in sandbox mode, and a network's members with no server of their own can run plugins
  # only that way.
  @impl true
  def essential, do: true

  @doc """
  Starts `program` with `arguments` behind this machine's sandbox, and gives its standard input and output as a port
  the calling process owns, in stream mode: `{:ok, port}`, or `{:error, why}`. `policy` says what it has:

  - `read`: paths it may read, each where it is on this machine. It can read nothing else of the machine's files
    but its system libraries, and write only a folder of its own, which `TMPDIR` names and which goes when it stops.
  - `write`, if it's given: paths it may also write, each where it is on this machine.
  - `env`: its environment, by name. Nothing else of this server's is passed on.
  - `memory`: the most memory it may have, in bytes, where the machine can say so (Linux), or `nil` for no limit
    but the machine's.

  Its standard error comes with its output. It has no network, can start no program outside `read`, and stops when
  the calling process does.
  """
  def start(program, arguments, %{read: read, env: env, memory: memory} = policy) do
    write = Map.get(policy, :write, [])

    case :os.type() do
      {:unix, :linux} -> linux(program, arguments, read, write, env, memory)
      {:unix, :darwin} -> mac(program, arguments, read, write, env)
      other -> {:error, "This server's machine (#{inspect(other)}) has no sandbox for plugins yet"}
    end
  end

  defp linux(program, arguments, read, write, env, memory) do
    with {:ok, bwrap} <- found("bwrap", "bubblewrap"),
         {:ok, prlimit} <- found("prlimit", "util-linux") do
      binds = Enum.flat_map(read, &["--ro-bind", &1, &1]) ++ Enum.flat_map(write, &["--bind", &1, &1])
      # The system's own libraries, which the program links against, wherever this distro keeps them.
      system = Enum.flat_map(~w(/usr/lib /usr/lib64 /lib /lib64), &["--ro-bind-try", &1, &1])
      vars = Enum.flat_map(Map.put(env, "TMPDIR", "/tmp"), fn {name, value} -> ["--setenv", name, value] end)

      # Mounts are made in order, so its own /tmp comes first, beneath anything bound inside it.
      walled =
        ["--unshare-all", "--die-with-parent", "--new-session", "--clearenv"] ++
          ["--tmpfs", "/tmp", "--proc", "/proc", "--dev", "/dev"] ++
          binds ++ system ++ ["--chdir", "/tmp", "--cap-drop", "ALL"] ++ vars ++ ["--", program | arguments]

      # Nothing of this server's environment is passed on, to bubblewrap either: its process is the sandbox's first,
      # whose environment the program could read.
      if memory,
        do: opened(prlimit, ["--as=#{memory}", "--", bwrap | walled], unset(), nil),
        else: opened(bwrap, walled, unset(), nil)
    end
  end

  # Seatbelt reads its profile from the command line. Paths go into it as string literals, which a path can't break
  # out of: they're quoted, and a quote or backslash in one is refused.
  defp mac(program, arguments, read, write, env) do
    folder = Path.join(System.tmp_dir!(), "one-sandbox-" <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower))

    if Enum.any?([folder | read ++ write], &String.contains?(&1, ["\"", "\\"])) do
      {:error, "A path the sandbox is given has a quote or a backslash in it"}
    else
      File.mkdir_p!(folder)
      folder = real(folder)
      paths = &Enum.map_join(&1, " ", fn path -> ~s|(subpath "#{real(path)}")| end)

      profile = """
      (version 1)
      (deny default)
      (allow process-fork)
      (allow process-exec #{paths.(read)})
      (allow file-read* #{paths.(read ++ ["/usr/lib", "/System", "/private/var/db/timezone"])})
      (allow file-read* (literal "/dev/null") (literal "/dev/urandom") (literal "/dev/random"))
      (allow file-read-metadata)
      (allow file-read* file-write* (subpath "#{folder}") #{paths.(write)})
      (allow file-write-data (literal "/dev/null"))
      (allow sysctl-read)
      (allow signal (target self))
      """

      # Nothing of this server's environment is passed on.
      env = Map.merge(unset(), Map.put(env, "TMPDIR", folder))
      opened("/usr/bin/sandbox-exec", ["-p", profile, program | arguments], env, folder)
    end
  end

  # Each of this server's environment variables, unset.
  defp unset, do: Map.new(System.get_env(), fn {name, _} -> {name, false} end)

  # A path as Seatbelt matches it: with every symlink in it followed (a Mac's /var is /private/var).
  defp real(path) do
    path
    |> Path.expand()
    |> Path.split()
    |> Enum.reduce(fn part, at ->
      next = Path.join(at, part)

      case :file.read_link_all(next) do
        {:ok, target} -> real(Path.expand(to_string(target), at))
        _ -> next
      end
    end)
  end

  defp found(name, package) do
    case System.find_executable(name) do
      nil -> {:error, "Sandbox mode needs #{name} on this server, from the package #{package}"}
      path -> {:ok, path}
    end
  end

  # The folder a Mac's sandbox writes to goes once the program stops.
  defp opened(executable, arguments, env, folder) do
    env = for {name, value} <- env, do: {String.to_charlist(name), value && String.to_charlist(value)}
    options = [:binary, :stream, :exit_status, :use_stdio, :stderr_to_stdout, args: arguments, env: env]
    port = Port.open({:spawn_executable, executable}, options)

    if folder do
      spawn(fn ->
        watched = Port.monitor(port)
        receive do: ({:DOWN, ^watched, :port, _, _} -> File.rm_rf(folder))
      end)
    end

    {:ok, port}
  rescue
    error -> {:error, "The sandbox didn't start: #{Exception.message(error)}"}
  end
end
