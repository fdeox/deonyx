# Draws the plugin's images from ui/logo.svg: the plugin and category icons,
# the action list icons (white on transparent) and the key images.
# Run from this folder: python make-images.py

import math
import re
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

HERE = Path(__file__).parent
OUT = HERE / "com.fdeox.deonyx.sdPlugin" / "imgs"
LOGO = (HERE.parent / "ui" / "logo.svg").read_text(encoding="utf8")

BG = (17, 17, 22, 255)
TEXT = (236, 236, 243, 255)
ACCENT = (139, 124, 246, 255)
ACCENT_2 = (169, 156, 255, 255)
LIGHT = (228, 228, 237, 255)
DARK = (22, 22, 30, 255)
WHITE = (255, 255, 255, 255)
MUTED = (108, 108, 128, 255)


def rgba(hex_color, opacity=1.0):
    h = hex_color.lstrip("#")
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), round(255 * opacity))


def logo(size):
    """The logo, drawn from its SVG (polygons, rects, star paths, dots)."""
    k = size * 4 / 200
    big = size * 4
    pts = lambda s: [(float(x) * k, float(y) * k) for x, y in re.findall(r"(-?[\d.]+)[ ,](-?[\d.]+)", s)]

    def layer(body, fill_default=None):
        im = Image.new("RGBA", (big, big), (0, 0, 0, 0))
        d = ImageDraw.Draw(im)
        for tag in re.finditer(r"<(polygon|rect|path|circle)([^>]*)/>", body):
            kind, attrs = tag.groups()
            a = dict(re.findall(r'([\w-]+)="([^"]*)"', attrs))
            color = a.get("fill") or fill_default
            if not color or color == "none":
                continue
            c = rgba(color, float(a.get("opacity", 1)))
            if kind == "polygon":
                d.polygon(pts(a["points"]), fill=c)
            elif kind == "rect":
                x, y, w, h = (float(a[n]) * k for n in ("x", "y", "width", "height"))
                d.rectangle([x, y, x + w, y + h], fill=c)
            elif kind == "path":
                d.polygon(pts(a["d"].replace("M", " ").replace("L", " ").replace("Z", " ")), fill=c)
            elif kind == "circle":
                cx, cy, r = (float(a[n]) * k for n in ("cx", "cy", "r"))
                d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=c)
        return im

    def half(left):
        m = Image.new("L", (big, big), 0)
        ImageDraw.Draw(m).rectangle([0, 0, big / 2, big] if left else [big / 2, 0, big, big], fill=255)
        return m

    gem_pts = pts(re.search(r'id="dx-gem">\s*<polygon points="([^"]+)"', LOGO).group(1))
    gem = Image.new("L", (big, big), 0)
    ImageDraw.Draw(gem).polygon(gem_pts, fill=255)

    groups = re.findall(r'<g clip-path="url\(#dx-(left|right)\)"([^>]*)>(.*?)</g>', LOGO, re.S)
    out = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    for side, attrs, body in groups:
        fill = re.search(r'fill="(#[0-9a-f]{6})"', attrs)
        im = layer(body, fill.group(1) if fill else None)
        im.putalpha(ImageChops.multiply(im.getchannel("A"), half(side == "left")))
        out = Image.alpha_composite(out, im)
    line = re.search(r'<rect x="98.5"[^>]*/>', LOGO).group(0)
    out = Image.alpha_composite(out, layer(line))
    out.putalpha(ImageChops.multiply(out.getchannel("A"), gem))
    ImageDraw.Draw(out).line(gem_pts + gem_pts[:2], fill=ACCENT, width=round(6 * k), joint="curve")
    return out.resize((size, size), Image.LANCZOS)


def canvas(size, bg=None):
    return Image.new("RGBA", (size * 4, size * 4), bg or (0, 0, 0, 0))


def done(im, size):
    return im.resize((size, size), Image.LANCZOS)


# Glyphs, drawn on a 4x canvas of the given size, inside the box (x, y, w).

def g_pause(d, x, y, w, color):
    bw = w * 0.24
    for bx in (x + w * 0.18, x + w * 0.58):
        d.rounded_rectangle([bx, y + w * 0.12, bx + bw, y + w * 0.88], radius=bw * 0.3, fill=color)


def g_play(d, x, y, w, color):
    d.polygon([(x + w * 0.24, y + w * 0.12), (x + w * 0.24, y + w * 0.88), (x + w * 0.88, y + w * 0.5)], fill=color)


def g_window(d, x, y, w, color, split=False):
    t = max(2, w * 0.08)
    box = [x + w * 0.06, y + w * 0.14, x + w * 0.94, y + w * 0.86]
    if split:
        d.rounded_rectangle(box, radius=w * 0.08, fill=LIGHT)
        d.rounded_rectangle([box[0], box[1], (box[0] + box[2]) / 2, box[3]], radius=w * 0.08, fill=DARK)
        d.rectangle([(box[0] + box[2]) / 2 - w * 0.02, box[1] + w * 0.02, (box[0] + box[2]) / 2, box[3] - w * 0.02], fill=DARK)
        d.line([((box[0] + box[2]) / 2, box[1] + t / 2), ((box[0] + box[2]) / 2, box[3] - t / 2)], fill=ACCENT_2, width=round(t * 0.6))
    d.rounded_rectangle(box, radius=w * 0.08, outline=color, width=round(t))
    d.line([(box[0], box[1] + w * 0.2), (box[2], box[1] + w * 0.2)], fill=color, width=round(t))


def g_apps(d, x, y, w, color, split=False):
    s = w * 0.36
    gap = w * 0.12
    for i, (cx, cy) in enumerate([(0, 0), (1, 0), (0, 1), (1, 1)]):
        bx, by = x + w * 0.08 + cx * (s + gap), y + w * 0.08 + cy * (s + gap)
        fill = (DARK if i == 3 else LIGHT) if split else None
        d.rounded_rectangle([bx, by, bx + s, by + s], radius=s * 0.25, fill=fill, outline=color, width=round(max(2, w * 0.07)))


def g_layers(d, x, y, w, color, fills=None):
    for i in range(3):
        cy = y + w * (0.30 + i * 0.20)
        rh = w * 0.18
        poly = [(x + w * 0.5, cy - rh), (x + w * 0.92, cy), (x + w * 0.5, cy + rh), (x + w * 0.08, cy)]
        f = fills[i] if fills else None
        d.polygon(poly, fill=f, outline=None)
        d.line(poly + poly[:1], fill=color, width=round(max(2, w * 0.07)), joint="curve")


def g_gem(d, x, y, w, color):
    p = [(64, 40), (136, 40), (182, 82), (100, 184), (18, 82)]
    poly = [(x + px / 200 * w, y + (py - 12) / 200 * w) for px, py in p]
    d.line(poly + poly[:2], fill=color, width=round(max(2, w * 0.08)), joint="curve")
    d.line([(x + w * 0.5, y + w * 0.14), (x + w * 0.5, y + w * 0.86)], fill=color, width=round(max(2, w * 0.06)))


def list_icon(name, draw):
    for size, suffix in ((20, ""), (40, "@2x")):
        im = canvas(size)
        draw(ImageDraw.Draw(im), size * 4 * 0.04, size * 4 * 0.04, size * 4 * 0.92)
        done(im, size).save(OUT / "actions" / f"{name}{suffix}.png")


def key(name, draw):
    for size, suffix in ((72, ""), (144, "@2x")):
        im = canvas(size, BG)
        draw(im, ImageDraw.Draw(im), size * 4)
        done(im, size).save(OUT / "keys" / f"{name}{suffix}.png")


def paste_logo(im, s, scale, dim=False, dy=0.0):
    w = round(s * scale)
    l = logo(w)
    if dim:
        gray = l.convert("LA").convert("RGBA")
        gray.putalpha(l.getchannel("A").point(lambda a: a * 0.38))
        l = gray
    im.alpha_composite(l, (round((s - w) / 2), round((s - w) / 2 + s * dy)))


def main():
    for sub in ("plugin", "actions", "keys"):
        (OUT / sub).mkdir(parents=True, exist_ok=True)

    logo(256).save(OUT / "plugin" / "icon.png")
    logo(512).save(OUT / "plugin" / "icon@2x.png")
    for size, suffix in ((28, ""), (56, "@2x")):
        im = canvas(size)
        g_gem(ImageDraw.Draw(im), size * 4 * 0.06, size * 4 * 0.06, size * 4 * 0.88, WHITE)
        done(im, size).save(OUT / "plugin" / f"category{suffix}.png")

    list_icon("toggle", lambda d, x, y, w: g_gem(d, x, y, w, WHITE))
    list_icon("pause", lambda d, x, y, w: g_pause(d, x, y, w, WHITE))
    list_icon("window", lambda d, x, y, w: g_window(d, x, y, w, WHITE))
    list_icon("app", lambda d, x, y, w: g_apps(d, x, y, w, WHITE))
    list_icon("profile", lambda d, x, y, w: g_layers(d, x, y, w, WHITE))

    key("on", lambda im, d, s: paste_logo(im, s, 0.62, dy=-0.06))
    key("off", lambda im, d, s: paste_logo(im, s, 0.62, dim=True, dy=-0.06))
    key("pause", lambda im, d, s: g_pause(d, s * 0.3, s * 0.22, s * 0.4, TEXT))
    key("paused", lambda im, d, s: g_play(d, s * 0.32, s * 0.22, s * 0.4, ACCENT_2))
    key("window", lambda im, d, s: g_window(d, s * 0.24, s * 0.16, s * 0.52, TEXT, split=True))
    key("app", lambda im, d, s: g_apps(d, s * 0.27, s * 0.17, s * 0.46, TEXT, split=True))
    key("app-off", lambda im, d, s: g_apps(d, s * 0.27, s * 0.17, s * 0.46, MUTED))
    key("profile", lambda im, d, s: g_layers(d, s * 0.24, s * 0.12, s * 0.52, MUTED, fills=[DARK, DARK, DARK]))
    key("profile-on", lambda im, d, s: g_layers(d, s * 0.24, s * 0.12, s * 0.52, ACCENT_2, fills=[DARK, DARK, ACCENT]))
    print("images written to", OUT)


if __name__ == "__main__":
    main()
