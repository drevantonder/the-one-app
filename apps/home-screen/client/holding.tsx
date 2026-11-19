import { onSettled, refresh, Show, untrack } from "solid-js"
import { call, confirm, covers, haptic, Icon, type IconName, Image, keydown, pinShortcut, Pressable, Text, toast, View, viewport } from "ui"
import { type Held } from "./machine.ts"
import { type Entrypoint, type Item, type Widget as WidgetItem, src } from "./types.ts"
import { corner } from "./layout.ts"
import { looks } from "./looks.ts"
import { home, state, pins, room, laidPage, windowWidth } from "./store.ts"
import { json, type Owner, owners, draft, subjectOf, and, addable, addToHome, removeFromHome, whose } from "./owners.ts"
import { Card, Tile } from "./tiles.tsx"

/// What a finger holds, lifted over the dimmed Home Screen, as a phone's is, with its menu beside it, which grows out of
/// it with a spring: Edit Home Screen and Remove App on a Home page; in the App Library, who the app is installed for,
/// Add to Home Screen, onto the pages of the owner Home was last on, and Remove App. Moving the finger, or on a Home
/// page holding it a while longer, carries it into editing; anywhere else, Escape, a tap beside it or the device's own
/// back puts it down.
export function Holding(props: { owner: Owner; index: number; held: Held }) {
  const dismiss = () => state().matches("holding") && home.send({ type: "dismiss" })
  const edit = () => home.send({ type: "edit", editing: draft(props.owner, props.index) })
  const removing = () => {
    const [owner, index, held] = [props.owner, props.index, props.held]
    dismiss()
    removeFromHome(owner, index, held.page, held.index, held.item!)
  }
  // The App Library holds every app that's installed: it goes from there by being uninstalled, once its alert has
  // said what goes with it, the app's other entrypoints and the apps that need it, into Recently Deleted.
  const uninstalls = async () => {
    const listed = props.held.listed!
    const subject = subjectOf(listed.installed.for)
    dismiss()
    try {
      const { apps } = await json<{ apps: { name: string; for: string; required_by: string[] }[] }>("apps", {}, subject)
      const needs = apps.find((one) => one.for === listed.installed.for && one.name === listed.installed.name)?.required_by ?? []
      // Every app of every App Library shown here, as an app installed for everyone that's about whoever opens it is
      // only in theirs.
      const all = (untrack(owners) ?? [props.owner]).flatMap((owner) => owner.library.flatMap((category) => category.entrypoints))
      const parts = [...new Set(all.filter((other) => other.app === listed.app && other.entrypoint && other.entrypoint !== listed.entrypoint).map((other) => other.name))]
      const needing = [...new Set(needs.map((name) => all.find((other) => other.installed.for === listed.installed.for && other.installed.name === name)?.title ?? name))]
      const text =
        [
          parts.length ? `${and(parts)} ${parts.length === 1 ? "is" : "are"} part of it and will be removed too.` : "",
          needing.length ? `${and(needing)} ${needing.length === 1 ? "needs it, so it" : "need it, so they"}'ll be removed too.` : "",
        ]
          .filter(Boolean)
          .join(" ") || "It stays in Recently Deleted for 30 days."
      if (!(await confirm({ heading: `Remove ${listed.title}?`, text, action: "Remove App", destructive: true }))) return
      await call("uninstall", { name: listed.installed.name, confirm: needs.length > 0 }, { subject })
    } catch (error) {
      toast(error as Error)
    }
    refresh(owners)
  }
  // Why whoever holds it can't remove it: it's a group's or everyone's, and they're not one of its owners.
  const cant = () => {
    const listed = props.held.listed
    if (!listed || listed.installed.removes) return undefined
    return `This is ${whose(listed.installed.for)}'s, and you're not one of its owners.`
  }
  const adding = () => {
    dismiss()
    addToHome(props.owner, props.index, props.held.item!)
  }
  onSettled(() => {
    haptic("medium")
    const shut = covers(dismiss)
    const keys = keydown((event) => event.key === "Escape" && dismiss())
    return () => (shut(), keys())
  })
  // Where it is in the window, which its menu is under, or over where there's no room under it.
  const held = props.held.rect ?? { x: 0, y: 0, width: 0, height: 0 }
  const scale = untrack(room).scale
  const [width, height] = [untrack(viewport).width, untrack(viewport).height]
  const below = held.y + held.height < height * 0.6
  const left = Math.max(12, Math.min(width - 252, held.x))
  const item = props.held.item
  const library = props.held.from === "library"
  // It's added from here only where it opens, onto the pages of whoever Home was last on; elsewhere, it's carried there.
  const adds = library && !!item && addable(item, props.owner)
  // An app the device's own home screen opens by its link alone: one that opens for whoever opens it, not a group.
  const app = item && !item.widget ? (item as Entrypoint) : undefined
  const pinnable = pins() && !!app && (app.subject === "user" || (!app.subject && !props.owner.subject))
  const pinning = () => {
    dismiss()
    pinShortcut(app!.entrypoint).catch((error) => toast(error as Error))
  }
  // Its copy is laid out at a phone's size, drawn as much larger as the pages are, from its middle.
  const [copyWidth, copyHeight] = [held.width / scale, held.height / scale]
  const tile = untrack(laidPage).tile
  return (
    <View style={[looks.dialog, looks.heldLayer, looks.sized(windowWidth(), viewport().height)]}>
      {/* The Home Screen blurs through a veil under what's held, and its menu floats in glass, as a window does. */}
      <Pressable aria-hidden="true" onPress={dismiss} style={[looks.cover, looks.frosted, looks.fadeIn]} />
      <Show when={item ?? (props.held.listed as Item | undefined)}>
        {(item) => (
          <View aria-hidden="true" style={[looks.copy, looks.copyAt(held.x + (held.width - copyWidth) / 2, held.y + (held.height - copyHeight) / 2, copyWidth, copyHeight, scale)]}>
            <View style={looks.lifting}>
              <Show when={item().widget} fallback={library ? <Image src={src((item() as Entrypoint).icon)} alt="" style={[looks.iconImage(corner(copyWidth)), looks.square(copyWidth)]} /> : <Tile name={(item() as Entrypoint).name} icon={(item() as Entrypoint).icon} size={tile} scale={scale} />}>
                <Card item={item() as WidgetItem} owner={props.owner} still />
              </Show>
            </View>
          </View>
        )}
      </Show>
      <View
        role="menu"
        aria-label={(item ?? props.held.listed)?.name}
        style={[
          looks.menu,
          looks.popup(left, 240), looks.opens,
          below ? looks.top(held.y + held.height + 12) : looks.bottom(height - held.y + 12),
        ]}
      >
        <Show when={library && props.held.listed}>
          {(listed) => (
            <View style={looks.installedFor}>
              <Text style={looks.note}>Installed for {whose(listed().installed.for)}</Text>
            </View>
          )}
        </Show>
        <Show when={!library}>
          <HoldItem label="Edit Home Screen" icon="squares-four" onPress={edit} />
        </Show>
        <Show when={adds}>
          <HoldItem label={props.index ? `Add to ${props.owner.name}'s Home Screen` : "Add to Home Screen"} icon="plus-circle" onPress={adding} />
        </Show>
        <Show when={pinnable}>
          <HoldItem label="Add to Android Home Screen" icon="device-mobile" onPress={pinning} />
        </Show>
        <Show when={item || props.held.listed}>
          <HoldItem label={item?.widget ? "Remove Widget" : "Remove App"} why={cant()} icon="minus-circle" destructive={!cant()} onPress={library ? uninstalls : removing} />
        </Show>
      </View>
    </View>
  )
}

/// One of what can be done with what's held: its words, why it can't be done if it can't, greyed, and its icon.
function HoldItem(props: { label: string; why?: string; icon: IconName; destructive?: boolean; onPress: () => void }) {
  return (
    <Pressable
      role="menuitem"
      aria-disabled={props.why ? "true" : undefined}
      aria-description={props.why}
      onPress={() => !props.why && props.onPress()}
      style={[looks.holdItem, props.why ? looks.holdWhy : props.destructive && looks.destructive]}
    >
      <View style={looks.shrinks}>
        <Text>{props.label}</Text>
        <Show when={props.why}>{(why) => <Text style={looks.note}>{why()}</Text>}</Show>
      </View>
      <Icon name={props.icon} />
    </Pressable>
  )
}
