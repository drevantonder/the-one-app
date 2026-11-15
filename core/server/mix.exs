defmodule OneCore.MixProject do
  use Mix.Project

  def project do
    [
      app: :one_core,
      version: "0.1.0",
      elixir: "~> 1.20",
      elixirc_paths: elixirc_paths(Mix.env()),
      deps: [
        {:ex_json_schema, "~> 0.11"},
        {:mime, "~> 2.0"},
        {:req, "~> 0.5"},
        # A crash in a Rust NIF ends the VM. Rust panics are caught, and what's left is rare.
        {:rustler, "~> 0.38", runtime: false},
        {:tz, "~> 0.28"},
        {:yaml_elixir, "~> 2.12"}
      ],
      aliases: if(Mix.env() == :prod, do: [], else: [compile: [&nifs/1, "compile"]]),
      # A server installed on a machine (`one serve on`) is this, as `tooling/release` builds it for each kind of machine:
      # the VM it runs on, ERTS, is the one `ONE_ERTS` names when it's built for another kind of machine than this.
      # It runs with no distribution (`rel/env.sh.eex`): nothing reaches it as an Erlang node, so no cookie guards
      # anything, and the one Mix puts on its command line, where every user of the machine sees it, is a word
      # rather than a secret.
      releases: [
        one_core: [
          include_executables_for: [:unix],
          include_erts: System.get_env("ONE_ERTS") || true,
          cookie: "no-distribution"
        ]
      ]
    ]
  end

  def application do
    [
      mod: {OneCore.Application, []},
      extra_applications: [:logger, :crypto, :inets, :xmerl],
      env: [build: build()]
    ]
  end

  # The program is One Core. On a developer's machine it's also `mix network`, which `tooling/network` runs, at the repo's
  # root.
  defp elixirc_paths(:dev), do: ["lib", Path.expand("../../tooling/network.ex", __DIR__)]
  defp elixirc_paths(_env), do: ["lib"]

  # The NIFs of the plugins The One App ships with, built for this machine and put beside their source, where a
  # plugin brings them (`OneCore.Plugin`): here, its folder is what's deployed. `tooling/nifs` builds them for every
  # kind of machine, as a server built for one is.
  defp nifs(_args) do
    root = Path.expand("../..", __DIR__)
    crates = for toml <- Path.wildcard("#{root}/plugins/*/server/nif/*/Cargo.toml"), do: Path.dirname(toml)
    {_, 0} = System.cmd("cargo", ["build", "--release" | Enum.flat_map(crates, &["-p", Path.basename(&1)])], cd: root)
    target = System.get_env("CARGO_TARGET_DIR", "#{root}/target")
    # The kind of machine this is, as builds are named (`OneCore.Builds.target/0`).
    [arch | _] = String.split(to_string(:erlang.system_info(:system_architecture)), "-")
    {os, extension} = if :os.type() == {:unix, :darwin}, do: {"macos", "dylib"}, else: {"linux", "so"}

    for crate <- crates,
        do: File.cp!("#{target}/release/lib#{Path.basename(crate)}.#{extension}", "#{crate}/#{os}-#{arch}.so")
  end

  # The build this is, as `tooling/version` names it from the commit it's built from, as a client's is.
  defp build do
    elem(System.cmd(Path.expand("../../tooling/version", __DIR__), []), 0) |> String.trim()
  end
end
