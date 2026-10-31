// Devices: every server in the install and how loaded it is (memory, CPU, GPU, disk), with the containers each one
// hosts listed under it. Each server measures itself and says so when it syncs; this app only draws what
// ctx.servers() gives it.

const GB = 1e9;
const GiB = 2 ** 30;

const os = (s) => (s.target || "").includes("apple") ? "macOS" : (s.target || "").includes("linux") ? "Linux" : "";
const pct = (sp) => (sp && sp.total ? Math.round((sp.used * 100) / sp.total) : null);
const last = (xs) => (xs && xs.length ? xs[xs.length - 1] : null);
/// Bytes as GB: memory counts them in 2^30s, as macOS and Activity Monitor do, disks in 10^9s.
const size = (b, g = GB) => (b >= 10 * g ? `${Math.round(b / g)} GB` : b >= g ? `${(b / g).toFixed(1)} GB` : `${Math.round(b / (g / 1000))} MB`);
const mem = (b) => size(b, GiB);

/// A container's memory, out of its limit, or its host's memory if it has none.
const share = (m, host) => (m ? { used: m.used, total: m.total || host?.total || 0 } : null);

/// Every server, each container that is a server itself right after its host, then the host's other containers.
async function devices(ctx) {
  const servers = await ctx.servers();
  const byName = new Map(servers.map((s) => [s.name, s]));
  const host = new Map();
  for (const s of servers) for (const c of s.load?.containers || []) if (byName.has(c.name) && c.name !== s.name) host.set(c.name, s.name);
  const device = (s, on) => ({
    name: s.name, os: os(s), online: !!s.online, here: !!s.me, host: on || null, build: s.build || null,
    memory: s.online ? s.load?.memory || null : null, disk: s.online ? s.load?.disk || null : null,
    cpu: s.online ? s.load?.cpu || [] : [], gpu: s.online ? s.load?.gpu || [] : [],
  });
  const out = [];
  for (const s of servers.filter((s) => !host.has(s.name)).sort((a, b) => b.me - a.me || b.online - a.online || a.name.localeCompare(b.name))) {
    out.push(device(s));
    // Containers that are servers themselves first, then the rest.
    const guests = [...((s.online && s.load?.containers) || [])].sort((a, b) => (host.get(b.name) === s.name) - (host.get(a.name) === s.name));
    for (const c of guests) {
      const self = byName.get(c.name);
      out.push(self && host.get(c.name) === s.name ? device(self, s.name) : { name: c.name, kind: c.kind, online: true, host: s.name, memory: share(c.memory, s.load.memory), cpu: [], gpu: [], disk: null });
    }
  }
  return out;
}

/// One line of a device's load: "RAM 41% (12.2 of 32 GB) · CPU 12% · GPU 3% · disk 8%".
function line(d) {
  if (!d.online) return "offline";
  const parts = [];
  if (d.memory) parts.push(d.kind ? `RAM ${mem(d.memory.used)}` : `RAM ${pct(d.memory)}% (${mem(d.memory.used)} of ${mem(d.memory.total)})`);
  if (last(d.cpu) != null) parts.push(`CPU ${last(d.cpu)}%`);
  if (last(d.gpu) != null) parts.push(`GPU ${last(d.gpu)}%`);
  if (d.disk) parts.push(`disk ${pct(d.disk)}% (${size(d.disk.total - d.disk.used)} free)`);
  return parts.join(" · ") || "online";
}

export default {
  name: "devices",
  title: "Devices",
  icon: "desktopcomputer",
  color: "#30b0c7",
  description: "How loaded each server is: memory, CPU, GPU and disk, with its containers",
  instructions: "devices_list shows every server in the install, online or not, with its memory (as Activity Monitor counts it on a Mac), CPU and GPU (percent busy, a sample every 5 seconds over the last two minutes, oldest first) and disk, and the containers each hosts under it. Use it to pick a server with room for a desktop, a model or a build.",

  ops: {
    list: {
      description: "List every server, online or not, with its memory, disk (bytes), CPU and GPU (percent busy, every 5 s over two minutes, oldest first), and each one's containers under it (host).",
      input: { type: "object", properties: {} },
      reads: true,
      async run(ctx) {
        const list = await devices(ctx);
        return { devices: list, text: list.map((d) => `${d.host ? "  " : ""}${d.name}${d.kind ? ` (${d.kind})` : ""}: ${line(d)}`).join("\n") };
      },
    },
  },

  widgets: {
    devices: {
      title: "Devices",
      size: "large",
      sizes: ["small", "medium", "large", "xl"],
      async render(ctx) {
        const list = await devices(ctx);
        if (!list.length) return { t: "text", text: "No servers yet", tone: "muted" };
        return ctx.size === "small" ? compact(list) : table(list, ctx.size);
      },
    },
  },

  screens: {
    main: {
      title: "Devices",
      html: `
<style>
.dev { background: var(--card); border-radius: var(--radius); padding: 14px 16px; margin: 12px 0; }
.dev.off { opacity: .55; }
.dev.sub { margin-left: 20px; }
.dev header { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }
.dev h3 { margin: 0; font-size: 17px; }
.dev .os { color: var(--muted); font-size: 13px; }
.stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 12px 24px; margin-top: 12px; }
.stat .label { display: flex; justify-content: space-between; gap: 8px; white-space: nowrap; font-size: 13px; color: var(--muted); margin-bottom: 4px; }
.stat .label b { color: var(--text); font-weight: 600; }
.bar { height: 6px; border-radius: 3px; background: var(--line); overflow: hidden; }
.bar i { display: block; height: 100%; border-radius: 3px; }
.spark { width: 100%; height: 28px; display: block; }
.guests { margin: 12px -16px -14px; border-top: .5px solid var(--line); }
.guest { display: flex; align-items: center; gap: 10px; padding: 9px 16px 9px 28px; font-size: 15px; }
.guest + .guest { border-top: .5px solid var(--line); }
.guest .n { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.guest .k { color: var(--muted); font-size: 13px; }
.guest .m { color: var(--muted); font-size: 13px; white-space: nowrap; }
.guest .bar { width: 64px; }
</style>
<div id="list"></div>
<script>
const GB = 1e9, GiB = 2 ** 30;
const size = (b, g = GB) => b >= 10 * g ? Math.round(b / g) + " GB" : b >= g ? (b / g).toFixed(1) + " GB" : Math.round(b / (g / 1000)) + " MB";
const mem = (b) => size(b, GiB);
const pct = (s) => s && s.total ? Math.round(s.used * 100 / s.total) : null;
const tone = (p) => p >= 85 ? "#ff3b30" : p >= 70 ? "#ff9500" : "#34c759";
const bar = (s) => s && s.total ? \`<div class="bar"><i style="width:\${pct(s)}%;background:\${tone(pct(s))}"></i></div>\` : "";
function spark(xs, color) {
  if (!xs.length) return '<div class="bar"></div>';
  const n = Math.max(xs.length, 24), w = 100, h = 28;
  const pts = xs.map((v, i) => [((n - xs.length + i) / (n - 1)) * w, h - 1 - (v / 100) * (h - 2)]);
  const d = pts.map((p, i) => (i ? "L" : "M") + p[0].toFixed(1) + " " + p[1].toFixed(1)).join(" ");
  return \`<svg class="spark" viewBox="0 0 \${w} \${h}" preserveAspectRatio="none"><path d="\${d} L\${w} \${h} L\${pts[0][0].toFixed(1)} \${h} Z" fill="\${color}" opacity=".15"/><path d="\${d}" fill="none" stroke="\${color}" stroke-width="1.5" vector-effect="non-scaling-stroke"/></svg>\`;
}
const stat = (label, value, body) => \`<div class="stat"><div class="label">\${label}<b>\${value}</b></div>\${body}</div>\`;
function card(d, guests) {
  const sub = [d.os, d.host ? "on " + one.esc(d.host) : "", d.here ? "this device" : ""].filter(Boolean).join(" · ");
  let stats = "";
  if (d.online) {
    if (d.memory) stats += stat("Memory", d.memory.total ? mem(d.memory.used) + " of " + mem(d.memory.total) : mem(d.memory.used), bar(d.memory));
    if (d.cpu.length) stats += stat("CPU", d.cpu[d.cpu.length - 1] + "%", spark(d.cpu, "#0a84ff"));
    if (d.gpu.length) stats += stat("GPU", d.gpu[d.gpu.length - 1] + "%", spark(d.gpu, "#bf5af2"));
    if (d.disk) stats += stat("Disk", size(d.disk.total - d.disk.used) + " free", bar(d.disk));
  }
  const rows = guests.map((g) => \`<div class="guest"><span class="n">\${one.esc(g.name)}</span><span class="k">\${g.kind === "podman" ? "podman" : "container"}</span>\${bar(g.memory)}<span class="m">\${g.memory ? mem(g.memory.used) : ""}</span></div>\`).join("");
  return \`<div class="dev\${d.online ? "" : " off"}\${d.host ? " sub" : ""}"><header><h3>\${one.esc(d.name)}</h3><span class="os">\${d.online ? sub : "Offline"}</span></header>
    \${stats ? '<div class="stats">' + stats + "</div>" : ""}\${rows ? '<div class="guests">' + rows + "</div>" : ""}</div>\`;
}
async function show() {
  try {
    const { devices } = await one.call("list", {});
    let html = "";
    for (let i = 0; i < devices.length; i++) {
      const d = devices[i];
      if (d.kind) continue;
      const guests = [];
      for (let j = i + 1; j < devices.length && devices[j].host === d.name; j++) if (devices[j].kind) guests.push(devices[j]);
      html += card(d, guests);
    }
    document.getElementById("list").innerHTML = html || '<div class="empty">No servers yet</div>';
  } catch (err) {
    document.getElementById("list").innerHTML = \`<div class="empty error">\${one.esc(err.message)}</div>\`;
  }
}
show();
setInterval(show, 5000);
</script>`,
    },
  },
};

// The widget, as the old Devices widget drew it: a header of column labels, then a row per device, its icon with
// its OS's logo, its name (and in larger sizes a line under it), a memory ring, CPU and GPU sparklines and a disk bar,
// in columns that line up across the rows.

/// Device outlines by kind, drawn in a 24-point box, and the logo of each OS for the badge on them (Simple Icons, CC0).
const ICONS = {
  macOS: "M6 6h12a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM3 18h18",
  Linux: "M5 4h14a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM5 13h14a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1zM8 7.5h.01M8 16.5h.01",
  desktop: "M4 4h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM9 20h6M12 16v4",
  container: "M3.5 6h17a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1h-17a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM6.5 9v6M10 9v6M14 9v6M17.5 9v6",
};
const LOGOS = {
  macOS: "M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701",
  Linux: "M12.504 0c-.155 0-.315.008-.48.021-4.226.333-3.105 4.807-3.17 6.298-.076 1.092-.3 1.953-1.05 3.02-.885 1.051-2.127 2.75-2.716 4.521-.278.832-.41 1.684-.287 2.489a.424.424 0 00-.11.135c-.26.268-.45.6-.663.839-.199.199-.485.267-.797.4-.313.136-.658.269-.864.68-.09.189-.136.394-.132.602 0 .199.027.4.055.536.058.399.116.728.04.97-.249.68-.28 1.145-.106 1.484.174.334.535.47.94.601.81.2 1.91.135 2.774.6.926.466 1.866.67 2.616.47.526-.116.97-.464 1.208-.946.587-.003 1.23-.269 2.26-.334.699-.058 1.574.267 2.577.2.025.134.063.198.114.333l.003.003c.391.778 1.113 1.132 1.884 1.071.771-.06 1.592-.536 2.257-1.306.631-.765 1.683-1.084 2.378-1.503.348-.199.629-.469.649-.853.023-.4-.2-.811-.714-1.376v-.097l-.003-.003c-.17-.2-.25-.535-.338-.926-.085-.401-.182-.786-.492-1.046h-.003c-.059-.054-.123-.067-.188-.135a.357.357 0 00-.19-.064c.431-1.278.264-2.55-.173-3.694-.533-1.41-1.465-2.638-2.175-3.483-.796-1.005-1.576-1.957-1.56-3.368.026-2.152.236-6.133-3.544-6.139zm.529 3.405h.013c.213 0 .396.062.584.198.19.135.33.332.438.533.105.259.158.459.166.724 0-.02.006-.04.006-.06v.105a.086.086 0 01-.004-.021l-.004-.024a1.807 1.807 0 01-.15.706.953.953 0 01-.213.335.71.71 0 00-.088-.042c-.104-.045-.198-.064-.284-.133a1.312 1.312 0 00-.22-.066c.05-.06.146-.133.183-.198.053-.128.082-.264.088-.402v-.02a1.21 1.21 0 00-.061-.4c-.045-.134-.101-.2-.183-.333-.084-.066-.167-.132-.267-.132h-.016c-.093 0-.176.03-.262.132a.8.8 0 00-.205.334 1.18 1.18 0 00-.09.4v.019c.002.089.008.179.02.267-.193-.067-.438-.135-.607-.202a1.635 1.635 0 01-.018-.2v-.02a1.772 1.772 0 01.15-.768c.082-.22.232-.406.43-.533a.985.985 0 01.594-.2zm-2.962.059h.036c.142 0 .27.048.399.135.146.129.264.288.344.465.09.199.14.4.153.667v.004c.007.134.006.2-.002.266v.08c-.03.007-.056.018-.083.024-.152.055-.274.135-.393.2.012-.09.013-.18.003-.267v-.015c-.012-.133-.04-.2-.082-.333a.613.613 0 00-.166-.267.248.248 0 00-.183-.064h-.021c-.071.006-.13.04-.186.132a.552.552 0 00-.12.27.944.944 0 00-.023.33v.015c.012.135.037.2.08.334.046.134.098.2.166.268.01.009.02.018.034.024-.07.057-.117.07-.176.136a.304.304 0 01-.131.068 2.62 2.62 0 01-.275-.402 1.772 1.772 0 01-.155-.667 1.759 1.759 0 01.08-.668 1.43 1.43 0 01.283-.535c.128-.133.26-.2.418-.2zm1.37 1.706c.332 0 .733.065 1.216.399.293.2.523.269 1.052.468h.003c.255.136.405.266.478.399v-.131a.571.571 0 01.016.47c-.123.31-.516.643-1.063.842v.002c-.268.135-.501.333-.775.465-.276.135-.588.292-1.012.267a1.139 1.139 0 01-.448-.067 3.566 3.566 0 01-.322-.198c-.195-.135-.363-.332-.612-.465v-.005h-.005c-.4-.246-.616-.512-.686-.71-.07-.268-.005-.47.193-.6.224-.135.38-.271.483-.336.104-.074.143-.102.176-.131h.002v-.003c.169-.202.436-.47.839-.601.139-.036.294-.065.466-.065zm2.8 2.142c.358 1.417 1.196 3.475 1.735 4.473.286.534.855 1.659 1.102 3.024.156-.005.33.018.513.064.646-1.671-.546-3.467-1.089-3.966-.22-.2-.232-.335-.123-.335.59.534 1.365 1.572 1.646 2.757.13.535.16 1.104.021 1.67.067.028.135.06.205.067 1.032.534 1.413.938 1.23 1.537v-.043c-.06-.003-.12 0-.18 0h-.016c.151-.467-.182-.825-1.065-1.224-.915-.4-1.646-.336-1.77.465-.008.043-.013.066-.018.135-.068.023-.139.053-.209.064-.43.268-.662.669-.793 1.187-.13.533-.17 1.156-.205 1.869v.003c-.02.334-.17.838-.319 1.35-1.5 1.072-3.58 1.538-5.348.334a2.645 2.645 0 00-.402-.533 1.45 1.45 0 00-.275-.333c.182 0 .338-.03.465-.067a.615.615 0 00.314-.334c.108-.267 0-.697-.345-1.163-.345-.467-.931-.995-1.788-1.521-.63-.4-.986-.87-1.15-1.396-.165-.534-.143-1.085-.015-1.645.245-1.07.873-2.11 1.274-2.763.107-.065.037.135-.408.974-.396.751-1.14 2.497-.122 3.854a8.123 8.123 0 01.647-2.876c.564-1.278 1.743-3.504 1.836-5.268.048.036.217.135.289.202.218.133.38.333.59.465.21.201.477.335.876.335.039.003.075.006.11.006.412 0 .73-.134.997-.268.29-.134.52-.334.74-.4h.005c.467-.135.835-.402 1.044-.7zm2.185 8.958c.037.6.343 1.245.882 1.377.588.134 1.434-.333 1.791-.765l.211-.01c.315-.007.577.01.847.268l.003.003c.208.199.305.53.391.876.085.4.154.78.409 1.066.486.527.645.906.636 1.14l.003-.007v.018l-.003-.012c-.015.262-.185.396-.498.595-.63.401-1.746.712-2.457 1.57-.618.737-1.37 1.14-2.036 1.191-.664.053-1.237-.2-1.574-.898l-.005-.003c-.21-.4-.12-1.025.056-1.69.176-.668.428-1.344.463-1.897.037-.714.076-1.335.195-1.814.12-.465.308-.797.641-.984l.045-.022zm-10.814.049h.01c.053 0 .105.005.157.014.376.055.706.333 1.023.752l.91 1.664.003.003c.243.533.754 1.064 1.189 1.637.434.598.77 1.131.729 1.57v.006c-.057.744-.48 1.148-1.125 1.294-.645.135-1.52.002-2.395-.464-.968-.536-2.118-.469-2.857-.602-.369-.066-.61-.2-.723-.4-.11-.2-.113-.602.123-1.23v-.004l.002-.003c.117-.334.03-.752-.027-1.118-.055-.401-.083-.71.043-.94.16-.334.396-.4.69-.533.294-.135.64-.202.915-.47h.002v-.002c.256-.268.445-.601.668-.838.19-.201.38-.336.663-.336zm7.159-9.074c-.435.201-.945.535-1.488.535-.542 0-.97-.267-1.28-.466-.154-.134-.28-.268-.373-.335-.164-.134-.144-.333-.074-.333.109.016.129.134.199.2.096.066.215.2.36.333.292.2.68.467 1.167.467.485 0 1.053-.267 1.398-.466.195-.135.445-.334.648-.467.156-.136.149-.267.279-.267.128.016.034.134-.147.332a8.097 8.097 0 01-.69.468zm-1.082-1.583V5.64c-.006-.02.013-.042.029-.05.074-.043.18-.027.26.004.063 0 .16.067.15.135-.006.049-.085.066-.135.066-.055 0-.092-.043-.141-.068-.052-.018-.146-.008-.163-.065zm-.551 0c-.02.058-.113.049-.166.066-.047.025-.086.068-.14.068-.05 0-.13-.02-.136-.068-.01-.066.088-.133.15-.133.08-.031.184-.047.259-.005.019.009.036.03.03.05v.02h.003z",
};

const level = (sp) => { const p = sp.used / sp.total; return p >= 0.85 ? "bad" : p >= 0.7 ? "warn" : "good"; };
const cell = (w, n) => ({ t: "col", w, align: "center", c: [n || { t: "col", c: [] }] });
const label = (text) => ({ t: "text", text, size: "xs", weight: "medium", tone: "muted", lines: 1 });
const ring = (sp, w) => sp && sp.total ? { t: "gauge", value: sp.used / sp.total, tone: level(sp), w, label: "" } : null;
const bar = (sp, w) => sp && sp.total ? { t: "progress", value: sp.used / sp.total, tone: level(sp), w } : null;
const spark = (values, tone, w, h) => ({ t: "spark", values, max: 100, tone, w, h });

/// The widths of each size's columns, shared by its header and every row so they line up.
const COLUMNS = {
  medium: { ram: 26, cpu: 32, gpu: 32, disk: 26, gap: 6, icon: 22, ring: 18, spark: 16 },
  large: { ram: 26, cpu: 32, gpu: 32, disk: 26, gap: 6, icon: 22, ring: 18, spark: 16 },
  xl: { ram: 32, cpu: 90, gpu: 90, disk: 70, gap: 14, icon: 22, ring: 20, spark: 18 },
};

function icon(d, w) {
  // Containers run Linux, on a Mac too.
  const kind = d.host || d.kind ? "container" : d.os;
  const os = d.kind ? "Linux" : d.os;
  return { t: "icon", path: ICONS[kind] || ICONS.desktop, w, ...(LOGOS[os] ? { badge: LOGOS[os] } : {}) };
}

/// "This device · On linux · 6d7ba15", or "Offline".
function detail(d) {
  if (!d.online) return "Offline";
  return [d.here && "This device", d.host && `On ${d.host}`, d.kind && (d.kind === "podman" ? "Podman" : "Container"), d.build && d.build.slice(0, 7)].filter(Boolean).join(" · ");
}

function table(list, size) {
  const k = COLUMNS[size] || COLUMNS.medium;
  // A medium widget fits only the servers, a line each; larger ones show every container, and a line under each name.
  const rows = size === "medium" ? list.filter((d) => !d.kind) : list;
  const details = size !== "medium";
  const stats = (d) => d.online
    ? { t: "row", gap: k.gap, c: [cell(k.ram, ring(d.memory, k.ring)), cell(k.cpu, d.kind ? null : spark(d.cpu, "blue", k.cpu, k.spark)),
        cell(k.gpu, d.kind ? null : spark(d.gpu, "purple", k.gpu, k.spark)), cell(k.disk, bar(d.disk, k.disk))] }
    : null;
  const row = (d) => ({
    t: "row", gap: 10, dim: !d.online,
    c: [
      icon(d, k.icon),
      { t: "col", grow: true, gap: 1, c: [{ t: "text", text: d.name, size: "sm", lines: 1 }, ...(details ? [{ t: "text", text: detail(d), size: "xs", tone: "muted", lines: 1 }] : [])] },
      stats(d),
    ].filter(Boolean),
  });
  const header = {
    t: "row", gap: 10,
    c: [{ t: "col", grow: true, c: [label("Devices")] }, { t: "row", gap: k.gap, c: [cell(k.ram, label("RAM")), cell(k.cpu, label("CPU")), cell(k.gpu, label("GPU")), cell(k.disk, label("DISK"))] }],
  };
  const body = [header, ...rows.map(row)];
  if (details) body.push(warning(list));
  return { t: "col", header: false, next: Date.now() + 60_000, c: spread(body) };
}

/// Nodes with the room left between them, so they fill the widget from top to bottom.
const spread = (nodes) => nodes.flatMap((n, i) => (i ? [{ t: "col", grow: true, gap: 0, c: [] }, n] : [n]));

/// What needs looking at: a disk or memory nearly full, or a server offline; or that nothing does.
function warning(list) {
  const worst = [];
  for (const d of list) {
    if (!d.online) worst.push([1, `${d.name} is offline`]);
    else {
      if (d.disk && d.disk.used / d.disk.total >= 0.85) worst.push([d.disk.used / d.disk.total, `${d.name}'s disk is ${pct(d.disk)}% full, ${size(d.disk.total - d.disk.used)} free`]);
      if (d.memory && d.memory.total && d.memory.used / d.memory.total >= 0.85 && !d.kind) worst.push([d.memory.used / d.memory.total, `${d.name} is using ${pct(d.memory)}% of its memory`]);
    }
  }
  worst.sort((a, b) => b[0] - a[0]);
  if (!worst.length) return { t: "row", gap: 6, c: [{ t: "icon", name: "checkmark.circle.fill", tone: "good", size: "xs" }, { t: "text", text: "All devices are fine", size: "xs", tone: "muted", lines: 1 }] };
  // Nearly full is red, as its bar is; only offline, orange.
  const tone = worst[0][0] === 1 && worst.every((w) => w[0] === 1) ? "warn" : "bad";
  return { t: "row", gap: 6, c: [{ t: "icon", name: "exclamationmark.triangle.fill", tone, size: "xs" }, { t: "text", text: worst.map((w) => w[1]).join(" · "), size: "xs", tone, lines: 1 }] };
}

/// A small widget: each server's name cut short, its memory ring, CPU and disk.
function compact(list) {
  const short = (name) => (name.length > 4 ? name.slice(0, 3) : name);
  const row = (d) => ({
    t: "row", gap: 6, dim: !d.online,
    c: [
      { t: "col", w: 28, c: [{ t: "text", text: short(d.name), size: "sm", lines: 1 }] },
      ...(d.online ? [cell(24, ring(d.memory, 15)), cell(26, spark(d.cpu, "blue", 26, 12)), cell(26, bar(d.disk, 26))] : []),
    ],
  });
  const header = { t: "row", gap: 6, c: [{ t: "col", w: 28, c: [] }, cell(24, label("RAM")), cell(26, label("CPU")), cell(26, label("DISK"))] };
  return { t: "col", header: false, next: Date.now() + 60_000, c: spread([header, ...list.filter((d) => !d.kind).slice(0, 5).map(row)]) };
}
