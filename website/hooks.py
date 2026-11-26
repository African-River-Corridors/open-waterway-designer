"""MkDocs hooks for the OWD docs.

on_config: expands a nav entry "adr/" or "specs/" into every page in that folder, so a new ADR or
spec shows up without editing mkdocs.yml.

on_page_content: turns every formula into MathML (the docs write formulas as $...$ and $$...$$, as
GitHub reads them; `pymdownx.arithmatex` marks them, and `latex2mathml` converts them here). A page
then needs no script and no request to show its formulas. A formula that cannot be converted fails
the build.

on_post_page: makes every relative URL an absolute, clean /owd/docs/... path. The ARC site
serves the docs at /owd/docs with no trailing slash, where a relative URL would resolve against
the wrong folder. Clean means no ".html": the ARC site redirects ".html" URLs, and GitHub Pages
serves page.html for /page.
"""

import html
import json
import re
from pathlib import Path
from urllib.parse import urljoin, urlsplit

PREFIX = "/owd/docs/"
ATTR = re.compile(r'\b(href|src)="([^"]*)"')
CONFIG = re.compile(r'(<script id="__config" type="application/json">)(.*?)(</script>)', re.S)


MATH = re.compile(r'<(span|div) class="arithmatex">(.*?)</\1>', re.S)


def to_mathml(m):
    from latex2mathml.converter import convert

    tex = html.unescape(m[2]).strip()
    block = m[1] == "div"
    opener, closer = ("\\[", "\\]") if block else ("\\(", "\\)")
    if not (tex.startswith(opener) and tex.endswith(closer)):
        raise ValueError(f"a formula is not wrapped as expected: {tex[:60]!r}")
    mathml = convert(tex[2:-2].strip(), display="block" if block else "inline")
    if block:
        return f'<div class="owd-math">{mathml}</div>'
    return mathml


def on_page_content(html_text, page, config, files):
    return MATH.sub(to_mathml, html_text)


def on_config(config):
    docs = Path(config["docs_dir"])
    nav = []
    for item in config["nav"]:
        if isinstance(item, dict):
            (title, value), = item.items()
            if isinstance(value, str) and value.endswith("/"):
                value = [f"{value}{p.name}" for p in sorted((docs / value).glob("*.md"))]
            item = {title: value}
        nav.append(item)
    config["nav"] = nav
    return config


def clean(path: str) -> str:
    """'/owd/docs/adr/x.html#a' -> '/owd/docs/adr/x#a'; the docs home -> '/owd/docs'."""
    parts = urlsplit(path)
    p = parts.path
    if p.endswith("/index.html"):
        p = p[: -len("/index.html")]
    elif p.endswith(".html"):
        p = p[: -len(".html")]
    elif p.endswith("/") and len(p) > 1:
        p = p[:-1]
    return p + (f"?{parts.query}" if parts.query else "") + (f"#{parts.fragment}" if parts.fragment else "")


def absolute(value: str, base: str) -> str:
    if not value or re.match(r"^([a-z][a-z0-9+.-]*:|#|/)", value, re.I):
        return value
    return clean(urljoin(base, value))


def on_post_page(output, page, config):
    base = PREFIX + page.file.url
    output = ATTR.sub(lambda m: f'{m[1]}="{absolute(m[2], base)}"', output)

    site = config["site_url"].rstrip("/")
    output = output.replace(
        f'<link rel="canonical" href="{site}/{page.file.url}">',
        f'<link rel="canonical" href="https://www.africanrivercorridors.com{clean(PREFIX + page.file.url)}">',
    )

    def fix_config(m):
        data = json.loads(m[2])
        data["base"] = PREFIX  # with the slash: material joins URLs onto it
        data["search"] = urljoin(base, data["search"])
        return m[1] + json.dumps(data) + m[3]

    return CONFIG.sub(fix_config, output)


def on_post_build(config):
    """Search results link to clean URLs too."""
    index = Path(config["site_dir"]) / "search" / "search_index.json"
    data = json.loads(index.read_text(encoding="utf-8"))
    for doc in data["docs"]:
        doc["location"] = clean(doc["location"]).removeprefix("/") if doc["location"] not in ("", "index.html") else ""
    index.write_text(json.dumps(data), encoding="utf-8")
