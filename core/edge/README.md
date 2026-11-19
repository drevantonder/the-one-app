# Putting a network's edge on Cloudflare

Make an R2 bucket and an S3 API token for it, and write the edge's secrets to `secrets`, a line each:
`BUCKET`, the bucket's S3 address with its name on the end (`https://<account>.r2.cloudflarestorage.com/<bucket>`),
`ACCESS_KEY_ID`, `SECRET_ACCESS_KEY`, and `FIRST_SERVER`, which you make up (`openssl rand -hex 16`). The network's
address is a name on a zone of that Cloudflare account, which deploying attaches to the edge. Then, from the repo's
root:

    cd core/edge && bun install && bun run deploy https://<the network's address>
    cd ../server && mix network /tmp/one-network --address https://<the network's address> --first <FIRST_SERVER> --owner <name>

The network's first server shows `FIRST_SERVER` that once, and nobody without it can become the first. It opens
nothing after. `--owner` names the network's first owner, and the server says where they join: a link they open to
make their passkey.

The Worker is TypeScript, which wrangler builds without checking. `bun run typecheck` checks it; `bun run types`
makes its `worker-configuration.d.ts` again after `wrangler.jsonc` changes.
