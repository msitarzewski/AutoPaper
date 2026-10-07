#!/usr/bin/env python3
"""Adds a release to AutoPaper's Sparkle feed, site/static/appcast.xml (published with the website).

    scripts/appcast.py <version> <build> <zip> <edSignature> [notes.html]

Called by scripts/release.sh after the zip is signed with `sign_update --account AutoPaper` (AudioPaper's
scripts/appcast.py, adapted). The newest release goes first; an existing entry for the same build is replaced. The
enclosure points at the zip on the GitHub release, so publish the release (with the zip) before pushing the
updated feed. `notes.html` (optional) is a fragment of HTML shown in Sparkle's update window.
"""
import html
import os
import sys
from email.utils import formatdate
from pathlib import Path
from xml.sax.saxutils import quoteattr

ROOT = Path(__file__).resolve().parent.parent
FEED = ROOT / "site" / "static" / "appcast.xml"
REPO = "https://github.com/msitarzewski/AutoPaper"
# The app's deployment target (apps/macos/project.yml).
MINIMUM_SYSTEM = "26.0"

HEADER = """<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>AutoPaper</title>
    <link>https://msitarzewski.github.io/AutoPaper/appcast.xml</link>
    <description>AutoPaper updates</description>
    <language>en</language>
"""
FOOTER = """  </channel>
</rss>
"""


def item(version, build, zip_path, signature, notes):
    url = f"{REPO}/releases/download/v{version}/{Path(zip_path).name}"
    description = f"\n      <description><![CDATA[{notes}]]></description>" if notes else ""
    return f"""    <item>
      <title>AutoPaper {html.escape(version)}</title>
      <pubDate>{formatdate(usegmt=True)}</pubDate>
      <sparkle:version>{html.escape(build)}</sparkle:version>
      <sparkle:shortVersionString>{html.escape(version)}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>{MINIMUM_SYSTEM}</sparkle:minimumSystemVersion>
      <sparkle:fullReleaseNotesLink>{REPO}/releases/tag/v{html.escape(version)}</sparkle:fullReleaseNotesLink>{description}
      <enclosure url={quoteattr(url)} length="{os.path.getsize(zip_path)}" type="application/octet-stream" sparkle:edSignature={quoteattr(signature)}/>
    </item>
"""


def main():
    if len(sys.argv) not in (5, 6):
        sys.exit(__doc__)
    version, build, zip_path, signature = sys.argv[1:5]
    notes = Path(sys.argv[5]).read_text().strip() if len(sys.argv) == 6 else ""
    items = []
    if FEED.exists():
        text = FEED.read_text()
        start = text.find("    <item>")
        while start != -1:
            end = text.index("    </item>\n", start) + len("    </item>\n")
            existing = text[start:end]
            if f"<sparkle:version>{build}</sparkle:version>" not in existing:
                items.append(existing)
            start = text.find("    <item>", end)
    FEED.write_text(HEADER + item(version, build, zip_path, signature, notes) + "".join(items) + FOOTER)
    print(f"Added AutoPaper {version} ({build}) to {FEED.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
