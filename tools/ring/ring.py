# Makes the ring art: icons/AppIcon.png, icons/MenuBarIcon.png, icons/MenuBarIcon@2x.png and docs/ring.png.
# Run: uv run --with numpy --with pillow tools/ring/ring.py <folder to write the four files to>
# (about three minutes: it raymarches the ring at 2800 px and shrinks it).
import os, sys
import numpy as np
from PIL import Image, ImageDraw, ImageFilter

RES, OUT = 2800, 1024          # the ring is traced at RES px and shrunk to OUT
ANGLES = (-52, 22, -30)        # tilt, yaw, roll of the ring in degrees
GOLD = np.array([1.0, 0.62, 0.18])
CONTRAST = 1.6                 # strength of the studio's horizon streak and dark band
BAND_THICKNESS, BAND_WIDTH = 0.13, 0.36   # of a ring of radius 1


def inscription():
    """The strip of line icons wrapped round the ring, as a grey image: white is the glow."""
    cell, height, lw = 300, 400, 17    # icon cell width, strip height (= band width), line width
    order = ["laptop", "robot", "phone", "server", "apps", "terminal", "monitor", "chat", "robot", "cloud", "laptop", "apps", "phone", "terminal", "robot"]
    img = Image.new("L", (cell * len(order), height), 0); d = ImageDraw.Draw(img)
    def box(x0, y0, x1, y1, r=18): d.rounded_rectangle((x0, y0, x1, y1), r, outline=255, width=lw)
    for i, g in enumerate(order):
        cx, cy = i * cell + cell // 2, height // 2
        if g == "laptop":
            box(cx - 95, cy - 95, cx + 95, cy + 40, 14); d.line((cx - 135, cy + 85, cx + 135, cy + 85), fill=255, width=lw)
        elif g == "monitor":
            box(cx - 110, cy - 110, cx + 110, cy + 45, 14); d.line((cx, cy + 45, cx, cy + 95), fill=255, width=lw); d.line((cx - 60, cy + 105, cx + 60, cy + 105), fill=255, width=lw)
        elif g == "phone":
            box(cx - 62, cy - 125, cx + 62, cy + 125, 28); d.line((cx - 22, cy + 90, cx + 22, cy + 90), fill=255, width=lw)
        elif g == "server":
            box(cx - 105, cy - 120, cx + 105, cy - 10, 12); box(cx - 105, cy + 10, cx + 105, cy + 120, 12)
            for yy in (cy - 65, cy + 65): d.ellipse((cx - 72, yy - 14, cx - 44, yy + 14), fill=255)
        elif g == "apps":
            for dx in (-62, 62):
                for dy in (-62, 62): box(cx + dx - 48, cy + dy - 48, cx + dx + 48, cy + dy + 48, 22)
        elif g == "robot":
            box(cx - 100, cy - 70, cx + 100, cy + 105, 34)
            d.line((cx, cy - 70, cx, cy - 115), fill=255, width=lw); d.ellipse((cx - 20, cy - 145, cx + 20, cy - 105), fill=255)
            for ex in (-45, 45): d.ellipse((cx + ex - 20, cy - 5, cx + ex + 20, cy + 35), fill=255)
            for ex in (-1, 1): d.line((cx + ex * 100, cy + 5, cx + ex * 125, cy + 5, cx + ex * 125, cy + 55), fill=255, width=lw)
        elif g == "terminal":
            box(cx - 120, cy - 105, cx + 120, cy + 105, 18)
            d.line((cx - 70, cy - 40, cx - 20, cy, cx - 70, cy + 40), fill=255, width=lw, joint="curve"); d.line((cx + 5, cy + 45, cx + 70, cy + 45), fill=255, width=lw)
        elif g == "chat":
            box(cx - 120, cy - 100, cx + 120, cy + 55, 40); d.polygon([(cx - 60, cy + 45), (cx - 85, cy + 120), (cx - 10, cy + 50)], fill=255)
        elif g == "cloud":
            parts = ((-55, 25, 60), (25, -10, 80), (85, 30, 52))
            for k, col in ((0, 255), (lw, 0)):
                for (ex, ey, er) in parts: d.ellipse((cx + ex - er + k, cy + ey - er + k, cx + ex + er - k, cy + ey + er - k), fill=col)
                d.rectangle((cx - 55, cy + 30 + k, cx + 85, cy + 82 - k), fill=col)
    return img


def render(strip_img):
    """The raymarched gold ring with the inscription glowing on it. Returns (gold, template): the
    colour ring, and the menu bar's template, black with alpha, where the inscription is cut out."""
    N, R, T, W = RES, 1.0, BAND_THICKNESS, BAND_WIDTH
    def rot(ax, ay, az):
        ax, ay, az = np.radians([ax, ay, az]); cx, sx, cy, sy, cz, sz = np.cos(ax), np.sin(ax), np.cos(ay), np.sin(ay), np.cos(az), np.sin(az)
        return np.array([[cz,-sz,0],[sz,cz,0],[0,0,1]]) @ np.array([[cy,0,sy],[0,1,0],[-sy,0,cy]]) @ np.array([[1,0,0],[0,cx,-sx],[0,sx,cx]])
    Minv = rot(*ANGLES).T
    def sdf(p):   # distance to a torus with an elliptical section
        o = p @ Minv.T; r = np.sqrt(o[...,0]**2 + o[...,1]**2); q = np.stack([r - R, o[...,2]], -1); b = np.array([T, W])
        k0 = np.linalg.norm(q / b, axis=-1); k1 = np.linalg.norm(q / b**2, axis=-1); return k0 * (k0 - 1) / np.maximum(k1, 1e-6)
    s = 1.42; xs = np.linspace(-s, s, N); X, Y = np.meshgrid(xs, -xs)
    P = np.stack([X, Y, np.full_like(X, 3.0)], -1); D = np.array([0, 0, -1.0])
    t = np.zeros(X.shape); hit = np.zeros(X.shape, bool)
    for _ in range(200):
        d = sdf(P + t[..., None] * D); hit |= d < 1e-4; t = np.where(hit, t, t + d * 0.85)
    hit &= t < 6
    p = P + t[..., None] * D; e = 1e-4
    n = np.stack([sdf(p + v) - sdf(p - v) for v in np.eye(3) * e], -1); n /= np.linalg.norm(n, axis=-1, keepdims=True) + 1e-9
    V = np.array([0, 0, 1.0]); rd = -V + 2 * (n @ V)[..., None] * n
    def soft(x, c, w): return np.exp(-((x - c) / w) ** 2)
    def env(r):  # dark studio, a big key softbox above, strip lights at the sides, a warm floor bounce
        x, y, z = r[..., 0], r[..., 1], r[..., 2]
        e = 0.06 + 0.07 * np.clip(y, 0, 1)
        e += 2.4 * soft(y, 0.78, 0.16) * soft(x, -0.15, 0.55)
        e += 1.3 * soft(x, -0.82, 0.07) * np.clip(1 - np.abs(y) * 0.8, 0, 1)
        e += 0.9 * soft(x, 0.86, 0.05) * np.clip(1 - np.abs(y - 0.1), 0, 1)
        e += 0.35 * soft(y, -0.75, 0.25)
        e += 0.25 * soft(z, 0.9, 0.2)
        e += CONTRAST * 1.6 * soft(y, 0.28, 0.035)                      # thin horizon light: a bright streak along the band
        e *= 1 - CONTRAST * 0.7 * soft(y, 0.08, 0.12)                    # dark band under it, for polished contrast
        e += CONTRAST * 0.8 * soft(x, 0.35, 0.04) * soft(y, 0.45, 0.3)
        return e
    cosv = np.clip(n @ V, 0, 1)
    fres = GOLD + (1 - GOLD) * (1 - cosv[..., None]) ** 5
    col = fres * env(rd)[..., None]
    L = np.array([-0.3, 0.8, 0.5]); L /= np.linalg.norm(L); Hh = (L + V) / np.linalg.norm(L + V)
    col += np.array([1.0, 0.9, 0.7]) * (np.clip(n @ Hh, 0, 1) ** 220 * 2.2)[..., None]
    o = p @ Minv.T; no = n @ Minv.T; r = np.sqrt(o[...,0]**2 + o[...,1]**2) + 1e-9
    inner = (no[..., 0] * o[..., 0] + no[..., 1] * o[..., 1]) / r < -0.2
    col += (inner * 0.2)[..., None] * np.array([1.0, 0.45, 0.1])                # the warm light inside the band
    strip = np.asarray(strip_img, float) / 255
    SH, SW = strip.shape; th = np.arctan2(o[...,1], o[...,0])
    u = (((th - np.radians(-60)) % (2 * np.pi)) / (2 * np.pi) * SW).astype(int) % SW
    span = 2 * np.pi * R * SH / SW
    vv = np.nan_to_num(-o[..., 2] / span + 0.5, nan=-1.0) * SH
    v = np.clip(vv.astype(int), 0, SH - 1); ok = hit & (vv >= 0) & (vv < SH)
    glow = np.where(ok, strip[v, u], 0.0) * np.where(inner, 0.85, 1.0)
    blur = lambda a, k: np.asarray(Image.fromarray((np.clip(a, 0, 1) * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(N * k)), float) / 255
    engr = blur(glow, 1 / 500)
    col *= (1 - 0.55 * np.clip(engr * 1.6, 0, 1))[..., None]
    core = np.clip((glow - 0.5) * 2, 0, 1)
    fire = np.array([1.0, 0.36, 0.04]) * (1 - core[..., None]) + np.array([1.0, 0.86, 0.48]) * core[..., None]
    col = col * (1 - glow[..., None]) + glow[..., None] * fire * 2.4
    halo = blur(glow, 1 / 220) * 1.4 + blur(glow, 1 / 70) * 0.9
    col += (hit * (1 - glow))[..., None] * halo[..., None] * np.array([1.0, 0.38, 0.05])
    col = col / (1 + col * 0.35)                                                # soft highlight rolloff
    col = np.clip(col, 0, 1) ** (1 / 2.0)
    rgba = np.dstack([(col * 255).astype(np.uint8), (hit * 255).astype(np.uint8)])
    gold = Image.fromarray(rgba).resize((OUT, OUT), Image.LANCZOS)
    a = hit * np.where(inner, 0.5, 1.0) * np.where(glow > 0.3, 0.2, 1.0)
    tmpl = Image.fromarray(np.dstack([np.zeros((N, N, 3), np.uint8), (a * 255).astype(np.uint8)])).resize((OUT, OUT), Image.LANCZOS)
    return gold, tmpl


def app_icon(ring):
    """The ring on a dark macOS-shaped tile."""
    ring = ring.crop(ring.getbbox())
    S = 1024; icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    inset = 100; size = S - 2 * inset                                # Apple's grid: the shape sits inside the canvas
    yy, xx = np.mgrid[0:size, 0:size] / size
    bg = np.dstack([0.07 + 0.0 * xx, 0.07 + 0.0 * xx, 0.08 + 0.0 * xx])
    g = np.exp(-(((xx - 0.5) ** 2 + (yy - 0.52) ** 2) / 0.09))      # a faint glow behind the ring
    bg = bg + g[..., None] * np.array([0.05, 0.05, 0.055])
    bg = bg * (1.0 - 0.25 * yy[..., None])
    tile = Image.fromarray((np.clip(bg, 0, 1) * 255).astype(np.uint8)).convert("RGBA")
    mask = Image.new("L", (size, size), 0); ImageDraw.Draw(mask).rounded_rectangle((0, 0, size - 1, size - 1), int(size * 0.225), fill=255)
    icon.paste(tile, (inset, inset), mask)
    r = ring.copy(); r.thumbnail((int(size * 0.74), int(size * 0.74)), Image.LANCZOS)
    shadow = Image.new("RGBA", r.size, (0, 0, 0, 0)); shadow.putalpha(r.split()[3].point(lambda a: a * 0.6))
    shadow = shadow.filter(ImageFilter.GaussianBlur(18))
    ox, oy = (S - r.width) // 2, (S - r.height) // 2
    layer = Image.new("RGBA", (S, S)); layer.alpha_composite(shadow, (ox + 6, oy + 22)); layer.alpha_composite(r, (ox, oy))
    icon.alpha_composite(layer)
    return icon


def menu_bar_icons(tmpl):
    """The menu bar's template image, at 18 and 36 px."""
    t = tmpl.crop(tmpl.getbbox())
    side = max(t.size); sq = Image.new("RGBA", (side, side)); sq.paste(t, ((side - t.width) // 2, (side - t.height) // 2))
    return {"MenuBarIcon.png": sq.resize((18, 18), Image.LANCZOS), "MenuBarIcon@2x.png": sq.resize((36, 36), Image.LANCZOS)}


if __name__ == "__main__":
    folder = sys.argv[1]
    os.makedirs(folder, exist_ok=True)
    gold, tmpl = render(inscription())
    app_icon(gold).save(f"{folder}/AppIcon.png")
    for name, img in menu_bar_icons(tmpl).items(): img.save(f"{folder}/{name}")
    ring = gold.crop(gold.getbbox()); ring.thumbnail((640, 640), Image.LANCZOS); ring.save(f"{folder}/ring.png", optimize=True)
