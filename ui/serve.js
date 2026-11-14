// The gallery and the example over HTTP, as plain static files: a browser won't load modules from file://.
export const serve = (port) => Bun.serve({
  port,
  async fetch(request) {
    const path = new URL(request.url).pathname
    const file = Bun.file(import.meta.dir + (path.endsWith("/") ? path + "index.html" : path))
    return (await file.exists()) ? new Response(file) : new Response("Not found", { status: 404 })
  },
})

if (import.meta.main) {
  const { url } = serve(4173)
  console.log(`${url}gallery/\n${url}example/`)
}
