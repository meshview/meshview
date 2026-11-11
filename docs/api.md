# API

## Versions

The API is served under a version: `/api/v1/...`. A version's addresses and the
shape of their answers do not change once published; anything that would
break a client goes into the next version instead, served beside it.

`v1` is the Python meshview 4.0 API, address for address and answer for answer,
so that the web pages and anything else written against the Python version work
unchanged. Its endpoints are being ported; until they are, `/api/v1` has none.

## The reference

| Address | |
|---|---|
| `/api/docs` | The reference, as a page to read and try calls from. |
| `/api/openapi.json` | The OpenAPI document behind it, for clients and code generators. |

Both are generated from the code that registers the routes, so they cannot
disagree with what is served. They sit outside the version because they
document all of them.

## The unversioned addresses

The Python version served the same API without a version: `/api/nodes`,
`/api/stats/top` and so on. Those addresses keep working for now, as permanent
redirects to their `/api/v1` counterparts:

```text
GET /api/stats/top?limit=5

HTTP/1.1 308 Permanent Redirect
location: /api/v1/stats/top?limit=5
link: </api/v1/stats/top>; rel="successor-version"
deprecation: @1794355200
```

- **308**, so the method and body are kept along with the query string.
- **`Deprecation`** ([RFC 9745](https://www.rfc-editor.org/rfc/rfc9745)) says
  since when the address is on its way out, here 2026-11-11.
- **`Link`** names the address that replaces it.

Browsers and `fetch` follow the redirect by themselves. A client that does not
follow redirects, `curl` without `-L` among them, sees the 308 instead of the
answer and needs pointing at `/api/v1`. The redirects will be removed once they
are no longer used.

## Outside the API

The probes, `/livez` and `/readyz`, are not part of the API and have no version.
They are described in [Deployment](deployment.md#probes).
