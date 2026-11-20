# Credo checks only what's ours to guard (`credo/`): `mix credo` from here, over One Core and every app's server.
%{
  configs: [
    %{
      name: "default",
      files: %{included: ["lib/", "../../apps/*/server/"], excluded: ["deps/", "_build/"]},
      requires: ["credo/*.ex"],
      checks: %{enabled: [{OneCore.Credo.RemoteWait, []}]}
    }
  ]
}
