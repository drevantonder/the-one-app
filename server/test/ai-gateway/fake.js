// A fake model server for server/test/ai-gateway.sh, on the port it's given: `bun fake.js <port>`. It answers as
// each kind of place AI Gateway asks would, by the path it's asked at:
//
// - /api/v1/models and /v1/chat/completions, as LM Studio does on a machine (on port 1234, LM Studio's own);
// - /hosted/v1/models and /hosted/v1/chat/completions, as a host speaking OpenAI's chat completions does;
// - /client/v4/accounts/proof/ai/..., as Workers AI does: its chat completions, and its models in OpenRouter's
//   marketplace shape.
//
// A hosted place wants `Bearer <KEY>`, KEY from the environment. The model asked for says how it answers:
// `broken` fails, saying back the key it was sent, and `slow` waits SLOW seconds. A schema is answered with an
// object that keeps to it, and an image is described by its size. GET /seen says what it was asked, without the key.
const port = Number(process.argv[2])
const { KEY, SLOW = "20" } = process.env
const seen = { asked: [], keys: { right: 0, wrong: 0 } }

// What it used is ten tokens in and one a character out, and a host says what that cost, as OpenRouter does.
const answer = (model, content, priced) =>
  Response.json({
    id: "chatcmpl-fake",
    object: "chat.completion",
    model,
    choices: [{ index: 0, message: { role: "assistant", content }, finish_reason: "stop" }],
    usage: { prompt_tokens: 10, completion_tokens: content.length, total_tokens: 10 + content.length, ...(priced && { cost: 0.0004 }) },
  })

// What a receipt says, for a schema that asks for it, and something of the right type for anything else.
const receipt = { vendor: "Proof Cafe", date: "2026-11-14", total: 12.5, gst: 1.14 }
const filled = (schema) =>
  Object.fromEntries(
    Object.entries(schema.properties ?? {}).map(([name, property]) => {
      const type = [property.type].flat().find((type) => type !== "null")
      return [name, receipt[name] ?? (type === "number" || type === "integer" ? 1 : type === "boolean" ? true : "fake")]
    }),
  )

async function chat(request, url) {
  const body = await request.json()
  const parts = body.messages.flatMap((message) => (Array.isArray(message.content) ? message.content : [{ type: "text", text: message.content }]))
  const images = parts.filter((part) => part.type === "image_url").map((part) => part.image_url.url ?? part.image_url)
  seen.asked.push({ at: url.pathname, model: body.model, images: images.length, schema: !!body.response_format })
  if (body.model === "broken") {
    const sent = request.headers.get("authorization") ?? ""
    return Response.json({ error: { message: `the model broke, with the key ${sent}` } }, { status: 500 })
  }
  if (body.model === "slow") await Bun.sleep(Number(SLOW) * 1000)
  const priced = url.pathname.startsWith("/hosted/")
  if (body.response_format?.type === "json_schema") return answer(body.model, JSON.stringify(filled(body.response_format.json_schema.schema)), priced)
  if (images.length) return answer(body.model, `An image of ${Buffer.from(images[0].split(",")[1], "base64").length} bytes.`, priced)
  const text = parts.filter((part) => part.type === "text").at(-1)?.text
  return answer(body.model, `You said: ${text}`, priced)
}

// Whether a hosted place was sent the key, counted without saying it.
const keyed = (request) => {
  const right = request.headers.get("authorization") === `Bearer ${KEY}`
  seen.keys[right ? "right" : "wrong"] += 1
  return right
}
const refused = () => Response.json({ error: { message: "That key isn't one of ours." } }, { status: 401 })

Bun.serve({
  port,
  hostname: "127.0.0.1",
  idleTimeout: 0,
  async fetch(request) {
    const url = new URL(request.url)
    const path = url.pathname
    if (path === "/seen") return Response.json(seen)
    // LM Studio, on a machine.
    if (path === "/api/v1/models")
      return Response.json({
        models: [
          { type: "llm", key: "fake-text", capabilities: { vision: false } },
          { type: "llm", key: "fake-vision", capabilities: { vision: true } },
          { type: "embedding", key: "fake-embedding" },
        ],
      })
    if (path === "/v1/chat/completions") return chat(request, url)
    // A host.
    if (path.startsWith("/hosted/") && !keyed(request)) return refused()
    if (path === "/hosted/v1/models")
      return Response.json({
        object: "list",
        data: [
          { id: "cloud-text", architecture: { input_modalities: ["text"], output_modalities: ["text"] }, supported_parameters: ["response_format"] },
          { id: "cloud-vision", architecture: { input_modalities: ["text", "image"], output_modalities: ["text"] }, supported_parameters: ["structured_outputs"] },
          { id: "broken" },
          { id: "slow" },
        ],
      })
    if (path === "/hosted/v1/chat/completions") return chat(request, url)
    // Workers AI.
    if (path.startsWith("/client/v4/") && !keyed(request)) return refused()
    if (path === "/client/v4/accounts/proof/ai/models/search" && url.searchParams.get("format") === "openrouter")
      return Response.json({
        data: [
          {
            id: "@cf/fake/vision",
            input_modalities: [{ type: "text" }, { type: "image" }],
            output_modalities: [{ type: "text", supported_parameters: { structured_outputs: { type: "boolean" } } }],
          },
          { id: "@cf/fake/painter", input_modalities: [{ type: "text" }], output_modalities: [{ type: "image" }] },
        ],
      })
    if (path === "/client/v4/accounts/proof/ai/v1/chat/completions") return chat(request, url)
    return new Response("Not found", { status: 404 })
  },
})
console.log(`A fake model server is at 127.0.0.1:${port}`)
