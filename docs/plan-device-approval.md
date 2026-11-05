# Plan: a new device gets in by its sign-in

Andre's decision, 2026-11-05. Designed, not built.

## Today

Signing in proves who you are, nothing more. A new device gets in only when one of its person's devices approves it by its code, which seals it the keys of the scopes it may open. That stays the rule. The kernel (auth) never opens or hands out a key, so a faked sign-in still opens nothing.

## The change

A sign-in that the person's own devices can check without trusting the kernel is enough to let a new device in. Any other sign-in asks the person with one tap.

1. **Signed proof.** The new device signs in with an OIDC provider (Google, Apple, Microsoft), starting the sign-in itself and putting its own device key, hashed, in the request's `nonce`. The provider's ID token, signed with the provider's key, then says who this is, for our client id, for this device. A passkey assertion whose challenge is the device key is the same kind of proof. Email links and passwords aren't: only the kernel can check those.
2. **Any online client of theirs approves it.** The request goes out to all of the person's devices. The first client online (the Mac app, a browser tab, the CLI where an agent runs, the phone woken by a silent push) checks the proof: the provider's signature against its published keys, our client id, the person's account (`sub`) as the signed entries record it, freshness, and that the nonce names the device asking. Then it seals the keys as an approval does today. Nodes don't: they hold no keys, nor does the serverless node.
3. **Otherwise, one tap.** When nothing approved it within a few seconds (a sign-in with no checkable proof, or none of the person's clients reachable), their phone shows "MacBook Pro wants in · Sydney" with **Approve** and **Not Me**. The code stays only as the check on that card. A client that comes online later still approves a request with a valid proof.
4. **First device:** a person's first device still joins by invite, approved by the inviting admin's device.

Not a setting: this is how signing in works, everywhere, until we decide otherwise.

## What it trusts

- The provider, not the kernel: whoever controls the person's Google (or Apple, Microsoft) account can add a device. Their 2-step verification matters.
- The new device chooses the nonce. A sign-in the kernel starts (the browsers', whose code the kernel serves) is no weaker than today, since those Workers could already take what a browser opens. Native clients talk to the provider directly for this proof, beside their usual sign-in to auth.

## Work

- Record each person's provider account (`sub`, issuer) in the signed entries when they join or link it, so clients check against that and not against auth.
- Native clients: sign in to the provider directly with the nonce; send the ID token with the join request. Auth passes it on unchanged.
- Clients: check the proof (provider keys fetched and cached; issuer, audience, `sub`, `iat`, nonce) and approve without a person; a test pins every refusal (wrong `sub`, audience, nonce, stale, bad signature).
- Push: a silent push wakes the phone to check; a visible one when nothing approved it.
- Settings › Devices: an auto-approved device shows as joined; the waiting card is only for requests nothing approved.
