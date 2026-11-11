#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

"""Bring the Python meshview's web pages into meshview-rs.

The pages are the Python version's: its Jinja templates, its static files and
the coverage documents the node page shows. This copies them from a checkout of
it and turns what has to change into what meshview-rs needs:

- the templates from Jinja to Tera 2: `tr("…")` becomes the filter `"…" | tr`,
  `range()` takes its arguments by name, the one macro becomes a component,
  and the `set`s outside any block, which Tera does not allow, are dropped:
  meshview-rs passes those values in the context instead;
- the API's addresses, `/api/…`, to their version, `/api/v1/…`, and the old
  API page, `/api.html`, to the reference, `/api/docs`;
- blocks a template defines that none of its parents has, which Jinja ignores
  and Tera refuses, are dropped;
- the old API pages themselves are left out, as the reference replaces them.

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
DOCS = ROOT / "assets" / "docs"
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


def convert_template(path):
    text = drop_unknown_blocks(path, path.read_text(encoding="utf-8"))

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
    shutil.rmtree(DOCS, ignore_errors=True)
    TEMPLATES.mkdir(parents=True)
    DOCS.mkdir(parents=True)

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

    for doc in sorted((source / "docs").glob("COVERAGE*.md")):
        shutil.copy2(doc, DOCS / doc.name)

    commit = git(source, "rev-parse", "HEAD")
    branch = git(source, "rev-parse", "--abbrev-ref", "HEAD")
    dirty = (
        " (with uncommitted changes)"
        if git(source, "status", "--porcelain", "meshview", "docs")
        else ""
    )
    SOURCE.write_text(f"{commit} {branch}{dirty}\n", encoding="utf-8")
    print(f"Imported from {commit[:7]} ({branch}){dirty}")


if __name__ == "__main__":
    main()
