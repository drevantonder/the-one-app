// Sheets: the person's own Google Sheets, through Google's API. Signing in is the same in every
// Google app: see google/google.js.

import settings from "./dist/settings.html" with { type: "text" };
import { google } from "./google/google.js";

const APP = "sheets";
const TITLE = "Google Sheets";
const SCOPES = "openid email https://www.googleapis.com/auth/spreadsheets";
const API = "https://sheets.googleapis.com/v4/";
const { account, api, str, withAccount, vault, schema, tools: signIn } = google({ app: APP, title: TITLE, scopes: SCOPES, base: API });

const sheetId = (v) => {
  const s = String(v || "").trim();
  if (!s) throw new Error("give spreadsheetId: the spreadsheet's id or address");
  return /\/d\/([a-zA-Z0-9_-]+)/.exec(s)?.[1] || s;
};
const ss = (v) => `spreadsheets/${encodeURIComponent(sheetId(v))}`;
const sheetInput = str("the spreadsheet's id, or its address, like https://docs.google.com/spreadsheets/d/1AbC…/edit; drive_files_list finds spreadsheets (q mimeType = 'application/vnd.google-apps.spreadsheet' and name contains '…')");
const valuesInput = { type: "array", items: { type: "array", items: {} }, description: "rows, each an array of values, like [[\"Date\", \"Amount\"], [\"2026-11-03\", 42]]; text is read as if typed (valueInputOption USER_ENTERED), so =SUM(A1:A3) is a formula and 3/11 a date" };
const noSheet = (v) => `no spreadsheet ${sheetId(v)}: give its id or its address (docs.google.com/spreadsheets/d/…); drive_files_list finds spreadsheets`;
const RANGE_HINT = `ranges are A1 notation, like Budget!A1:F20, Budget!A:C or Budget; ${APP}_spreadsheet lists the sheets`;

async function rangeOr(ctx, email, spreadsheet, range) {
  if (range) return String(range);
  const s = await api(ctx, email, "GET", ss(spreadsheet), { query: { fields: "sheets.properties.title" }, missing: noSheet(spreadsheet) });
  return `'${s.sheets[0].properties.title.replace(/'/g, "''")}'`;
}

export default {
  name: APP,
  title: "Google Sheets",
  tagline: "Your Google Sheets",
  icon: "table",
  color: "#34a853",
  description: "Your Google Sheets: read ranges, add rows, change cells and make spreadsheets",
  instructions: `Sheets is the person's Google Sheets. ${APP}_spreadsheet lists a spreadsheet's sheets (tabs); ${APP}_read reads a range (A1 notation, like Budget!A1:F20; the first sheet when left out). ${APP}_append adds rows after a table, ${APP}_update writes over a range and ${APP}_create makes a spreadsheet: change only what the person asked. Spreadsheets are given as spreadsheetId, an id or address. With several accounts, each tool takes account.`,
  vault,
  schema,

  tools: {
    ...signIn,
    spreadsheet: {
      reads: true,
      description: "A spreadsheet's title, address, time zone and sheets (tabs), each with its size (Sheets' spreadsheets.get, trimmed): { id, title, url, time_zone, sheets: [{ title, id, rows, columns }] }. Sheet titles are what ranges start with.",
      input: withAccount({ spreadsheetId: sheetInput }, ["spreadsheetId"]),
      async run(ctx, { account: a, spreadsheetId: spreadsheet }) {
        const s = await api(ctx, await account(ctx, a), "GET", ss(spreadsheet), { query: { fields: "spreadsheetId,spreadsheetUrl,properties(title,timeZone,locale),sheets.properties(sheetId,title,index,gridProperties)" }, missing: noSheet(spreadsheet) });
        return {
          id: s.spreadsheetId, title: s.properties.title, url: s.spreadsheetUrl, time_zone: s.properties.timeZone,
          sheets: s.sheets.map(({ properties: p }) => ({ title: p.title, id: p.sheetId, rows: p.gridProperties?.rowCount, columns: p.gridProperties?.columnCount })),
        };
      },
    },
    read: {
      reads: true,
      description: "The values in a range (Sheets' values.get): { range, values }, values a row per array, with the range they came from. Over limit rows, it says how many more.",
      input: withAccount({
        spreadsheetId: sheetInput, range: str("A1 notation, like Budget!A1:F20, Budget!A:C or Budget; the whole first sheet when left out"),
        valueRenderOption: { type: "string", enum: ["FORMATTED_VALUE", "UNFORMATTED_VALUE", "FORMULA"], description: "FORMATTED_VALUE: as shown (the default); UNFORMATTED_VALUE: numbers unformatted; FORMULA: formulas, like =SUM(B2:B9)" },
        limit: { type: "integer", description: "at most this many rows; default 500 (the rest are left out, and more says how many)" },
      }, ["spreadsheetId"]),
      async run(ctx, { account: a, spreadsheetId: spreadsheet, range, valueRenderOption, limit }) {
        const email = await account(ctx, a);
        const r = await api(ctx, email, "GET", `${ss(spreadsheet)}/values/${encodeURIComponent(await rangeOr(ctx, email, spreadsheet, range))}`,
          { query: { valueRenderOption: valueRenderOption || "FORMATTED_VALUE" }, missing: noSheet(spreadsheet), hint: RANGE_HINT });
        const rows = r.values || [];
        const most = Number(limit) || 500;
        return { range: r.range, values: rows.slice(0, most), ...(rows.length > most ? { more: `${rows.length - most} more ${rows.length - most === 1 ? "row" : "rows"}: read a smaller range, like one starting at row ${most + 1}` } : {}) };
      },
    },
    append: {
      reads: false,
      description: "Add rows after the table in a range (or the first sheet) (Sheets' values.append); says where they went. This changes the person's spreadsheet: do it only when they asked.",
      input: withAccount({ spreadsheetId: sheetInput, range: str("the table's range or sheet, like Expenses or Expenses!A:E; the first sheet when left out"), values: valuesInput }, ["spreadsheetId", "values"]),
      async run(ctx, { account: a, spreadsheetId: spreadsheet, range, values: rows }) {
        const email = await account(ctx, a);
        const r = await api(ctx, email, "POST", `${ss(spreadsheet)}/values/${encodeURIComponent(await rangeOr(ctx, email, spreadsheet, range))}:append`,
          { query: { valueInputOption: "USER_ENTERED", insertDataOption: "INSERT_ROWS" }, json: { values: rows }, missing: noSheet(spreadsheet), hint: RANGE_HINT });
        return `Added ${r.updates?.updatedRows ?? rows.length} rows at ${r.updates?.updatedRange}.`;
      },
    },
    update: {
      reads: false,
      confirm: true,
      description: "Write rows over a range, from its top-left cell, replacing what was there (Sheets' values.update); says which cells changed. Do it only when the person asked.",
      input: withAccount({ spreadsheetId: sheetInput, range: str("where to write, in A1 notation, like Budget!B2 or Budget!B2:D5"), values: valuesInput }, ["spreadsheetId", "range", "values"]),
      async run(ctx, { account: a, spreadsheetId: spreadsheet, range, values: rows }) {
        const r = await api(ctx, await account(ctx, a), "PUT", `${ss(spreadsheet)}/values/${encodeURIComponent(String(range))}`,
          { query: { valueInputOption: "USER_ENTERED" }, json: { values: rows }, missing: noSheet(spreadsheet), hint: RANGE_HINT });
        return `Wrote ${r.updatedCells} cells in ${r.updatedRange}.`;
      },
    },
    create: {
      reads: false,
      description: "Make a spreadsheet in the person's Drive (in My Drive; Sheets' spreadsheets.create), with sheets and first rows if given. Gives { id, title, url }.",
      input: withAccount({
        title: str("its title, like Budget 2026"), sheets: { type: "array", items: { type: "string" }, description: "its sheets' (tabs') titles, like [\"Income\", \"Expenses\"]; default one, Sheet1" },
        values: { ...valuesInput, description: `rows to put in the first sheet, from A1, ${valuesInput.description.slice(6)}` },
      }, ["title"]),
      async run(ctx, { account: a, title, sheets: tabs, values: rows }) {
        const email = await account(ctx, a);
        const s = await api(ctx, email, "POST", "spreadsheets", { json: { properties: { title }, ...(tabs?.length ? { sheets: tabs.map((t) => ({ properties: { title: t } })) } : {}) } });
        if (rows?.length) {
          const first = `'${s.sheets[0].properties.title.replace(/'/g, "''")}'!A1`;
          await api(ctx, email, "PUT", `${ss(s.spreadsheetId)}/values/${encodeURIComponent(first)}`, { query: { valueInputOption: "USER_ENTERED" }, json: { values: rows } });
        }
        return { id: s.spreadsheetId, title: s.properties.title, url: s.spreadsheetUrl };
      },
    },
  },

  screens: {
    settings: { title: TITLE, html: settings },
  },
};
