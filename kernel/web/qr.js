// A QR code of a short text, as an SVG: byte mode, error correction level M, versions 1 to 6 (up to 106 bytes), which
// is what a link to approve a device needs. Written here rather than taken as a dependency, as it's this small.

// Each version's codewords in all, error correction codewords per block, and blocks, at level M.
const VERSIONS = [null, [26, 10, 1], [44, 16, 1], [70, 26, 1], [100, 18, 2], [134, 24, 2], [172, 16, 4]];

const EXP = new Uint8Array(512), LOG = new Uint8Array(256);
for (let i = 0, x = 1; i < 255; i++, x = x & 0x80 ? (x << 1) ^ 0x11d : x << 1) (EXP[i] = x), (LOG[x] = i);
for (let i = 255; i < 512; i++) EXP[i] = EXP[i - 255];
const mul = (a, b) => (a && b ? EXP[LOG[a] + LOG[b]] : 0);

/// Reed-Solomon's error correction codewords of `data`, `n` of them.
function correction(data, n) {
  let g = [1];
  for (let i = 0; i < n; i++) {
    const next = new Array(g.length + 1).fill(0);
    g.forEach((c, j) => ((next[j] ^= c), (next[j + 1] ^= mul(c, EXP[i]))));
    g = next;
  }
  const r = [...data, ...new Array(n).fill(0)];
  for (let i = 0; i < data.length; i++) if (r[i]) for (let j = 0, c = r[i]; j < g.length; j++) r[i + j] ^= mul(g[j], c);
  return r.slice(data.length);
}

/// The codewords of `bytes` in version `v`, blocks interleaved.
function codewords(bytes, v) {
  const [total, ec, blocks] = VERSIONS[v];
  const capacity = total - ec * blocks;
  const bits = [];
  const put = (value, n) => { for (let i = n - 1; i >= 0; i--) bits.push((value >>> i) & 1); };
  put(4, 4);
  put(bytes.length, 8);
  bytes.forEach((b) => put(b, 8));
  put(0, Math.min(4, capacity * 8 - bits.length));
  while (bits.length % 8) bits.push(0);
  const data = [];
  for (let i = 0; i < bits.length; i += 8) data.push(bits.slice(i, i + 8).reduce((a, b) => (a << 1) | b, 0));
  for (let pad = 0xec; data.length < capacity; pad ^= 0xec ^ 0x11) data.push(pad);
  const size = capacity / blocks;
  const split = Array.from({ length: blocks }, (_, b) => data.slice(b * size, (b + 1) * size));
  const fixes = split.map((d) => correction(d, ec));
  const out = [];
  for (let i = 0; i < size; i++) split.forEach((d) => out.push(d[i]));
  for (let i = 0; i < ec; i++) fixes.forEach((f) => out.push(f[i]));
  return out;
}

const MASKS = [
  (x, y) => (x + y) % 2 === 0, (x, y) => y % 2 === 0, (x) => x % 3 === 0, (x, y) => (x + y) % 3 === 0,
  (x, y) => (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0, (x, y) => ((x * y) % 2) + ((x * y) % 3) === 0,
  (x, y) => (((x * y) % 2) + ((x * y) % 3)) % 2 === 0, (x, y) => (((x + y) % 2) + ((x * y) % 3)) % 2 === 0,
];

/// The modules of a QR code of `text`: rows of booleans, dark true.
export function qr(text) {
  const bytes = [...new TextEncoder().encode(text)];
  const v = VERSIONS.findIndex((t) => t && t[0] - t[1] * t[2] - 2 >= bytes.length);
  if (v < 1) throw new Error("too long for a QR code here");
  const size = 17 + 4 * v;
  const dark = Array.from({ length: size }, () => new Array(size).fill(false));
  const fixed = Array.from({ length: size }, () => new Array(size).fill(false));
  const set = (x, y, on) => ((dark[y][x] = on), (fixed[y][x] = true));
  for (let i = 0; i < size; i++) set(6, i, i % 2 === 0), set(i, 6, i % 2 === 0);
  for (const [cx, cy] of [[3, 3], [size - 4, 3], [3, size - 4]]) {
    for (let dy = -4; dy <= 4; dy++) for (let dx = -4; dx <= 4; dx++) {
      const x = cx + dx, y = cy + dy, d = Math.max(Math.abs(dx), Math.abs(dy));
      if (x >= 0 && x < size && y >= 0 && y < size) set(x, y, d !== 2 && d !== 4);
    }
  }
  if (v > 1) for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) set(size - 7 + dx, size - 7 + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
  const format = (mask) => {
    const data = mask; // level M's bits are 00
    let rem = data;
    for (let i = 0; i < 10; i++) rem = (rem << 1) ^ ((rem >>> 9) * 0x537);
    const bits = ((data << 10) | rem) ^ 0x5412;
    const bit = (i) => ((bits >>> i) & 1) === 1;
    for (let i = 0; i <= 5; i++) set(8, i, bit(i));
    set(8, 7, bit(6)), set(8, 8, bit(7)), set(7, 8, bit(8));
    for (let i = 9; i < 15; i++) set(14 - i, 8, bit(i));
    for (let i = 0; i < 8; i++) set(size - 1 - i, 8, bit(i));
    for (let i = 8; i < 15; i++) set(8, size - 15 + i, bit(i));
    set(8, size - 8, true);
  };
  format(0);
  const words = codewords(bytes, v);
  let i = 0;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right = 5;
    for (let vert = 0; vert < size; vert++) for (let j = 0; j < 2; j++) {
      const x = right - j, y = ((right + 1) & 2) === 0 ? size - 1 - vert : vert;
      if (!fixed[y][x] && i < words.length * 8) (dark[y][x] = ((words[i >>> 3] >>> (7 - (i & 7))) & 1) === 1), i++;
    }
  }
  // The mask that scans best: least penalty, as the standard counts it.
  const masked = (m) => dark.map((row, y) => row.map((on, x) => (fixed[y][x] ? on : on !== MASKS[m](x, y))));
  let best = null;
  for (let m = 0; m < 8; m++) {
    format(m);
    const grid = masked(m);
    const score = penalty(grid);
    if (!best || score < best.score) best = { m, score };
  }
  format(best.m);
  return masked(best.m);
}

function penalty(grid) {
  const n = grid.length;
  let score = 0, darkCount = 0;
  const lines = [...grid, ...grid.map((_, x) => grid.map((row) => row[x]))];
  for (const line of lines) {
    for (let i = 0, run = 1; i < n; i++, run++) {
      if (i === n - 1 || line[i] !== line[i + 1]) {
        if (run >= 5) score += run - 2;
        run = 0;
      }
    }
    const s = line.map((on) => (on ? 1 : 0)).join("");
    score += 40 * ((s.match(/(?=10111010000|00001011101)/g) || []).length);
  }
  for (let y = 0; y < n - 1; y++) for (let x = 0; x < n - 1; x++) {
    const c = grid[y][x];
    if (c === grid[y][x + 1] && c === grid[y + 1][x] && c === grid[y + 1][x + 1]) score += 3;
  }
  grid.forEach((row) => row.forEach((on) => (darkCount += on)));
  return score + Math.floor(Math.abs(darkCount * 20 - n * n * 10) / (n * n)) * 10;
}

/// A QR code of `text` as an SVG, with its quiet zone, drawn in the current text colour on the background.
export function qrSvg(text) {
  const grid = qr(text);
  const n = grid.length + 8;
  let path = "";
  grid.forEach((row, y) => row.forEach((on, x) => on && (path += `M${x + 4} ${y + 4}h1v1h-1z`)));
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${n} ${n}" shape-rendering="crispEdges" role="img"><rect width="${n}" height="${n}" fill="#fff"/><path d="${path}" fill="#000"/></svg>`;
}
