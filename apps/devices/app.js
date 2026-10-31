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
    name: s.name, os: os(s), online: !!s.online, here: !!s.me, host: on || null,
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
      async render(ctx) {
        const list = await devices(ctx);
        if (!list.length) return { t: "text", text: "No servers yet", tone: "muted" };
        return {
          t: "col",
          gap: 5,
          c: list.map((d) => ({
            t: "row",
            gap: 6,
            c: [
              { t: "text", text: (d.host ? "↳ " : "") + d.name, size: "sm", weight: d.host ? undefined : "medium" },
              { t: "text", text: d.online ? short(d) : "offline", size: "sm", tone: "muted" },
            ],
          })),
        };
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

/// A widget row's load: "41% RAM · CPU 12% · disk 8%", or a container's memory.
function short(d) {
  const parts = [];
  if (d.memory) parts.push(d.kind ? mem(d.memory.used) : `RAM ${pct(d.memory)}%`);
  if (last(d.cpu) != null) parts.push(`CPU ${last(d.cpu)}%`);
  if (last(d.gpu) != null) parts.push(`GPU ${last(d.gpu)}%`);
  if (d.disk) parts.push(`disk ${pct(d.disk)}%`);
  return parts.join(" · ") || "online";
}
