# Draws assets/name.svg, the animated card that shows where the name comes
# from: fdeox, onyx and Nyx stack up, slide until their shared letters line
# up, fold into one word, and a legend says what each part means. Letters are
# glyph outlines from Segoe UI, so they line up the same on every machine.
# Loops every 15 s; with reduced motion only the final picture shows. "tall"
# makes the narrow version the website uses on phones.
#   python assets/make-name.py [out.svg] [tall]
import os
import re
import sys

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "assets", "name.svg")
# "tall": a narrow version for phones, the legend listed under the word.
TALL = len(sys.argv) > 2 and sys.argv[2] == "tall"
SEMI = TTFont(r"C:\Windows\Fonts\seguisb.ttf")
REG = TTFont(r"C:\Windows\Fonts\segoeui.ttf")

W, H = (560, 580) if TALL else (900, 320)
CYCLE = 15.0
TEXT = "#ececf3"
MUTED = "#8d8aa8"
ONYX = "#b4aed6"
VIOLET = "#a99cff"
ACCENT = "#8b7cf6"


class Font:
    def __init__(self, tt):
        self.tt = tt
        self.gs = tt.getGlyphSet()
        self.cmap = tt.getBestCmap()
        self.upm = tt["head"].unitsPerEm
        self.hmtx = tt["hmtx"]

    def adv(self, ch, size):
        return self.hmtx[self.cmap[ord(ch)]][0] * size / self.upm

    def width(self, s, size):
        return sum(self.adv(c, size) for c in s)

    def glyph(self, ch, x, y, size):
        """Path data for one character with its origin at (x, baseline y)."""
        name = self.cmap[ord(ch)]
        pen = SVGPathPen(self.gs)
        k = size / self.upm
        self.gs[name].draw(TransformPen(pen, (k, 0, 0, -k, x, y)))
        d = pen.getCommands()
        return re.sub(r"(\d+\.\d{2})\d+", r"\1", d)

    def text(self, s, x, y, size):
        out = []
        for ch in s:
            if ch != " ":
                out.append(self.glyph(ch, x, y, size))
            x += self.adv(ch, size)
        return " ".join(out)


semi, reg = Font(SEMI), Font(REG)
BIG = 76

# The final word and where each letter sits.
word = "Deonyx"
BASE = 290 if TALL else 180
X0 = round((W - semi.width(word, BIG)) / 2) if TALL else 262
cols, x = [], X0
for ch in word:
    cols.append((x, x + semi.adv(ch, BIG)))
    x += semi.adv(ch, BIG)
WORD_END = x


def col_center(i):
    return (cols[i][0] + cols[i][1]) / 2


def letter_at(ch, i):
    """A lowercase/uppercase letter centered in column i of the final word."""
    w = semi.adv(ch, BIG)
    return semi.glyph(ch, col_center(i) - w / 2, 0, BIG)


# Rows, drawn at baseline 0 and moved by CSS. Each row is laid out in its
# aligned place; the first frames shift it back to start at X0.
ROW_Y = [BASE - 74, BASE, BASE + 74]
# d sits right against e (centered in the wider D column it would leave a gap)
d_x = cols[0][1] - semi.adv("d", BIG)
row1_main = " ".join([semi.glyph("d", d_x, 0, BIG)] + [letter_at(c, i) for c, i in (("e", 1), ("o", 2))])
f_x = d_x - semi.adv("f", BIG)
x_x = col_center(2) + semi.adv("o", BIG) / 2
row1_dim = semi.glyph("f", f_x, 0, BIG) + " " + semi.glyph("x", x_x, 0, BIG)
row1_start = f_x
row2 = " ".join(letter_at(c, i) for c, i in (("o", 2), ("n", 3), ("y", 4), ("x", 5)))
row2_start = col_center(2) - semi.adv("o", BIG) / 2
row3 = " ".join(letter_at(c, i) for c, i in (("N", 3), ("y", 4), ("x", 5)))
row3_start = col_center(3) - semi.adv("N", BIG) / 2
final_word = semi.text(word, X0, BASE, BIG)

# Legend: a bracket under each part, a dotted leader, a label on the right.
LAB_X, LAB = 618, 21
LIST_X = X0 - 22
legend = [
    ("deo", "from fdeox, the maker", TEXT, cols[0][0], cols[2][1], BASE + 26),
    ("onyx", "the black gemstone", ONYX, cols[2][0], cols[5][1], BASE + 48),
    ("Nyx", "goddess of the night", VIOLET, cols[3][0], cols[5][1], BASE + 70),
]

# Gem from the logo, 150 px, left of the word.
GEM = 110 if TALL else 150
GEM_X, GEM_Y = ((W - GEM) / 2, BASE - 186) if TALL else (72, BASE - 100)
LIST_Y = [BASE + 132, BASE + 170, BASE + 208]
logo = open(os.path.join(REPO, "assets", "logo", "deonyx.svg"), encoding="utf8").read()
inner = logo[logo.index(">", logo.index("<svg")) + 1: logo.rindex("</svg>")]
inner = re.sub(r"<title>.*?</title>", "", inner, flags=re.S)
inner = inner.replace("dx-", "nm-")
# Stars twinkle: tag each star shape with a class.
star_i = [0]


def tag_star(m):
    star_i[0] += 1
    return m.group(0).replace("<", f'<g class="tw t{star_i[0] % 4}"><', 1) + "</g>"


inner = re.sub(r"<(path|circle) [^>]*?/>", lambda m: tag_star(m) if "opacity" in m.group(0) else m.group(0), inner)


def kf(name, frames):
    """frames: [(seconds, css)] across the 15 s cycle."""
    body = " ".join(f"{t / CYCLE * 100:.2f}% {{ {css} }}" for t, css in frames)
    return f"@keyframes {name} {{ {body} }}"


END_A, END_B = 13.6, 14.3
css = []


def row_kf(name, t_in, dx, dy, early=False):
    """early: fade out while folding (the capital N of Nyx would sit on n)."""
    tr = lambda a, b: f"transform: translate({a:.1f}px, {b:.1f}px)"
    fold = [
        (4.35, f"opacity: 1; {tr(0, dy * 0.2)}"),
        (4.75, f"opacity: 0; {tr(0, dy * 0.8)}"),
        (4.9, f"opacity: 0; {tr(0, dy)}"),
    ] if early else [
        (4.9, f"opacity: 1; {tr(0, dy)}"),
        (5.3, f"opacity: 0; {tr(0, dy)}"),
    ]
    return kf(name, [
        (0, f"opacity: 0; {tr(dx, 14)}"),
        (t_in, f"opacity: 0; {tr(dx, 14)}"),
        (t_in + 0.6, f"opacity: 1; {tr(dx, 0)}"),
        (2.6, f"opacity: 1; {tr(dx, 0)}"),
        (3.6, f"opacity: 1; {tr(0, 0)}"),
        (4.2, f"opacity: 1; {tr(0, 0)}"),
        *fold,
        (CYCLE, f"opacity: 0; {tr(0, dy)}"),
    ])


css.append(row_kf("r1", 0.2, X0 - row1_start, BASE - ROW_Y[0]))
css.append(row_kf("r2", 0.7, X0 - row2_start, 0))
css.append(row_kf("r3", 1.2, X0 - row3_start, BASE - ROW_Y[2], early=True))
css.append(kf("dim", [(0, "opacity: .32"), (3.0, "opacity: .32"), (3.6, "opacity: 0"), (CYCLE, "opacity: 0")]))
for i, (_, _, _, _, _, ly) in enumerate(legend):
    t_in = 0.4 + 0.5 * i
    up = 0 if TALL else ROW_Y[i] - 26 - (ly + 7)
    tr = lambda b: f"transform: translateY({b:.1f}px)"
    css.append(kf(f"l{i}", [
        (0, f"opacity: 0; {tr(up)}"), (t_in, f"opacity: 0; {tr(up)}"), (t_in + 0.6, f"opacity: 1; {tr(up)}"),
        (4.4, f"opacity: 1; {tr(up)}"), (5.2, f"opacity: 1; {tr(0)}"),
        (END_A, f"opacity: 1; {tr(0)}"), (END_B, f"opacity: 0; {tr(0)}"), (CYCLE, f"opacity: 0; {tr(0)}"),
    ]))
    css.append(kf(f"b{i}", [
        (0, "stroke-dashoffset: 1"), (5.2 + 0.15 * i, "stroke-dashoffset: 1"), (6.0 + 0.15 * i, "stroke-dashoffset: 0"),
        (END_A, "stroke-dashoffset: 0; opacity: 1"), (END_B, "stroke-dashoffset: 0; opacity: 0"), (CYCLE, "opacity: 0"),
    ]))
fade_in = lambda name, a, b, extra0="", extra1="": kf(name, [
    (0, f"opacity: 0; {extra0}"), (a, f"opacity: 0; {extra0}"), (b, f"opacity: 1; {extra1}"),
    (END_A, f"opacity: 1; {extra1}"), (END_B, f"opacity: 0; {extra1}"), (CYCLE, f"opacity: 0; {extra1}"),
])
css.append(fade_in("word", 4.9, 5.4))
css.append(fade_in("lead", 5.9, 6.4))
css.append(fade_in("gem", 5.0, 5.9, "transform: scale(.6) rotate(-8deg)", "transform: scale(1) rotate(0deg)"))
css.append(fade_in("glow", 5.0, 6.2))
css.append(fade_in("say", 6.2, 6.8))
css.append(kf("tw", [(0, "opacity: 1"), (CYCLE / 2, "opacity: .25"), (CYCLE, "opacity: 1")]))

style = "\n".join(css) + f"""
.anim {{ animation-duration: {CYCLE}s; animation-iteration-count: infinite; animation-timing-function: cubic-bezier(.45,0,.25,1); }}
.r1 {{ animation-name: r1; opacity: 0; }} .r2 {{ animation-name: r2; opacity: 0; }} .r3 {{ animation-name: r3; opacity: 0; }}
.dim {{ animation-name: dim; opacity: 0; }}
.l0 {{ animation-name: l0; }} .l1 {{ animation-name: l1; }} .l2 {{ animation-name: l2; }}
.b0 {{ animation-name: b0; }} .b1 {{ animation-name: b1; }} .b2 {{ animation-name: b2; }}
.word {{ animation-name: word; }} .lead {{ animation-name: lead; }} .say {{ animation-name: say; }}
.gem {{ animation-name: gem; transform-box: fill-box; transform-origin: center; }}
.glow {{ animation-name: glow; }}
.tw {{ animation: tw 2.6s ease-in-out infinite; }}
.t1 {{ animation-delay: -.7s; }} .t2 {{ animation-delay: -1.4s; }} .t3 {{ animation-delay: -2s; }}
@media (prefers-reduced-motion: reduce) {{ .anim, .tw {{ animation: none; }} }}
"""

parts = []
add = parts.append
add(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="Deonyx: deo from fdeox, onyx the black gemstone, Nyx the goddess of the night. Say it de-o-niks.">')
add("<title>Where the name Deonyx comes from</title>")
add(f"<style>{style}</style>")
add("<defs>")
add('<radialGradient id="nm-glow" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#6b4fd6" stop-opacity=".55"/><stop offset="1" stop-color="#6b4fd6" stop-opacity="0"/></radialGradient>')
add('<linearGradient id="nm-bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#13121d"/><stop offset="1" stop-color="#0b0b11"/></linearGradient>')
add("</defs>")
add(f'<rect x="1" y="1" width="{W - 2}" height="{H - 2}" rx="22" fill="url(#nm-bg)" stroke="#262438" stroke-width="2"/>')
add(f'<ellipse class="anim glow" cx="{GEM_X + GEM / 2}" cy="{GEM_Y + GEM / 2}" rx="{120 if TALL else 150}" ry="{100 if TALL else 120}" fill="url(#nm-glow)"/>')
# small header
add(f'<path fill="{MUTED}" d="{reg.text("WHERE THE NAME COMES FROM", 34, 36, 14)}"/>')
# rows
add(f'<g transform="translate(0 {ROW_Y[0]})"><g class="anim r1"><path fill="{TEXT}" d="{row1_main}"/><path class="anim dim" fill="{TEXT}" d="{row1_dim}"/></g></g>')
add(f'<g transform="translate(0 {ROW_Y[1]})"><g class="anim r2"><path fill="{ONYX}" d="{row2}"/></g></g>')
add(f'<g transform="translate(0 {ROW_Y[2]})"><g class="anim r3"><path fill="{VIOLET}" d="{row3}"/></g></g>')
# final word
add(f'<path class="anim word" fill="{TEXT}" d="{final_word}"/>')
# legend
for i, (name, desc, color, a, b, ly) in enumerate(legend):
    add(f'<path class="anim b{i}" d="M{a + 3:.1f} {ly} H{b - 3:.1f}" stroke="{color}" stroke-width="4" stroke-linecap="round" fill="none" pathLength="1" stroke-dasharray="1 1"/>')
    if not TALL:
        add(f'<path class="anim lead" d="M{b + 10:.1f} {ly} H{LAB_X - 12}" stroke="{color}" stroke-opacity=".45" stroke-width="2" stroke-dasharray="2 6" stroke-linecap="round" fill="none"/>')
    nw = semi.width(name, LAB)
    if TALL:
        y = LIST_Y[i]
        add(f'<g class="anim l{i}"><path d="M{LIST_X} {y - 7} H{LIST_X + 22}" stroke="{color}" stroke-width="4" stroke-linecap="round"/><path fill="{color}" d="{semi.text(name, LIST_X + 36, y, LAB)}"/><path fill="{MUTED}" d="{reg.text(desc, LIST_X + 36 + nw + 10, y, LAB)}"/></g>')
    else:
        add(f'<g class="anim l{i}"><path fill="{color}" d="{semi.text(name, LAB_X, ly + 7, LAB)}"/><path fill="{MUTED}" d="{reg.text(desc, LAB_X + nw + 10, ly + 7, LAB)}"/></g>')
# say it
say_w = reg.width("say it", 18) + 10 + semi.width("de · o · niks", 18)
say_x, say_y = ((W - say_w) / 2, H - 34) if TALL else (LAB_X, BASE + 116)
add(f'<g class="anim say"><path fill="{MUTED}" d="{reg.text("say it", say_x, say_y, 18)}"/><path fill="{TEXT}" d="{semi.text("de · o · niks", say_x + reg.width("say it", 18) + 10, say_y, 18)}"/></g>')
# gem
add(f'<g class="anim gem"><svg x="{GEM_X}" y="{GEM_Y}" width="{GEM}" height="{GEM}" viewBox="0 0 200 200">{inner}</svg></g>')
add("</svg>")

svg = "\n".join(parts)
open(OUT, "w", encoding="utf8").write(svg)
print(OUT, len(svg) // 1024, "KB", "word end", round(WORD_END))
