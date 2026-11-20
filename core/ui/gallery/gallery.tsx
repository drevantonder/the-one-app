// Every component of the UI library in each of its states, as scenes (`<family>.scenes.tsx`): one page that
// draws them all, under their families, or the one its link names (`#Buttons/Variants`), alone, as the scenes'
// runner (core/ui/scenes.js) draws each to compare the DOM's with a native host's; `#scenes` lists them for it.
// Nothing here calls a tool.
import { For, Show } from "solid-js"
import { link, render, type Scene, type Scenes, ScrollView, Section, styles, Text, tokens, View } from "ui"
import actions from "./actions.scenes.tsx"
import buttons from "./buttons.scenes.tsx"
import fields from "./fields.scenes.tsx"
import images from "./images.scenes.tsx"
import lists from "./lists.scenes.tsx"
import live from "./live.scenes.tsx"
import motion from "./motion.scenes.tsx"
import over from "./over.scenes.tsx"
import screens from "./screens.scenes.tsx"
import states from "./states.scenes.tsx"
import styling from "./styling.scenes.tsx"
import tabs from "./tabs.scenes.tsx"
import text from "./text.scenes.tsx"
import widgets from "./widgets.scenes.tsx"

const look = styles({
  page: { padding: tokens.grid[4], gap: tokens.grid[6] },
  title: { font: tokens.type.title },
  scene: { gap: tokens.grid[2] },
  name: { font: tokens.type.caption, color: tokens.colour.secondaryLabel },
  alone: { padding: tokens.grid[4], gap: tokens.grid[4] },
})

const scenes: Scenes = { ...text, ...images, ...buttons, ...lists, ...fields, ...states, ...tabs, ...widgets, ...over, ...actions, ...live, ...motion, ...styling, ...screens }
const draw = (scene: Scene) => (typeof scene === "function" ? scene() : scene.render())
const named = () => {
  const to = link()
  return to && decodeURIComponent(to)
}
const families = [...new Set(Object.keys(scenes).map((name) => name.split("/")[0]))]

/** Every scene by name, with what's pressed first and the gestures played on it, which the scenes' runners read
 * (`#scenes`). */
const listed = () =>
  JSON.stringify(Object.entries(scenes).map(([name, scene]) => (typeof scene === "function" ? { name, taps: [] } : { name, taps: scene.taps ?? [], size: scene.size, touch: scene.touch, motions: scene.motions })))

render(() => (
  <Show when={named() !== "scenes"} fallback={<Text>{listed()}</Text>}>
  <Show
    when={named() ? scenes[named()!] : undefined}
    fallback={
      <ScrollView style={look.page}>
        <Text role="heading" aria-level={1} style={look.title}>
          Gallery
        </Text>
        <For each={families}>
          {(family) => (
            <Section heading={family}>
              <For each={Object.keys(scenes).filter((name) => name.startsWith(`${family}/`) && typeof scenes[name] === "function")}>
                {(name) => (
                  <View style={look.scene}>
                    <Text style={look.name}>{name.slice(family.length + 1)}</Text>
                    {draw(scenes[name])}
                  </View>
                )}
              </For>
            </Section>
          )}
        </For>
      </ScrollView>
    }
  >
    {(scene) => <View style={look.alone}>{draw(scene())}</View>}
  </Show>
  </Show>
))
