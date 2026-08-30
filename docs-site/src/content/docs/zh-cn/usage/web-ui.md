---
title: Web UI
description: 用 `sivtr web` 在浏览器中浏览和搜索统一的本地 archive。
---

`sivtr web` 在统一 archive（`archive.db`）之上提供本地 web UI 和只读 JSON API。用它浏览终端和 Agent session，或运行按 BM25 相关性排序的全文搜索，而不必打开 TUI。

## 启动 server

```bash
sivtr web
```

然后在浏览器打开 <http://127.0.0.1:8080>。

| 选项 | 含义 |
| --- | --- |
| `--port <PORT>` | 要绑定的 TCP 端口（默认 `8080`） |
| `--host <HOST>` | 绑定地址（默认 `127.0.0.1`，仅 loopback） |

## 使用 UI

- **Session 浏览器** — 按 provider 过滤 session、打开一个 session、复制它的 refs，供 `sivtr show` 使用。
- **搜索** — 在任意位置按 `/` 聚焦搜索框；结果按 BM25 相关性排序。

## JSON API

同样的表面也可以作为只读 JSON API 使用：

| Endpoint | 用途 |
| --- | --- |
| `GET /api/v1/health` | Server 健康检查 |
| `GET /api/v1/providers` | 列出已注册 provider |
| `GET /api/v1/sessions?provider=&limit=&offset=` | 列出 session，支持可选过滤 |
| `GET /api/v1/sessions/{provider}/{session_id}` | 单个完整 session |
| `GET /api/v1/search?q=&source=all|all:agent|all:terminal|<selector>&limit=` | 全文搜索 |

## 隐私

server 默认只绑定 loopback，因此 UI 只能从本机访问。它是严格只读的——不做任何写入——数据也不会离开本机。server 还会校验浏览器的 `Host` header，以防范 DNS rebinding。

修改 `--port` 也会改变接受的 `Host`，所以请求必须指向 server 实际绑定的同一个 `host:port`。
