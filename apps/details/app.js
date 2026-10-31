// Your details: your phone, address, RC, supervisor, and the last 4 numbers of your bank account and cards, kept once
// in your own scope for every app you use (an expense claim, a form), so none of them asks again. Only the last 4
// numbers of any account or card: never the whole number.

const FIELDS = ["phone", "address", "rc", "approver", "bankAccount", "card1", "card2", "ptcCard"];
const NONE = Object.fromEntries(FIELDS.map((f) => [f, null]));
const LAST4 = [/^\d{4}$/, "The last 4 numbers"];
const RULES = {
  phone: [/^.{1,30}$/, "At most 30 characters"],
  address: [/^.{1,200}$/, "One line, at most 200 characters"],
  rc: [/^\d{4,5}$/, "4 or 5 numbers"],
  approver: [/^.{1,100}$/, "At most 100 characters"],
  bankAccount: LAST4, card1: LAST4, card2: LAST4, ptcCard: LAST4,
};

/// What's wrong with a value, or null; empty is fine (it clears the field).
export const problem = (key, value) => (value && RULES[key] && !RULES[key][0].test(value) ? RULES[key][1] : null);

async function read(ctx) {
  if (!ctx.user) throw new Error("your details are in your own scope, which this server doesn't hold");
  const rows = await ctx.user.query("select key, value from details");
  return { ...NONE, ...Object.fromEntries(rows.map((r) => [r.key, r.value])) };
}

export default {
  name: "details",
  title: "Your details",
  icon: "person.text.rectangle",
  color: "#8e8e93",
  instructions: "Your details are your phone, address, RC, supervisor (approver) and the last 4 numbers of your bank account and cards, kept once for every app. details_get reads them; details_set changes the fields given (null clears one). Never ask for or keep a whole account or card number.",
  schema: { user: "create table details (key text primary key, value text);" },

  ops: {
    get: {
      description: "Read your details: phone, address, rc (missionary RC, 4 or 5 numbers), approver (your supervisor, first and last name), and the last 4 numbers of bankAccount, card1, card2 and ptcCard. Missing ones are null.",
      input: { type: "object", properties: {} },
      run: read,
    },
    set: {
      description: "Change your details: give only the fields to change; null or \"\" clears one. Only the last 4 numbers of accounts and cards. Answers all your details.",
      input: {
        type: "object",
        properties: Object.fromEntries(FIELDS.map((f) => [f, { type: ["string", "null"] }])),
      },
      async run(ctx, input) {
        const now = await read(ctx);
        const change = {};
        for (const key of FIELDS) {
          if (!(key in input)) continue;
          const value = input[key] == null ? null : String(input[key]).trim() || null;
          const wrong = problem(key, value);
          if (wrong) throw new Error(`${key}: ${wrong}.`);
          if (value !== now[key]) change[key] = value;
        }
        if (Object.keys(change).length) ctx.user.append("set", change);
        return { ...now, ...change };
      },
    },
  },

  on: {
    set(db, e) {
      for (const [key, value] of Object.entries(e.data || {})) {
        if (!FIELDS.includes(key) || problem(key, value)) continue;
        if (value === null) db.exec("delete from details where key = ?", [key]);
        else db.exec("insert or replace into details values (?, ?)", [key, value]);
      }
    },
  },

  widgets: {
    details: {
      title: "Your details",
      async render(ctx) {
        const d = await read(ctx);
        const filled = FIELDS.filter((f) => d[f]).length;
        return { t: "col", gap: 4, c: [
          { t: "text", text: d.rc ? `RC ${d.rc}${d.approver ? ` · ${d.approver}` : ""}` : "No RC yet", weight: "medium" },
          { t: "text", text: `${filled} of ${FIELDS.length} filled in`, size: "sm", tone: "muted" },
        ] };
      },
    },
  },

  screens: {
    main: {
      title: "Your details",
      html: `
<style>
  .head { display: flex; justify-content: space-between; align-items: baseline; gap: 8px; margin: 4px 4px 0; }
  .status.bad { color: var(--danger); }
  h3 { font-size: 20px; margin: 22px 4px 4px; }
  .note { color: var(--muted); font-size: 14px; margin: 0 4px 10px; }
  label { display: block; margin: 0 0 14px; }
  label span { display: block; color: var(--muted); font-size: 14px; margin: 0 4px 5px; }
  input { width: 100%; font: inherit; padding: 11px 14px; border-radius: 10px; border: 0.5px solid var(--line); background: var(--card); color: var(--text); outline: none; }
  input:focus { border-color: var(--accent); }
  input[aria-invalid=true] { border-color: var(--danger); }
  .wrong { color: var(--danger); font-size: 14px; margin: 5px 4px 0; }
  .banner { background: var(--card); border-radius: 12px; padding: 12px 14px; margin: 12px 0; }
  .banner button { font: inherit; color: var(--accent); background: none; border: 0; padding: 6px 0 0; }
</style>
<div class="head"><span class="muted">Every app you use can read these, so you fill them in once.</span><span id="status" class="status muted small"></span></div>
<div id="failed" class="banner" hidden><div>Your last change didn't save. It's still here.</div><button id="retry">Try again</button></div>
<form id="form" autocomplete="on" onsubmit="return false"></form>
<script>
const RULES = { rc: [/^\\d{4,5}$/, "4 or 5 numbers"], bankAccount: [/^\\d{4}$/, "The last 4 numbers"], card1: [/^\\d{4}$/, "The last 4 numbers"], card2: [/^\\d{4}$/, "The last 4 numbers"], ptcCard: [/^\\d{4}$/, "The last 4 numbers"] };
const SECTIONS = [
  ["Contact", null, [["phone", "Phone", { type: "tel", autocomplete: "tel", placeholder: "Mobile or work number" }], ["address", "Address", { autocomplete: "street-address", placeholder: "Street, suburb and postcode" }]]],
  ["Work", null, [["rc", "RC", { digits: 5, placeholder: "4 or 5 numbers" }], ["approver", "Supervisor", { autocapitalize: "words", placeholder: "First and last name" }]]],
  ["Payments", "Just the last 4 numbers. Never the whole number.", [["bankAccount", "Bank account", { digits: 4 }], ["card1", "Credit card 1", { digits: 4 }], ["card2", "Credit card 2", { digits: 4 }], ["ptcCard", "PTC card", { digits: 4 }]]],
];
let saved = null, form = {}, left = new Set(), timer = null, state = "";
const problem = (k) => { const v = (form[k] || "").trim(); return v && RULES[k] && !RULES[k][0].test(v) ? RULES[k][1] : null; };
const clean = (f) => Object.fromEntries(Object.entries(f).map(([k, v]) => [k, (v || "").trim() || null]));
const status = (s) => { state = s; const el = document.getElementById("status"); el.textContent = { saving: "Saving…", saved: "Saved", failed: "Couldn't save" }[s] || ""; el.classList.toggle("bad", s === "failed"); document.getElementById("failed").hidden = s !== "failed"; };
function draw() {
  document.getElementById("form").innerHTML = SECTIONS.map(([title, note, fields]) => \`<h3>\${title}</h3>\${note ? \`<p class="note">\${note}</p>\` : ""}\` + fields.map(([k, label, o]) => {
    const attrs = o.digits ? \`inputmode="numeric" maxlength="\${o.digits}" placeholder="\${o.digits === 4 ? "Last 4" : o.placeholder}"\` : \`type="\${o.type || "text"}" placeholder="\${one.esc(o.placeholder || "")}" autocomplete="\${o.autocomplete || "off"}" autocapitalize="\${o.autocapitalize || "sentences"}"\`;
    return \`<label><span>\${label}</span><input name="\${k}" aria-label="\${label}" \${attrs} value="\${one.esc(form[k] || "")}"><div class="wrong" id="w-\${k}"></div></label>\`;
  }).join("")).join("");
  for (const input of document.querySelectorAll("input")) {
    const digits = input.inputMode === "numeric";
    input.oninput = () => { if (digits) input.value = input.value.replace(/\\D/g, ""); form[input.name] = input.value; later(); };
    input.onblur = () => { left.add(input.name); marks(); save(); };
  }
  marks();
}
function marks() {
  for (const k of Object.keys(form)) {
    const w = left.has(k) ? problem(k) : null;
    const el = document.getElementById("w-" + k); if (el) el.textContent = w || "";
    document.querySelector(\`input[name="\${k}"]\`)?.setAttribute("aria-invalid", !!w);
  }
}
// Saves a moment after you stop typing, and when you leave a field, once every field is right.
const later = () => { clearTimeout(timer); timer = setTimeout(save, 800); };
async function save() {
  clearTimeout(timer);
  const next = clean(form);
  if (JSON.stringify(next) === JSON.stringify(clean(saved)) || Object.keys(form).some(problem)) return;
  status("saving");
  try { saved = await one.call("set", next); status("saved"); } catch { status("failed"); }
}
document.getElementById("retry").onclick = save;
addEventListener("pagehide", save);
document.addEventListener("visibilitychange", () => { if (document.visibilityState === "hidden") save(); });
one.call("get").then((d) => { saved = d; form = Object.fromEntries(Object.entries(d).map(([k, v]) => [k, v || ""])); draw(); },
  (e) => { document.getElementById("form").innerHTML = \`<div class="banner">Your details didn't load. \${one.esc(e.message)}</div>\`; });
one.on("changed", async () => { if (state === "saving" || document.activeElement?.tagName === "INPUT") return; const d = await one.call("get"); if (JSON.stringify(clean(form)) === JSON.stringify(clean(saved))) { saved = d; form = Object.fromEntries(Object.entries(d).map(([k, v]) => [k, v || ""])); draw(); } });
</script>`,
    },
  },
};
