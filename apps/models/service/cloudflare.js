// Models' service built for Cloudflare: the Cloudflare node runs it once an admin switches it on there, with the
// account's Workers AI (`ai`, its binding). A function that gives nothing (undefined) can't answer here, and the call
// goes on to a device that might, as a device without the model passes it on.

/// Workers AI's models are named `@cf/...` or `@hf/...`.
const ours = (model) => /^@(cf|hf)\//.test(String(model || ""));

export default {
  /// The account's text models, each { name, description, price, loaded }: the price per call as Workers AI says it.
  async here(_, { ai }) {
    const out = [];
    for (let page = 1; page <= 10; page++) {
      const got = await ai.models({ task: "Text Generation", per_page: 100, page });
      for (const m of got) {
        const price = (m.properties || []).find((p) => p.property_id === "price")?.value;
        out.push({ name: m.name, description: m.description, price: Array.isArray(price) ? price.map((p) => `$${p.price} ${p.unit}`).join(", ") : undefined, loaded: true });
      }
      if (got.length < 100) break;
    }
    return out;
  },

  /// One answer from one of its models, as text (or JSON with format); a model it doesn't have, or images, go on.
  async chat(input, { ai }) {
    if (!ours(input.model) || input.images?.length || input.messages?.some((m) => m.images?.length)) return undefined;
    const messages = [...(input.system ? [{ role: "system", content: input.system }] : []), ...(input.messages || []).map(({ role, content }) => ({ role, content }))];
    if (input.prompt) messages.push({ role: "user", content: input.prompt });
    if (!messages.some((m) => m.role !== "system")) throw new Error("give a prompt or messages");
    const o = input.options || {};
    const body = { messages, ...(o.temperature !== undefined && { temperature: o.temperature }), ...(o.num_predict && { max_tokens: o.num_predict }) };
    if (input.format) body.response_format = input.format === "json" ? { type: "json_object" } : { type: "json_schema", json_schema: input.format };
    const r = await ai.run(input.model, body);
    const said = r?.response ?? r?.choices?.[0]?.message?.content ?? "";
    return typeof said === "string" ? said : JSON.stringify(said);
  },
};
