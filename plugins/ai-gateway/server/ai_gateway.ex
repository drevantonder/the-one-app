defmodule AiGateway do
  @moduledoc """
  AI Gateway: how plugins and agents use AI models, in one shape whatever runs them. A request is OpenAI's chat
  completions' messages, text and images, with a JSON Schema for the answer if it wants one; the answer is the
  model's text, and that text read as JSON when a schema was given. OpenAI's shape is what's sent on, since local
  model servers, Workers AI and most hosts speak it.

  A model runs in one of two kinds of place. On a machine of whoever asks, through the service `AiGateway.Local`,
  which talks to the model server running there: its id is `local/<model>`. Or at a hosted endpoint, Workers AI
  among them: a Vault item switched on for this plugin, with the endpoint's `url` and its `api_key`, whose id is
  `<item>/<model>`. The Vault fills the key into the request, so it's never read here.

  Which model a job uses is an ordered list, whoever asks's own or the network's (`set`), and the first in it
  that's up answers. With no list, a model on their own machines does: nothing hosted, which is paid for by the
  token, is used unless they set it up and named it.
  """
  @behaviour OneCore.Plugin
  alias AiGateway.Local
  alias OneCore.{Artifact, Call, Service}

  # How long a model may take to answer, in seconds.
  @answers_within 300

  @impl true
  def uses, do: ["vault"]

  @impl true
  def apps, do: [%{name: "AI Gateway", page: "client/index.html", icon: "client/icon.svg"}]

  @impl true
  def tools do
    model = %{
      type: "string",
      description: "A model's id, as `models` lists them: local/<model> or <Vault item>/<model>"
    }

    job = %{type: "string", description: "What it's for, such as chat or vision: lowercase letters, digits and -"}

    [
      %{
        name: "chat",
        description:
          "Asks a model, and answers with its `text`, the `model` that answered and where it `runs_on`. With `schema`, `json` is the answer read as JSON",
        inputs: %{
          type: "object",
          properties: %{
            messages: %{
              type: "array",
              items: %{type: "object"},
              description:
                ~s(OpenAI's chat messages, each {role, content}. Content is text, or parts: {type: "text", text} and {type: "image_url", image_url: {url: "data:image/jpeg;base64,..."}})
            },
            model: model,
            job: %{
              job
              | description:
                  "The job whose models to try, in order. Left out, vision when there's an image and chat otherwise"
            },
            schema: %{type: "object", description: "A JSON Schema the answer keeps to"},
            max_tokens: %{type: "integer", description: "The most the answer may be, in tokens"},
            temperature: %{type: "number", description: "0 for the same answer each time, higher for more varied ones"}
          },
          required: ["messages"]
        }
      },
      %{
        name: "models",
        read_only: true,
        description:
          "Every model you can use, by where it runs, with whether that's up and what each does (vision, structured answers), and which models each job tries, in order",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "set",
        description:
          "Sets the models a job tries, in order: the first that's up answers. Yours, or everyone's when asked for the network. No models clears it",
        inputs: %{
          type: "object",
          properties: %{job: job, models: %{type: "array", items: model}},
          required: ["job", "models"]
        }
      }
    ]
  end

  @doc """
  Asks a model, for a plugin that uses this one: `request` is the `chat` tool's inputs, by string keys, and the
  answer is its answer. Raises when no model answered, saying why for each it tried.

      ai = OneCore.Call.plugin(call, "ai-gateway")
      ai.chat(call, %{"messages" => [%{"role" => "user", "content" => "Hello"}]})["text"]
  """
  def chat(%Call{} = call, request), do: Call.tool(call, "ai-gateway", "chat", request)

  @impl true
  def call("chat", %{"messages" => messages} = inputs, call) do
    images? = Enum.any?(messages, &image?/1)
    job = inputs["job"] || if(images?, do: "vision", else: "chat")

    tries =
      cond do
        model = inputs["model"] ->
          [model]

        list = mine(call)[job] || everyone(call)[job] ->
          list

        true ->
          for %{"id" => id, "vision" => vision} <- own_models(call), vision == true or not images?, uniq: true, do: id
      end

    if tries == [],
      do: raise("No model on your machines can do that, and no models are set for #{job}. `models` lists them.")

    body =
      Map.merge(
        Map.take(inputs, ["messages", "max_tokens", "temperature"]),
        if(schema = inputs["schema"],
          do: %{
            "response_format" => %{"type" => "json_schema", "json_schema" => %{"name" => "answer", "schema" => schema}}
          },
          else: %{}
        )
      )

    Enum.reduce_while(tries, [], fn id, why ->
      case asked(call, id, body, inputs["schema"] != nil) do
        {:ok, answer} -> {:halt, answer}
        {:down, said} -> {:cont, why ++ ["#{id}: #{said}"]}
      end
    end)
    |> case do
      %{} = answer -> answer
      why -> raise "No model answered. " <> Enum.map_join(why, "; ", &String.trim_trailing(&1, ".")) <> "."
    end
  end

  def call("models", _inputs, call) do
    # The Vault first: a server that can't open it passes the whole call to one that can.
    hosted = for item <- items(call), do: hosted(call, item)
    machines = Task.async_stream(Service.servers(call, Local), &machine(call, &1), timeout: :infinity)
    mine = mine(call)

    jobs =
      for {job, models} <- Map.merge(everyone(call), mine),
          do: %{job: job, models: models, whose: if(Map.has_key?(mine, job), do: "yours", else: "everyone's")}

    %{places: for({:ok, place} <- machines, do: place) ++ hosted, jobs: Enum.sort_by(jobs, & &1.job)}
  end

  def call("set", %{"job" => job, "models" => models}, call) do
    if not (job =~ ~r/^[a-z0-9-]+$/), do: raise("A job's name is lowercase letters, digits and -.")

    for model <- models,
        not String.contains?(model, "/"),
        do: raise("#{model} isn't a model's id: they're local/<model> or <Vault item>/<model>.")

    changed(call.own, "jobs/" <> call.subject, fn jobs ->
      if models == [], do: Map.delete(jobs, job), else: Map.put(jobs, job, Enum.uniq(models))
    end)

    whose = if call.subject == "network", do: "Everyone's", else: "Your"

    if models == [],
      do: "#{whose} #{job} tries no models of its own now.",
      else: "#{whose} #{job} tries #{Enum.join(Enum.uniq(models), ", then ")}."
  end

  # Each model asked is a span of the call, as OpenTelemetry's conventions for AI name it: Insights shows how long
  # each took, and what each used.
  defp asked(call, id, body, json?) do
    began = System.monotonic_time(:microsecond)
    asked = ask(call, id, body, json?)
    took = (System.monotonic_time(:microsecond) - began) / 1000
    attributes = %{"gen_ai.operation.name" => "chat", "gen_ai.request.model" => id}
    error = if match?({:down, _why}, asked), do: elem(asked, 1)
    record(call, [%{"name" => "chat #{id}", "attributes" => attributes, "took" => took, "error" => error}])
    asked
  end

  # What Insights keeps, where it's added: names, sizes and outcomes, never what was asked or answered.
  defp record(call, records) do
    if Enum.any?(Call.plugins(call), &(&1.name == "insights")),
      do:
        Call.tool(call, "insights", "record", %{
          "records" => Enum.map(records, &Map.reject(&1, fn {_, v} -> v == nil end))
        })
  end

  # Asks one model: `{:ok, answer}`, or `{:down, why}` when it isn't up, said no, or answered with what isn't an
  # answer, so the next is asked.
  defp ask(call, "local/" <> model, body, json?) do
    servers = Service.servers(call, Local)

    if servers == [],
      do: {:down, "No machine of yours runs AiGateway.Local"},
      else: ask_machines(call, servers, model, body, json?)
  end

  defp ask(call, id, body, json?) do
    [item, model] = String.split(id, "/", parts: 2)
    vault = Call.plugin(call, "vault")

    sent =
      try do
        request = %{
          url: endpoint(vault.value(call, item, "url")) <> "/chat/completions",
          method: "post",
          headers: %{"authorization" => "Bearer {{api_key}}", "content-type" => "application/json"},
          body: JSON.encode!(Map.put(body, "model", model)),
          timeout: @answers_within
        }

        vault.fetch(call, item, request)
      rescue
        problem in RuntimeError -> {:down, problem.message}
      end

    with %{status: status, body: said} <- sent, do: answered(call, id, item, status, said, json?)
  end

  defp ask_machines(_call, [], _model, _body, _json?), do: {:down, "No machine of yours that's up has it"}

  defp ask_machines(call, [server | rest], model, body, json?) do
    case (try do
            Service.call(call, Local, :chat, [model, body], server)
          rescue
            problem in RuntimeError -> %{"down" => problem.message}
          end) do
      %{"status" => status, "body" => said} ->
        answered(call, "local/" <> model, Service.name(server), status, said, json?)

      %{"down" => _why} ->
        ask_machines(call, rest, model, body, json?)
    end
  end

  # What a model said, as an answer: its text, and that read as JSON when a schema was asked for.
  defp answered(call, id, runs_on, status, said, json?) when status in 200..299 do
    case JSON.decode(said) do
      {:ok, %{"choices" => [%{"message" => %{"content" => content}} = choice | _]} = answered} ->
        used(call, id, answered["usage"])
        text = text(content)
        json = if json?, do: JSON.decode(text), else: {:ok, nil}

        cond do
          text == "" and choice["finish_reason"] == "length" ->
            {:down, "It ran out of room before it answered: give it more max_tokens"}

          match?({:error, _}, json) ->
            {:down, "It answered with what isn't JSON: #{String.slice(text, 0, 200)}"}

          true ->
            {:ok, json} = json
            {:ok, Map.merge(%{model: id, runs_on: runs_on, text: text}, if(json?, do: %{json: json}, else: %{}))}
        end

      _ ->
        {:down, "It answered with what isn't an answer"}
    end
  end

  defp answered(_call, _id, _runs_on, status, said, _json?) do
    why =
      case JSON.decode(said) do
        {:ok, %{"error" => %{"message" => message}}} -> message
        {:ok, %{"error" => message}} when is_binary(message) -> message
        {:ok, %{"errors" => [%{"message" => message} | _]}} -> message
        _ -> String.slice(said, 0, 200)
      end

    {:down, "It answered #{status}: #{String.slice(why, 0, 200)}"}
  end

  # What a model said it used: its tokens in and out, and what it cost where the endpoint says, as OpenRouter's
  # does. Insights reads them as AI Gateway's spending.
  defp used(call, id, %{} = usage) do
    model = %{"gen_ai.request.model" => id}

    tokens =
      for {type, tokens} <- [{"input", usage["prompt_tokens"]}, {"output", usage["completion_tokens"]}],
          is_integer(tokens),
          do: %{
            "name" => "gen_ai.client.token.usage",
            "attributes" => Map.put(model, "gen_ai.token.type", type),
            "value" => tokens
          }

    cost =
      if is_number(usage["cost"]),
        do: [%{"name" => "gen_ai.client.cost", "attributes" => model, "value" => usage["cost"]}],
        else: []

    if tokens ++ cost != [], do: record(call, tokens ++ cost)
  end

  defp used(_call, _id, _usage), do: :ok

  # Some servers answer with the content's parts rather than its text.
  defp text(parts) when is_list(parts), do: Enum.map_join(parts, &(&1["text"] || ""))
  defp text(text), do: text || ""

  defp image?(%{"content" => parts}) when is_list(parts), do: Enum.any?(parts, &(&1["type"] == "image_url"))
  defp image?(_message), do: false

  # The models on whoever asks's machines that are up, this machine's first.
  defp own_models(call) do
    servers = Service.servers(call, Local)
    here = Enum.filter(servers, &(&1 == call.server))

    for server <- here ++ (servers -- here),
        %{up: true, models: models} <- [machine(call, server)],
        model <- models,
        do: model
  end

  # A machine that's off says when it was last seen, which its screen says in its grey line, and isn't asked.
  defp machine(call, server) do
    place = %{name: Service.name(server), kind: "machine", server: server}

    with at when is_integer(at) <- Service.seen(server) do
      Map.merge(place, %{up: false, models: [], seen: at})
    else
      _ -> models(call, server, place)
    end
  end

  defp models(call, server, place) do
    try do
      case Service.call(call, Local, :models, [], server) do
        %{"running" => []} ->
          Map.merge(place, %{
            up: false,
            models: [],
            problem: "No model server runs on it: Ollama, llama.cpp's llama-server or LM Studio."
          })

        %{"models" => models} ->
          Map.merge(place, %{up: true, models: for(m <- models, do: Map.put(m, "id", "local/" <> m["name"]))})
      end
    rescue
      problem in RuntimeError -> Map.merge(place, %{up: false, models: [], problem: problem.message})
    end
  end

  # A hosted endpoint's models, as it lists them. Workers AI lists its own elsewhere than the rest, in
  # OpenRouter's marketplace shape, which says what each takes and gives.
  defp hosted(call, item) do
    place = %{name: item, kind: "hosted"}
    vault = Call.plugin(call, "vault")

    try do
      url = endpoint(vault.value(call, item, "url"))

      listing =
        if String.ends_with?(url, "/ai/v1"),
          do: String.replace_suffix(url, "/v1", "/models/search?format=openrouter"),
          else: url <> "/models"

      case vault.fetch(call, item, %{url: listing, headers: %{"authorization" => "Bearer {{api_key}}"}}) do
        %{status: status, body: body} when status in 200..299 ->
          listed =
            with {:ok, %{} = said} <- JSON.decode(body), do: said["data"] || said["result"] || [], else: (_ -> [])

          models =
            for model <- listed,
                is_map(model),
                model = listed(model),
                model != nil,
                do: Map.put(model, "id", "#{item}/#{model["name"]}")

          Map.merge(place, %{up: true, models: Enum.sort_by(models, & &1["name"])})

        %{status: status} ->
          Map.merge(place, %{up: false, models: [], problem: "It answered #{status} when asked for its models."})
      end
    rescue
      problem in RuntimeError -> Map.merge(place, %{up: false, models: [], problem: problem.message})
    end
  end

  @doc false
  # One model as an endpoint lists it: OpenAI's `id` alone, or with what it takes and gives in OpenRouter's
  # shapes (its public API's `architecture` and `supported_parameters`, or the marketplace's modalities, as
  # Workers AI and llama.cpp say it). What's not said is `nil`. A model that gives no text is none.
  def listed(%{"id" => id} = model) do
    types = fn list -> list && Enum.map(list, &if(is_map(&1), do: &1["type"], else: &1)) end
    takes = types.(get_in(model, ["architecture", "input_modalities"]) || model["input_modalities"])
    gives = types.(get_in(model, ["architecture", "output_modalities"]) || model["output_modalities"])

    parameters =
      case model["supported_parameters"] do
        list when is_list(list) ->
          list

        _ ->
          for %{"type" => "text", "supported_parameters" => %{} = said} <- model["output_modalities"] || [],
              {name, _} <- said,
              do: name
      end

    if gives == nil or "text" in gives do
      %{
        "name" => id,
        "vision" => takes && "image" in takes,
        "structured" =>
          if(parameters != [], do: Enum.any?(["structured_outputs", "response_format"], &(&1 in parameters)))
      }
    end
  end

  def listed(_model), do: nil

  defp endpoint(url), do: String.trim_trailing(String.trim(url), "/")

  # The Vault's items switched on for this plugin: the hosted endpoints whoever asks set up.
  defp items(call), do: for(%{name: name} <- Call.plugin(call, "vault").items(call), do: name)

  # The lists of models each job tries: whoever asks's own, and the network's, which are everyone's.
  defp mine(%Call{subject: "network"}), do: %{}
  defp mine(call), do: jobs(call, call.subject)
  defp everyone(call), do: jobs(call, "network")

  defp jobs(call, who) do
    case Artifact.read(call.own, "jobs/" <> who) do
      {:ok, data, _tag} -> JSON.decode!(data)
      :absent -> %{}
    end
  end

  # Changes a blob of JSON, again if another call changed it first.
  defp changed(own, name, change) do
    written =
      case Artifact.read(own, name) do
        {:ok, data, tag} -> Artifact.replace(own, name, JSON.encode!(change.(JSON.decode!(data))), tag)
        :absent -> Artifact.write(own, name, JSON.encode!(change.(%{})))
      end

    if written == :ok, do: :ok, else: changed(own, name, change)
  end
end
