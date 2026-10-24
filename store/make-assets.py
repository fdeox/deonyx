# Draws the Microsoft Store package's images from ui/logo.svg: tiles, the
# Store logo, the splash screen, and the app icon at every size Windows
# asks for (taskbar, Start, Alt+Tab), plated and unplated.
# Run from this folder: python make-assets.py

import importlib.util
from pathlib import Path

from PIL import Image

HERE = Path(__file__).parent
OUT = HERE / "Assets"

# The logo drawing lives with the Stream Deck plugin's images.
spec = importlib.util.spec_from_file_location("images", HERE.parent / "streamdeck" / "make-images.py")
images = importlib.util.module_from_spec(spec)
spec.loader.exec_module(images)
logo = images.logo


def centered(w, h, size, bg=(0, 0, 0, 0)):
    im = Image.new("RGBA", (w, h), bg)
    g = logo(size)
    im.alpha_composite(g, ((w - size) // 2, (h - size) // 2))
    return im


def main():
    OUT.mkdir(exist_ok=True)
    for f in OUT.glob("*.png"):
        f.unlink()
    for scale in (100, 200, 400):
        k = scale / 100
        # Tiles and the Store logo: the gem with some room around it.
        centered(round(150 * k), round(150 * k), round(96 * k)).save(OUT / f"Square150x150Logo.scale-{scale}.png")
        centered(round(50 * k), round(50 * k), round(42 * k)).save(OUT / f"StoreLogo.scale-{scale}.png")
        centered(round(44 * k), round(44 * k), round(40 * k)).save(OUT / f"Square44x44Logo.scale-{scale}.png")
        if scale <= 200:
            centered(round(310 * k), round(150 * k), round(110 * k)).save(OUT / f"Wide310x150Logo.scale-{scale}.png")
            centered(round(620 * k), round(300 * k), round(200 * k)).save(OUT / f"SplashScreen.scale-{scale}.png")
    # The icon in the taskbar, Start and Alt+Tab, at its exact pixel sizes.
    for size in (16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256):
        pad = 0 if size <= 24 else round(size * 0.04)
        icon = centered(size, size, size - 2 * pad)
        icon.save(OUT / f"Square44x44Logo.targetsize-{size}.png")
        icon.save(OUT / f"Square44x44Logo.targetsize-{size}_altform-unplated.png")
        icon.save(OUT / f"Square44x44Logo.targetsize-{size}_altform-lightunplated.png")
    print(len(list(OUT.glob("*.png"))), "images in", OUT)


if __name__ == "__main__":
    main()
