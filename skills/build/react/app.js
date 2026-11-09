// The smallest app with a React screen: copy this folder, rename it, and change both files. Deploying it builds
// screens/main.jsx into dist/main.html, which this imports as text.
import main from "./dist/main.html" with { type: "text" };

export default {
  name: "hello",
  title: "Hello",
  icon: "hand-waving",
  description: "Says hello",
  tools: {
    hello: {
      title: "Say hello",
      reads: true,
      description: "Says hello to whoever asks. Gives { text }.",
      input: { type: "object", properties: {} },
      output: { type: "object", required: ["text"], properties: { text: { type: "string" } } },
      run: (ctx) => ({ text: `Hello, ${ctx.me.user_name}` }),
    },
  },
  screens: {
    main: { title: "Hello", html: main },
  },
};
