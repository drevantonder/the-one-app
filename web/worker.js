// oneapp.drevan.me: `curl -fsSL oneapp.drevan.me | sh` gets install.sh from
// main; a browser goes to the repo.
const repo = "https://github.com/drevantonder/the-one-app";
const script = "https://raw.githubusercontent.com/drevantonder/the-one-app/main/install.sh";

export default {
  async fetch(request) {
    const { pathname } = new URL(request.url);
    const browser = (request.headers.get("accept") || "").includes("text/html");
    if (browser || (pathname !== "/" && pathname !== "/install.sh")) return Response.redirect(repo, 302);
    const res = await fetch(script, { cf: { cacheTtl: 300 } });
    return new Response(res.body, { status: res.status, headers: { "content-type": "text/plain; charset=utf-8" } });
  },
};
