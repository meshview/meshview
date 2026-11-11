#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>
"""Compare the answers of the Python meshview's web server and meshview-rs.

    scripts/compare-api.py PYTHON_URL RUST_URL [REQUESTS_FILE]

Each request is sent to both servers, the Python one at its unversioned address
and meshview-rs at the same address under /api/v1 (and as it is for those
outside the API), and the two answers compared: status, content type, and the
JSON body value for value. Both have to serve copies of the same database, with
nothing ingesting into either, or they will rightly differ.

REQUESTS_FILE has one request per line, a path with its query string, optionally
followed by `ignore=` and comma separated dotted keys left out of the comparison
because they differ by nature (`ignore=version,timestamp`, where `a.b` is key b
of object a and `*` matches any key or index). Blank lines and lines starting with
# are skipped. Without a file, the requests are read from standard input.

Exits with 1 when any answer differs. Only the standard library is used.
"""

import json
import sys
import urllib.error
import urllib.request

# Answers outside the API, not moved under /api/v1.
UNVERSIONED = ("/health", "/version", "/livez", "/readyz")


def fetch(url):
    request = urllib.request.Request(url, headers={"Accept-Encoding": "identity"})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status, response.headers.get("Content-Type", ""), response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.headers.get("Content-Type", ""), error.read()


def rust_path(path):
    if path.startswith("/api/") and not path.startswith("/api/v1/"):
        return "/api/v1/" + path[len("/api/") :]
    return path


def drop(value, keys):
    """The value with the dotted keys removed, wherever `*` says."""
    for key in keys:
        value = _drop(value, key.split("."))
    return value


def _drop(value, parts):
    if not parts:
        return value
    head, rest = parts[0], parts[1:]
    if isinstance(value, dict):
        targets = list(value) if head == "*" else [head]
        out = dict(value)
        for target in targets:
            if target not in out:
                continue
            if rest:
                out[target] = _drop(out[target], rest)
            else:
                del out[target]
        return out
    if isinstance(value, list) and head == "*":
        return [_drop(item, rest) if rest else item for item in value]
    return value


def differences(python, rust, where="$"):
    """Where two JSON values differ, as readable lines."""
    if type(python) is not type(rust) and not (
        isinstance(python, (int, float)) and isinstance(rust, (int, float))
        and not isinstance(python, bool) and not isinstance(rust, bool)
    ):
        return [f"{where}: python {type(python).__name__} {short(python)}, "
                f"rust {type(rust).__name__} {short(rust)}"]
    if isinstance(python, dict):
        out = []
        for key in sorted(set(python) | set(rust)):
            if key not in rust:
                out.append(f"{where}.{key}: only in python ({short(python[key])})")
            elif key not in python:
                out.append(f"{where}.{key}: only in rust ({short(rust[key])})")
            else:
                out += differences(python[key], rust[key], f"{where}.{key}")
        return out
    if isinstance(python, list):
        out = []
        if len(python) != len(rust):
            out.append(f"{where}: python has {len(python)} items, rust {len(rust)}")
        for index, (a, b) in enumerate(zip(python, rust)):
            out += differences(a, b, f"{where}[{index}]")
        return out
    if python != rust:
        return [f"{where}: python {short(python)}, rust {short(rust)}"]
    return []


def short(value, length=100):
    text = json.dumps(value, ensure_ascii=False)
    return text if len(text) <= length else text[:length] + "…"


def compare(python_url, rust_url, path, ignore, show=10):
    p_status, p_type, p_body = fetch(python_url + path)
    r_status, r_type, r_body = fetch(rust_url + rust_path(path))
    problems = []
    if p_status != r_status:
        problems.append(f"status: python {p_status}, rust {r_status}")
    if p_type.split(";")[0] != r_type.split(";")[0]:
        problems.append(f"content type: python {p_type!r}, rust {r_type!r}")
    if "json" in p_type and "json" in r_type:
        try:
            python = drop(json.loads(p_body), ignore)
            rust = drop(json.loads(r_body), ignore)
            problems += differences(python, rust)
        except json.JSONDecodeError as error:
            problems.append(f"not JSON: {error}")
    elif p_body != r_body:
        problems.append(f"body: python {len(p_body)} bytes, rust {len(r_body)} bytes, not equal")

    print(f"{'OK' if not problems else 'DIFF':4} {path}")
    for problem in problems[:show]:
        print(f"       {problem}")
    if len(problems) > show:
        print(f"       ... and {len(problems) - show} more")
    return not problems


def requests(lines):
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        path, _, rest = line.partition(" ")
        ignore = []
        for option in rest.split():
            if option.startswith("ignore="):
                ignore += [key for key in option[len("ignore="):].split(",") if key]
        yield path, ignore


def main():
    if len(sys.argv) not in (3, 4):
        sys.exit(__doc__)
    python_url, rust_url = (url.rstrip("/") for url in sys.argv[1:3])
    source = open(sys.argv[3], encoding="utf-8") if len(sys.argv) == 4 else sys.stdin
    with source:
        results = [compare(python_url, rust_url, path, ignore) for path, ignore in requests(source)]
    same = sum(results)
    print(f"{same} of {len(results)} the same" + ("" if same == len(results) else ": DIFFERENCES FOUND"))
    sys.exit(0 if same == len(results) else 1)


if __name__ == "__main__":
    main()
