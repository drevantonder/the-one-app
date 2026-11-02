// Stand-in sites for tests: whatever is asked of any host, it answers with what it was sent.
export default {
  async fetch(request) {
    return Response.json({ method: request.method, url: request.url, headers: Object.fromEntries(request.headers), body: await request.text() });
  },
};
