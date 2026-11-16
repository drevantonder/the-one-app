// Sends /plugins and everything under it to the plugin directory, and the rest to the marketing site.
export default {
  fetch(request, env) {
    const first = new URL(request.url).pathname.split("/")[1];
    return (first === "plugins" ? env.PLUGIN_DIRECTORY : env.MARKETING).fetch(request);
  },
};
