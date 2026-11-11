# API

## Versions

The API is served under a version: `/api/v1/...`. A version's addresses and the
shape of their answers do not change once published; anything that would
break a client goes into the next version instead, served beside it.

`v1` is the Python meshview 4.0 API, address for address and answer for answer,
so that the web pages and anything else written against the Python version work
unchanged, apart from the [differences](#differences-from-the-python-version)
below. Each endpoint is checked against the Python version, answer by answer,
before it is called ported (see [Development](development.md#comparing-the-api)).

| Endpoint | State |
|---|---|
| `/version` | Ported |
| `/api/v1/channels` | Ported |
| `/api/v1/snapshots/daily` | Ported |
| `/api/v1/packets_seen/{packet_id}` | Ported |
| `/health` | Not yet |
| `/api/v1/config`, `/api/v1/lang` | Not yet |
| `/api/v1/nodes`, `/api/v1/nodes/facets` | Not yet |
| `/api/v1/packets` | Not yet |
| `/api/v1/stats`, `/api/v1/stats/count`, `/api/v1/stats/top` | Not yet |
| `/api/v1/edges` | Not yet |
| `/api/v1/traceroute/{packet_id}` | Not yet |
| `/api/v1/node/{node_id}/qr` | Not yet |
| `/api/v1/coverage/{node_id}` | Later |

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

## Differences from the Python version

What `v1` answers is what the Python version answers, with these exceptions.
Plain bugs (a crash, a wrong figure, an answer that depends on chance) are fixed;
behaviour a client could rely on is kept even where it is odd, and is a candidate
for changing in a future version instead.

| | Python meshview | meshview-rs |
|---|---|---|
| JSON | Python's `json.dumps`: spaces after `,` and `:`, everything outside ASCII escaped, and bare `NaN` or `Infinity`, which is not JSON, where a value is not a number | Standard compact JSON in UTF-8; a value that is not a number is `null`. The values are the same. |
| Errors the handlers did not foresee | Plain text: `404: Not Found`, `500 Internal Server Error` | JSON under `/api/v1`, `{"error": "..."}`, as the errors the handlers do foresee already were. |
| Compression | gzip whenever `Accept-Encoding` contains `gzip`, even `gzip;q=0` | `Accept-Encoding` negotiated properly. Same content types, same 1 KiB threshold. |
| `/api/v1/channels` window | Computed from the local time taken as UTC: off by the server's offset from UTC | From now. The same answer on a server running in UTC. |
| `/version` `release_date` | The release's date | The date of the commit the build comes from. |
| `/version` `git_revision` | Asked of git on every request, from the checkout the server runs from | Recorded at build time. |

## Outside the API

`/version` is not part of the API and has no version, as in the Python
version. Neither have the probes, `/livez` and `/readyz`, which are described in
[Deployment](deployment.md#probes).
