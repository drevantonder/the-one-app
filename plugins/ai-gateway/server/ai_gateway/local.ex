defmodule AiGateway.Local do
  @moduledoc """
  The models on the machine it runs on, served by a model server already running there: Ollama, llama.cpp's
  llama-server or LM Studio, whichever answers on its own port. It lists their models with what each does, and
  sends a chat to the one that has the model, at its OpenAI-compatible chat completions.

  It holds nothing: each call asks the model servers afresh, so one started or stopped later is seen at once.
  """
  use GenServer
  @behaviour OneCore.Service

  # Each model server on the port it listens on as it's installed, and how it says what its models do: that's
  # each one's own, from its documentation.
  @servers [{"Ollama", 11434}, {"llama.cpp", 8080}, {"LM Studio", 1234}]

  # How long a model may take to answer, and a server to say what it has, in milliseconds.
  @answers_within 300_000
  @lists_within 3_000

  @impl OneCore.Service
  def called, do: "AI Gateway's models on this machine"

  @impl OneCore.Service
  def needs,
    do:
      "A model server running on the machine with models in it: Ollama, llama.cpp's llama-server or LM Studio, on its usual port"

  @impl GenServer
  def init(_), do: {:ok, nil}

  @doc """
  The model servers running here, and their models: each one's `name`, the `server` it's in, whether it takes
  images (`vision`), and whether it keeps to a JSON Schema (`structured`), which all three do.
  """
  def models(_service) do
    running = for {name, port} <- @servers, models = models(name, port), do: {name, models}

    %{
      "running" => for({name, _} <- running, do: name),
      "models" =>
        for({name, models} <- running, model <- models, do: Map.merge(model, %{"server" => name, "structured" => true}))
    }
  end

  @doc """
  Sends a chat (OpenAI's chat completions, without `model`) to the model server here that has `model`. Gives what
  it answered, `status` and `body`, or `down` with why when none has it or it stopped answering.
  """
  def chat(_service, model, body) do
    case Enum.find(@servers, fn {name, port} -> Enum.any?(models(name, port) || [], &(&1["name"] == model)) end) do
      nil ->
        %{"down" => "No model server on #{OneCore.Service.name(OneCore.Service.server())} has #{model}"}

      {name, port} ->
        sent = Map.merge(body, %{"model" => model, "stream" => false})

        case Req.post(url(port, "/v1/chat/completions"),
               json: sent,
               receive_timeout: @answers_within,
               retry: false,
               decode_body: false
             ) do
          {:ok, answer} -> %{"status" => answer.status, "body" => answer.body}
          {:error, _} -> %{"down" => "#{name} stopped answering"}
        end
    end
  end

  # A model server's models, or nil when it isn't running. Embedding models, which don't chat, are left out.
  defp models("Ollama", port) do
    with %{"models" => models} <- get(port, "/api/tags") do
      for %{"name" => name} <- models,
          said = post(port, "/api/show", %{model: name}),
          "completion" in (said["capabilities"] || []),
          do: %{"name" => name, "vision" => "vision" in said["capabilities"]}
    end
  end

  defp models("llama.cpp", port) do
    with %{"data" => models} <- get(port, "/v1/models"),
         do:
           for(
             model <- models,
             listed = AiGateway.listed(model),
             listed != nil,
             do: %{listed | "vision" => listed["vision"] == true}
           )
  end

  defp models("LM Studio", port) do
    with %{"models" => models} <- get(port, "/api/v1/models") do
      for %{"type" => "llm", "key" => key} = model <- models,
          do: %{"name" => key, "vision" => get_in(model, ["capabilities", "vision"]) == true}
    end
  end

  defp get(port, path) do
    case Req.get(url(port, path), receive_timeout: @lists_within, retry: false) do
      {:ok, %{status: 200, body: %{} = body}} -> body
      _ -> nil
    end
  end

  defp post(port, path, body) do
    case Req.post(url(port, path), json: body, receive_timeout: @lists_within, retry: false) do
      {:ok, %{status: 200, body: %{} = body}} -> body
      _ -> %{}
    end
  end

  defp url(port, path), do: "http://127.0.0.1:#{port}#{path}"
end
