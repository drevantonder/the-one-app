// moves.ts's paces as CSS variables, for Tailwind's build: `--move-settle` and `--ease-settle`, and so on.
import plugin from "tailwindcss/plugin"
import { moves } from "./moves"

export default plugin(({ addBase }) =>
  addBase({
    ":root": Object.fromEntries(Object.entries(moves).flatMap(([name, { duration, ease }]) => [
      [`--move-${name}`, `${Math.round(duration * 1000)}ms`],
      [`--ease-${name}`, `cubic-bezier(${ease.join(", ")})`],
    ])),
  }),
)
