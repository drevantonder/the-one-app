// A page's `video` nodes on the web (renderer.ts's `video`): each a screen's live video over WebRTC, connected and played
// by the browser as the DOM's `Video` does (core/ui/src/drawing-primitives/primitives.tsx, whose connection, data channel
// and input this is), each frame it plays given to the renderer at the size it's drawn (`video_frame`), which copies it
// into its texture and draws it in the node's box. What the person does on it, unless it's
// `watch`ed, goes down its data channel (core/ui/src/drawing-primitives/input.ts's shapes); what it says the page hears
// as events (`offer`, `state`, `size`).

/** Keys as the browser names them, as X does, which is how a screen is told them (input.ts's `named`). */
const KEYS = { Enter: "Return", Backspace: "BackSpace", Tab: "Tab", Escape: "Escape", Delete: "Delete", ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down", Home: "Home", End: "End", PageUp: "Prior", PageDown: "Next" }
const mac = /Mac|iPhone|iPad/.test(navigator.platform)
/** How far a wheel turns for one of its steps, in pixels (primitives.tsx's). */
const STEP = 60

/** @param {{ key: string, ctrlKey: boolean, altKey: boolean, shiftKey: boolean, metaKey: boolean }} event */
const mods = (event) => [(event.ctrlKey || (mac && event.metaKey)) && "Control_L", event.altKey && "Alt_L", event.shiftKey && "Shift_L", !mac && event.metaKey && "Super_L"].filter(Boolean)

/**
 * The page's videos: `created`, `set` and `dropped` follow its mutations, and `pointer`, `wheel` and `key` say whether
 * what the DOM said happened on the canvas was a video's, and send it down its channel.
 * @param {any} renderer
 */
export function videos(renderer) {
  /** @type {Map<number, any>} */
  const all = new Map()
  /** The video last pressed, which keys go to. @type {any} */
  let active = null
  const tell = (/** @type {object} */ event) => globalThis.ui?.event({ timeStamp: performance.now(), ...event })

  /** @param {number} id */
  const make = (id) => {
    const video = Object.assign(document.createElement("video"), { autoplay: true, muted: true, playsInline: true, disablePictureInPicture: true })
    // In the frame, unseen, so it plays: the renderer draws its frames.
    video.style.cssText = "position:fixed;left:0;top:0;width:1px;height:1px;opacity:0;pointer-events:none"
    document.body.append(video)
    const stream = { id, video, peer: null, channel: null, watch: false, over: () => {} }
    video.addEventListener("resize", () => tell({ id, name: "size", width: video.videoWidth, height: video.videoHeight }))
    // Each frame it plays, at the size it's drawn, to the renderer, its pixels read from a canvas it's drawn into.
    const canvas = new OffscreenCanvas(1, 1)
    const context = canvas.getContext("2d", { willReadFrequently: true })
    const frame = () => {
      if (!all.has(id)) return
      const [, , , , width, height] = renderer.video_drawn(id)
      const [natural, tall] = [video.videoWidth, video.videoHeight]
      if (natural && tall) {
        const scale = width && height ? Math.min(width / natural, height / tall, 1) : 1
        const [w, h] = [Math.max(1, Math.round(natural * scale)), Math.max(1, Math.round(tall * scale))]
        if (canvas.width !== w || canvas.height !== h) [canvas.width, canvas.height] = [w, h]
        context?.drawImage(video, 0, 0, w, h)
        const pixels = context?.getImageData(0, 0, w, h).data
        if (pixels) renderer.video_frame(id, w, h, new Uint8Array(pixels.buffer))
      }
      video.requestVideoFrameCallback(frame)
    }
    video.requestVideoFrameCallback(frame)
    return stream
  }

  /** A new connection, each `ice`, the last one closed, as the DOM's `Video` makes one. */
  const connect = (/** @type {any} */ stream, /** @type {RTCConfiguration} */ ice) => {
    stream.over()
    if (!globalThis.RTCPeerConnection) return tell({ id: stream.id, name: "state", state: "lost" })
    const pc = (stream.peer = new RTCPeerConnection(ice))
    let over = false
    const lost = () => !over && ((over = true), tell({ id: stream.id, name: "state", state: "lost" }))
    stream.over = () => ((over = true), pc.close())
    pc.addTransceiver("video", { direction: "recvonly" })
    // Both ends make the data channel (id 0), so it opens with the connection, with nothing more to say.
    const data = (stream.channel = pc.createDataChannel("input", { negotiated: true, id: 0 }))
    pc.ontrack = ({ track, receiver }) => {
      receiver.jitterBufferTarget = 0
      stream.video.srcObject = new MediaStream([track])
    }
    data.onopen = () => !over && tell({ id: stream.id, name: "state", state: "connected" })
    data.onclose = lost
    pc.onconnectionstatechange = () => ["failed", "closed"].includes(pc.connectionState) && lost()
    pc.setLocalDescription()
      .then(
        () =>
          new Promise((done) => {
            const timer = setTimeout(done, 5000)
            pc.onicegatheringstatechange = () => pc.iceGatheringState === "complete" && done(undefined)
            pc.onicecandidate = ({ candidate }) => candidate?.type === "relay" && (clearTimeout(timer), setTimeout(done, 300))
          }),
      )
      .then(() => !over && tell({ id: stream.id, name: "offer", sdp: pc.localDescription?.sdp }), lost)
  }

  const send = (/** @type {any} */ stream, /** @type {object} */ input) => !stream.watch && stream.channel?.readyState === "open" && stream.channel.send(JSON.stringify(input))

  /** The video under `x`, `y` (CSS pixels on the canvas), and where on its screen that is, in its pixels. */
  const under = (/** @type {number} */ x, /** @type {number} */ y) => {
    for (const stream of all.values()) {
      const [left, top, width, height] = renderer.video_drawn(stream.id)
      const [natural, tall] = [stream.video.videoWidth, stream.video.videoHeight]
      if (!width || !natural || x < left || y < top || x > left + width || y > top + height) continue
      const scale = Math.min(width / natural, height / tall)
      const at = { x: (x - left - (width - natural * scale) / 2) / scale, y: (y - top - (height - tall * scale) / 2) / scale }
      return { stream, at: { x: Math.round(Math.min(Math.max(at.x, 0), natural - 1)), y: Math.round(Math.min(Math.max(at.y, 0), tall - 1)) } }
    }
  }
  /** @type {{ x: number, y: number } | undefined} */
  let moved
  let turned = { x: 0, y: 0 }

  return {
    created: (/** @type {number} */ id) => all.set(id, make(id)),
    /** @param {number} id @param {string} name @param {any} value */
    set(id, name, value) {
      const stream = all.get(id)
      if (!stream) return
      if (name === "ice" && value) connect(stream, value)
      else if (name === "answer" && value && stream.peer?.signalingState === "have-local-offer") stream.peer.setRemoteDescription({ type: "answer", sdp: value }).catch(() => stream.over())
      else if (name === "watch") stream.watch = !!value
      else if (name === "autofocus" && value) active = stream
    },
    dropped(/** @type {number} */ id) {
      const stream = all.get(id)
      if (!stream) return
      stream.over()
      stream.video.remove()
      all.delete(id)
      if (active === stream) active = null
      renderer.video_gone(id)
    },
    /** @param {string} kind @param {PointerEvent} event → whether it was on a video. */
    pointer(kind, event) {
      const found = under(event.offsetX, event.offsetY)
      // Keys go to the page again once it's pressed somewhere else.
      if (!found || found.stream.watch) {
        if (kind === "down") active = null
        return false
      }
      if (kind === "move") {
        if (!moved) requestAnimationFrame(() => (send(found.stream, { t: "move", ...moved }), (moved = undefined)))
        moved = found.at
      } else if (kind === "down" || kind === "up") {
        active = found.stream
        send(found.stream, { t: "move", ...found.at })
        send(found.stream, { t: "button", b: [1, 2, 3][event.button] ?? 1, down: kind === "down", mods: mods(event) })
      }
      return true
    },
    /** @param {WheelEvent} event @param {number} unit → whether it was on a video. */
    wheel(event, unit) {
      const found = under(event.offsetX, event.offsetY)
      if (!found || found.stream.watch) return false
      turned = { x: turned.x + event.deltaX * unit, y: turned.y + event.deltaY * unit }
      send(found.stream, { t: "move", ...found.at })
      for (const [axis, less, more] of /** @type {const} */ ([["y", 4, 5], ["x", 6, 7]])) {
        while (Math.abs(turned[axis]) >= STEP) {
          const b = turned[axis] > 0 ? more : less
          send(found.stream, { t: "button", b, down: true, mods: [] })
          send(found.stream, { t: "button", b, down: false, mods: [] })
          turned[axis] -= Math.sign(turned[axis]) * STEP
        }
      }
      return true
    },
    /** @param {KeyboardEvent} event → whether it went to the video last pressed. */
    key(event) {
      if (!active || active.watch || !all.has(active.id)) return false
      // The device's full screen is the page's.
      if (event.key === "F11" || (event.ctrlKey && event.metaKey && event.key.toLowerCase() === "f")) return false
      const modified = event.ctrlKey || event.metaKey || event.altKey
      const name = KEYS[event.key] ?? (event.key.length === 1 ? event.key.toLowerCase() : undefined)
      if (name && (modified || event.key.length > 1)) send(active, { t: "key", key: name, mods: mods(event) })
      else if (event.key.length === 1) send(active, { t: "text", text: event.key })
      else return false
      return true
    },
  }
}
