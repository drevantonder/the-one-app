// `bun units.js`: writes src/units.json, how each locale says a number of each unit ECMA-402 sanctions for
// Intl.NumberFormat's `style: "unit"`, from CLDR's own JSON (unicode-org/cldr-json) at the version ICU4X's data is
// built from (49.0.0), as ICU4X formats only some of them (lengths, areas, masses, volumes and durations, not bytes,
// temperatures or percentages). Each locale keeps only what differs from its parent's, as CLDR's own data does:
// `{ "<locale>": { "<width>": { "<unit>": { "<plural category>": "{0} kB", "per": "{0}/h" }, "per": "{0}/{1}" } } }`.
import { writeFileSync } from "node:fs"

const CLDR = "https://raw.githubusercontent.com/unicode-org/cldr-json/49.0.0/cldr-json"
// ECMA-402's sanctioned single units (IsSanctionedSingleUnitIdentifier), and the compounds of them CLDR names itself.
const units = [
  "acre", "bit", "byte", "celsius", "centimeter", "day", "degree", "fahrenheit", "fluid-ounce", "foot", "gallon", "gigabit",
  "gigabyte", "gram", "hectare", "hour", "inch", "kilobit", "kilobyte", "kilogram", "kilometer", "liter", "megabit",
  "megabyte", "meter", "microsecond", "mile", "mile-scandinavian", "milliliter", "millimeter", "millisecond", "minute",
  "month", "nanosecond", "ounce", "percent", "petabyte", "pound", "second", "stone", "terabit", "terabyte", "week", "yard",
  "year",
]
const widths = ["long", "short", "narrow"]

const json = async (path) => {
  const response = await fetch(`${CLDR}/${path}`)
  if (!response.ok) throw new Error(`${path}: ${response.status}`)
  return response.json()
}
const locales = (await json("cldr-core/availableLocales.json")).availableLocales.full
const { parentLocale } = (await json("cldr-core/supplemental/parentLocales.json")).supplemental.parentLocales
// CLDR's JSON calls its root locale `und`, and its parentLocales `root`.
const parentOf = (locale) => {
  const parent = parentLocale[locale] ?? (locale.includes("-") ? locale.slice(0, locale.lastIndexOf("-")) : locale === "und" ? undefined : "und")
  return parent === "root" ? "und" : parent
}
const named = [...units, ...units.flatMap((a) => units.map((b) => `${a}-per-${b}`))]

// Each locale's patterns, whole, as CLDR's JSON resolves them.
const whole = {}
const read = async (locale) => {
  const data = (await json(`cldr-units-full/main/${locale}/units.json`)).main[locale].units
  const own = {}
  for (const width of widths) {
    own[width] = { per: data[width].per?.compoundUnitPattern }
    for (const [key, value] of Object.entries(data[width])) {
      // CLDR names a unit by its category and its id: digital-kilobyte, speed-kilometer-per-hour.
      const unit = named.find((unit) => key.endsWith(`-${unit}`) && !key.slice(0, -unit.length - 1).includes("-"))
      if (!unit) continue
      const patterns = Object.fromEntries(Object.entries(value).filter(([name]) => name.startsWith("unitPattern-count-")).map(([name, pattern]) => [name.slice(18), pattern]))
      if (value.perUnitPattern) patterns.per = value.perUnitPattern
      own[width][unit] = patterns
    }
  }
  whole[locale] = own
}
for (let at = 0; at < locales.length; at += 32) await Promise.all(locales.slice(at, at + 32).map(read))

// What each says that its parent doesn't.
const kept = {}
for (const locale of Object.keys(whole).sort()) {
  const parent = whole[parentOf(locale)]
  const differs = {}
  for (const width of widths) {
    const own = Object.fromEntries(Object.entries(whole[locale][width]).filter(([unit, value]) => JSON.stringify(value) !== JSON.stringify(parent?.[width]?.[unit])))
    if (Object.keys(own).length) differs[width] = own
  }
  if (Object.keys(differs).length) kept[locale] = differs
}
const parents = Object.fromEntries(Object.entries(parentLocale).map(([locale, parent]) => [locale, parent === "root" ? "und" : parent]))
writeFileSync(new URL("src/units.json", import.meta.url), JSON.stringify({ parents, locales: kept }))
console.log(`${Object.keys(kept).length} of ${locales.length} locales differ from their parents`)
