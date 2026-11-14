#!/usr/bin/env python3
"""Draw the virt-interact app icon (crosscheck 2026-11-14-03, improvement 1).

A white rounded square with a black border holding three "VM slots", each
with a status light (green, green, grey), in the app's white/black style.
Writes packaging/icon/:
  virt-interact.png      512 px (Linux .desktop icon)
  virt-interact-256.png  256 px (window icon, embedded in the binary)
  virt-interact.ico      16-256 px (Windows exe resource)
  virt-interact.icns     16-1024 px (macOS app bundle)
Needs Pillow. Run: python3 scripts/make_icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024
BLACK = (0, 0, 0, 255)
WHITE = (255, 255, 255, 255)
GREEN = (26, 127, 55, 255)
GREY = (110, 110, 110, 255)


def draw() -> Image.Image:
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((72, 72, S - 72, S - 72), radius=190, fill=WHITE, outline=BLACK, width=56)
    left, right = 210, S - 210
    for i, light in enumerate((GREEN, GREEN, GREY)):
        top = 236 + i * 196
        d.rounded_rectangle((left, top, right, top + 140), radius=40, fill=WHITE, outline=BLACK, width=36)
        cy = top + 70
        d.ellipse((left + 52, cy - 34, left + 120, cy + 34), fill=light, outline=BLACK, width=10)
        for k in range(3):
            x = right - 80 - k * 56
            d.rounded_rectangle((x - 14, cy - 34, x + 14, cy + 34), radius=10, fill=BLACK)
    return im


def main() -> None:
    out = Path(__file__).resolve().parent.parent / "packaging" / "icon"
    out.mkdir(parents=True, exist_ok=True)
    big = draw()
    big.resize((512, 512), Image.LANCZOS).save(out / "virt-interact.png", optimize=True)
    big.resize((256, 256), Image.LANCZOS).save(out / "virt-interact-256.png", optimize=True)
    big.save(out / "virt-interact.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
    big.save(out / "virt-interact.icns")


if __name__ == "__main__":
    main()
