---
name: repos
description: Read before making, cloning, pushing to or reading a git repo in the network, or suggesting a change to a repo you can't push to.
---

Repos is git hosting in the network. A repo is a person's, a group's or the network's, and is named as any artifact
is: `users/alice/made/repos/notes`. Its address for git is `one::` and its name, the same on every device that has
`one`:

    git clone one::users/alice/made/repos/notes

Git signs in as the device does, with no password. `one` puts `git-remote-one` beside itself on the PATH the first
time it runs, which is what git runs for `one::` addresses; where it couldn't, link it there: `ln -s one
git-remote-one`, or on Windows `mklink /H git-remote-one.exe one.exe`.

- `make --name notes` makes an empty repo, yours, or a group's with `ONE_FOR=groups/<name>`. Then push to it from a
  repo you have: `git remote add origin one::users/alice/made/repos/notes` and `git push -u origin main`.
- `list` lists every repo you may read. `branches`, `tree`, `file`, `log` and `commit` read one without cloning it:
  a folder or a file at a ref, the history, one commit's diff.
- Who reads and who pushes is sharing, as for anything: `relations share --artifact <repo> --relation viewer` (reads
  and clones, and suggests changes) or `editor` (pushes, and takes suggestions). A group's members push to its
  repos. Share only when asked.
- A push moves a branch only from where you fetched it. If someone pushed first, git says `fetch first`: fetch,
  rebase or merge, and push again. Nothing pushed is lost.

To change a repo you may read but not push to, suggest the change: commit it on a branch of yours, push it to
`refs/suggestions/<name>`, a short name of your own for it, and tell its owner. The commit's subject is its title.

    git push origin HEAD:refs/suggestions/fix-typo

Pushing there again updates it. `suggestions` lists a repo's suggestions with their commits. Whoever pushes to the
repo `take`s one, which fast-forwards the branch it's for or merges it, or `decline`s it; whoever suggested it can
decline it too. A suggestion that doesn't merge cleanly says which files conflict: fetch the branch it's for, merge it
into yours, and push the suggestion again. `git fetch origin refs/suggestions/fix-typo` fetches one to look at.

`remove` puts a repo you own in Recently Deleted, gone for everyone until `restore` brings it back as it was. After 30
days it's erased for good. The tool `git` is what `git-remote-one` calls; don't call it yourself.
