// Intl, as ECMA-402 says, run after prelude.js: this resolves each constructor's locales and options as ECMA-402 says,
// and ICU4X formats what they resolve to (`__intl`, intl.rs). Its default locale is the host's (`navigator.language`),
// and its time zone the host's. Date's, Number's, BigInt's, Array's and String's locale methods are its own, as
// ECMA-402 has them. Not here, as ICU4X has nothing to make them with: DisplayNames, DurationFormat, supportedValuesOf,
// formatRange and selectRange.
;(() => {
  const native = globalThis.__intl
  delete globalThis.__intl

  const options = (given) => (given === undefined ? Object.create(null) : Object(given))
  const option = (given, name, values, fallback) => {
    let value = given[name]
    if (value === undefined) return fallback
    value = typeof fallback === "boolean" ? Boolean(value) : String(value)
    if (values && !values.includes(value)) throw new RangeError(`Value ${value} out of range for ${name} options property`)
    return value
  }
  const integer = (given, name, min, max, fallback) => {
    let value = given[name]
    if (value === undefined) return fallback
    value = Number(value)
    if (Number.isNaN(value) || value < min || value > max) throw new RangeError(`${name} value is out of range.`)
    return Math.floor(value)
  }

  // Locales (CanonicalizeLocaleList, ResolveLocale): a locale is one ICU4X has data for, which it knows the likely
  // script and region of.
  const known = (tag) => {
    const base = tag.split("-u-")[0]
    return base.split("-").length >= 3 || native.maximize(base) !== base
  }
  const canonical = (locales) => {
    if (locales === undefined) return []
    const list = typeof locales === "string" || locales instanceof Locale ? [locales] : [...Object(locales)]
    const tags = []
    for (const tag of list) {
      if (typeof tag !== "string" && (typeof tag !== "object" || tag === null)) throw new TypeError("Language ID should be string or object.")
      const each = native.canonicalize(String(tag))
      if (!tags.includes(each)) tags.push(each)
    }
    return tags
  }
  const host = (() => {
    try {
      return native.canonicalize(navigator.language)
    } catch {
      return "en-US"
    }
  })()
  const unicode = (tag) => {
    const keys = {}
    const extension = tag.split("-u-")[1]?.split(/-[a-z0-9]-/)[0]
    if (!extension) return keys
    let key
    for (const subtag of extension.split("-")) {
      if (subtag.length === 2) keys[(key = subtag)] = "true"
      else if (key) keys[key] = keys[key] === "true" ? subtag : `${keys[key]}-${subtag}`
    }
    return keys
  }
  // The locale a constructor formats in, with the Unicode extension keys it takes (`relevant`), its options' first.
  const resolve = (locales, relevant = [], overrides = {}) => {
    const requested = canonical(locales).find(known) ?? host
    const keys = unicode(requested)
    const base = requested.split("-u-")[0]
    const resolved = {}
    for (const key of relevant) {
      const value = overrides[key] ?? keys[key]
      if (value !== undefined) resolved[key] = value
    }
    const extension = Object.entries(resolved).map(([key, value]) => (value === "true" ? key : `${key}-${value}`)).join("-")
    return { locale: extension ? `${base}-u-${extension}` : base, keys: resolved }
  }
  const supportedLocalesOf = (locales, given) => {
    option(options(given), "localeMatcher", ["lookup", "best fit"], "best fit")
    return canonical(locales).filter(known)
  }
  const named = (Class, tag) => {
    Object.defineProperty(Class.prototype, Symbol.toStringTag, { value: tag, configurable: true })
    Object.defineProperty(Class, "supportedLocalesOf", { value: supportedLocalesOf, writable: true, configurable: true })
    return Class
  }

  // ECMA-402's digit options (SetNumberFormatDigitOptions), with the fraction digits a style defaults to.
  const digits = (given, minimumFraction, maximumFraction, notation) => {
    const minimumIntegerDigits = integer(given, "minimumIntegerDigits", 1, 21, 1)
    let { minimumFractionDigits: mnfd, maximumFractionDigits: mxfd, minimumSignificantDigits: mnsd, maximumSignificantDigits: mxsd } = given
    const roundingMode = option(given, "roundingMode", ["ceil", "floor", "expand", "trunc", "halfCeil", "halfFloor", "halfExpand", "halfTrunc", "halfEven"], "halfExpand")
    const roundingPriority = option(given, "roundingPriority", ["auto", "morePrecision", "lessPrecision"], "auto")
    const trailingZeroDisplay = option(given, "trailingZeroDisplay", ["auto", "stripIfInteger"], "auto")
    const hasSignificant = mnsd !== undefined || mxsd !== undefined
    const hasFraction = mnfd !== undefined || mxfd !== undefined
    let needSignificant = true
    let needFraction = true
    if (roundingPriority === "auto") {
      needSignificant = hasSignificant
      if (needSignificant || (!hasFraction && notation === "compact")) needFraction = false
    }
    const resolved = { minimumIntegerDigits, roundingMode, roundingPriority, trailingZeroDisplay }
    if (needSignificant) {
      resolved.minimumSignificantDigits = mnsd = hasSignificant ? integer({ mnsd }, "mnsd", 1, 21, 1) : 1
      resolved.maximumSignificantDigits = hasSignificant ? integer({ mxsd }, "mxsd", mnsd, 21, 21) : 21
    }
    if (needFraction) {
      if (hasFraction) {
        mnfd = integer({ mnfd }, "mnfd", 0, 100, undefined)
        mxfd = integer({ mxfd }, "mxfd", 0, 100, undefined)
        if (mnfd === undefined) mnfd = Math.min(minimumFraction, mxfd)
        else if (mxfd === undefined) mxfd = Math.max(maximumFraction, mnfd)
        else if (mnfd > mxfd) throw new RangeError("maximumFractionDigits value is out of range.")
      } else [mnfd, mxfd] = [minimumFraction, maximumFraction]
      Object.assign(resolved, { minimumFractionDigits: mnfd, maximumFractionDigits: mxfd })
    }
    if (!needSignificant && !needFraction) {
      Object.assign(resolved, { minimumFractionDigits: 0, maximumFractionDigits: 0, minimumSignificantDigits: 1, maximumSignificantDigits: 2, roundingPriority: "morePrecision" })
    }
    return resolved
  }
  // A number as ECMA-402 takes one (ToIntlMathematicalValue): a number, or a BigInt's or a string's digits.
  const mathematical = (value) => {
    if (typeof value === "bigint") return String(value)
    if (typeof value === "string" && /^\s*[+-]?(\d+\.?\d*|\.\d+)\s*$/.test(value)) return value.trim()
    return Number(value)
  }
  // ISO 4217's currencies whose minor unit isn't 2 digits, by how many it is.
  const minor = { BIF: 0, CLP: 0, DJF: 0, GNF: 0, ISK: 0, JPY: 0, KMF: 0, KRW: 0, PYG: 0, RWF: 0, UGX: 0, UYI: 0, VND: 0, VUV: 0, XAF: 0, XOF: 0, XPF: 0, BHD: 3, IQD: 3, JOD: 3, KWD: 3, LYD: 3, OMR: 3, TND: 3, CLF: 4, UYW: 4 }

  class NumberFormat {
    #numbers
    #resolved
    #format
    constructor(locales, given) {
      given = options(given)
      option(given, "localeMatcher", ["lookup", "best fit"], "best fit")
      const numberingSystem = option(given, "numberingSystem", undefined, undefined)
      const { locale, keys } = resolve(locales, ["nu"], { nu: numberingSystem })
      const style = option(given, "style", ["decimal", "percent", "currency", "unit"], "decimal")
      const currency = option(given, "currency", undefined, undefined)
      if (currency !== undefined && !/^[A-Za-z]{3}$/.test(currency)) throw new RangeError(`Invalid currency code : ${currency}`)
      const currencyDisplay = option(given, "currencyDisplay", ["code", "symbol", "narrowSymbol", "name"], "symbol")
      const currencySign = option(given, "currencySign", ["standard", "accounting"], "standard")
      const unit = option(given, "unit", undefined, undefined)
      const unitDisplay = option(given, "unitDisplay", ["short", "narrow", "long"], "short")
      if (style === "currency" && currency === undefined) throw new TypeError("Currency code is required with currency style.")
      if (style === "unit" && unit === undefined) throw new TypeError("Unit is required with unit style.")
      const notation = option(given, "notation", ["standard", "scientific", "engineering", "compact"], "standard")
      const fraction = style === "currency" ? (minor[currency.toUpperCase()] ?? 2) : 0
      const resolvedDigits = digits(given, fraction, style === "currency" ? fraction : style === "percent" ? 0 : 3, notation)
      const compactDisplay = option(given, "compactDisplay", ["short", "long"], "short")
      let useGrouping = given.useGrouping
      const groupingDefault = notation === "compact" ? "min2" : "auto"
      if (useGrouping === undefined || useGrouping === "true" || useGrouping === "false") useGrouping = groupingDefault
      else if (useGrouping === true) useGrouping = "always"
      else if (!useGrouping) useGrouping = false
      else if (!["min2", "auto", "always"].includes(String(useGrouping))) throw new RangeError(`Value ${useGrouping} out of range for useGrouping options property`)
      else useGrouping = String(useGrouping)
      const signDisplay = option(given, "signDisplay", ["auto", "never", "always", "exceptZero", "negative"], "auto")
      this.#resolved = {
        locale,
        numberingSystem: keys.nu ?? "latn",
        style,
        ...(style === "currency" && { currency: currency.toUpperCase(), currencyDisplay, currencySign }),
        ...(style === "unit" && { unit, unitDisplay }),
        ...resolvedDigits,
        useGrouping,
        notation,
        ...(notation === "compact" && { compactDisplay }),
        signDisplay,
        roundingIncrement: 1,
      }
      this.#numbers = new native.Numbers(locale, JSON.stringify({ ...this.#resolved, currencyDisplay, unitDisplay, compactDisplay, useGrouping: String(useGrouping) }))
    }
    // NaN and the infinities, which ICU4X's numbers don't hold, as CLDR says them.
    #special(value) {
      if (typeof value !== "number" || Number.isFinite(value)) return undefined
      if (Number.isNaN(value)) return "NaN"
      const sign = value < 0 ? (this.#resolved.signDisplay === "never" ? "" : "-") : ["always", "exceptZero"].includes(this.#resolved.signDisplay) ? "+" : ""
      return `${sign}∞${this.#resolved.style === "percent" ? "%" : ""}`
    }
    get format() {
      return (this.#format ??= (value) => {
        value = mathematical(value)
        return this.#special(value) ?? this.#numbers.format(value)
      })
    }
    formatToParts(value) {
      value = mathematical(value)
      const special = this.#special(value)
      if (special !== undefined) return [{ type: Number.isNaN(value) ? "nan" : "infinity", value: special }]
      return this.#numbers.formatToParts(value)
    }
    resolvedOptions() {
      return { ...this.#resolved }
    }
  }
  named(NumberFormat, "Intl.NumberFormat")

  // Which of a date and time's fields a format shows, given none (ToDateTimeOptions).
  const fields = ["weekday", "era", "year", "month", "day", "dayPeriod", "hour", "minute", "second", "fractionalSecondDigits", "timeZoneName"]
  class DateTimeFormat {
    #dates
    #resolved
    #format
    constructor(locales, given, required = "any", defaults = "date") {
      given = options(given)
      option(given, "localeMatcher", ["lookup", "best fit"], "best fit")
      const calendar = option(given, "calendar", undefined, undefined)
      const numberingSystem = option(given, "numberingSystem", undefined, undefined)
      const hour12 = given.hour12 === undefined ? undefined : Boolean(given.hour12)
      let hourCycle = option(given, "hourCycle", ["h11", "h12", "h23", "h24"], undefined)
      if (hour12 !== undefined) hourCycle = hour12 ? "h12" : "h23"
      let timeZone = given.timeZone === undefined ? native.hostTimeZone : native.timeZone(String(given.timeZone))
      const chosen = {}
      for (const [name, values] of [
        ["weekday", ["narrow", "short", "long"]],
        ["era", ["narrow", "short", "long"]],
        ["year", ["2-digit", "numeric"]],
        ["month", ["2-digit", "numeric", "narrow", "short", "long"]],
        ["day", ["2-digit", "numeric"]],
        ["dayPeriod", ["narrow", "short", "long"]],
        ["hour", ["2-digit", "numeric"]],
        ["minute", ["2-digit", "numeric"]],
        ["second", ["2-digit", "numeric"]],
      ]) {
        const value = option(given, name, values, undefined)
        if (value !== undefined) chosen[name] = value
      }
      const fractionalSecondDigits = integer(given, "fractionalSecondDigits", 1, 3, undefined)
      if (fractionalSecondDigits !== undefined) chosen.fractionalSecondDigits = fractionalSecondDigits
      const timeZoneName = option(given, "timeZoneName", ["short", "long", "shortOffset", "longOffset", "shortGeneric", "longGeneric"], undefined)
      if (timeZoneName !== undefined) chosen.timeZoneName = timeZoneName
      option(given, "formatMatcher", ["basic", "best fit"], "best fit")
      const dateStyle = option(given, "dateStyle", ["full", "long", "medium", "short"], undefined)
      const timeStyle = option(given, "timeStyle", ["full", "long", "medium", "short"], undefined)
      if ((dateStyle || timeStyle) && Object.keys(chosen).length) throw new TypeError("Can't set option dateStyle or timeStyle with other date and time options")
      if (!dateStyle && !timeStyle) {
        const dates = ["weekday", "year", "month", "day"].some((name) => name in chosen)
        const times = ["dayPeriod", "hour", "minute", "second", "fractionalSecondDigits"].some((name) => name in chosen)
        const needDefaults = required === "date" ? !dates : required === "time" ? !times : !dates && !times
        if (needDefaults && (defaults === "date" || defaults === "all")) Object.assign(chosen, { year: "numeric", month: "numeric", day: "numeric" })
        if (needDefaults && (defaults === "time" || defaults === "all")) Object.assign(chosen, { hour: "numeric", minute: "numeric", second: "numeric" })
      }
      if (required === "date" && dateStyle === undefined && timeStyle !== undefined) throw new TypeError("Invalid option : timeStyle")
      if (required === "time" && timeStyle === undefined && dateStyle !== undefined) throw new TypeError("Invalid option : dateStyle")
      const { locale, keys } = resolve(locales, ["ca", "hc", "nu"], { ca: calendar, hc: hourCycle, nu: numberingSystem })
      this.#dates = new native.Dates(locale, JSON.stringify({ timeZone, dateStyle, timeStyle, ...chosen }))
      const hours = "hour" in chosen || timeStyle !== undefined
      const cycle = hours ? (keys.hc ?? this.#dates.hourCycle()) : undefined
      this.#resolved = {
        locale,
        calendar: keys.ca ?? "gregory",
        numberingSystem: keys.nu ?? "latn",
        timeZone,
        ...(hours && { hourCycle: cycle, hour12: cycle === "h11" || cycle === "h12" }),
        ...chosen,
        ...(dateStyle && { dateStyle }),
        ...(timeStyle && { timeStyle }),
      }
    }
    static #time(date) {
      const time = date === undefined ? Date.now() : Number(date)
      if (!Number.isFinite(time)) throw new RangeError("Invalid time value")
      return time
    }
    get format() {
      return (this.#format ??= (date) => this.#dates.format(DateTimeFormat.#time(date)))
    }
    formatToParts(date) {
      return this.#dates.formatToParts(DateTimeFormat.#time(date))
    }
    resolvedOptions() {
      return { ...this.#resolved }
    }
  }
  named(DateTimeFormat, "Intl.DateTimeFormat")

  class PluralRules {
    #plurals
    #resolved
    constructor(locales, given) {
      given = options(given)
      option(given, "localeMatcher", ["lookup", "best fit"], "best fit")
      const type = option(given, "type", ["cardinal", "ordinal"], "cardinal")
      const { locale } = resolve(locales)
      const resolvedDigits = digits(given, 0, 3, "standard")
      this.#plurals = new native.Plurals(locale, type === "ordinal", JSON.stringify(resolvedDigits))
      this.#resolved = { locale, type, ...resolvedDigits, pluralCategories: this.#plurals.categories(), roundingIncrement: 1 }
    }
    select(value) {
      value = Number(value)
      return Number.isFinite(value) ? this.#plurals.select(value) : "other"
    }
    resolvedOptions() {
      return { ...this.#resolved, pluralCategories: [...this.#resolved.pluralCategories] }
    }
  }
  named(PluralRules, "Intl.PluralRules")

  const units = ["second", "minute", "hour", "day", "week", "month", "quarter", "year"]
  class RelativeTimeFormat {
    #locale
    #resolved
    #formatters = {}
    constructor(locales, given) {
      given = options(given)
      option(given, "localeMatcher", ["lookup", "best fit"], "best fit")
      const numberingSystem = option(given, "numberingSystem", undefined, undefined)
      const { locale, keys } = resolve(locales, ["nu"], { nu: numberingSystem })
      const style = option(given, "style", ["long", "short", "narrow"], "long")
      const numeric = option(given, "numeric", ["always", "auto"], "always")
      this.#locale = locale
      this.#resolved = { locale, style, numeric, numberingSystem: keys.nu ?? "latn" }
    }
    #formatter(value, unit) {
      value = Number(value)
      if (!Number.isFinite(value)) throw new RangeError("Invalid number value")
      unit = String(unit)
      const singular = unit.endsWith("s") ? unit.slice(0, -1) : unit
      if (!units.includes(singular)) throw new RangeError(`Invalid unit argument for format() '${unit}'`)
      const { style, numeric } = this.#resolved
      return [(this.#formatters[singular] ??= new native.Relative(this.#locale, singular, style, numeric)), value, singular]
    }
    format(value, unit) {
      const [formatter, number] = this.#formatter(value, unit)
      return formatter.format(number)
    }
    formatToParts(value, unit) {
      const [formatter, number, singular] = this.#formatter(value, unit)
      return formatter.formatToParts(number).map((part) => (part.type === "literal" ? part : { ...part, unit: singular }))
    }
    resolvedOptions() {
      return { ...this.#resolved }
    }
  }
  named(RelativeTimeFormat, "Intl.RelativeTimeFormat")

  const strings = (list) => {
    const items = []
    for (const item of list ?? []) {
      if (typeof item !== "string") throw new TypeError(`Iterable yielded ${String(item)} which is not a string`)
      items.push(item)
    }
    return items
  }
  class ListFormat {
    #lists
    #resolved
    constructor(locales, given) {
      given = options(given)
      option(given, "localeMatcher", ["lookup", "best fit"], "best fit")
      const { locale } = resolve(locales)
      const type = option(given, "type", ["conjunction", "disjunction", "unit"], "conjunction")
      const style = option(given, "style", ["long", "short", "narrow"], "long")
      this.#lists = new native.Lists(locale, type, style)
      this.#resolved = { locale, type, style }
    }
    format(list) {
      return this.#lists.format(strings(list))
    }
    formatToParts(list) {
      return this.#lists.formatToParts(strings(list))
    }
    resolvedOptions() {
      return { ...this.#resolved }
    }
  }
  named(ListFormat, "Intl.ListFormat")

  class Collator {
    #collation
    #resolved
    #compare
    constructor(locales, given) {
      given = options(given)
      const usage = option(given, "usage", ["sort", "search"], "sort")
      option(given, "localeMatcher", ["lookup", "best fit"], "best fit")
      const collation = option(given, "collation", undefined, undefined)
      const numeric = given.numeric === undefined ? undefined : String(Boolean(given.numeric))
      const caseFirst = option(given, "caseFirst", ["upper", "lower", "false"], undefined)
      const { locale, keys } = resolve(locales, ["co", "kn", "kf"], { co: collation, kn: numeric, kf: caseFirst })
      const sensitivity = option(given, "sensitivity", ["base", "accent", "case", "variant"], "variant")
      const ignorePunctuation = option(given, "ignorePunctuation", undefined, locale.startsWith("th"))
      this.#collation = new native.Collation(usage === "search" ? `${locale.split("-u-")[0]}-u-co-search` : locale, sensitivity, ignorePunctuation)
      this.#resolved = { locale, usage, sensitivity, ignorePunctuation, collation: keys.co ?? "default", numeric: keys.kn === "true", caseFirst: keys.kf ?? "false" }
    }
    get compare() {
      return (this.#compare ??= (a, b) => this.#collation.compare(String(a), String(b)))
    }
    resolvedOptions() {
      return { ...this.#resolved }
    }
  }
  named(Collator, "Intl.Collator")

  class Segmenter {
    #segments
    #resolved
    constructor(locales, given) {
      given = options(given)
      option(given, "localeMatcher", ["lookup", "best fit"], "best fit")
      const { locale } = resolve(locales)
      const granularity = option(given, "granularity", ["grapheme", "word", "sentence"], "grapheme")
      this.#segments = new native.Segments(granularity)
      this.#resolved = { locale, granularity }
    }
    segment(input) {
      input = String(input)
      const { ends, words } = this.#segments.segment(input)
      const word = this.#resolved.granularity === "word"
      const at = (index) => {
        const segment = { segment: input.slice(index === 0 ? 0 : ends[index - 1], ends[index]), index: index === 0 ? 0 : ends[index - 1], input }
        if (word) segment.isWordLike = words[index]
        return segment
      }
      return {
        containing(index = 0) {
          index = Math.trunc(Number(index)) || 0
          if (index < 0 || index >= input.length) return undefined
          return at(ends.findIndex((end) => end > index))
        },
        *[Symbol.iterator]() {
          for (let index = 0; index < ends.length; index++) yield at(index)
        },
      }
    }
    resolvedOptions() {
      return { ...this.#resolved }
    }
  }
  named(Segmenter, "Intl.Segmenter")

  class Locale {
    #tag
    constructor(tag, given) {
      if (typeof tag !== "string" && !(tag instanceof Locale)) throw new TypeError("First argument to Intl.Locale constructor can't be empty or missing")
      given = options(given)
      let [base, extension] = native.canonicalize(String(tag)).split("-u-")
      const parts = base.split("-")
      const language = option(given, "language", undefined, undefined)
      const script = option(given, "script", undefined, undefined)
      const region = option(given, "region", undefined, undefined)
      if (language) parts[0] = language
      const rest = parts.slice(1)
      let scriptAt = rest.findIndex((part) => /^[A-Za-z]{4}$/.test(part))
      if (script) scriptAt < 0 ? rest.unshift(script) : (rest[scriptAt] = script)
      scriptAt = rest.findIndex((part) => /^[A-Za-z]{4}$/.test(part))
      const regionAt = rest.findIndex((part) => /^([A-Za-z]{2}|\d{3})$/.test(part))
      if (region) regionAt < 0 ? rest.splice(scriptAt + 1, 0, region) : (rest[regionAt] = region)
      base = [parts[0], ...rest].join("-")
      const keys = unicode(extension ? `x-u-${extension}` : "")
      for (const [key, name, values] of [
        ["ca", "calendar"],
        ["co", "collation"],
        ["hc", "hourCycle", ["h11", "h12", "h23", "h24"]],
        ["kf", "caseFirst", ["upper", "lower", "false"]],
        ["nu", "numberingSystem"],
      ]) {
        const value = option(given, name, values, undefined)
        if (value !== undefined) keys[key] = value
      }
      if (given.numeric !== undefined) keys.kn = String(Boolean(given.numeric))
      const keywords = Object.keys(keys).sort().map((key) => (keys[key] === "true" ? key : `${key}-${keys[key]}`)).join("-")
      this.#tag = native.canonicalize(keywords ? `${base}-u-${keywords}` : base)
    }
    #part(test) {
      return this.baseName.split("-").slice(1).find(test)
    }
    #key(key) {
      return unicode(this.#tag)[key]
    }
    get baseName() { return this.#tag.split("-u-")[0] }
    get language() { return this.baseName.split("-")[0] }
    get script() { return this.#part((part) => /^[A-Za-z]{4}$/.test(part)) }
    get region() { return this.#part((part) => /^([A-Za-z]{2}|\d{3})$/.test(part)) }
    get calendar() { return this.#key("ca") }
    get collation() { return this.#key("co") }
    get hourCycle() { return this.#key("hc") }
    get caseFirst() { return this.#key("kf") }
    get numeric() { return this.#key("kn") === "true" }
    get numberingSystem() { return this.#key("nu") }
    maximize() { return new Locale(native.maximize(this.#tag)) }
    minimize() { return new Locale(native.minimize(this.#tag)) }
    toString() { return this.#tag }
    toJSON() { return this.#tag }
  }
  Object.defineProperty(Locale.prototype, Symbol.toStringTag, { value: "Intl.Locale", configurable: true })

  // What ECMA-402 lets be called as a function too, as the first edition's could be.
  const callable = (Class) => new Proxy(Class, { apply: (target, _, args) => new target(...args) })
  const Intl = {
    getCanonicalLocales: (locales) => canonical(locales),
    Collator: callable(Collator),
    DateTimeFormat: callable(DateTimeFormat),
    ListFormat,
    Locale,
    NumberFormat: callable(NumberFormat),
    PluralRules,
    RelativeTimeFormat,
    Segmenter,
  }
  Object.defineProperty(Intl, Symbol.toStringTag, { value: "Intl", configurable: true })
  for (const name of Object.keys(Intl)) Object.defineProperty(Intl, name, { enumerable: false })
  Object.defineProperty(globalThis, "Intl", { value: Intl, writable: true, configurable: true })

  // The language-sensitive methods of the language's own objects, as ECMA-402 has them, each formatter made once for
  // the locales and options it's called with, of the last few hundred that are plain data.
  const made = new Map()
  const formatter = (Class, locales, given, ...rest) => {
    let key
    try {
      key = JSON.stringify([Class.name, rest, locales, given])
    } catch {
      return new Class(locales, given, ...rest)
    }
    let each = made.get(key)
    if (!each) {
      each = new Class(locales, given, ...rest)
      if (made.size >= 256) made.delete(made.keys().next().value)
      made.set(key, each)
    }
    return each
  }
  const method = (prototype, name, value) => Object.defineProperty(prototype, name, { value, writable: true, configurable: true })
  const time = (date) => {
    const at = Date.prototype.getTime.call(date)
    return Number.isNaN(at) ? "Invalid Date" : undefined
  }
  method(Date.prototype, "toLocaleString", function toLocaleString(locales, given) {
    return time(this) ?? formatter(DateTimeFormat, locales, given, "any", "all").format(this)
  })
  method(Date.prototype, "toLocaleDateString", function toLocaleDateString(locales, given) {
    return time(this) ?? formatter(DateTimeFormat, locales, given, "date", "date").format(this)
  })
  method(Date.prototype, "toLocaleTimeString", function toLocaleTimeString(locales, given) {
    return time(this) ?? formatter(DateTimeFormat, locales, given, "time", "time").format(this)
  })
  method(Number.prototype, "toLocaleString", function toLocaleString(locales, given) {
    return formatter(NumberFormat, locales, given).format(Number.prototype.valueOf.call(this))
  })
  method(BigInt.prototype, "toLocaleString", function toLocaleString(locales, given) {
    return formatter(NumberFormat, locales, given).format(BigInt.prototype.valueOf.call(this))
  })
  method(String.prototype, "localeCompare", function localeCompare(that, locales, given) {
    if (this == null) throw new TypeError("String.prototype.localeCompare called on null or undefined")
    return formatter(Collator, locales, given).compare(String(this), String(that))
  })
  method(Array.prototype, "toLocaleString", function toLocaleString(locales, given) {
    return Array.from(this, (item) => (item == null ? "" : item.toLocaleString(locales, given))).join(",")
  })
})()
