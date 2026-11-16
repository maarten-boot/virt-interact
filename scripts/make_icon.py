#!/usr/bin/env python3
"""Build the virt-interact app icons from the logo (Plan 6.0).

The logo is a blue "V" on a green rounded square. Its master,
packaging/icon/logo.png (1024 px, transparent outside the square), is made
once from the supplied picture with --source; every icon is made from the
master:
  virt-interact.png      512 px (Linux .desktop icon)
  virt-interact-256.png  256 px (window icon, embedded in the binary)
  virt-interact.ico      16-256 px (Windows exe resource)
  virt-interact.icns     16-1024 px (macOS app bundle)

--source PICTURE cuts the rounded square out of the picture (any plain
background), removes JPEG noise by snapping every pixel onto the line
between the two logo colours (edges keep their anti-aliasing) and writes
the master.

Needs Pillow. Run: python3 scripts/make_icon.py [--source PICTURE]
"""
import argparse
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024
MARGIN = 72  # transparent border around the square, as macOS/Windows icons have
GREEN = (128, 207, 159)
BLUE = (6, 86, 243)
RADIUS = 0.094  # corner radius as a fraction of the square's side (80 of 854 px)

OUT = Path(__file__).resolve().parent.parent / "packaging" / "icon"
MASTER = OUT / "logo.png"


def is_logo(p) -> bool:
    r, g, b = p[:3]
    return (g - r > 40 and g - b > 20) or (b - r > 100 and b - g > 60)


def square_bbox(im: Image.Image) -> tuple[int, int, int, int]:
    """Bounding box of the green/blue square, found on the middle row and column."""
    w, h = im.size
    px = im.load()
    xs = [x for x in range(w) if is_logo(px[x, h // 2])]
    ys = [y for y in range(h) if is_logo(px[w // 2, y])]
    if not xs or not ys:
        raise SystemExit("no green/blue square found in the source picture")
    return xs[0], ys[0], xs[-1] + 1, ys[-1] + 1


def clean(im: Image.Image) -> Image.Image:
    """Snap each pixel onto the green-blue line (JPEG noise off, edges kept)."""
    d = [b - g for g, b in zip(GREEN, BLUE)]
    dd = sum(c * c for c in d)
    lut = {}
    out = Image.new("RGB", im.size)
    src, dst = im.load(), out.load()
    for y in range(im.size[1]):
        for x in range(im.size[0]):
            p = src[x, y]
            if p not in lut:
                t = sum((p[i] - GREEN[i]) * d[i] for i in range(3)) / dd
                t = min(1.0, max(0.0, t))
                lut[p] = tuple(round(GREEN[i] + t * d[i]) for i in range(3))
            dst[x, y] = lut[p]
    return out


def master_from(source: Path) -> Image.Image:
    pic = Image.open(source).convert("RGB")
    square = clean(pic.crop(square_bbox(pic)))
    side = S - 2 * MARGIN
    square = square.resize((side, side), Image.LANCZOS)
    # Rounded-square mask, drawn 4x larger and scaled down for smooth corners.
    big = Image.new("L", (side * 4, side * 4), 0)
    ImageDraw.Draw(big).rounded_rectangle(
        (0, 0, side * 4 - 1, side * 4 - 1), radius=round(RADIUS * side * 4), fill=255
    )
    mask = big.resize((side, side), Image.LANCZOS)
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    im.paste(square, (MARGIN, MARGIN), mask)
    return im


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--source", type=Path, help="picture of the logo; rewrites the master")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    if args.source:
        master_from(args.source).save(MASTER, optimize=True)
    big = Image.open(MASTER).convert("RGBA")
    if big.size != (S, S):
        raise SystemExit(f"{MASTER} must be {S}x{S}")
    big.resize((512, 512), Image.LANCZOS).save(OUT / "virt-interact.png", optimize=True)
    big.resize((256, 256), Image.LANCZOS).save(OUT / "virt-interact-256.png", optimize=True)
    big.save(OUT / "virt-interact.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
    big.save(OUT / "virt-interact.icns")


if __name__ == "__main__":
    main()
