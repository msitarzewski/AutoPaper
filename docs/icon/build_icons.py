#!/usr/bin/env python3
"""AutoPaper icon: SVG masters → one Icon Composer document per accent palette, plus previews.

AudioPaper's build (`~/Software/AudioPaper/docs/icon/build_icons.py`), repurposed: the same display
(GlassPowerTools' bezel and screen geometry) with Heroicons' sparkles on the screen instead of the
musical note. Family layered technique (icon-lab, GlassPowerTools): each semantic layer is one white
SVG mask; colour lives in per-appearance `fill-specializations`, so light and dark are native Icon
Composer specializations and Clear/Tinted are derived by the system.

  python3 docs/icon/build_icons.py                         # every palette → docs/icon/palettes/, board
  python3 docs/icon/build_icons.py --ship display red      # also install it as apps/macos/AppIcon.icon
  python3 docs/icon/build_icons.py --site display red      # also export the website's PNGs (site/static/)

Colour roles (GlassPowerTools convention), as vector linear gradients (top → bottom):
  primary   (screen)          the accent, lit top-left → deep bottom-right, both appearances
  secondary (bezel, stand)    silver in both appearances
  inset     (screen rim)      a shadow line, deepest along the top (no glass)
  glyph     (sparkles)        solid white in both, always on the primary
"""
import colorsys
import json
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
SOURCES = HERE / "Sources"  # one folder per design
OUT = HERE / "palettes"
ICTOOL = Path("/Applications/Xcode-beta.app/Contents/Applications/Icon Composer.app/Contents/Executables/ictool")

PALETTES = {
    "red": (0.90588, 0.03529, 0.03922),   # GlassPowerTools' screen red
    "cobalt": (0.0, 71 / 255, 171 / 255),  # family's approved alternate
    "emerald": (0.0, 0.62, 0.38),
    "violet": (0.45, 0.20, 0.90),
    "amber": (1.0, 0.58, 0.0),
}
WHITE = (1.0, 1.0, 1.0)
# Neutral gradients, top → bottom, as (light appearance, dark appearance). Vector fills, so the system's
# Liquid Glass stays live on top; no shading is baked into the artwork. Silver in both appearances, as
# GlassPowerTools' shipped display is (flat white in dark mode read as unlit).
SECONDARY = (((0.93, 0.93, 0.95), (0.68, 0.68, 0.71)), ((0.92, 0.92, 0.94), (0.64, 0.64, 0.68)))
# The screen's inset rim: a shadow, deepest along the top edge (black with alpha).
INSET = (((0, 0, 0, 0.55), (0, 0, 0, 0.18)),) * 2
# The primary accent is lit from the top-left, like a real screen (Icon Composer gradient orientation).
DIAGONAL = {"start": {"x": 0, "y": 0}, "stop": {"x": 1, "y": 1}}

# Groups per design, top to bottom (index 0 renders in front). Icon Composer allows at most four groups;
# a group may hold several layers, each one SVG mask with its own colour role.
DESIGNS = {
    # Mirrors GlassPowerTools' layer stack: glyph, bezel (a true frame), screen, then the base.
    "display": [
        ("Sparkles", 0.10, [("Sparkles", "glyph")]),
        ("Bezel", 0.22, [("Inset", "inset"), ("Bezel", "secondary")]),
        ("Screen", 0.12, [("Screen", "primary")]),
        ("Stand", 0.22, [("Stand", "secondary")]),
    ],
}
# Icon Composer maps an SVG's viewBox onto the icon grid, not the full canvas; geometry drawn at
# full-canvas size renders at ~82% and sits low (see GlassPowerTools 3b39e7b). Scale about the centre.
GRID_SCALE = 1.22
# Per-design size within the grid. The display is narrow, so it's enlarged to fill the icon (as in AudioPaper).
DESIGN_SCALE = {"display": 1.12}
APPEARANCES = ["Default", "Dark", "ClearLight", "ClearDark", "TintedDark"]


def srgb(color):
    """An Icon Composer colour from (r, g, b) or (r, g, b, alpha)."""
    rgba = color if len(color) == 4 else (*color, 1)
    return "srgb:" + ",".join(f"{v:.5f}" for v in rgba)


def lit(rgb):
    """The accent at full brightness, slightly less saturated — brighter without washing toward pink."""
    h, s, v = colorsys.rgb_to_hsv(*rgb)
    return colorsys.hsv_to_rgb(h, s * 0.9, min(1.0, v * 1.12))


def deep(rgb):
    h, s, v = colorsys.rgb_to_hsv(*rgb)
    return colorsys.hsv_to_rgb(h, min(1.0, s * 1.05), v * 0.62)


def fill(value, orientation=None):
    if isinstance(value[0], tuple):
        gradient = {"linear-gradient": [srgb(value[0]), srgb(value[1])]}
        if orientation:
            gradient["orientation"] = orientation
        return gradient
    return {"solid": srgb(value)}


def fills(role, accent):
    """Per-appearance fill: a colour (solid) or a (top, bottom) pair (linear gradient)."""
    light, dark = {
        # The accent keeps its hue in both appearances, lit at the top-left and deep at the bottom-right.
        "primary": ((lit(accent), deep(accent)),) * 2,
        "secondary": SECONDARY,
        "inset": INSET,
        "glyph": (WHITE, WHITE),
    }[role]
    orientation = DIAGONAL if role == "primary" else None
    return [{"value": fill(light, orientation)}, {"appearance": "dark", "value": fill(dark, orientation)}]


def scaled_svg(source: Path, design: str) -> str:
    text = source.read_text()
    open_end = text.index(">", text.index("<svg")) + 1
    close = text.rindex("</svg>")
    scale = round(GRID_SCALE * DESIGN_SCALE[design], 4)
    return (
        text[:open_end]
        + f'\n  <g transform="translate(512 512) scale({scale}) translate(-512 -512)">'
        + text[open_end:close]
        + "  </g>\n"
        + text[close:]
    )


def build(design, name, accent):
    icon = OUT / design / f"AppIcon-{name}.icon"
    shutil.rmtree(icon, ignore_errors=True)
    (icon / "Assets").mkdir(parents=True)
    groups = []
    for group, shadow, layers in DESIGNS[design]:
        entries = []
        for layer, role in layers:
            (icon / "Assets" / f"{layer}.svg").write_text(scaled_svg(SOURCES / design / f"{layer}.svg", design))
            # The inset is a shadow line, not a glass object, so it gets no glass treatment of its own.
            entries.append({"name": layer, "image-name": f"{layer}.svg", "glass": role != "inset",
                            "fill-specializations": fills(role, accent)})
        groups.append({
            "name": group,
            "layers": entries,
            "shadow": {"kind": "neutral", "opacity": shadow},
            "specular": True,
            "translucency": {"enabled": False, "value": 0.05},
        })
    manifest = {
        "fill-specializations": [
            {"value": {"linear-gradient": [srgb((0.985,) * 3), srgb((0.945,) * 3)]}},
            {"appearance": "dark", "value": {"linear-gradient": [srgb((0.11, 0.115, 0.125)), srgb((0.035, 0.038, 0.045))]}},
        ],
        "groups": groups,
        "supported-platforms": {"circles": ["watchOS"], "squares": "shared"},
    }
    (icon / "icon.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return icon


def export(icon, name, size=512):
    paths = {}
    for mode in APPEARANCES:
        out = OUT / "previews" / f"{name}-{mode}.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        args = [str(ICTOOL), str(icon), "--export-image", "--output-file", str(out), "--platform", "macOS",
                "--rendition", mode, "--width", str(size), "--height", str(size), "--scale", "1",
                "--design-generation", "27"]
        if mode == "TintedDark":
            args += ["--tint-color", "0.58", "--tint-strength", "0.75"]
        subprocess.run(args, check=True, capture_output=True)
        paths[mode] = out
    small = OUT / "previews" / f"{name}-dock64.png"
    subprocess.run([str(ICTOOL), str(icon), "--export-image", "--output-file", str(small), "--platform", "macOS",
                    "--rendition", "Default", "--width", "64", "--height", "64", "--scale", "1",
                    "--design-generation", "27"], check=True, capture_output=True)
    paths["dock64"] = small
    return paths


def board(rows, filename):
    from PIL import Image, ImageDraw
    cell, pad = 256, 16
    cols = APPEARANCES + ["dock64"]
    width = pad + len(cols) * (cell + pad) + 110
    height = pad + len(rows) * (cell + pad)
    sheet = Image.new("RGBA", (width, height), (128, 128, 128, 255))
    draw = ImageDraw.Draw(sheet)
    for r, (name, paths) in enumerate(rows):
        y = pad + r * (cell + pad)
        draw.text((pad, y + cell // 2), name, fill="white")
        for c, mode in enumerate(cols):
            img = Image.open(paths[mode]).convert("RGBA")
            if mode != "dock64":
                img = img.resize((cell, cell), Image.LANCZOS)
            x = 110 + pad + c * (cell + pad)
            sheet.paste(img, (x + (cell - img.width) // 2, y + (cell - img.height) // 2), img)
            if r == 0:
                draw.text((x, 2), mode, fill="white")
    sheet.save(OUT / filename)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    renders = {}
    for design in DESIGNS:
        rows = []
        for name, accent in PALETTES.items():
            icon = build(design, name, accent)
            rows.append((name, export(icon, f"{design}-{name}")))
        renders[design] = dict(rows)
        board(rows, f"board-{design}.png")
    if "--ship" in sys.argv:
        design, palette = sys.argv[sys.argv.index("--ship") + 1: sys.argv.index("--ship") + 3]
        target = ROOT / "apps" / "macos" / "AppIcon.icon"
        shutil.rmtree(target, ignore_errors=True)
        shutil.copytree(OUT / design / f"AppIcon-{palette}.icon", target)
        print(f"Shipped {design} {palette} → {target.relative_to(ROOT)}")
    if "--site" in sys.argv:
        design, palette = sys.argv[sys.argv.index("--site") + 1: sys.argv.index("--site") + 3]
        site_icons(OUT / design / f"AppIcon-{palette}.icon")
    print("Boards: " + ", ".join(str((OUT / f).relative_to(ROOT)) for f in [f"board-{d}.png" for d in DESIGNS]))


# The website's icons, rendered by ictool from the same document (macOS light appearance).
SITE_ICONS = {"favicon-16.png": 16, "favicon-32.png": 32, "icon-64.png": 64, "apple-touch-icon.png": 180,
              "icon-256.png": 256, "icon-512.png": 512}


def site_icons(icon):
    target = ROOT / "site" / "static"
    target.mkdir(parents=True, exist_ok=True)
    for name, size in SITE_ICONS.items():
        subprocess.run([str(ICTOOL), str(icon), "--export-image", "--output-file", str(target / name),
                        "--platform", "macOS", "--rendition", "Default", "--width", str(size), "--height",
                        str(size), "--scale", "1", "--design-generation", "27"], check=True, capture_output=True)
    print(f"Site icons → {target.relative_to(ROOT)}: " + ", ".join(SITE_ICONS))


if __name__ == "__main__":
    main()
