defmodule OneCore.CodeMode.Runner do
  @moduledoc """
  A code mode script's runner: OpenCode's interpreter, bundled (`core/codemode`, One Core's `priv/codemode.js`), in a
  QuickJS sandbox of its own on a thread of its own, a server's Rust NIF (`nif/codemode`). It tells the process that
  started it `{:codemode, tag, message}`: `{:call, json}` for each call its script makes, which `reply/2` answers, then
  `{:done, ok, text}`, or `{:exit, why}` where it couldn't finish. Its memory is limited, and `stop/1`, or its process
  ending, stops it where it is, even in a loop that never waits.
  """
  use OneCore.Nif, library: "codemode"

  @doc "Starts `script`, the JSON `run` in run.js takes, with the interpreter's `bundle`, its messages tagged `tag`."
  def start(_bundle, _script, _tag), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Answers a call its script made, as `reply` in run.js takes it."
  def reply(_run, _line), do: :erlang.nif_error(:nif_not_loaded)

  @doc "Stops it, which then says nothing more."
  def stop(_run), do: :erlang.nif_error(:nif_not_loaded)
end
