Modern Web Guidance, from https://github.com/GoogleChrome/modern-web-guidance at commit
64a6d708f5a43f2cd3fde5d4d7db3905ba1bdf65, as it is, under its LICENSE (Apache-2.0). Our own agents read it here, and
`core/ui/build.js` serves it beneath `ui` to the agents that build a network's apps.

To take upstream's latest, run this at the repo's root, then put its commit above:

    curl -sL https://codeload.github.com/GoogleChrome/modern-web-guidance/tar.gz/main | tar xz -C .agents/skills --strip-components=2 modern-web-guidance-main/skills/modern-web-guidance
