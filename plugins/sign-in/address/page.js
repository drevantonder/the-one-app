// The sign-in page. It asks one thing, the passkey, and what that lets happen depends on how the page was
// opened: with an invitation, someone joins; with a device's key, that device becomes theirs too; from an
// assistant's request to be let in, the assistant does. Whichever it was, this browser is signed in after, and is
// one of that person's devices.
import { createAuthClient } from "better-auth/client";
import { passkeyClient } from "@better-auth/passkey/client";

const auth = createAuthClient({ basePath: "/sign-in/auth", plugins: [passkeyClient()] });
const opened = new URLSearchParams(location.hash.slice(1));
/** What Better Auth carries an assistant's request in while a person answers it. It's signed, so it goes back as it came. */
const request = new URLSearchParams(location.search).has("client_id") ? location.search.slice(1) : null;
const invite = opened.get("invite");
const device = opened.get("device");

const part = (id) => document.getElementById(id);
const say = (id, text) => Object.assign(part(id), { textContent: text ?? "", hidden: !text });

/** What this page asks Better Auth besides the passkey. */
async function ask(path, body) {
  const answer = await fetch(`/sign-in/auth${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  const said = await answer.json();
  if (!answer.ok) throw new Error(said.message ?? "That didn't work. Try again.");
  return said;
}

/** The passkey question. Someone joining makes theirs, and is signed in with it. Whoever was signed in here no longer is. */
async function passkey() {
  await auth.signOut();
  const { error } = invite ? await auth.passkey.addPasskey({ context: invite, createSession: true }) : await auth.signIn.passkey();
  if (error) throw new Error(error.message ?? "That passkey didn't work. Try again.");
}

/** Who will act as whoever signs in: the assistant that asked, or the device this was opened for. */
let acts;
try {
  if (request) acts = (await ask("/oauth2/public-client-prelogin", { client_id: new URLSearchParams(request).get("client_id"), oauth_query: request })).client_name;
  else if (device) acts = opened.get("name") || "A new device";
} catch (error) {
  say("problem", error.message);
}
say("title", invite ? "Join" : "Sign in");
say("acts", acts && `${acts} will act as you.`);
say("sign-in", invite ? "Create a passkey" : "Sign in");

part("sign-in").addEventListener("click", async () => {
  say("problem");
  try {
    await passkey();
    let done = "You're signed in.";
    if (request) {
      // Signing in was saying yes, so the assistant is sent back with what it came for.
      const { url } = await ask("/oauth2/consent", { accept: true, oauth_query: request });
      return location.replace(url);
    }
    if (device) {
      await ask("/device/add", { device, name: acts });
      done = `${acts} is yours now. Go back to it.`;
    }
    say("sign-in");
    say("acts");
    say("done", done);
  } catch (error) {
    say("problem", error.message);
  }
});
