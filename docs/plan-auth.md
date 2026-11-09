# Who gets in, and who can open what

Andre's decisions, 2026-11-09, from T3 thread 8c27e598. `ARCHITECTURE.md`'s Trust section changes with step 2.

**The one rule: signing in only lets you ask. Someone already in the network has to say yes, either at the time or in advance with an invite.**

Every network works the same way. A network's settings say which providers it has, its domain if any, and who holds the recovery key. There's no second code path for PTC, and this repo never names PTC.

## The model

**Identity.** A user has any number of linked identities: an OIDC provider's account (Google, Microsoft, any other, by issuer and subject) or a passkey. The signed log records which are theirs. A further one is linked from a device already signed in. Passkeys need no setup. A provider's button appears only where the network has registered with that provider. A passkey always belongs to an account, so there are no passkey-only users and no placeholder emails, and an admin signs in like anyone else. Better Auth stays for sign-in, with far fewer of our rules around it.

**Getting in.**

| Who | What happens |
|---|---|
| Invited | An admin names their email. They sign in with it and go straight in: the invite is the approval, given in advance |
| Not invited, at the network's domain | They ask. An admin approves, with a short code showing on both screens |
| Not invited, anyone else | Refused |
| A user's next device | Sign in, then approve from one of their own devices, with the same short code |
| Lost every device | Sign in, then a holder of the recovery key approves and restores them; or the recovery key itself |
| A computer with no browser, or no working sign-in | QR, or a one-time link or code, from a device already in. Single use, short-lived |

A sign-in never lets anyone in by itself, for any network or user, so the "no tap" path goes. Being at the domain only decides who may ask. It's checked in the provider's signed token, not by the `hd` hint. Requests go out as ordinary pushes (a sleeping phone must still alert), and stay until answered or expired. Every join shows "joined · Remove" on the user's other devices. The approving device checks the sign-in's proof itself (`src/onboarding/sign_in_proof.rs`), so a hacked auth Worker can make requests and never approve one. The link-phishing guard stays (ARCHITECTURE.md's Trust): the short code doesn't cover it.

An invited user's device is listed with no tap, by the provider's signed token as proof against the invite in the log. Its device holds no key to write that step, so an admin's device that's online writes it for it. Every device checks that proof. Providers change their signing keys, so the key that was valid is saved with the proof, or old joins couldn't be checked. Shared data reaches them once a device holding those keys is online, with no tap.

**Keys.** Each scope has random keys, sealed to who may open them and to the network's recovery key, as today. A device holds its user's keys, so a new device needs one handover. Keys are never derived from anything a rename can change: not from an app's, database's or scope's name.

The recovery key's holders, named by an admin, can read everything and restore anyone, and we say so plainly when someone joins: "The owner of this network can see everything in it." ("root" is already the network's first step in the code: don't use it for this.) Nodes still get only the keys one call needs. New keys are made only when someone is removed or a device is stolen.

**Losing access.** Another of your devices, then a holder of the recovery key, then the recovery key on paper. No devices, no holder and no recovery key means access is gone.

**What stays where it is.** Cloudflare; the relay, bucket and URL signer, push, update files, browser client and remote MCP.

**Not doing, unless Andre asks.** KMS or AWS, a keeper, moving off Cloudflare, one key for the whole network, app attestation. Passkey PRF that skips the approval is a later shortcut.

**Stored data is migrated, never lost.** Andre's own network is real. Anything already sealed must still open, on every build from here on.

## Order of work

1. **Protect data.** A format version on everything sealed, with what's already written still opening. A test holding data sealed by an earlier build, which every later build must open. Permanent IDs instead of changeable names in keys and seals.
2. **This page, and `ARCHITECTURE.md` and `GLOSSARY.md` to match.**
3. **Identities.** Linked logins, passkeys that belong to an account, domain from the signed token.
4. **One ask-and-approve flow.** Push, short code, invites and the domain rule in the log, QR and links as the fallback.
5. **User keys and sealing to the recovery key's holders.**
6. **Recovery.** Holders restoring someone, and the recovery key admitting a device.
7. **Delete the old rules:** `passkey_only`, placeholder emails, today's request workflow and code pages, and any start and freshness rule the short code makes needless.
8. **Tighten Cloudflare.** A deploy token that may only edit Workers, kept in 1Password; audit logs sent elsewhere.
9. **Test passkey PRF on real devices,** only if Andre still wants that shortcut.

Steps 1 to 7 land before PTC's staff are onboarded.
