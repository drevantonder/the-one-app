// Models: the AI models on the user's devices, for AI Gateway. Its service (service/), on each device it's switched on
// for, answers `ctx.ai.run` with `local/<model>` from the models Ollama has there, in AI Gateway's shapes (`ai: true`),
// and a call goes on to the next device when a model isn't there. Apps and agents use them through AI Gateway
// (`ctx.ai.run`, `ai-gateway_run`, `ai-gateway_models`), never this app.

export default {
  name: "models",
  title: "Models",
  tagline: "AI on your devices",
  icon: "cpu",
  color: "#32ade6",
  advanced: true,
  description: "Runs models on your devices for AI Gateway",
  needs: "Needs a device with a GPU",
  instructions: "Models on your devices are local/<model> in AI Gateway: ai-gateway_models lists each device's, and ai-gateway_run (or an app's ctx.ai.run) asks one, on whichever of your devices has it, at no cost.",
  uses: { services: ["models"] },
  service: { network: ["127.0.0.1:11434"], user: false, platforms: ["linux", "mac"], ai: true },
};
