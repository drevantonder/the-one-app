defmodule OneCore.MixProject do
  use Mix.Project

  def project do
    [
      app: :one_core,
      version: "0.1.0",
      elixir: "~> 1.20",
      elixirc_paths: elixirc_paths(Mix.env()),
      deps: [
        {:castore, "~> 1.0"},
        {:credo, "~> 1.7", only: :dev, runtime: false},
        {:ex_json_schema, "~> 0.11"},
        {:fuse, "~> 2.5"},
        {:mime, "~> 2.0"},
        {:observer_cli, "~> 1.8"},
        {:opentelemetry, "~> 1.7"},
        {:opentelemetry_api, "~> 1.5"},
        {:opentelemetry_experimental, "~> 0.6"},
        {:opentelemetry_api_experimental, "~> 0.6"},
        {:opentelemetry_exporter, "~> 1.11"},
        {:recon, "~> 2.5"},
        {:req, "~> 0.5"},
        {:sweet_xml, "~> 0.7"},
        {:tz, "~> 0.28"},
        {:yaml_elixir, "~> 2.12"}
      ],
      aliases: if(Mix.env() == :prod, do: [], else: [compile: [&nifs/1, "compile", &cycles/1]]),
      # A server installed on a machine (`one serve on`) is this, as `tooling/release` builds it for each kind of machine:
      # the VM it runs on, ERTS, is the one `ONE_ERTS` names when it's built for another kind of machine than this.
      # It runs with no distribution (`rel/env.sh.eex`, `rel/env.bat.eex`): nothing reaches it as an Erlang node, so no cookie guards
      # anything, and the one Mix puts on its command line, where every user of the machine sees it, is a word
      # rather than a secret.
      # Its version is the build it is, as `tooling/version` names the commit, which it says it is (`config/runtime.exs`),
      # read only as it's released, so nothing is compiled again for a new commit.
      releases: [
        one_core: fn ->
          [
            version: String.trim(elem(System.cmd(Path.expand("../../tooling/version", __DIR__), []), 0)),
            include_executables_for: [:unix, :windows],
            include_erts: System.get_env("ONE_ERTS") || true,
            cookie: "no-distribution"
          ]
        end
      ]
    ]
  end

  def application do
    [
      mod: {OneCore.Application, []},
      extra_applications: [:logger, :crypto, :inets]
    ]
  end

  # The program is One Core. On a developer's machine it's also `mix network`, which `tooling/network` runs, at the repo's
  # root.
  # `OneCore.App.Manifest`, `OneCore.Redact` and `OneCore.Nif` are compiled by an untrusted app's VM too, which has none of the server's
  # code, so it's where that VM finds them, in `priv/sandbox`.
  defp elixirc_paths(:dev), do: elixirc_paths(:prod) ++ [Path.expand("../../tooling/network.ex", __DIR__)]
  defp elixirc_paths(_env), do: ["lib", "priv/sandbox/manifest.ex", "priv/sandbox/redact.ex", "priv/sandbox/nif.ex"]

  # One Core is in layers, as `mix xref graph --format cycles` shows them: what's low never calls up into what's
  # above it, and what's above calls down. Most of it still calls back and forth in one cycle of 27 files, so a
  # developer's build fails where a bigger one forms, as a module that calls up into it makes. Lower the number as
  # that cycle shrinks.
  def cycles(_args), do: Mix.Task.run("xref", ~w(graph --format cycles --min-cycle-size 28 --fail-above 0 --no-compile))

  # The NIFs, built for this machine with the rest of the Rust workspace, as `tooling/network` and its code reloader
  # run it: built as one, it has one set of features, so nothing in it is built twice or again for nothing. It's the
  # debug build, as a developer's is (`Cargo.toml`). Each is named for its build, as
  # `tooling/nifs` names it (`OneCore.Nif`). One Core's go where the server loads them (`priv/native`), and those of this
  # repo's apps beside their source, where an app brings them (`OneCore.App`): here, its
  # folder is what's deployed. `tooling/release` and `tooling/nifs` build them for every kind of machine, as a server
  # built for one is. A crash in a Rust NIF ends the VM: Rust panics are caught, and what's left is rare. Each is
  # removed before it's put there again, since a server running from this checkout has it loaded. A worktree's first
  # build starts from a copy of the build beside it (`tooling/copy-build`).
  def nifs(_args) do
    root = Path.expand("../..", __DIR__)
    target = System.get_env("CARGO_TARGET_DIR", "#{root}/target")
    if not File.exists?(target), do: System.cmd("#{root}/tooling/copy-build", [], into: IO.stream())
    {rustc, 0} = System.cmd("rustc", ["-vV"])
    [host] = Regex.run(~r/^host: (.+)$/m, rustc, capture: :all_but_first)
    # The kind of machine this is, as builds are named (`OneCore.Builds.target/0`).
    [arch | _] = String.split(to_string(:erlang.system_info(:system_architecture)), "-")
    {os, extension} = if :os.type() == {:unix, :darwin}, do: {"macos", "dylib"}, else: {"linux", "so"}
    File.mkdir_p!(Path.join(__DIR__, "priv/native"))

    built =
      for toml <- Path.wildcard("#{__DIR__}/nif/*/Cargo.toml"),
          crate = Path.dirname(toml),
          do: {crate, Path.join(__DIR__, "priv/native/#{Path.basename(crate)}.so")}

    bundled =
      for toml <- Path.wildcard("#{root}/apps/*/server/nif/*/Cargo.toml"),
          crate = Path.dirname(toml),
          do: {crate, "#{crate}/#{os}-#{arch}.so"}

    builds =
      for {crate, _put} <- built ++ bundled do
        {hash, 0} = System.cmd("#{root}/tooling/input-hash", [crate, host])
        {"NIF_BUILD_" <> String.upcase(Path.basename(crate)), String.trim(hash)}
      end

    with {_, status} when status != 0 <- System.cmd("cargo", ["build"], cd: root, env: builds),
         do: Mix.raise("cargo build failed (#{status}), saying why above")

    for {crate, put} <- built ++ bundled do
      File.rm(put)
      File.cp!("#{target}/debug/lib#{Path.basename(crate)}.#{extension}", put)
    end
  end
end
