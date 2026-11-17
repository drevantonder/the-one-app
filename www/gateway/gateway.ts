// Sends /apps and everything under it to the app directory, and the rest to the marketing site.
export default {
  fetch(request, env) {
    const first = new URL(request.url).pathname.split("/")[1];
    return (first === "apps" ? env.APP_DIRECTORY : env.MARKETING).fetch(request);
  },
} satisfies ExportedHandler<Env>;
