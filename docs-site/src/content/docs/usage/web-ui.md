---
title: Web UI
description: Browse and search the unified local archive in a browser with `sivtr web`.
---

`sivtr web` serves a local web UI and a read-only JSON API over the unified archive (`archive.db`). Use it to browse terminal and agent sessions and to run full-text search ranked by BM25 relevance without opening the TUI.

## Start the server

```bash
sivtr web
```

Then open <http://127.0.0.1:8080> in a browser.

| Option | Meaning |
| --- | --- |
| `--port <PORT>` | TCP port to bind (default `8080`) |
| `--host <HOST>` | Bind address (default `127.0.0.1`, loopback only) |

## Using the UI

- **Session browser** — filter sessions by provider, open a session, and copy its refs for use with `sivtr show`.
- **Search** — press `/` anywhere to focus the search box; results are ranked by BM25 relevance.

## JSON API

The same surface is available as a read-only JSON API:

| Endpoint | Purpose |
| --- | --- |
| `GET /api/v1/health` | Server health check |
| `GET /api/v1/providers` | List registered providers |
| `GET /api/v1/sessions?provider=&limit=&offset=` | List sessions with optional filters |
| `GET /api/v1/sessions/{provider}/{session_id}` | One full session |
| `GET /api/v1/search?q=&source=all|all:agent|all:terminal|<selector>&limit=` | Full-text search |

## Privacy

The server binds to loopback by default, so the UI is only reachable from this machine. It is strictly read-only — no writes — and data never leaves the machine. The server also validates the browser `Host` header to guard against DNS rebinding.

Changing `--port` also changes the accepted `Host`, so requests must target the same `host:port` the server is bound to.
