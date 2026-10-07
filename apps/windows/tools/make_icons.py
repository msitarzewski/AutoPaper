#!/usr/bin/env python3
"""AutoPaper's Windows icon assets, from the same artwork as the macOS icon (docs/icon/Sources/display).

Windows 11 app icons are free-standing shapes on a transparent background (no plate, unlike macOS), with
soft gradients and a gentle shadow. This script composes the shipped SVG masks (bezel, inset, screen,
sparkles, stand) into one such SVG, with the colours of docs/icon/build_icons.py's shipped "red" palette in
its light appearance, and rasterises it with rsvg-convert (Homebrew librsvg) into every size the MSIX
manifest names, plus AppIcon.ico for the window and the notification area.

    python3 apps/windows/tools/make_icons.py        # writes apps/windows/AutoPaper/Assets/

Needs rsvg-convert and Pillow. The PNGs are committed; run this again only when the artwork changes.
"""
import colorsys
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent.parent
SOURCES = ROOT / "docs" / "icon" / "Sources" / "display"
ASSETS = HERE.parent / "AutoPaper" / "Assets"

RED = (0.90588, 0.03529, 0.03922)  # build_icons.py PALETTES["red"], the shipped accent
# Silver for the bezel and stand, top → bottom (build_icons.py SECONDARY, light appearance).
SILVER = ((0.93, 0.93, 0.95), (0.68, 0.68, 0.71))
# The artwork's bounds in the 1024 grid (bezel left/right, bezel top to stand foot, after the masks' +55 shift).
BOUNDS = (246.6, 294.3, 778.2, 737.0)


def lit(rgb):
    h, s, v = colorsys.rgb_to_hsv(*rgb)
    return colorsys.hsv_to_rgb(h, s * 0.9, min(1.0, v * 1.12))


def deep(rgb):
    h, s, v = colorsys.rgb_to_hsv(*rgb)
    return colorsys.hsv_to_rgb(h, min(1.0, s * 1.05), v * 0.62)


def hex_colour(rgb):
    return "#" + "".join(f"{round(c * 255):02x}" for c in rgb)


def inner(name):
    """The mask's drawing (its <g> with the +55 shift), without the <svg> wrapper and comments."""
    text = (SOURCES / f"{name}.svg").read_text()
    text = re.sub(r"<!--.*?-->", "", text, flags=re.S)
    start = text.index(">", text.index("<svg")) + 1
    return text[start:text.rindex("</svg>")].strip()


def fill(svg, paint, edge=False):
    """Colours a white mask. `edge` adds the thin darker rim Fluent icons use so light shapes hold their
    outline on light surfaces."""
    attributes = f'fill="{paint}"'
    if edge:
        attributes += ' stroke="#7c7c84" stroke-opacity="0.55" stroke-width="5" stroke-linejoin="round"'
    return svg.replace('fill="#fff"', attributes)


def artwork(shadow: bool) -> str:
    """The display at its 1024-grid position, coloured."""
    defs = f"""
  <defs>
    <linearGradient id="screen" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="{hex_colour(lit(RED))}"/>
      <stop offset="1" stop-color="{hex_colour(deep(RED))}"/>
    </linearGradient>
    <linearGradient id="silver" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{hex_colour(SILVER[0])}"/>
      <stop offset="1" stop-color="{hex_colour(SILVER[1])}"/>
    </linearGradient>
    <linearGradient id="inset" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#000" stop-opacity="0.55"/>
      <stop offset="1" stop-color="#000" stop-opacity="0.18"/>
    </linearGradient>
    <filter id="shadow" x="-20%" y="-20%" width="140%" height="150%">
      <feGaussianBlur in="SourceAlpha" stdDeviation="9"/>
      <feOffset dy="10" result="blur"/>
      <feComponentTransfer><feFuncA type="linear" slope="0.28"/></feComponentTransfer>
      <feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
  </defs>"""
    layers = "\n".join([
        fill(inner("Stand"), "url(#silver)", edge=True),
        fill(inner("Screen"), "url(#screen)"),
        fill(inner("Bezel"), "url(#silver)", edge=True),
        fill(inner("Inset"), "url(#inset)"),
        fill(inner("Sparkles"), "#ffffff"),
    ])
    group = f'<g filter="url(#shadow)">{layers}</g>' if shadow else f"<g>{layers}</g>"
    return defs, group


def svg_for(width, height, fraction_w, fraction_h=None, shadow=True):
    """An SVG of width×height with the artwork centred, as wide as fraction_w of the width (and no taller
    than fraction_h of the height)."""
    left, top, right, bottom = BOUNDS
    art_w, art_h = right - left, bottom - top
    scale = width * fraction_w / art_w
    if fraction_h is not None:
        scale = min(scale, height * fraction_h / art_h)
    cx, cy = (left + right) / 2, (top + bottom) / 2
    defs, group = artwork(shadow)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
            f'viewBox="0 0 {width} {height}">{defs}\n'
            f'<g transform="translate({width / 2} {height / 2}) scale({scale:.6f}) translate({-cx} {-cy})">'
            f"{group}</g></svg>\n")


def render(svg_text, out: Path, width, height):
    with tempfile.NamedTemporaryFile("w", suffix=".svg", delete=False) as handle:
        handle.write(svg_text)
        source = handle.name
    subprocess.run(["rsvg-convert", "-w", str(width), "-h", str(height), "-o", str(out), source], check=True)
    Path(source).unlink()


SCALES = {100: 1.0, 125: 1.25, 150: 1.5, 200: 2.0, 400: 4.0}
# Square44x44Logo target sizes (taskbar, Start, Alt+Tab, Settings > Apps); each also as altform-unplated
# and altform-lightunplated (same full-colour artwork: it reads on light and dark).
TARGET_SIZES = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256]


def square_fraction(size):
    """Small icons use nearly the whole square; larger ones keep room for the shadow."""
    return 0.98 if size <= 24 else 0.94 if size <= 48 else 0.9


def main():
    if not shutil.which("rsvg-convert"):
        sys.exit("rsvg-convert is needed (brew install librsvg)")
    ASSETS.mkdir(parents=True, exist_ok=True)
    for old in ASSETS.glob("*.png"):
        old.unlink()
    written = []

    def emit(name, width, height, fraction_w, fraction_h=None, shadow=True):
        out = ASSETS / name
        render(svg_for(width, height, fraction_w, fraction_h, shadow and min(width, height) >= 40), out, width, height)
        written.append(name)

    for size in TARGET_SIZES:
        for suffix in ["", "_altform-unplated", "_altform-lightunplated"]:
            emit(f"Square44x44Logo.targetsize-{size}{suffix}.png", size, size, square_fraction(size))
    for scale, factor in SCALES.items():
        s44 = round(44 * factor)
        emit(f"Square44x44Logo.scale-{scale}.png", s44, s44, square_fraction(s44))
        s150 = round(150 * factor)
        emit(f"Square150x150Logo.scale-{scale}.png", s150, s150, 0.52)
        emit(f"Wide310x150Logo.scale-{scale}.png", round(310 * factor), round(150 * factor), 0.3, 0.55)
        s50 = round(50 * factor)
        emit(f"StoreLogo.scale-{scale}.png", s50, s50, square_fraction(s50))
        emit(f"SplashScreen.scale-{scale}.png", round(620 * factor), round(300 * factor), 0.3, 0.6)

    # AppIcon.ico: the window's title bar and taskbar icon, and the notification-area icon (WinUIEx loads
    # the size for the DPI). Each size rendered on its own, not downscaled from one bitmap.
    ico_sizes = [16, 20, 24, 32, 40, 48, 64, 256]
    frames = []
    with tempfile.TemporaryDirectory() as tmp:
        for size in ico_sizes:
            path = Path(tmp) / f"{size}.png"
            render(svg_for(size, size, square_fraction(size), shadow=size >= 40), path, size, size)
            frames.append(Image.open(path).convert("RGBA"))
        frames[-1].save(ASSETS / "AppIcon.ico", format="ICO", sizes=[(s, s) for s in ico_sizes],
                        append_images=frames[:-1])
    written.append("AppIcon.ico")
    print(f"{len(written)} files → {ASSETS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
