defmodule OneCore.MixProject do
  use Mix.Project

  def project do
    [
      app: :one_core,
      version: "0.1.0",
      elixir: "~> 1.20",
      deps: [{:req, "~> 0.5"}, {:rustler, "~> 0.38", runtime: false}],
      # Tests start only what they test: the server's key and socket aren't theirs to take.
      aliases: [test: "test --no-start"]
    ]
  end

  def application do
    [mod: {OneCore.Application, []}, extra_applications: [:logger, :crypto, :xmerl]]
  end
end
