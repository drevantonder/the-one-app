# Plan: a new device gets in by its sign-in

Andre's decision, 2026-11-05. Built on 2026-11-05 but for what's left below: the code is `src/sign_in_proof.rs` (the checks), `kernel/auth/worker.js` (the nonce, the challenge, passing the proof on), `src/node/devices.rs` and `kernel/web/web.js` (approving by it), and Settings › Devices (joined).

Left:
- Push, once the push Worker is on main: a silent push that wakes the phone to check, "wants in" with Approve and Not Me when nothing let it in within a few seconds, and "joined" as a push.
- The iPhone and Mac apps' sign-ins naming their device (`join` in the authorization, as web, Android and the CLI's do), so they're let in by it too; today they're approved by code.
- A passkey's public key is recorded from what auth holds when a device that signed in with it is approved by its code, so a tampered auth then could slip in its own: as with a tampered auth deploy, "joined · Remove" is what shows it.

## Today

Signing in proves who you are, nothing more. A new device gets in only when one of its person's devices approves it by its code, which seals it the keys of the scopes it may open. That stays the rule. The kernel (auth) never opens or hands out a key, so a faked sign-in still opens nothing.

## The change

A sign-in that the person's own devices can check without trusting the kernel is enough for one of them to let a new device in, with no one tapping anything. Any other sign-in asks the person with one tap.

1. **Signed proof.** The new device signs in to auth as usual, and its join request names its device key. With an OIDC provider (Google, Apple, Microsoft), auth puts that key, hashed, in the request's `nonce` and passes the provider's ID token on unchanged with the join request. The token, signed with the provider's key, says who this is, for our client id, for this device. With a passkey, the assertion's challenge names the device key and the time. Email links and passwords aren't proof: only the kernel can check those.
2. **Any online client of theirs approves it.** The request goes out to all of the person's devices. The first client online (the Mac app, a browser tab, the CLI where an agent runs, the phone woken by a silent push) checks the proof and then seals the keys as an approval does today. For an ID token it checks the provider's signature against the provider's published keys, our client id, the person's account (`sub`) as the signed entries record it, freshness, and that the nonce names the device asking. For a passkey it checks the assertion against the person's passkey as the signed entries record it. Nodes don't approve: they hold no keys, nor does the serverless node.
3. **Otherwise, one tap.** When nothing approved it within a few seconds (a sign-in with no checkable proof, or none of the person's clients reachable), their phone shows "MacBook Pro wants in · Sydney" with **Approve** and **Not Me**. The code stays only as the check on that card. A client that comes online later still approves a request with a valid proof.
4. **Then, on every device:** "MacBook Pro joined · Sydney" with **Remove**, so a device let in by a sign-in the person didn't mean (a phished one) is one tap from gone.
5. **First device:** a person's first device still joins by invite, approved by the inviting admin's device.

Not a setting: this is how signing in works, everywhere, until we decide otherwise.

Considered and declined (Andre, 11-05): letting a new device in with none of the person's devices online, by a Worker that holds keys or by a secret kept with the provider (a Drive app folder). Approval stays with the person's own clients.

## What it trusts

- The provider, not the kernel: whoever controls the person's Google (or Apple, Microsoft) account, or their passkey, can add a device. Their 2-step verification matters.
- Auth runs the sign-in but can't make the proof: a bug in auth, or someone reading its database, lets no device in. A tampered auth deploy could get a person to sign in for its own device, as the browsers' code it serves could already take what a browser opens; that's the Cloudflare account's trust, and the "joined · Remove" notice is what shows it.

## Work

- Record each person's provider account (`sub`, issuer) and their passkeys' public keys in the signed entries when they join, link one or add one, so clients check against those and not against auth.
- Auth: take the device key from the join request into the OIDC `nonce` and the passkey challenge; keep the ID token or assertion and send it with the join request, unchanged.
- Clients: check the proof (provider keys fetched and cached; issuer, audience, `sub`, `iat`, nonce; passkey signature, challenge) and approve without a person; a test pins every refusal (wrong `sub`, audience, nonce, stale, bad signature, unknown passkey).
- Push: a silent push wakes the phone to check; a visible one when nothing approved it; the "joined" notice.
- Settings › Devices: an auto-approved device shows as joined; the waiting card is only for requests nothing approved.
