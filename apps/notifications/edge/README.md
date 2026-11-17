# Notifications' edge function

This Worker wakes phones and browsers when someone is notified. It holds the network's keys to the push services as
its secrets, and nothing else holds them. Whoever deploys the edge writes them to `secrets` here, which git
ignores, one `NAME=value` a line, with a value that spans lines in double quotes. A service with no keys fails
each send to it, and the app's `send` says so.

- APNs, for iPhones: `APNS_KEY`, the team's `.p8` key as its text; `APNS_KEY_ID`, its id; `APNS_TEAM_ID`, the
  team's; `APNS_TOPIC`, the app's bundle id (`app.theone.client`, or a network's own); and `APNS_SANDBOX=1` for an
  app built for development.
- FCM, for Android: `FCM_ACCOUNT`, the Firebase service account's JSON key, with the Firebase Cloud Messaging API
  on for its project.
- Web Push, for browsers: `VAPID_PUBLIC_KEY` and `VAPID_PRIVATE_KEY`, a P-256 key pair as base64url (the public
  key's 65 bytes, the private key's 32), and `VAPID_SUBJECT`, a `mailto:` or `https:` address the push services
  can reach whoever runs the network at. `bunx web-push generate-vapid-keys` makes a pair.

It is TypeScript, which wrangler builds without checking; `bun run typecheck` here checks it.
