// Models: the local AI models on the person's devices. The models service, built into every device's node, asks the
// ones on its machine, and a call goes on to the next device when a model isn't there; this app is what people,
// agents and other apps use.

/// Runs one of the models service's operations, on `device` if it's given, else wherever it runs for the caller. When
/// none of their devices runs it, says what's needed instead of how the call failed.
async function service(ctx, op, { device, ...input }) {
  try {
    return await ctx.services.models[op](input, device ? { device } : undefined);
  } catch (err) {
    if (String(err.message).includes("no device runs models for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Models runs on your devices, and none of them is online.";

const IMAGES = "images (JPEG, PNG or WebP) as base64 strings, shown to the model with the prompt";

export default {
  name: "models",
  title: "Models",
  tagline: "AI on your devices",
  icon: "cpu",
  color: "#32ade6",
  advanced: true,
  description: "AI models on your own devices",
  connector: true,
  needs: "Needs a device with a GPU",
  instructions: "Local models run on the device that has them. models_here lists this device's models; give device \"all\" for every device's, and which are loaded. models_chat asks one and returns its answer, on whichever of your devices has it; use it for private or bulk work instead of a hosted model. models_decide asks a decision model (like clef-flash) typed questions and answers with probabilities, fast.",
  uses: { services: ["models"] },

  ops: {
    devices: {
      reads: true,
      description: "The devices Models runs on: each { device, online }, device the device's name. models_here lists the models on each.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.devices()).filter((s) => (s.services || []).includes("models")).map((s) => ({ device: s.name, online: s.online })),
    },
    remove: {
      reads: false,
      confirm: true,
      description: "Stop using Models on one of your devices: removes it from that device, for one of its owners. The models stay on its disk.",
      input: { type: "object", required: ["device"], properties: { device: { type: "string", description: "the device's name, as models_devices lists it" } } },
      run: (ctx, { device }) => {
        ctx.service("models", device, false);
        return `Models is removed from ${device}.`;
      },
    },
    here: {
      reads: true,
      description: "The local models on this device, or with device \"all\" on every device: each { name, size, family, parameters, loaded }, as Ollama gives them (size in bytes, parameters like 12.2B), loaded when it's in memory and answers at once. Use a name as model in models_chat or models_decide.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => service(ctx, "here", input),
    },
    chat: {
      reads: true,
      description: "Ask a local model and get its answer as text, on whichever of your devices has it (or the device given): for private or bulk work instead of a hosted model. Give prompt for one question, or messages for a conversation; format makes it answer in JSON.",
      input: {
        type: "object",
        required: ["model"],
        properties: {
          model: { type: "string", description: "a model's name, as models_here lists them, like gemma4:12b" },
          device: { type: "string", description: "one of your devices, by name, to ask; else whichever of yours has the model" },
          prompt: { type: "string", description: "the question or task, as the person's message; given with messages, it comes after them" },
          messages: { type: "array", items: { type: "object" }, description: "the conversation so far, oldest first, each { role: user or assistant, content: text, images }" },
          system: { type: "string", description: "instructions the model follows throughout, like \"Answer in one sentence.\"" },
          images: { type: "array", items: { type: "string" }, description: IMAGES },
          format: { description: "\"json\" for any JSON, or a JSON schema the answer must match" },
          think: { type: "boolean", description: "whether a thinking model reasons before it answers; slower, often better" },
          options: { type: "object", description: "Ollama's options, like { temperature: 0, num_predict: 1024 }" },
        },
      },
      run: (ctx, input) => service(ctx, "chat", input),
    },
    decide: {
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
      run: (ctx, input) => service(ctx, "decide", input),
    },
  },
};
