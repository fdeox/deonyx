# Draws the app icon (icon.ico and the PNGs Tauri wants) from the logo: just
# the gem and its outline on a clear background, no tile behind it. 32 px and
# below use assets/logo/deonyx-small.svg (fewer facets, thicker outline).
# Run from the repository: python icons/make-icons.py

import importlib.util
import io
import re
import struct
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).parent
ROOT = HERE.parent

spec = importlib.util.spec_from_file_location("images", ROOT / "streamdeck" / "make-images.py")
images = importlib.util.module_from_spec(spec)
spec.loader.exec_module(images)

SMALL = (ROOT / "assets" / "logo" / "deonyx-small.svg").read_text(encoding="utf8")
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]


def small_logo(size):
    """deonyx-small.svg: a few polygons and a thick outline, drawn 8x big."""
    big = size * 8
    k = big / 200
    pts = lambda s: [(float(x) * k, float(y) * k) for x, y in re.findall(r"(-?[\d.]+),(-?[\d.]+)", s)]
    im = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    for m in re.finditer(r'<polygon points="([^"]+)" fill="(#[0-9a-f]{6})"', SMALL):
        d.polygon(pts(m.group(1)), fill=m.group(2))
    outline = re.search(r'<polygon points="([^"]+)" fill="none" stroke="(#[0-9a-f]{6})" stroke-width="([\d.]+)"', SMALL)
    p = pts(outline.group(1))
    d.line(p + p[:2], fill=outline.group(2), width=round(float(outline.group(3)) * k), joint="curve")
    return im.resize((size, size), Image.LANCZOS)


def icon(size):
    """The gem filling the square: drawn larger, cropped to what it covers,
    then fitted and centred."""
    draw = small_logo if size <= 32 else images.logo
    art = draw(size * 4)
    art = art.crop(art.getchannel("A").getbbox())
    room = size - 2 * (0 if size <= 32 else round(size * 0.03))
    k = room / max(art.size)
    art = art.resize((max(1, round(art.width * k)), max(1, round(art.height * k))), Image.LANCZOS)
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.alpha_composite(art, ((size - art.width) // 2, (size - art.height) // 2))
    return out


def write_ico(path, pics):
    """An .ico with one PNG per size."""
    blobs = []
    for im in pics:
        b = io.BytesIO()
        im.save(b, "PNG", optimize=True)
        blobs.append(b.getvalue())
    head = struct.pack("<HHH", 0, 1, len(pics))
    offset = 6 + 16 * len(pics)
    entries = b""
    for im, data in zip(pics, blobs):
        w, h = im.size
        entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    path.write_bytes(head + entries + b"".join(blobs))


def main():
    pics = [icon(s) for s in SIZES]
    write_ico(HERE / "icon.ico", pics)
    icon(32).save(HERE / "32x32.png")
    icon(128).save(HERE / "128x128.png")
    icon(256).save(HERE / "128x128@2x.png")
    print("icon.ico", SIZES)


if __name__ == "__main__":
    main()
