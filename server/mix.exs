defmodule OneCore.MixProject do
  use Mix.Project

  def project do
    [
      app: :one_core,
      version: "0.1.0",
      elixir: "~> 1.20",
      deps: [
        {:ex_json_schema, "~> 0.11"},
        {:mime, "~> 2.0"},
        {:req, "~> 0.5"},
        {:rustler, "~> 0.38", runtime: false},
        {:tz, "~> 0.28"},
        {:yaml_elixir, "~> 2.12"}
      ],
      # Tests start only what they test: the server's key and socket aren't theirs to take.
      aliases: [test: "test --no-start"],
      # A server installed on a machine (`one serve on`) is this, as `./release` builds it for each kind of machine:
      # the VM it runs on, ERTS, is the one `ONE_ERTS` names when it's built for another kind of machine than this.
      releases: [one_core: [include_executables_for: [:unix], include_erts: System.get_env("ONE_ERTS") || true]],
      # The plugins the proofs deploy are files to deploy, not tests.
      test_ignore_filters: [~r{^test/plugins/}]
    ]
  end

  def application do
    [
      mod: {OneCore.Application, []},
      extra_applications: [:logger, :crypto, :inets, :xmerl],
      env: [build: build()]
    ]
  end

  # The build this is: the commit it's built from, said as `client/build.rs` says a client's, and `-dirty` after it
  # when what's built isn't all committed. `./release` names it.
  defp build do
    System.get_env("ONE_BUILD") ||
      elem(System.cmd("git", ~w(describe --always --dirty --abbrev=12 --exclude=*), cd: __DIR__), 0) |> String.trim()
  end
end
