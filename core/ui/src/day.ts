import { type CalendarDate, parseDate as parse } from "@internationalized/date"

/// `parseDate("2026-11-14")`: a day, written as `<input type="date">` writes it, as Ark and Zag hold days. Its
/// `toString()` writes it back.
export const parseDate = (day: string): CalendarDate => parse(day)
