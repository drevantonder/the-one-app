// What the client's own pages ask their host (`respond` in native.rs), apart from the UI library's own asks, by ids of
// their own, each answered as it's heard.
const waiting = new Map()
let asked = 0
const heard = globalThis.ui.message
globalThis.ui.message = (message) => {
  const answer = typeof message.id === "string" ? waiting.get(message.id) : undefined
  if (!answer) return heard(message)
  waiting.delete(message.id)
  answer(message)
}

/// Asks the page's host `method`, → what it answered, or throws what it said instead.
export const ask = (method, params = {}) =>
  new Promise((done, failed) => {
    const id = `client-${++asked}`
    waiting.set(id, ({ result, error }) => (error ? failed(new Error(error.message)) : done(result)))
    globalThis.host.post({ jsonrpc: "2.0", id, method, params })
  })
