# What a server logs is shown with the test that failed, and otherwise not at all. What a test waits to be told is
# waited for longer than ExUnit's 100 ms, which a busy machine takes to run a process that has just been started.
ExUnit.start(capture_log: true, assert_receive_timeout: 2_000)

defmodule OneCore.Test do
  @doc "The first owner of each test's network: who may do anything in it while no plugin decides who may."
  def owner, do: "users/owner"

  @doc "Makes the test's network one being built, where a request says which user it's from and nothing is refused for want of a plugin deciding."
  def being_built do
    Application.put_env(:one_core, :development, true)
    ExUnit.Callbacks.on_exit(fn -> Application.delete_env(:one_core, :development) end)
  end

  @doc """
  A plugin that says whose a device is, as a plugin that signs users in does: from blobs in its own artifact,
  which its `place` tool writes and its `remove` tool removes.
  """
  def placer do
    """
    defmodule Placer do
      @behaviour OneCore.Plugin
      alias OneCore.Artifact

      @impl true
      def tools do
        for name <- ~w(place remove), do: %{name: name, description: name, inputs: %{type: "object"}}
      end

      @impl true
      def call("place", %{"device" => device, "who" => who}, call), do: Artifact.write(call.own, device, who)
      def call("remove", %{"device" => device}, call), do: Artifact.remove(call.own, device)

      @impl true
      def hooks, do: ["device"]

      @impl true
      def hook("device", %{"device" => device}, call) do
        with {:ok, who, _tag} <- Artifact.read(call.own, device), do: %{"who" => who}, else: (:absent -> %{})
      end
    end
    """
  end

  @doc "The files of a plugin The One App ships with, as the repo has it, as a deploy from its folder sends them."
  def shipped(name) do
    folder = Path.expand("../../plugins/#{name}", __DIR__)
    {:ok, paths} = OneCore.Folder.files(folder)
    Map.new(paths, &{&1, File.read!(Path.join(folder, &1))})
  end

  @doc """
  The files of a kit, the plugin `ui`, whose code is `code`: its build as the repo has it, which builds an app
  against it.
  """
  def kit(code) do
    build =
      for file <- ~w(app.js package.json bun.lock), into: %{}, do: {"build/" <> file, File.read!("../ui/" <> file)}

    Map.put(build, "app/ui.js", code)
  end

  @doc """
  Starts a network's first server for the test that calls this, kept in `folder`: its bucket there, its home,
  and what answers and runs plugins. It acts for the network, so for everyone, and the network has its first
  owner.
  """
  def server(folder) do
    Application.delete_env(:one_core, :development)
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.Folder, Path.join(folder, "bucket")})
    Application.put_env(:one_core, :home, Path.join(folder, "server"))
    {:ok, key} = OneCore.Iroh.key(OneCore.Server.key_file())
    true = OneCore.Servers.joined?(key, "network")

    for child <- [OneCore.Plugins.Deploy, OneCore.Plugins, OneCore.Server, OneCore.Services],
        do: ExUnit.Callbacks.start_supervised!(child)

    :ok = OneCore.User.make_first_owner(owner())
    key
  end
end
