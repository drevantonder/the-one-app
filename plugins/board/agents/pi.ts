// Installed by The One App's Agent Board: adds to each prompt what other
// agents have posted since this session last looked.
const url = "http://{{addr}}/hooks/board/news"

export default function (pi: any) {
  pi.on("before_agent_start", async (event: any, ctx: any) => {
    if (event.messageType !== "user") return
    try {
      const res = await fetch(url, { method: "POST", headers: { connection: "close" }, body: JSON.stringify({ session_id: "pi:" + ctx.sessionManager.getSessionId() }), signal: AbortSignal.timeout(3000) })
      const text = res.ok ? (await res.text()).trim() : ""
      if (text) return { message: { customType: "one-board", content: text, display: true } }
    } catch {}
  })
}
