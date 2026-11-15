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
        {:opentelemetry, "~> 1.7"},
        {:opentelemetry_api, "~> 1.5"},
        {:req, "~> 0.5"},
        {:rustler, "~> 0.38", runtime: false},
        {:tz, "~> 0.28"},
        {:yaml_elixir, "~> 2.12"}
      ],
      # Tests start only what they test: the server's key and socket aren't theirs to take.
      aliases: [test: &test/1],
      # A server installed on a machine (`one serve on`) is this, as `./release` builds it for each kind of machine:
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
      ],
      # The plugins the proofs deploy are files to deploy, not tests.
      test_ignore_filters: [~r{/test/plugins/}, ~r{^test/plugins/}]
    ]
  end

  def application do
    [
      mod: {OneCore.Application, []},
      extra_applications: [:logger, :crypto, :inets, :xmerl],
      env: [build: build()]
    ]
  end

  # The program is One Core and the NIFs of the plugins it ships with, each in its plugin's `server/nif/`. On a
  # developer's machine it's also `mix network`, which `./network` runs, at the repo's root.
  defp elixirc_paths(:dev), do: elixirc_paths(:prod) ++ [Path.expand("../../network.ex", __DIR__)]
  defp elixirc_paths(_env), do: ["lib" | Path.wildcard(Path.expand("../../plugins/*/server/nif", __DIR__))]

  # One Core's tests, and the tests of the plugins it ships with, each in its plugin's `test/`, which run with it as
  # its NIFs are built with it. Naming tests runs those alone.
  defp test(args) do
    named = Enum.filter(args, &File.exists?(String.replace(&1, ~r/:\d+$/, "")))
    plugins = Path.wildcard(Path.expand("../../plugins/*/test/*_test.exs", __DIR__))
    Mix.Task.run("test", ["--no-start" | args] ++ if(named == [], do: ["test" | plugins], else: []))
  end

  # The build this is, as `build-name` names it from the commit it's built from, as a client's is.
  defp build do
    elem(System.cmd(Path.expand("../../build-name", __DIR__), []), 0) |> String.trim()
  end
end
