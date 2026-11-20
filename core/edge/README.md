# Putting a network's edge on Cloudflare

Make an R2 bucket and an S3 API token for it, and write the edge's secrets to `secrets`, a line each:
`BUCKET`, the bucket's S3 address with its name on the end (`https://<account>.r2.cloudflarestorage.com/<bucket>`),
`ACCESS_KEY_ID`, `SECRET_ACCESS_KEY`, and `FIRST_SERVER`, which you make up (`openssl rand -hex 16`). The network's
address is a name on a zone of that Cloudflare account, which deploying attaches to the edge. From the repo's root:

    cd core/edge && bun install && bun run deploy https://<the network's address>

Then, on the machine that's to be its first server, install `one` from the core's latest release, which lists each
kind of machine's builds (`linux-x86_64`, `macos-aarch64`, `windows-x86_64`), and make it the network's first server:

    curl -fsSL https://github.com/ptc-au/the-one-app/releases/download/builds/linux-x86_64.json |
      jq -r '."one-cli".link.url' | xargs curl -fsSL | gunzip >~/.local/bin/one && chmod +x ~/.local/bin/one
    one serve on --address https://<the network's address> --first <FIRST_SERVER> --owner <name>

It's a server as every other is, run by the system (`one serve run`) and updated by the network, on the core's latest
build, which `tooling/release` keeps there. It shows `FIRST_SERVER` as it first starts, and nobody without it can
become the first; it opens nothing after. It deploys the core's apps for the network from git, at its build's
commit, and makes `--owner` its first owner, saying in its log where they join: a link they open to make their
passkey. Its device signs in as any does once they have.

The Worker is TypeScript, which wrangler builds without checking. `bun run typecheck` checks it; `bun run types`
makes its `worker-configuration.d.ts` again after `wrangler.jsonc` changes.
