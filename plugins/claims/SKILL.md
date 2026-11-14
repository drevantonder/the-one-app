---
name: claims
description: Read before recording an expense or its receipt, or gathering, sending, approving, sending back or paying an expense claim.
---

Claims is a workplace's expense claims. Someone records each expense as it happens (`record`, then `attach`
for each receipt, whose file is an upload), gathers expenses into a claim (`gather`) and sends it (`send`). An approver lists what's
waiting (`waiting`), opens one (`show`), looks at each receipt (`receipt`), and approves it (`approve`) or sends
it back with the reason (`send_back`). Once the money has been paid, `pay` says so.

- A claim is `draft`, `sent`, `sent_back`, `approved` or `paid`. Its expenses change only while it's `draft` or
  `sent_back`: put them right, then `send` it again.
- An amount is a number in AUD: 12.5 is $12.50. A date is written 2026-11-14.
- Sending a claim, approving one, sending one back and saying one is paid are a person's decisions. Do them
  when asked to, and say what you did.
- An approver is an editor of Claims' own artifact, which whoever owns it says with `relations share`.
  Being refused names the artifact. Everyone else sees only their own expenses and claims.
