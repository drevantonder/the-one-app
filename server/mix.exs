defmodule OneCore.MixProject do
  use Mix.Project

  def project do
    [
      app: :one_core,
      version: "0.1.0",
      elixir: "~> 1.20",
      deps: [
        {:ex_json_schema, "~> 0.11"},
        {:req, "~> 0.5"},
        {:rustler, "~> 0.38", runtime: false},
        {:yaml_elixir, "~> 2.12"}
      ],
      # Tests start only what they test: the server's key and socket aren't theirs to take.
      aliases: [test: "test --no-start"],
      # The plugins the proofs deploy are files to deploy, not tests.
      test_ignore_filters: [~r{^test/plugins/}]
    ]
  end

  def application do
    [mod: {OneCore.Application, []}, extra_applications: [:logger, :crypto, :inets, :xmerl]]
  end
end
