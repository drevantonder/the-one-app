// Reads git objects out of a repo's packs, which are blobs. A pack is indexed the first time it's read (each object's
// offset and hash) and kept in memory while the app runs; packs are never thin, so every delta's base is in its pack.
//
// pako's low-level inflate, because a pack's objects are zlib streams back to back with no lengths: it says how much
// input each stream used, which DecompressionStream can't.
import zlib from "pako/lib/zlib/inflate.js";
import ZStream from "pako/lib/zlib/zstream.js";
import crc32 from "pako/lib/zlib/crc32.js";

const TYPES = { 1: "commit", 2: "tree", 3: "blob", 4: "tag" };
const OFS_DELTA = 6, REF_DELTA = 7;
const hex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
const utf8 = new TextDecoder();

/// Inflates the zlib stream at `at`, `size` bytes when inflated: [bytes, where the stream ends].
function inflate(buf, at, size) {
  const s = new ZStream();
  zlib.inflateInit(s);
  s.input = buf;
  s.next_in = at;
  s.avail_in = buf.length - at;
  s.output = new Uint8Array(size + 1);
  s.next_out = 0;
  s.avail_out = size + 1;
  const status = zlib.inflate(s, 4);
  if (status !== 1) throw new Error("a pack is damaged");
  return [s.output.subarray(0, size), s.next_in];
}

/// Applies a git delta to its base.
function patch(base, delta) {
  let i = 0;
  const varint = () => {
    let n = 0, shift = 0, b;
    do { b = delta[i++]; n += (b & 0x7f) * 2 ** shift; shift += 7; } while (b & 0x80);
    return n;
  };
  varint();
  const out = new Uint8Array(varint());
  let o = 0;
  while (i < delta.length) {
    const op = delta[i++];
    if (op & 0x80) {
      let off = 0, len = 0;
      for (let k = 0; k < 4; k++) if (op & (1 << k)) off += delta[i++] * 2 ** (8 * k);
      for (let k = 0; k < 3; k++) if (op & (1 << (4 + k))) len += delta[i++] * 2 ** (8 * k);
      out.set(base.subarray(off, off + (len || 0x10000)), o);
      o += len || 0x10000;
    } else {
      out.set(delta.subarray(i, i + op), o);
      i += op;
      o += op;
    }
  }
  return out;
}

/// A pack: every object's place in it, by hash, and recently read objects.
class Pack {
  constructor(bytes) {
    this.bytes = bytes;
    this.at = new Map(); // hash → offset
    this.headers = new Map(); // offset → { type, size, data (where its stream starts), base }
    this.recent = new Map(); // offset → { type, data }, oldest first
    this.held = 0;
  }

  async index() {
    const b = this.bytes;
    if (utf8.decode(b.subarray(0, 4)) !== "PACK") throw new Error("a pack is damaged");
    const count = ((b[8] << 24) | (b[9] << 16) | (b[10] << 8) | b[11]) >>> 0;
    let at = 12;
    for (let n = 0; n < count; n++) {
      const start = at;
      let c = b[at++], type = (c >> 4) & 7, size = c & 15, shift = 4;
      while (c & 0x80) { c = b[at++]; size += (c & 0x7f) * 2 ** shift; shift += 7; }
      let base = null;
      if (type === OFS_DELTA) {
        c = b[at++];
        let off = c & 0x7f;
        while (c & 0x80) { c = b[at++]; off = (off + 1) * 128 + (c & 0x7f); }
        base = start - off;
      } else if (type === REF_DELTA) {
        base = hex(b.subarray(at, at + 20));
        at += 20;
      }
      const header = { type, size, data: at, base };
      this.headers.set(start, header);
      [, at] = inflate(b, at, size);
    }
    // Hash every object; a REF_DELTA's base may come later in the pack, so those wait for a second pass.
    let waiting = [...this.headers.keys()];
    while (waiting.length) {
      const later = [];
      for (const offset of waiting) {
        const h = this.headers.get(offset);
        if (h.type === REF_DELTA && !this.at.has(h.base) && typeof h.base === "string") {
          later.push(offset);
          continue;
        }
        const { type, data } = this.read(offset);
        const head = new TextEncoder().encode(`${type} ${data.length}\0`);
        const whole = new Uint8Array(head.length + data.length);
        whole.set(head);
        whole.set(data, head.length);
        this.at.set(hex(new Uint8Array(await crypto.subtle.digest("SHA-1", whole))), offset);
      }
      if (later.length === waiting.length) throw new Error("a pack is thin: a delta's base isn't in it");
      waiting = later;
    }
    return this;
  }

  /// The object at `offset`: { type, data }.
  read(offset) {
    const hit = this.recent.get(offset);
    if (hit) {
      this.recent.delete(offset);
      this.recent.set(offset, hit);
      return hit;
    }
    const h = this.headers.get(offset);
    const [raw] = inflate(this.bytes, h.data, h.size);
    let obj;
    if (h.type === OFS_DELTA || h.type === REF_DELTA) {
      const base = this.read(h.type === OFS_DELTA ? h.base : this.at.get(h.base));
      obj = { type: base.type, data: patch(base.data, raw) };
    } else {
      obj = { type: TYPES[h.type], data: raw };
    }
    this.recent.set(offset, obj);
    this.held += obj.data.length;
    for (const [k, v] of this.recent) {
      if (this.held < 32e6) break;
      this.recent.delete(k);
      this.held -= v.data.length;
    }
    return obj;
  }
}

const packs = new Map(); // blob hash → Promise<Pack>

function pack(ctx, hash) {
  if (!packs.has(hash)) {
    const p = ctx.blob.get(hash).then((bytes) => new Pack(bytes).index());
    p.catch(() => packs.delete(hash));
    packs.set(hash, p);
  }
  return packs.get(hash);
}

/// A repo's objects, from the packs that hold them (blob hashes).
export async function objects(ctx, hashes) {
  const all = await Promise.all(hashes.map((h) => pack(ctx, h)));
  const parsed = new Map();
  const raw = (sha) => {
    for (const p of all) {
      const offset = p.at.get(sha);
      if (offset !== undefined) return p.read(offset);
    }
    throw new Error(`There's no ${sha.slice(0, 12)} in this repo`);
  };
  return {
    has: (sha) => all.some((p) => p.at.has(sha)),
    raw,
    commit(sha) {
      if (parsed.has(sha)) return parsed.get(sha);
      const { type, data } = raw(sha);
      if (type !== "commit") throw new Error(`${sha.slice(0, 12)} isn't a commit`);
      const c = parseCommit(sha, utf8.decode(data));
      parsed.set(sha, c);
      return c;
    },
    tree(sha) {
      if (parsed.has(sha)) return parsed.get(sha);
      const { type, data } = raw(sha);
      if (type !== "tree") throw new Error(`${sha.slice(0, 12)} isn't a folder`);
      const t = parseTree(data);
      parsed.set(sha, t);
      return t;
    },
    blob(sha) {
      const { data } = raw(sha);
      return data;
    },
  };
}

const sha1 = async (bytes) => new Uint8Array(await crypto.subtle.digest("SHA-1", bytes));

/// A pack someone pushed, checked whole and never thin: its name (its checksum, as git names packs) and each object's
/// hash, CRC and offset, for its index.
export async function check(bytes) {
  if (bytes.length < 32) throw new Error("a pack is damaged");
  if (hex(await sha1(bytes.subarray(0, -20))) !== hex(bytes.subarray(-20))) throw new Error("a pack is damaged: its checksum is wrong");
  const p = await new Pack(bytes).index();
  const starts = [...p.headers.keys()];
  const ends = new Map(starts.map((s, i) => [s, starts[i + 1] ?? bytes.length - 20]));
  const entries = [...p.at].map(([sha, offset]) => ({ sha, offset, crc: crc32(0, bytes.subarray(offset, ends.get(offset)), ends.get(offset) - offset, 0) >>> 0 }));
  return { name: hex(bytes.subarray(-20)), entries };
}

/// Packs one after another as one pack, with each object's place in it. Each is whole (no delta reaches outside it),
/// and a delta's base is a distance back within its own pack, so their objects are copied as they are.
export async function merge(packs) {
  const total = 12 + packs.reduce((n, p) => n + p.bytes.length - 32, 0) + 20;
  const out = new Uint8Array(total);
  out.set(packs[0].bytes.subarray(0, 8));
  let at = 12, count = 0;
  const entries = [];
  for (const p of packs) {
    const view = new DataView(p.bytes.buffer, p.bytes.byteOffset);
    count += view.getUint32(8);
    out.set(p.bytes.subarray(12, -20), at);
    for (const e of p.entries) entries.push({ ...e, offset: e.offset - 12 + at });
    at += p.bytes.length - 32;
  }
  new DataView(out.buffer).setUint32(8, count);
  out.set(await sha1(out.subarray(0, at)), at);
  return { bytes: out, name: hex(out.subarray(at)), entries };
}

/// A pack's index file (git's version 2), from its objects; an object in it twice is listed twice, as git does.
export async function idx(entries, name) {
  const rows = [...entries].sort((a, b) => (a.sha < b.sha ? -1 : a.sha > b.sha ? 1 : a.offset - b.offset));
  const large = rows.filter((r) => r.offset >= 0x80000000);
  const n = rows.length;
  const out = new Uint8Array(8 + 1024 + n * 28 + large.length * 8 + 40);
  const view = new DataView(out.buffer);
  out.set([0xff, 0x74, 0x4f, 0x63, 0, 0, 0, 2]);
  const fan = new Uint32Array(256);
  for (const r of rows) fan[parseInt(r.sha.slice(0, 2), 16)]++;
  for (let i = 0, acc = 0; i < 256; i++) view.setUint32(8 + 4 * i, (acc += fan[i]));
  let at = 8 + 1024;
  for (const r of rows) for (let i = 0; i < 20; i++) out[at++] = parseInt(r.sha.slice(2 * i, 2 * i + 2), 16);
  for (const r of rows) { view.setUint32(at, r.crc); at += 4; }
  let big = 0;
  for (const r of rows) { view.setUint32(at, r.offset >= 0x80000000 ? 0x80000000 | big++ : r.offset); at += 4; }
  for (const r of large) { view.setBigUint64(at, BigInt(r.offset)); at += 8; }
  for (let i = 0; i < 20; i++) out[at++] = parseInt(name.slice(2 * i, 2 * i + 2), 16);
  out.set(await sha1(out.subarray(0, at)), at);
  return out;
}

/// Someone and when, from `Name <email> 1700000000 +0100`.
function person(line) {
  const m = /^(.*?) <([^>]*)> (\d+) ([+-]\d{4})$/.exec(line || "");
  return m ? { name: m[1], email: m[2], time: Number(m[3]) * 1000, tz: m[4] } : { name: line || "", email: "", time: 0, tz: "+0000" };
}

function parseCommit(sha, text) {
  const split = text.indexOf("\n\n");
  const head = split < 0 ? text : text.slice(0, split);
  const message = split < 0 ? "" : text.slice(split + 2);
  const c = { sha, tree: "", parents: [], author: null, committer: null, message };
  for (const line of head.split("\n")) {
    const sp = line.indexOf(" ");
    const key = line.slice(0, sp), value = line.slice(sp + 1);
    if (key === "tree") c.tree = value;
    else if (key === "parent") c.parents.push(value);
    else if (key === "author") c.author = person(value);
    else if (key === "committer") c.committer = person(value);
  }
  c.author ||= c.committer || person("");
  c.committer ||= c.author;
  return c;
}

function parseTree(data) {
  const out = [];
  for (let i = 0; i < data.length; ) {
    const sp = data.indexOf(32, i), nul = data.indexOf(0, sp);
    const mode = utf8.decode(data.subarray(i, sp));
    const name = utf8.decode(data.subarray(sp + 1, nul));
    const sha = hex(data.subarray(nul + 1, nul + 21));
    const type = mode === "40000" ? "tree" : mode === "160000" ? "commit" : "blob";
    out.push({ mode: mode.padStart(6, "0"), name, sha, type });
    i = nul + 21;
  }
  return out;
}
