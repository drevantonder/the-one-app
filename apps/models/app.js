// Models: the local AI models on the person's servers. The models sidecar, built into every server's node, asks the
// ones on its machine, and a call goes on to the next server when a model isn't there; this app is what people,
// agents and other apps use.

/// Runs one of the models sidecar's operations, wherever it runs for the caller. When none of their servers runs it,
/// says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars.models[op](input);
  } catch (err) {
    if (String(err.message).includes("no device runs models for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Models runs on your devices, and none of them is online.";

export default {
  name: "models",
  title: "Models",
  icon: "cpu",
  color: "#32ade6",
  description: "AI models on your own devices",
  connector: true,
  needs: "Needs a device with a GPU",
  instructions: "Local models run on the device that has them. models_here lists this device's models; give server \"all\" for every device's, and which are loaded. models_chat asks one and returns its answer, on whichever of your devices has it; use it for private or bulk work instead of a hosted model. models_decide asks a decision model (like clef-flash) typed questions and answers with probabilities, fast.",
  uses: { sidecars: ["models"] },

  ops: {
    accounts: {
      reads: true,
      description: "The devices it runs on: each { account, ok }, the device’s name, ok while it’s online.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.servers()).filter((s) => (s.sidecars || []).includes("models")).map((s) => ({ account: s.name, ok: s.online })),
    },
    sign_out: {
      confirm: true,
      description: "Stop using Models on one of your devices: removes it from that device, for one of its owners.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the device, by name" } } },
      run: (ctx, { account }) => {
        ctx.sidecar("models", account, false);
        return `Models is removed from ${account}.`;
      },
    },
    here: {
      reads: true,
      description: "This device's local models, and whether each is loaded; give server \"all\" for every device's.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "here", input),
    },
    chat: {
      reads: true,
      description: "Ask a local model and get its answer, on whichever of your devices has it (or the device given). Give prompt, or messages as [{role, content}]; system sets its instructions; format \"json\" or a JSON schema makes it answer in JSON.",
      input: {"properties": {"format": {"description": "\"json\", or a JSON schema the answer must match"}, "images": {"description": "images (JPEG, PNG or WebP) in base64, shown with the prompt", "items": {"type": "string"}, "type": "array"}, "messages": {"items": {"type": "object"}, "type": "array"}, "model": {"description": "a model's name as here lists it", "type": "string"}, "options": {"description": "Ollama's options, like { temperature: 0, num_predict: 1024 }", "type": "object"}, "prompt": {"type": "string"}, "system": {"type": "string"}, "think": {"description": "whether a thinking model reasons first", "type": "boolean"}}, "required": ["model"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "chat", input),
    },
    decide: {
      reads: true,
      description: "Ask a decision model (a model with the decision capability, like clef-flash) questions about something and get typed answers with probabilities: choice (one of criteria, an object of option: description), noul (yes/no) or score (levels, lowest first). Answers { answers: { name: { choice, probabilities, confidence } }, usage }.",
      input: {"properties": {"images": {"description": "images in base64 (or [{ content_type, base64 }]), named in state as image 1, 2...", "type": "array"}, "model": {"description": "a decision model, like clef-flash", "type": "string"}, "questions": {"description": "named questions, each { type, instructions, criteria | levels }", "type": "object"}, "state": {"description": "what to decide about: named fields", "type": "object"}}, "required": ["model", "questions"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "decide", input),
    },
  },
};
