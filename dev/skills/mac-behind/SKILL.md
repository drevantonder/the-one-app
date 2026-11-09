---
name: mac-behind
description: A Mac that isn't on the build it should be.
---

# A Mac that's behind

Its job (`client/native/mac/pull.sh`) takes what the record says, or builds it. It's offline, or it's still building, or it failed.

1. `one-dev status --until <commit> --device <mac>` says which, and waits.
2. `one logs <mac>`, then `~/Library/Caches/one/why` and `update.log` there.
3. Too old to have the job: `one-dev mac-job | ssh <mac> sh`, once.
