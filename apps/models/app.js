// Models: the local AI models on the user's devices. Its service (service/), on each device it's switched on for, asks
// the ones on its machine, and a call goes on to the next device when a model isn't there; this app is what users,
// agents and other apps use.

const service = (ctx, fn, { device, ...input }) => ctx.services.models[fn](input, device ? { device } : undefined);

const IMAGES = "images (JPEG, PNG or WebP) as base64 strings, shown to the model with the prompt";

export default {
  name: "models",
  title: "Models",
  tagline: "AI on your devices",
  icon: "cpu",
  color: "#32ade6",
  advanced: true,
  description: "AI models on your own devices",
  needs: "Needs a device with a GPU",
  instructions: "Local models run on the device that has them. models_here lists this device's models; give device \"all\" for every device's, and which are loaded. models_chat asks one and returns its answer, on whichever of your devices has it; use it for private or bulk work instead of a hosted model. models_decide asks a decision model (like clef-flash) typed questions and answers with probabilities, fast. Where an admin switched Models on for Cloudflare (Settings › Devices), Workers AI's models (named @cf/..., listed by models_here on device \"cloudflare\") answer there, at the network's Cloudflare account's per-call price.",
  uses: { services: ["models"] },
  service: { network: ["127.0.0.1:11434"], user: false, platforms: ["linux", "mac"] },

  tools: {
    here: {
      title: "The local models on a device",
      reads: true,
      description: "The local models on this device, or with device \"all\" on every device: each { name, size, family, parameters, loaded }, as Ollama gives them (size in bytes, parameters like 12.2B), loaded when it's in memory and answers at once. Use a name as model in models_chat or models_decide.",
      input: { type: "object", properties: {} },
      output: {
        type: ["array", "object"],
        description: "this device's models; with device \"all\", an object of each device's name to its models, or { error } for one that failed",
        items: {
          type: "object",
          required: ["name", "loaded"],
          properties: {
            name: { type: "string" },
            size: { type: "integer", description: "bytes" },
            family: { type: "string" },
            parameters: { type: "string", description: "like 12.2B" },
            loaded: { type: "boolean", description: "whether it's in memory" },
            description: { type: "string", description: "on Cloudflare" },
            price: { type: "string", description: "on Cloudflare: the price per call" },
          },
        },
        additionalProperties: true,
      },
      run: (ctx, input) => service(ctx, "here", input),
    },
    chat: {
      title: "Ask a local model",
      reads: true,
      description: "Ask a local model and get its answer as text, on whichever of your devices has it (or the device given): for private or bulk work instead of a hosted model. Give prompt for one question, or messages for a conversation; format makes it answer in JSON.",
      input: {
        type: "object",
        required: ["model"],
        properties: {
          model: { type: "string", description: "a model's name, as models_here lists them, like gemma4:12b" },
          device: { type: "string", description: "one of your devices, by name, to ask; else whichever of yours has the model" },
          prompt: { type: "string", description: "the question or task, as the user's message; given with messages, it comes after them" },
          messages: { type: "array", items: { type: "object" }, description: "the conversation so far, oldest first, each { role: user or assistant, content: text, images }" },
          system: { type: "string", description: "instructions the model follows throughout, like \"Answer in one sentence.\"" },
          images: { type: "array", items: { type: "string" }, description: IMAGES },
          format: { description: "\"json\" for any JSON, or a JSON schema the answer must match" },
          think: { type: "boolean", description: "whether a thinking model reasons before it answers; slower, often better" },
          options: { type: "object", description: "Ollama's options, like { temperature: 0, num_predict: 1024 }" },
        },
      },
      output: { type: "string", description: "the model's answer, JSON text when format asked for it" },
      run: (ctx, input) => service(ctx, "chat", input),
    },
    decide: {
      title: "Ask a decision model",
      reads: true,
      description: "Ask a decision model (one with the decision capability, like clef-flash) typed questions about something, for fast labels and scores rather than prose. Each question is a choice (one of criteria), noul (yes or no) or score (one of levels, lowest first). Gives { answers: { <question>: { choice, probabilities, confidence } }, usage }.",
      input: {
        type: "object",
        required: ["model", "questions"],
        properties: {
          model: { type: "string", description: "a decision model's name, as models_here lists them, like clef-flash" },
          device: { type: "string", description: "one of your devices, by name, to ask; else whichever of yours has the model" },
          state: { type: "object", description: "what to decide about, as named fields: { subject: \"...\", body: \"...\" }" },
          questions: { type: "object", description: "named questions, each { type: choice, noul or score, instructions, criteria: { option: what it means } for choice, levels: [lowest, ..., highest] for score }" },
          images: { type: "array", description: "images as base64 strings (or { content_type, base64 }), which state names as image 1, image 2..." },
        },
      },
      output: {
        type: "object",
        required: ["answers"],
        properties: {
          answers: { type: "object", description: "each question's name to { choice, probabilities, confidence }", additionalProperties: true },
          usage: { type: "object", description: "what the model spent", additionalProperties: true },
        },
      },
      run: (ctx, input) => service(ctx, "decide", input),
    },
  },
};
