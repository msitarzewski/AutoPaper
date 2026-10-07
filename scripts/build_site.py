#!/usr/bin/env python3
"""Builds the GitHub Pages site into _site/.

Hand-written pages are HTML fragments in site/pages/; PRIVACY.md and NETWORK.md are rendered from the repo root
with pandoc, so the site can never disagree with them. Every page shares site/layout.html. Static files, including
the update feeds the release scripts write (appcast.xml, AutoPaper.appinstaller), are copied as they are.
Adapted from AudioPaper's scripts/build_site.py.

    python3 scripts/build_site.py            # build into _site/
    python3 -m http.server -d _site 8090     # preview at http://localhost:8090
"""
import html
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
OUT = ROOT / "_site"
# Links from the rendered documents to other files in the repo go here.
REPO = "https://github.com/msitarzewski/AutoPaper"
BASE_URL = "https://msitarzewski.github.io/AutoPaper/"

# (output file, nav label or None). Order is the nav order; pages without a label are linked from the footer
# instead (as on AudioPaper's site). Only pages that were actually built are linked, and every built entry goes
# in the sitemap.
NAV = [("index.html", "Home"), ("help.html", "Help"), ("shortcuts.html", "Shortcuts"),
       ("reference.html", "Reference"), ("privacy.html", "Privacy"), ("network.html", "Network"),
       ("credits.html", None)]

# Markdown documents rendered into pages, when they exist: source → (output, description).
MARKDOWN = {
    "PRIVACY.md": ("privacy.html", "What AutoPaper sends, to whom, and what stays on your computer."),
    "NETWORK.md": ("network.html", "Every network request AutoPaper makes: host, trigger, what's sent, and the limits."),
}
# Links between the rendered documents stay on the site; other repo paths go to GitHub.
LOCAL_LINKS = {"PRIVACY.md": "privacy.html", "NETWORK.md": "network.html"}


def front_matter(text):
    """Reads the leading <!-- key: value --> block of a page fragment."""
    match = re.match(r"\s*<!--(.*?)-->", text, re.S)
    meta = {}
    if match:
        for line in match.group(1).strip().splitlines():
            key, _, value = line.partition(":")
            meta[key.strip()] = value.strip()
        text = text[match.end():]
    return meta, text


def render_markdown(source):
    body = subprocess.run(
        ["pandoc", "--from=gfm", "--to=html5", "--wrap=none", str(ROOT / source)],
        check=True, capture_output=True, text=True,
    ).stdout

    def link(match):
        target, attrs, text = match.groups()
        if re.match(r"[a-z]+:|#", target):
            return match.group(0)
        path, _, anchor = target.removeprefix("./").partition("#")
        if path in LOCAL_LINKS:
            url = LOCAL_LINKS[path]
        else:
            url = f"{REPO}/{'tree' if path.endswith('/') else 'blob'}/main/{path}"
        return f'<a href="{url}{"#" + anchor if anchor else ""}"{attrs}>{text}</a>'

    body = re.sub(r'<a href="([^"]+)"([^>]*)>(.*?)</a>', link, body, flags=re.S)
    title = re.search(r"<h1[^>]*>(.*?)</h1>", body, re.S).group(1)
    source_note = (f'<p class="doc-source">This page is <a href="{REPO}/blob/main/{source}">{source}</a> '
                   f'from the repository, published as written.</p>')
    body = re.sub(r"(</h1>)", lambda m: m.group(1) + source_note, body, count=1)
    # Wide tables scroll inside their own box on narrow screens.
    body = body.replace("<table>", '<div class="table-wrap"><table>').replace("</table>", "</table></div>")
    return re.sub(r"<[^>]+>", "", title), f'<article class="doc">{body}</article>'


def page(layout, name, built, title, description, content):
    links = []
    for href, label in NAV:
        if label and href in built:
            current = ' aria-current="page"' if href == name else ""
            links.append(f'<a href="{href}"{current}>{label}</a>')
    full_title = "AutoPaper" if name == "index.html" else f"{title} — AutoPaper"
    return (layout
            .replace("{{title}}", html.escape(full_title))
            .replace("{{description}}", html.escape(description, quote=True))
            .replace("{{url}}", BASE_URL + ("" if name == "index.html" else name))
            .replace("{{nav}}", "\n".join(links))
            .replace("{{content}}", content))


def main():
    markdown = {source: entry for source, entry in MARKDOWN.items() if (ROOT / source).exists()}
    if markdown and not shutil.which("pandoc"):
        sys.exit("pandoc is required to render " + ", ".join(markdown) + " (brew install pandoc)")

    shutil.rmtree(OUT, ignore_errors=True)
    # examples.json is the source for the example captions and credits, not part of the site.
    shutil.copytree(SITE / "static", OUT, ignore=shutil.ignore_patterns("examples.json", ".DS_Store"))
    layout = (SITE / "layout.html").read_text()

    pages = {}
    for fragment in sorted((SITE / "pages").glob("*.html")):
        meta, content = front_matter(fragment.read_text())
        pages[fragment.name] = (meta["title"], meta["description"], content)
    for source, (name, description) in markdown.items():
        title, content = render_markdown(source)
        pages[name] = (title, description, content)

    built = set(pages)
    for name, (title, description, content) in pages.items():
        (OUT / name).write_text(page(layout, name, built, title, description, content))

    (OUT / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
        + "".join(f"  <url><loc>{BASE_URL}{'' if n == 'index.html' else n}</loc></url>\n"
                  for n, _ in NAV if n in built)
        + "</urlset>\n")
    skipped = [source for source in MARKDOWN if source not in markdown]
    note = f" (skipped {', '.join(skipped)}: not written yet)" if skipped else ""
    print(f"Built {len(pages)} pages into {OUT.relative_to(ROOT)}/{note}")


if __name__ == "__main__":
    main()
