// AI for apps: read a photo or PDF with a prompt, and ask a decision model typed questions. It reads with the user's
// local models (the models service, on the device that has them) or with Gemini once the person's Vault has a Gemini
// key item for it (its secret field `gemini_key`). Apps use it as ctx.apps.ai.read and ctx.apps.ai.decide, so none of them holds a key.

const DEFAULTS = { provider: "local", local_device: null, read_model: "gemma4:12b", decide_model: "clef-flash", gemini_model: "gemini-3.5-flash-lite" };
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

/// Errors that mean a limit stopped the call: apps tell the person when to try again.
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
  tagline: "Reads photos and PDFs",
  icon: "sparkle",
  color: "#af52de",
  advanced: true,
  description: "Reads photos and PDFs and answers questions, for your apps",
  instructions: "AI for apps: ai_read reads photos and PDFs with a prompt, ai_decide asks a decision model typed questions. ai_settings shows which models answer (local models on one of your devices, or Gemini once the person's Vault has a Gemini key for it: ask with vault_request, app ai, need gemini); ai_set changes them.",
  uses: { services: ["models"] },
  vault: {
    gemini: { title: "Gemini key", type: "API key", domain: "aistudio.google.com", hosts: ["generativelanguage.googleapis.com"], fields: { gemini_key: { label: "API key", secret: true } } },
  },
  schema: Object.fromEntries(kinds.map((k) => [k, schema])),

  ops: {
    settings: {
      reads: true,
      description: "Which AI answers ai_read and ai_decide for you: { provider, local_device, read_model, decide_model, gemini_model }, your own settings over everyone's, over the defaults. provider is local (models on local_device, or any of your devices when it's null) or gemini.",
      input: { type: "object", properties: {} },
      run: (ctx) => settings(ctx),
    },
    set: {
      reads: false,
      description: "Change which AI answers ai_read and ai_decide, for you, or with scope global for everyone (admins only): only the settings given change. Gemini needs a Gemini key in the Vault first: vault_request with app ai and need gemini. Says Saved.",
      input: {
        type: "object",
        properties: {
          provider: { type: "string", enum: ["local", "gemini"], description: "local, the models on your devices, or gemini, Google's, which reads PDFs too" },
          local_device: { type: "string", description: "the device whose local models answer, by its name as models_devices lists them" },
          read_model: { type: "string", description: "the local model ai_read uses, as models_here lists them; default gemma4:12b" },
          decide_model: { type: "string", description: "the local decision model ai_decide uses, as models_here lists them; default clef-flash" },
          gemini_model: { type: "string", description: "the Gemini model ai_read uses with provider gemini, as Google names it; default gemini-3.5-flash-lite" },
          scope: { type: "string", enum: ["user", "global"], description: "user, for you (the default), or global, for everyone" },
        },
      },
      async run(ctx, input) {
        const s = input.scope === "global" ? ctx.global : ctx.user;
        if (!s) throw new Error(input.scope === "global" ? "AI isn't added for everyone: an admin adds it with apps_add, app ai and scope global" : "AI isn't added for you: apps_add with app ai and scope user adds it");
        if (input.scope === "global" && s.role !== "admin") throw new Error("only admins change everyone's AI: leave out scope to change your own");
        if (input.provider && !["local", "gemini"].includes(input.provider)) throw new Error(`provider is local or gemini, not ${JSON.stringify(input.provider)}`);
        for (const key of Object.keys(DEFAULTS)) if (input[key] !== undefined) s.exec("insert or replace into settings values (?, ?)", [key, JSON.stringify(input[key])]);
        return "Saved.";
      },
    },
    read: {
      reads: true,
      description: "Read photos or PDFs with a prompt, with the AI ai_settings names, for text out of a receipt, a form or a scan. Gives { text, provider, cut }: the answer, which AI gave it, and cut when it hit max_tokens. Local models read photos only. Fails with \"AI limit: minute\" or \"AI limit: day\" when a limit stops it: try again then. For a question with no file, models_chat.",
      input: {
        type: "object",
        required: ["prompt"],
        properties: {
          prompt: { type: "string", description: "what to do with the files, like \"The total and date of this receipt\"" },
          system: { type: "string", description: "instructions it follows throughout, like \"Answer only with JSON.\"" },
          files: { type: "array", items: { type: "object", description: "{ type, data }: a MIME type (image/jpeg, image/png, application/pdf) and the file's bytes in base64" }, description: "the photos or PDFs to read, as { type, data }" },
          max_tokens: { type: "number", description: "the longest answer, in tokens; default 8192" },
          json_schema: { type: "object", description: "a JSON schema the answer's text must match, like { type: \"object\", properties: { total: { type: \"number\" } } }" },
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
        const reply = await ctx.services.models.chat({
          model: s.read_model, system, think: false,
          options: { temperature: 0, num_predict: max_tokens },
          messages: [{ role: "user", content: prompt, images: files.map((f) => f.data) }],
          ...(json_schema ? { format: json_schema } : {}),
        }, { device: s.local_device || undefined });
        return { text: String(reply ?? ""), provider: "local", cut: false };
      },
    },
    decide: {
      reads: true,
      description: "Ask the decision model ai_settings names typed questions about something, for fast labels and scores rather than prose. Each question is a choice (one of criteria), noul (yes or no) or score (one of levels, lowest first). Gives { answers: { <question>: { choice, probabilities, confidence } }, usage }. To pick the model yourself, models_decide.",
      input: {
        type: "object",
        required: ["questions"],
        properties: {
          state: { type: "object", description: "what to decide about, as named fields: { subject: \"...\", body: \"...\" }" },
          questions: { type: "object", description: "named questions, each { type: choice, noul or score, instructions, criteria: { option: what it means } for choice, levels: [lowest, ..., highest] for score }" },
          images: { type: "array", items: { type: "object" }, description: "images as { content_type, base64 } (or base64 strings), which state names as image 1, image 2..." },
        },
      },
      async run(ctx, { state = {}, questions, images }) {
        const s = await settings(ctx);
        return ctx.services.models.decide({ model: s.decide_model, state, questions, ...(images?.length ? { images: images.map((i) => i.base64 ?? i) } : {}) }, { device: s.local_device || undefined });
      },
    },
  },

};
