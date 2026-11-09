type Arrangement = { items?: { app: string; widget?: string; id?: string }[]; hidden?: string[] }

/// What a page shows, in its user's order: each thing they put there, an app of `apps` as `{ app }` or a widget as
/// `widget` makes it (or leaves it out, with null), then the apps added since they last arranged it, without those they
/// took off it (which may keep a place among the items, to come back to). A page never arranged shows its apps.
export function arranged<A extends { name: string }, W>(apps: A[], arrangement: Arrangement | undefined, widget: (item: NonNullable<Arrangement["items"]>[number]) => W | null = () => null): ({ app: A } | W)[] {
  if (!arrangement) return apps.map((app) => ({ app }))
  const out: ({ app: A } | W)[] = [], seen = new Set<string>(), hidden = new Set(arrangement.hidden || [])
  for (const item of arrangement.items || []) {
    if (item.widget) {
      const made = widget(item)
      if (made) out.push(made)
      continue
    }
    const app = apps.find((a) => a.name === item.app)
    if (app && !seen.has(app.name)) {
      seen.add(app.name)
      if (!hidden.has(app.name)) out.push({ app })
    }
  }
  return out.concat(apps.filter((a) => !seen.has(a.name) && !hidden.has(a.name)).map((app) => ({ app })))
}
