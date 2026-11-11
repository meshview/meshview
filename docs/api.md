# API: what changed from the Python meshview

The API documents itself: every endpoint, its parameters and its answers are
in the reference meshview-rs serves, generated from the code that answers, so it
cannot disagree with it.

| Address | |
|---|---|
| `/api/docs` | The reference, as a page to read and try calls from. |
| `/api/openapi.json` | The OpenAPI document behind it, for clients and code generators. |

Both are in the site's language, `[site] language`, when there is a translation
of the reference for it (`es` and `ru`), and in English otherwise.
`/api/openapi.json?lang=en` (or `es`, `ru`) asks for another; a language there
is no translation for gets English.

This page is only about what is different for someone coming from the Python
meshview 4.0: where the API moved to, and what it answers differently.

## Versions

The API is now served under a version, `/api/v1/...`. A version's addresses and
the shape of its answers do not change once published; anything that would break
a client goes into the next version instead, served beside it.

`v1` is the Python meshview 4.0 API, address for address and answer for answer,
apart from the [differences](#differences-from-the-python-version) below. Each
endpoint is checked against the Python version before it is called ported (see
[Development](development.md#comparing-the-api)).

`/health` and `/version` stay where they were, outside any version, as do the
probes `/livez` and `/readyz` (see [Deployment](deployment.md#probes)).

## The Python addresses

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
| `/api/v1/config` `cleanup` | The values as written in `config.ini`, so `minute = 00` gave `"00"` | The configured numbers, as strings, as before: `"0"`. TOML numbers have no leading zeros. |
| `/api/v1/config` `site.version` | The Python version's | meshview-rs's |
| `/health` `cleanup` | From the JSON file the cleanup wrote, with its path as `status_file` | From the cleanup's latest run in `job_run`, with no `status_file`; `last_run` has the same keys. |
| `/health` `backup` | The scheduled backups' settings and last run | `{"enabled": false, "status": "disabled"}`: meshview-rs does not back up by itself (see [Deployment](deployment.md#backups)). |
| `/health` `snapshot` | Not reported | The daily snapshot's schedule, status and latest run, as `cleanup`'s. |
| Empty filters (`?channel=`, `?type=`) | Sometimes a filter on the empty string, which matches nothing, sometimes no filter, sometimes a different default | Always no filter: a filter on nothing filters nothing. The one exception is `ids=` in `/api/v1/nodes`, a list of no nodes, which still matches none, as the list of favourites needs when there are none. |
| `/api/v1/stats?node=` | `sent` and `seen` always 0: it read a key its query does not return | The totals it meant: packets the node sent, and packets sent to it, in the window. |
| `/api/v1/stats` invalid `period_type` | The message lists `{'hour', 'day'}` in an order that changes from one start to the next | Always `{'hour', 'day'}`. |
| `/api/v1/stats/count` unknown `period_type` | 500, in plain text | 400, `{"error": "period_type must be 'hour' or 'day'"}`. |
| `/api/v1/nodes` when the database fails | `{"nodes": []}`, as if there were none | 500, `{"error": "Failed to fetch nodes"}`. |
| `/api/v1/nodes` with a `node_id` or `days_active` too large | An empty list without paging, a 500 with it | No nodes; or, for `days_active`, every node. |
| `/api/v1/packets?contains=` outside text messages | `%` and `_` are wildcards: `contains=%` matches every packet | Taken literally, as the parameter is documented: a substring. Text messages (`portnum=1`) were searched literally already. |
| `/api/v1/packets` with a packet whose port or payload is null | 500 for the whole page | The packet, with a null `portnum`, or `Did not decode`. |
| `/api/v1/traceroute/{id}` with a report that has no route | 500 | The report left out, as one whose route cannot be read. |
| `/api/v1/coverage/{id}`: the model | pyitm 0.3, a Python translation of ITM 1.2.2 with four bugs of its own | NTIA's ITM, version 1.4, through the [`longley-rice`](../longley-rice) crate. At a reliability of 50% or more, as the pages ask for, the loss differs by about 0.001 dB and the reach comes out the same, or one step shorter; below 50% pyitm was up to 6.6 dB too pessimistic. |
| `/api/v1/coverage/{id}` when the model refuses the inputs (an antenna height of 0 m) | 500 | The points it refuses are left out, and the answer has those it does not. |
| `/api/v1/coverage/{id}` asked for at the same time | pyitm keeps its working values in globals and the predictions run in threads, so predictions made at once can mix them up and come out wrong | Each prediction is computed on its own; answers do not depend on what else is asked for. |
| `/api/v1/coverage/{id}` without the model | 503, when pyitm is not installed | Always available: the model is built in. |
