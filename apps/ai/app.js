// AI for apps: read a photo or PDF with a prompt, and ask a decision model typed questions. It reads with the user's
// local models (the models driver, on the server that has them) or with Gemini once its key is set as this app's
// `gemini_key` secret. Apps use it as ctx.apps.ai.read and ctx.apps.ai.decide, so none of them holds a key.

const DEFAULTS = { provider: "local", local_server: null, read_model: "gemma4:12b", decide_model: "clef-flash", gemini_model: "gemini-3.5-flash-lite" };
const GEMINI = "generativelanguage.googleapis.com";

const kinds = ["user", "global"];
const schema = "create table settings (key text primary key, value text);";

/// The settings that apply: the user's own over the install's, over the defaults.
async function settings(ctx) {
  const out = { ...DEFAULTS };
  for (const s of [ctx.global, ctx.user]) {
    if (!s) continue;
    for (const r of await s.query("select key, value from settings")) out[r.key] = JSON.parse(r.value);
  }
  return out;
}

/// Errors that mean a limit stopped the call: Claims and others tell the person when to try again.
const limit = (when) => new Error(when === "day" ? "AI limit: day. Today's AI is used up; more tomorrow." : "AI limit: minute. Try again in a minute.");

async function gemini(ctx, s, body) {
  const ask = async (tier) => {
    const r = await fetch(`https://${GEMINI}/v1beta/models/${s.gemini_model}:generateContent`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-goog-api-key": "{{gemini_key}}" },
      body: JSON.stringify({ ...(tier === "flex" ? { service_tier: "flex" } : {}), ...body }),
    });
    const text = await r.text();
    if (r.status === 429) throw /day|quota/i.test(text) ? limit("day") : limit("minute");
    if (!r.ok) throw new Error(`Gemini answered ${r.status}: ${text.slice(0, 200)}`);
    return JSON.parse(text);
  };
  // Flex is Gemini's half-price tier, which may be busy; a failure gets one more go on the standard tier. Limits don't.
  return ask("flex").catch(async (e) => {
    if (/AI limit/.test(e.message)) throw e;
    return ask("standard");
  });
}

export default {
  name: "ai",
  title: "AI",
  icon: "sparkles",
  color: "#af52de",
  instructions: "AI for apps: ai_read reads photos and PDFs with a prompt, ai_decide asks a decision model typed questions. ai_settings shows which models answer (local models on one of your servers, or Gemini once its gemini_key secret is set); ai_set changes them.",
  uses: { drivers: ["models"] },
  schema: Object.fromEntries(kinds.map((k) => [k, schema])),

  ops: {
    settings: {
      description: "Which AI answers: provider (local or gemini), local_server (the server with the local models), and the models for reading and deciding.",
      input: { type: "object", properties: {} },
      run: (ctx) => settings(ctx),
    },
    set: {
      description: "Change which AI answers, for you (or, with scope global, for everyone: admins). provider is local or gemini (Gemini needs this app's gemini_key secret, for generativelanguage.googleapis.com).",
      input: {
        type: "object",
        properties: {
          provider: { type: "string", enum: ["local", "gemini"] },
          local_server: { type: "string", description: "the server whose local models answer, by name" },
          read_model: { type: "string" }, decide_model: { type: "string" }, gemini_model: { type: "string" },
          scope: { type: "string", description: "user (the default) or global" },
        },
      },
      async run(ctx, input) {
        const s = input.scope === "global" ? ctx.global : ctx.user;
        if (!s) throw new Error(`ai isn't installed for ${input.scope || "you"}`);
        if (input.scope === "global" && s.role !== "admin") throw new Error("only admins change everyone's AI");
        if (input.provider && !["local", "gemini"].includes(input.provider)) throw new Error("provider is local or gemini");
        for (const key of Object.keys(DEFAULTS)) if (input[key] !== undefined) s.append("set", { key, value: input[key] });
        return "Saved.";
      },
    },
    read: {
      description: "Read files (photos or PDFs, base64) with a prompt and answer the model's text. system sets its instructions; max_tokens caps the answer; json_schema asks for JSON that matches it. Answers { text, provider, cut } (cut: the answer hit max_tokens). Fails with \"AI limit: minute|day\" when a limit stops it.",
      input: {
        type: "object",
        required: ["prompt"],
        properties: {
          prompt: { type: "string" }, system: { type: "string" },
          files: { type: "array", items: { type: "object", description: "{ type, data }: a MIME type and base64 bytes" } },
          max_tokens: { type: "number" }, json_schema: { type: "object" },
        },
      },
      async run(ctx, { prompt, system, files = [], max_tokens = 8192, json_schema }) {
        const s = await settings(ctx);
        if (s.provider === "gemini") {
          const reply = await gemini(ctx, s, {
            ...(system ? { systemInstruction: { parts: [{ text: system }] } } : {}),
            contents: [{ role: "user", parts: [{ text: prompt }, ...files.map((f) => ({ inline_data: { mime_type: f.type, data: f.data } }))] }],
            generationConfig: {
              temperature: 0, maxOutputTokens: max_tokens, thinkingConfig: { thinkingLevel: "low" },
              ...(json_schema ? { responseMimeType: "application/json", responseJsonSchema: json_schema } : {}),
            },
          });
          const c = reply.candidates?.[0];
          const text = (c?.content?.parts || []).filter((p) => p.text && !p.thought).map((p) => p.text).join("");
          // Token counts only, never content: what each read costs.
          const u = reply.usageMetadata || {};
          console.log("ai read", JSON.stringify({ provider: "gemini", input: u.promptTokenCount, output: u.candidatesTokenCount, thinking: u.thoughtsTokenCount }));
          return { text, provider: "gemini", cut: reply.candidates?.some((x) => x.finishReason === "MAX_TOKENS") || false };
        }
        if (files.some((f) => !f.type.startsWith("image/"))) throw new Error("Local models read photos, not PDFs: set Gemini up with ai_set to read PDFs.");
        const reply = await ctx.drivers.models.chat({
          model: s.read_model, system, think: false,
          options: { temperature: 0, num_predict: max_tokens },
          messages: [{ role: "user", content: prompt, images: files.map((f) => f.data) }],
          ...(json_schema ? { format: json_schema } : {}),
        }, { server: s.local_server || undefined });
        return { text: String(reply ?? ""), provider: "local", cut: false };
      },
    },
    decide: {
      description: "Ask a decision model questions about something: state is named fields; questions are named, each { type: choice, instructions, criteria: { option: description } } (or noul, score); images are [{ content_type, base64 }] (or base64 strings). Answers { answers: { name: { choice, probabilities, confidence } }, usage }.",
      input: {
        type: "object",
        required: ["questions"],
        properties: { state: { type: "object" }, questions: { type: "object" }, images: { type: "array", items: { type: "object" } } },
      },
      async run(ctx, { state = {}, questions, images }) {
        const s = await settings(ctx);
        return ctx.drivers.models.decide({ model: s.decide_model, state, questions, ...(images?.length ? { images: images.map((i) => i.base64 ?? i) } : {}) }, { server: s.local_server || undefined });
      },
    },
  },

  on: {
    set(db, e) {
      if (e.scope === "global" && e.role !== "admin") return;
      db.exec("insert or replace into settings values (?, ?)", [e.data.key, JSON.stringify(e.data.value)]);
    },
  },
};
