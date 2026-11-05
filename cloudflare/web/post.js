// post.html's form: the fields in the address's fragment, sent to its `to` once the person says Continue.
const q = new URLSearchParams(location.hash.slice(1));
const form = document.getElementById("form");
let to = null;
try {
  to = new URL(q.get("to") || "");
} catch {}
if (to?.protocol !== "https:" && to?.hostname !== "127.0.0.1") {
  document.getElementById("title").textContent = "Nothing to send";
  document.getElementById("where").textContent = "This link is missing where it goes. Go back and try again.";
} else {
  form.action = to.href;
  for (const [name, value] of q) {
    if (name === "to") continue;
    form.append(Object.assign(document.createElement("input"), { type: "hidden", name, value }));
  }
  document.getElementById("title").textContent = `Continue to ${to.hostname}`;
  document.getElementById("where").textContent = `You'll finish there, then come back here.`;
  document.getElementById("go").hidden = false;
}
