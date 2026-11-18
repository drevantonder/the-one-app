// What Solid's scheduler needs that Lynx's background runtime doesn't have.
globalThis.queueMicrotask ??= (fn) => Promise.resolve().then(fn)
