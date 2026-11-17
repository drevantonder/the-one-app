Skills from Lauren Tan's pstack, https://github.com/cursor/plugins/tree/main/pstack/skills at commit
e1cc03121a0e71129a4fb730d7c5528a0a3e43c8, as they are, under its LICENSE. `build.js` serves them beneath `ui`, each with that LICENSE.

To take upstream's latest, run this in `core/ui/`, then put its commit above:

    curl -sL https://codeload.github.com/cursor/plugins/tar.gz/main | tar xz -C vendor/pstack --strip-components=3 plugins-main/pstack/skills/{typescript-best-practices,principle-prove-it-works,principle-fix-root-causes,principle-model-the-domain,principle-make-operations-idempotent,principle-experience-first}
