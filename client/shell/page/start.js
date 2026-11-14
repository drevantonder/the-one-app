// The screen a device starts on while it isn't in its network: one button, which opens the network's sign-in page
// in the device's own browser, where its person's passkey is. When they come back from there, it lets itself in.
const { invoke } = window.__TAURI__.core

const problem = document.getElementById("problem")
const said = (error) => Object.assign(problem, { textContent: `${error}`, hidden: false })

document.getElementById("sign-in").addEventListener("click", () => invoke("sign_in").catch(said))

async function enter() {
  if (await invoke("enter").catch(said)) location.replace("index.html")
}
addEventListener("focus", enter)
document.addEventListener("visibilitychange", () => document.hidden || enter())
