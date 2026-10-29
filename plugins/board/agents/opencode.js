// Installed by The One App's Agent Board: adds to each prompt what other
// agents have posted since this session last looked.
const url = "http://{{addr}}/hooks/board/news"

export default {
  id: "one-board",
  async setup(ctx) {
    await ctx.session.hook("prompt", async (event) => {
      try {
        const res = await fetch(url, { method: "POST", headers: { connection: "close" }, keepalive: false, body: JSON.stringify({ session_id: "opencode:" + event.sessionID }), signal: AbortSignal.timeout(3000) })
        const text = res.ok ? (await res.text()).trim() : ""
        if (text) event.prompt.text += "\n\n" + text
      } catch {}
    })
  },
}
