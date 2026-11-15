---
name: ai-gateway
description: Read before asking an AI model something for a person or a plugin (reading a photo, sorting, summing up), when someone wants to add a model or an AI key, or to choose which model a job uses.
---

AI Gateway asks AI models, text and images, in one shape whatever runs them: on the person's own machines, at
Workers AI, or at any host that speaks OpenAI's chat completions.

- `chat` asks one. `messages` are OpenAI's, and an image goes in as a part, `{type: "image_url", image_url: {url:
  "data:image/jpeg;base64,..."}}`. With `schema`, a JSON Schema, the answer keeps to it and `json` is that answer.
  It says which `model` answered and where it `runs_on`.
- Which model answers: `model` if given, else the models the `job` tries, in order, the first that's up. The job
  is `vision` when there's an image and `chat` otherwise, unless you name another. With nothing set for it, a
  model on the person's own machines answers, and never a hosted one.
- `models` lists every model they can use, by where it runs (each machine of theirs, each hosted endpoint), with
  whether that place is up and what each model does: `vision` takes images, `structured` keeps to a schema
  (`null` when the endpoint doesn't say). It also lists what each job tries, and whose that list is.
- `set` sets the models a job tries, in order. It's the person's own; for the network (`ONE_FOR=network`) it's
  everyone's, which theirs overrides. Set what they ask for, and say what you set.

Hosted models cost money per use, so they're used only once their person set them up and named them in a job,
or you name one as `model` because they asked for it.

- A model on a machine of theirs comes from a model server already running there: Ollama, llama.cpp's
  llama-server or LM Studio. AI Gateway's service `AiGateway.Local` talks to it, so it runs on that machine
  (`services`, then `install_service`). Its models are `local/<model>`.
- A hosted endpoint is a Vault item switched on for `ai-gateway`, with two fields: `url`, the address its chat
  completions are under, not secret, and `api_key`, secret, with the endpoint's host as its site. Its models
  are `<item>/<model>`. Ask for it with `vault_request` and `plugin: "ai-gateway"`. For Workers AI, the `url`
  is `https://api.cloudflare.com/client/v4/accounts/<account id>/ai/v1`, the key is an API token that may use
  Workers AI, and the site is `api.cloudflare.com`.

A plugin asks a model by using this plugin:

    def uses, do: ["ai-gateway"]

    ai = OneCore.Call.plugin(call, "ai-gateway")
    ai.chat(call, %{"messages" => messages, "schema" => schema})["json"]

- `chat(call, request)` takes `chat`'s inputs by string keys and gives its answer, or raises with why each model
  it tried didn't answer. Nothing may be up, so whatever uses it keeps working without an answer.
- What a model says is its guess. Show it as one, and let the person change it.
