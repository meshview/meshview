#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

"""Bring the Python meshview's web pages into meshview-rs.

The pages are the Python version's: its Jinja templates and its static files.
This copies them from a checkout of it and turns what has to change into what
meshview-rs needs:

- the templates from Jinja to Tera 2: `tr("…")` becomes the filter `"…" | tr`,
  `range()` takes its arguments by name, the one macro becomes a component,
  and the `set`s outside any block, which Tera does not allow, are dropped:
  meshview-rs passes those values in the context instead;
- the API's addresses, `/api/…`, to their version, `/api/v1/…`, and the old
  API page, `/api.html`, to the reference, `/api/docs`;
- blocks a template defines that none of its parents has, which Jinja ignores
  and Tera refuses, are dropped;
- the old API pages themselves are left out, as the reference replaces them;
- the node page's coverage help, which the Python version fetched from its
  docs/COVERAGE*.md and rendered, is meshview-rs's own template instead,
  local/templates/coverage-help.html, translated as the rest of the page is;
- the map's node dots get a border that stands out from their fill,
  local/templates/marker-stroke.js, rather than always a white one;
- the footer names and links meshview-rs, local/templates/footer-project.html,
  rather than the Python version.

Anything in a template it does not know how to convert stops it, rather than
leaving a page that fails to render. What it copied from is written to
meshview-rs/assets/IMPORTED_FROM.

    scripts/import-web.py ~/devel/mesh/meshview
"""

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "meshview-rs"
TEMPLATES = ROOT / "templates"
STATIC = ROOT / "assets" / "static"
SOURCE = ROOT / "assets" / "IMPORTED_FROM"

# Replaced by the reference at /api/docs.
LEFT_OUT = {"api.html", "api-chat.html"}

# The `set`s outside any block, and what meshview-rs passes instead.
TOP_LEVEL_SETS = {
    "index.html": "{% set dashboard = true %}",
    "map.html": "{% set dashboard = false %}",
}

# Every Jinja statement the templates are known to use, after conversion.
KNOWN_STATEMENTS = re.compile(
    r"\{%-?\s*(extends|block|endblock|include|if|else|elif|endif|for|endfor|set|endset"
    r"|component|endcomponent|<|</)\b"
)

STRING = r"""(?P<q>["'])(?P<s>(?:\\.|(?!(?P=q)).)*)(?P=q)"""
TR_CALL = re.compile(r"\btr\(\s*" + STRING + r"\s*\)")
RANGE_CALL = re.compile(r"\brange\(\s*(-?\d+)\s*,\s*(-?\d+)\s*\)")
TAG = re.compile(r"\{\{.*?\}\}|\{%.*?%\}", re.DOTALL)
API_URL = re.compile(r"/api/(?!v1/|docs\b)(?=[a-z_])")

MACRO = re.compile(
    r"\{%-?\s*macro chart_tools\(chart, export_label\)\s*-?%\}(?P<body>.*?)\{%-?\s*endmacro\s*-?%\}",
    re.DOTALL,
)
MACRO_CALL = re.compile(
    r"\{\{\s*chart_tools\(\s*" + STRING + r"\s*,\s*(?P<label>\w+)\s*\)\s*\}\}"
)


def fail(path, message):
    sys.exit(f"{path.name}: {message}")


def convert_tags(text):
    """`tr()` and `range()` inside Jinja tags, and only there."""

    def tag(match):
        inside = match.group(0)
        inside = TR_CALL.sub(lambda m: f"{m['q']}{m['s']}{m['q']} | tr", inside)
        return RANGE_CALL.sub(r"range(start=\1, end=\2)", inside)

    return TAG.sub(tag, text)


BLOCK = re.compile(r"\{%-?\s*block (\w+)\s*-?%\}")
EXTENDS = re.compile(r'\{%-?\s*extends "([^"]+)"\s*-?%\}')


def blocks_of(directory, name):
    """The blocks a template and the ones it extends define."""
    text = (directory / name).read_text(encoding="utf-8")
    found = set(BLOCK.findall(text))
    parent = EXTENDS.search(text)
    return found | (blocks_of(directory, parent[1]) if parent else set())


def drop_unknown_blocks(path, text):
    parent = EXTENDS.search(text)
    if not parent:
        return text
    known = blocks_of(path.parent, parent[1])
    for name in sorted(set(BLOCK.findall(text)) - known):
        block = re.compile(
            r"\{%-?\s*block "
            + name
            + r"\s*-?%\}(?P<body>.*?)\{%-?\s*endblock(?: "
            + name
            + r")?\s*-?%\}\n?",
            re.DOTALL,
        )
        match = block.search(text)
        if not match or "{% block" in match["body"]:
            fail(path, f"block {name} is in no parent, and cannot be dropped")
        text = text[: match.start()] + text[match.end() :]
    return text


# The node page's coverage help: the dialog's body, and the script that fetched
# and rendered the Markdown into it when the dialog first opened.
HELP_BODY = re.compile(
    r'(<div id="coverage-help-body" class="mesh-dialog-body">).*?(</div>)'
)
HELP_FETCH = re.compile(
    r"\n        if \(loaded\) return;\n        try \{.*?\n        \} catch \(err\) \{.*?\n        \}\n",
    re.DOTALL,
)


def local_coverage_help(path, text):
    text, bodies = HELP_BODY.subn(r'\1{% include "coverage-help.html" %}\2', text)
    text, fetches = HELP_FETCH.subn("\n", text)
    text = re.sub(
        r"<!-- Coverage help:[^>]*-->",
        "<!-- Coverage help: meshview-rs's own, local/templates/coverage-help.html. -->",
        text,
    )
    if (bodies, fetches) != (1, 1) or "COVERAGE" in text:
        fail(path, "the coverage help is not where it was: teach local_coverage_help")
    return text


# The map's node dots: drawn, and given back their look when deselected, with a
# thin white border.
MARKER_DRAWN = (
    '            color: "white",\n            fillColor: color,\n'
    "            fillOpacity: 1,\n            weight: 0.7\n"
)
MARKER_RESTORED = 'selectedMarker.setStyle({ color: "white", weight: 0.7 });'
MARKER_COLOURS = "function hashToColor(str){"


def local_marker_stroke(path, text):
    replacements = [
        (
            MARKER_DRAWN,
            (
                "            color: markerStroke(color),\n            fillColor: color,\n"
                "            fillOpacity: 1,\n            weight: 1\n"
            ),
        ),
        (
            MARKER_RESTORED,
            (
                "selectedMarker.setStyle({ color: markerStroke(selectedMarker.originalColor), "
                "weight: 1 });"
            ),
        ),
        (MARKER_COLOURS, '{% include "marker-stroke.js" %}\n' + MARKER_COLOURS),
    ]
    for old, new in replacements:
        if text.count(old) != 1:
            fail(
                path,
                "the node dots are not drawn as they were: teach local_marker_stroke",
            )
        text = text.replace(old, new)
    return text


# The footer's link to the project, and the script that writes the version in it.
FOOTER_LINK = re.compile(
    r'<a class="mesh-footer-project" href="[^"]*".*?</a>', re.DOTALL
)
FOOTER_VERSION = 'site.version ? `Meshview ${site.version}` : "Meshview";'
PROJECT_NAME = "Meshview Rust"


def local_footer(path, text):
    text, links = FOOTER_LINK.subn('{% include "footer-project.html" %}', text)
    if links != 1 or text.count(FOOTER_VERSION) != 1:
        fail(path, "the footer is not as it was: teach local_footer")
    return text.replace(
        FOOTER_VERSION,
        f'site.version ? `{PROJECT_NAME} ${{site.version}}` : "{PROJECT_NAME}";',
    )


def convert_template(path):
    text = drop_unknown_blocks(path, path.read_text(encoding="utf-8"))
    if path.name == "node.html":
        text = local_coverage_help(path, text)
    if path.name == "map-view.html":
        text = local_marker_stroke(path, text)
    if path.name == "base.html":
        text = local_footer(path, text)

    top_level = TOP_LEVEL_SETS.get(path.name)
    if top_level:
        if top_level not in text:
            fail(path, f"expected {top_level!r}")
        text = text.replace(top_level + "\n", "").replace(top_level, "")

    # The macro becomes a component, defined at the top level of the template
    # (Tera allows components there, not inside a block), and its calls tags.
    macro = MACRO.search(text)
    if macro:
        component = (
            "{% component chart_tools(chart: string, export_label: string) %}"
            + macro["body"].strip()
            + "{% endcomponent chart_tools %}\n"
        )
        text = text[: macro.start()] + text[macro.end() :]
        extends = re.match(r'\{% extends "[^"]+" %\}\n', text)
        if not extends:
            fail(path, "a macro outside a child template")
        text = text[: extends.end()] + component + text[extends.end() :]
        text = MACRO_CALL.sub(
            lambda m: (
                f'{{{{<chart_tools chart="{m["s"]}" export_label={{{m["label"]}}} />}}}}'
            ),
            text,
        )
    if "macro" in re.sub(r"\{\{.*?\}\}", "", "".join(TAG.findall(text))):
        fail(path, "a macro it does not know")

    text = convert_tags(text)

    for statement in re.findall(r"\{%-?\s*[^%]*?%\}", text):
        if not KNOWN_STATEMENTS.match(statement):
            fail(path, f"a statement it does not know: {statement}")
    for tag in TAG.findall(text):
        if re.search(r"\btr\(|\brange\((?!start=)", tag):
            fail(path, f"a call it could not convert: {tag}")

    return rewrite_urls(text)


def rewrite_urls(text):
    text = API_URL.sub("/api/v1/", text)
    return text.replace('href="/api.html"', 'href="/api/docs"')


def git(source, *args):
    return subprocess.run(
        ["git", "-C", str(source), *args], capture_output=True, text=True, check=True
    ).stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("source", type=Path, help="a checkout of the Python meshview")
    args = parser.parse_args()
    source = args.source.resolve()
    package = source / "meshview"
    if not (package / "templates" / "base.html").is_file():
        sys.exit(f"{source} is not a checkout of the Python meshview")

    shutil.rmtree(TEMPLATES, ignore_errors=True)
    shutil.rmtree(STATIC, ignore_errors=True)
    TEMPLATES.mkdir(parents=True)

    for template in sorted((package / "templates").glob("*.html")):
        (TEMPLATES / template.name).write_text(
            convert_template(template), encoding="utf-8"
        )

    shutil.copytree(
        package / "static",
        STATIC,
        ignore=lambda _, names: [n for n in names if n in LEFT_OUT],
    )
    for path in STATIC.rglob("*"):
        if (
            path.suffix in {".html", ".js"}
            and "vendor" not in path.relative_to(STATIC).parts
        ):
            path.write_text(
                rewrite_urls(path.read_text(encoding="utf-8")), encoding="utf-8"
            )

    commit = git(source, "rev-parse", "HEAD")
    branch = git(source, "rev-parse", "--abbrev-ref", "HEAD")
    dirty = (
        " (with uncommitted changes)"
        if git(source, "status", "--porcelain", "meshview")
        else ""
    )
    SOURCE.write_text(f"{commit} {branch}{dirty}\n", encoding="utf-8")
    print(f"Imported from {commit[:7]} ({branch}){dirty}")


if __name__ == "__main__":
    main()
