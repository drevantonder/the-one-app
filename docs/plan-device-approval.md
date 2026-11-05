# Plan: a new device gets in by its sign-in

Andre's decision, 2026-11-05. Built on 2026-11-05 but for what's left below: the code is `src/sign_in_proof.rs` (the checks), `cloudflare/auth/worker.js` (the nonce, the challenge, passing the proof on, telling a sign-in from a link, and the pushes), `src/node/devices.rs` and `cloudflare/web/web.js` (approving by it), and Settings › Devices (joined).

Left:
- Push: a silent push that wakes the phone to check. Auth pushes "wants in" (with the action `{ "join": "<code>" }`, which the apps show with Approve and Not Me: `apple/Shared/Notes.swift`, `android/…/Push.kt`, `windows/Notes.cs`) when nothing let a device in within 10 seconds, and "joined" once one is; a phone woken by "wants in" approves only through its normal approval path.
- Auth knows whom to push to as the devices that asked to join as that person, and browsers that say they're theirs as they look for devices waiting. Native apps that joined before 2026-11-05's pushes aren't known until they sign in again.
- A passkey's public key is recorded from what auth holds when a device that signed in with it is approved by its code, so a tampered auth then could slip in its own: as with a tampered auth deploy, "joined · Remove" is what shows it.

## Today

Signing in proves who you are, nothing more. A new device gets in only when one of its person's devices approves it by its code, which seals it the keys of the scopes it may open. That stays the rule. Sign-in (auth) never opens or hands out a key, so a faked sign-in still opens nothing.

## The change

A sign-in that the person's own devices can check without trusting sign-in is enough for one of them to let a new device in, with no one tapping anything. Any other sign-in asks the person with one tap.

1. **Signed proof.** The new device signs in to auth as usual, and its join request names its device key. With an OIDC provider (Google, Apple, Microsoft), auth puts that key, hashed, in the request's `nonce` and passes the provider's ID token on unchanged with the join request. The token, signed with the provider's key, says who this is, for our client id, for this device. With a passkey, the assertion's challenge names the device key and the time. Email links and passwords aren't proof: only sign-in can check those.
2. **Any online client of theirs approves it.** The request goes out to all of the person's devices. The first client online (the Mac app, a browser tab, the CLI where an agent runs, the phone woken by a silent push) checks the proof and then seals the keys as an approval does today. For an ID token it checks the provider's signature against the provider's published keys, our client id, the person's account (`sub`) as the signed entries record it, freshness, and that the nonce names the device asking. For a passkey it checks the assertion against the person's passkey as the signed entries record it. Nodes don't approve: they hold no keys, nor does the serverless node.
3. **Otherwise, one tap.** When nothing approved it within a few seconds (a sign-in with no checkable proof, one finished from a link opened elsewhere, or none of the person's clients reachable), their phone shows "MacBook Pro wants in · Sydney" with **Approve** and **Not Me**. The code stays only as the check on that card. A client that comes online later still approves a request with a valid proof.
4. **Then, on every device:** "MacBook Pro joined · Sydney" with **Remove**, so a device let in by a sign-in the person didn't mean (a phished one) is one tap from gone.
5. **First device:** a person's first device still joins by invite, approved by the inviting admin's device.

Not a setting: this is how signing in works, everywhere, until we decide otherwise.

Considered and declined (Andre, 11-05): letting a new device in with none of the person's devices online, by a Worker that holds keys or by a secret kept with the provider (a Drive app folder). Approval stays with the person's own clients.

## What it trusts

- The provider, not sign-in: whoever controls the person's Google (or Apple, Microsoft) account, or their passkey, can add a device. Their 2-step verification matters.
- Auth runs the sign-in but can't make the proof: a bug in auth, or someone reading its database, lets no device in by itself. What a device is checked against is only what the person's own devices recorded of their own clients (an admin's device recording a machine or someone's first device counts for nothing), and an ID token counts only for the install's own client, named by the `sign_in_clients` setting an admin's device signs as it deploys to Cloudflare, never by auth. A proof names the install (its first entry's hash) as well as the device.
- Where auth is still trusted: when a person approves a device by its code, what that device signed in with is recorded. For Google that's checked against Google's keys and the install's client, so a tampered auth can't plant its own account. For a passkey, the public key comes from auth, which keeps it: passkeys are made on auth's own page, so no client sees one made without auth's code in between. The approval card names what's recorded ("a@example.com · Google", or "Passkey"), and each device's ⋯ › Sign-ins… shows it and removes it, by an entry every device of theirs honours.
- Not Me sticks: it's an entry that takes the device's key out before it's ever listed, so none of the person's devices lets it in, by its sign-in or its code, and auth forgets its proof. A browser refuses to seal anything to a key its own replay has taken out, whatever a node says.
- Link phishing (Andre, 11-05): someone could start a sign-in on their own machine (the CLI's `sign-in`), send the person its link, and a Google pick or passkey tap would sign in their device. So a device is let in by its sign-in only when it signed in itself, in its own browser or our apps' own sign-in on it. Auth tells which by the client whose token asks to join: if its every sign-in comes back to the device it was made on (the web client's page, `<cli>://signed-in/`, loopback), it did; if it comes back to the sign-in code's page or a site, it was finished from a link, and auth passes the proof on as `{ "by_link": … }`. A device of theirs then records what it signed in with only when the person taps Approve on its card, which names the device and where it asked from; it never lets it in by itself. Under an honest auth phishing can't change where a sign-in comes back to; a tampered auth could mislabel one, which is the Cloudflare account's trust as below. The CLI always signs in by its link, so it always waits for a tap.
- A tampered auth deploy could still get a person to sign in for its own device, as the browsers' code it serves could already take what a browser opens; that's the Cloudflare account's trust, and the "joined · Remove" notice is what shows it.
- An older device (a phone or Mac behind) doesn't know Not Me's or Sign-ins' entries and goes on as before; its check fails on a sign-in naming the install, so it lets that device in only by code. It reads a `by_link` sign-in as none, so it too lets that device in only by code.

## Work

- Record each person's provider account (`sub`, issuer) and their passkeys' public keys in the signed entries when they join, link one or add one, so clients check against those and not against auth.
- Auth: take the device key from the join request into the OIDC `nonce` and the passkey challenge; keep the ID token or assertion and send it with the join request, unchanged.
- Clients: check the proof (provider keys fetched and cached; issuer, audience, `sub`, `iat`, nonce; passkey signature, challenge) and approve without a person; a test pins every refusal (wrong `sub`, audience, nonce, stale, bad signature, unknown passkey).
- Push: a silent push wakes the phone to check; a visible one when nothing approved it; the "joined" notice.
- Settings › Devices: an auto-approved device shows as joined; the waiting card is only for requests nothing approved.
