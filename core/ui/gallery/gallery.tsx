// Every component of the UI library in each of its states, as scenes (`<family>.scenes.tsx`): one strict page that
// draws them all, under their families, or the one its link names (`#Buttons/Variants`), alone, as the scenes'
// runner (core/ui/scenes.js) draws each to compare the DOM's with a native host's; `#scenes` lists them for it.
// Nothing here calls a tool.
import { For, Show } from "solid-js"
import { Box, link, render, Scroll, Section, Text, type Scene, type Scenes } from "ui"
import actions from "./actions.scenes.tsx"
import buttons from "./buttons.scenes.tsx"
import fields from "./fields.scenes.tsx"
import images from "./images.scenes.tsx"
import lists from "./lists.scenes.tsx"
import live from "./live.scenes.tsx"
import over from "./over.scenes.tsx"
import states from "./states.scenes.tsx"
import styling from "./styling.scenes.tsx"
import tabs from "./tabs.scenes.tsx"
import text from "./text.scenes.tsx"
import widgets from "./widgets.scenes.tsx"

const scenes: Scenes = { ...text, ...images, ...buttons, ...lists, ...fields, ...states, ...tabs, ...widgets, ...over, ...actions, ...live, ...styling }
const draw = (scene: Scene) => (typeof scene === "function" ? scene() : scene.render())
const named = () => {
  const to = link()
  return to && decodeURIComponent(to)
}
const families = [...new Set(Object.keys(scenes).map((name) => name.split("/")[0]))]

/** Every scene by name, with what's pressed first, which the scenes' runner reads (`#scenes`). */
const listed = () => JSON.stringify(Object.entries(scenes).map(([name, scene]) => (typeof scene === "function" ? { name, taps: [] } : { name, taps: scene.taps ?? [], size: scene.size, touch: scene.touch })))

render(() => (
  <Show when={named() !== "scenes"} fallback={<Text>{listed()}</Text>}>
  <Show
    when={named() ? scenes[named()!] : undefined}
    fallback={
      <Scroll style={{ padding: 16, gap: 24 }}>
        <Text role="heading" aria-level={1} style={{ font: "title" }}>
          Gallery
        </Text>
        <For each={families}>
          {(family) => (
            <Section heading={family}>
              <For each={Object.keys(scenes).filter((name) => name.startsWith(`${family}/`) && typeof scenes[name] === "function")}>
                {(name) => (
                  <Box style={{ gap: 8 }}>
                    <Text style={{ font: "footnote", color: "muted-foreground" }}>{name.slice(family.length + 1)}</Text>
                    {draw(scenes[name])}
                  </Box>
                )}
              </For>
            </Section>
          )}
        </For>
      </Scroll>
    }
  >
    {(scene) => <Box style={{ padding: 16, gap: 16 }}>{draw(scene())}</Box>}
  </Show>
  </Show>
))
