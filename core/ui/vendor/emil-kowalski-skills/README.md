Emil Kowalski's skills, from https://github.com/emilkowalski/skills at commit
0f2e23b32bb1265e84cff8d3e75785fec953fb1b, as they are, under their LICENSE. `build.js` serves them beneath `ui`, each
with that LICENSE, and adds a line to those that name React's APIs.

To take upstream's latest, run this in `core/ui/`, then put its commit above:

    curl -sL https://codeload.github.com/emilkowalski/skills/tar.gz/main | tar xz -C vendor/emil-kowalski-skills --strip-components=2 skills-main/skills/{gestures,apple-design,review-animations,break-ui,mobile-native}
