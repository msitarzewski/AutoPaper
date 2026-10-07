#!/usr/bin/env python3
"""Makes the website's social card (site/static/social-card.png, 1200 × 630) for Open Graph and Twitter.

The layout follows AudioPaper's card: the app icon on the left; the name, tagline and a few facts on the right.
The background is a blurred, darkened crop of the home page's example wallpaper (rain-ruins.jpg).
If you change the words here, change og:image:alt in site/layout.html to match.

    python3 scripts/make_social_card.py                          # needs Pillow (pip install pillow)
    python3 scripts/make_social_card.py --background other.jpg --out /tmp/card.png
"""
import argparse
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
STATIC = ROOT / "site" / "static"
W, H = 1200, 630
RED = (0xE7, 0x09, 0x0A)             # the icon's screen (docs/icon/build_icons.py, palette "red", as AudioPaper)
VOID = (0x0C, 0x0B, 0x10)            # the site's dark background (site/static/style.css --void)

TITLE = "AutoPaper"
TAGLINE = ["Tell your computer what", "you’d like to see."]  # the post's own line, broken by hand
FACTS = ["Free for macOS, Windows and Linux",
         "Your own key or a local model · no account, no telemetry"]

# System fonts, first found wins: macOS (SF Pro), then common Linux and Windows fonts.
FONTS = {
    "bold": [("/System/Library/Fonts/SFNS.ttf", "Bold"), ("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", None),
             ("C:/Windows/Fonts/segoeuib.ttf", None)],
    "regular": [("/System/Library/Fonts/SFNS.ttf", "Regular"), ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", None),
                ("C:/Windows/Fonts/segoeui.ttf", None)],
}


def font(kind, size):
    for path, variation in FONTS[kind]:
        if Path(path).exists():
            f = ImageFont.truetype(path, size)
            if variation:
                try:
                    f.set_variation_by_name(variation)
                except (OSError, ValueError):
                    pass
            return f
    sys.exit(f"No {kind} font found; add one to FONTS")


def cover(image, width, height, focus_y=0.45):
    """Scales and crops to fill width × height, keeping the horizontal centre and a point a little above
    the vertical centre (the example wallpapers keep their subject low and their sky high)."""
    scale = max(width / image.width, height / image.height)
    resized = image.resize((round(image.width * scale), round(image.height * scale)), Image.LANCZOS)
    left = (resized.width - width) // 2
    top = round((resized.height - height) * focus_y)
    return resized.crop((left, top, left + width, top + height))


def horizontal_gradient(width, height, stops):
    """An RGBA image whose colour/alpha moves across `stops`: [(x 0…1, (r, g, b, a)), …]."""
    strip = Image.new("RGBA", (width, 1))
    px = strip.load()
    for x in range(width):
        t = x / (width - 1)
        for (x0, c0), (x1, c1) in zip(stops, stops[1:]):
            if x0 <= t <= x1:
                k = (t - x0) / (x1 - x0) if x1 > x0 else 0
                px[x, 0] = tuple(round(a + (b - a) * k) for a, b in zip(c0, c1))
                break
    return strip.resize((width, height))


def wrap(draw, text, face, max_width):
    lines, line = [], ""
    for word in text.split():
        trial = f"{line} {word}".strip()
        if draw.textlength(trial, font=face) <= max_width or not line:
            line = trial
        else:
            lines.append(line)
            line = word
    return lines + [line]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--background", type=Path, default=STATIC / "examples" / "rain-ruins.jpg")
    parser.add_argument("--out", type=Path, default=STATIC / "social-card.png")
    args = parser.parse_args()
    if not args.background.exists():
        sys.exit(f"{args.background} doesn't exist yet; it's painted separately (see site/static/examples/examples.json)")

    background = cover(Image.open(args.background).convert("RGB"), W, H)
    card = background.filter(ImageFilter.GaussianBlur(14)).convert("RGBA")
    # Darken: a red glow behind the icon on the left, near-black behind the words on the right.
    card = Image.alpha_composite(card, Image.new("RGBA", (W, H), VOID + (120,)))
    card = Image.alpha_composite(card, horizontal_gradient(W, H, [
        (0.0, RED + (80,)), (0.3, RED + (40,)), (0.55, VOID + (150,)), (1.0, VOID + (205,))]))

    icon = Image.open(STATIC / "icon-512.png").convert("RGBA").resize((380, 380), Image.LANCZOS)
    card.alpha_composite(icon, (64, (H - 380) // 2))

    draw = ImageDraw.Draw(card)
    x, max_width = 492, W - 492 - 56
    title_face, tagline_face, facts_face = font("bold", 96), font("regular", 40), font("regular", 23)
    tagline = [line for part in TAGLINE for line in wrap(draw, part, tagline_face, max_width)]
    facts = [line for fact in FACTS for line in wrap(draw, fact, facts_face, max_width)]

    heights = [96 * 1.1] + [40 * 1.3] * len(tagline) + [28] + [23 * 1.6] * len(facts)
    y = (H - sum(heights)) / 2
    draw.text((x - 4, y), TITLE, font=title_face, fill=(248, 247, 252))
    y += 96 * 1.1 + 6
    for line in tagline:
        draw.text((x, y), line, font=tagline_face, fill=(226, 223, 234))
        y += 40 * 1.3
    y += 28
    for line in facts:
        draw.text((x, y), line, font=facts_face, fill=(184, 180, 196))
        y += 23 * 1.6

    card.convert("RGB").save(args.out, optimize=True)
    print(f"Wrote {args.out} ({W} × {H})")


if __name__ == "__main__":
    main()
