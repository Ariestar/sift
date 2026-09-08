//! Local web UI + JSON API over the unified archive.
//!
//! Binds loopback by default, validates the `Host` header against DNS
//! rebinding, and serves the embedded single-page UI plus a read-only JSON
//! surface (`/api/v1/...`) backed by the same archive functions the CLI
//! uses. Nothing here writes to the archive: freshness comes from the same
//! stamp-gated sync pass the CLI performs before queries.

use anyhow::{Context, Result};
use axum::extract::{Path as AxPath, Query as AxQuery};
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::json;

use sivtr_core::archive::store::{self, BlobMode};
use sivtr_core::query::NO_RECORD_FOR_SELECTOR;
use sivtr_core::search::{Field, Filter, Sort};

use crate::cli::{SearchArgs, WebArgs};
use crate::commands::memory::workset;

/// Embedded static assets for the UI (`web/` in the repository).
#[derive(RustEmbed)]
#[folder = "web"]
struct Assets;

/// Loopback names `sivtr web` will bind. Anything else would expose the
/// unauthenticated archive to the network; `guard_host` cannot prevent that
/// because clients control the `Host` header.
fn is_loopback_bind(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// Serve the web UI until interrupted.
pub fn execute(args: &WebArgs) -> Result<()> {
    if !is_loopback_bind(&args.host) {
        anyhow::bail!(
            "sivtr web only binds loopback (127.0.0.1, localhost, ::1); got --host {}",
            args.host
        );
    }
    sivtr_core::archive::sync::ensure_fresh()
        .context("Failed to refresh the archive before serving the web UI")?;
    let runtime = tokio::runtime::Runtime::new().context("Failed to start async runtime")?;
    runtime.block_on(serve(args.host.clone(), args.port))
}

fn bind_addr(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

async fn serve(host: String, port: u16) -> Result<()> {
    let addr = bind_addr(&host, port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind web server on {addr}"))?;
    let bound = listener
        .local_addr()
        .context("Failed to read the bound web server address")?;
    let app = router(bound.port());
    crate::output::info(format!("sivtr web UI listening on http://{bound}"));
    axum::serve(listener, app).await.context("web server error")
}

fn router(port: u16) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/providers", get(providers))
        .route("/api/v1/sessions", get(sessions))
        .route(
            "/api/v1/sessions/{provider}/{session_id}",
            get(session_detail),
        )
        .route("/api/v1/search", get(search))
        .route("/api/v1/usage", get(usage))
        .route("/api/v1/stats", get(stats))
        .route("/", get(index))
        .fallback(static_asset)
        .layer(middleware::from_fn(move |req, next| {
            guard_host(req, next, port)
        }))
}

/// Reject requests whose `Host` is not this server's loopback origin — the
/// classic DNS-rebinding guard for a loopback-bound server.
async fn guard_host(req: Request<axum::body::Body>, next: Next, port: u16) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let allowed = [
        format!("localhost:{port}"),
        format!("127.0.0.1:{port}"),
        format!("[::1]:{port}"),
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "[::1]".to_string(),
    ];
    if allowed.iter().any(|candidate| candidate == &host) {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "unrecognized Host header").into_response()
    }
}

fn with_archive<T: serde::Serialize>(
    f: impl FnOnce(&rusqlite::Connection) -> anyhow::Result<T>,
) -> Response {
    match sivtr_core::archive::open().and_then(|conn| f(&conn)) {
        Ok(value) => Json(value).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{error:#}") })),
        )
            .into_response(),
    }
}

async fn health() -> Response {
    with_archive(|conn| {
        let counts = store::provider_counts(conn)?;
        Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "providers": counts,
        }))
    })
}

async fn providers() -> Response {
    with_archive(|conn| Ok(json!(store::provider_counts(conn)?)))
}

#[derive(Deserialize)]
struct SessionsQuery {
    provider: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    100
}

const MAX_PAGE: i64 = 200;
const MAX_SEARCH: usize = 200;

fn page_bounds(limit: i64, offset: i64) -> Option<(i64, i64)> {
    if limit < 0 || offset < 0 {
        return None;
    }
    Some((limit.min(MAX_PAGE), offset))
}

async fn sessions(AxQuery(query): AxQuery<SessionsQuery>) -> Response {
    let Some((limit, offset)) = page_bounds(query.limit, query.offset) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "limit and offset must be >= 0" })),
        )
            .into_response();
    };
    with_archive(|conn| {
        Ok(json!(store::list_sessions_meta(
            conn,
            query.provider.as_deref(),
            limit,
            offset
        )?))
    })
}

async fn session_detail(AxPath((provider, session_id)): AxPath<(String, String)>) -> Response {
    let result = sivtr_core::archive::open().and_then(|conn| {
        Ok(
            match store::session_meta_by_key(&conn, &provider, &session_id)? {
                None => None,
                Some(meta) => {
                    let records =
                        store::load_records_by_key(&conn, &provider, &session_id, BlobMode::Full)?
                            .unwrap_or_default();
                    Some(json!({ "session": meta, "records": records }))
                }
            },
        )
    });
    match result {
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("no archived session `{provider}/{session_id}`") })),
        )
            .into_response(),
        Ok(Some(value)) => Json(value).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{error:#}") })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    /// Selector understood by the query layer (`all:agent`, `all:terminal`,
    /// `codex`, `all`, …). `all` runs the agent and terminal corpora and
    /// merges the hits.
    source: Option<String>,
    limit: Option<usize>,
    #[serde(default)]
    semantic: bool,
    #[serde(default)]
    hybrid: bool,
}

async fn search(AxQuery(query): AxQuery<SearchQuery>) -> Response {
    let limit = query.limit.unwrap_or(50).min(MAX_SEARCH);
    let rank = query.q.clone().unwrap_or_default();
    if query.semantic || query.hybrid {
        let Some(query_text) = query.q.clone().filter(|query| !query.trim().is_empty()) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "semantic and hybrid search require q" })),
            )
                .into_response();
        };
        let source = query.source.clone().unwrap_or_else(|| "all".to_string());
        let args = SearchArgs {
            source,
            query: Some(query_text),
            semantic: query.semantic,
            hybrid: query.hybrid,
            match_: None,
            exclude: None,
            in_field: Field::Content,
            kind: None,
            status: None,
            exit_code: None,
            min_duration: None,
            max_duration: None,
            sort: Some(Sort::Relevance),
            cwd: None,
            since: None,
            until: None,
            last: None,
            latest: Some(limit),
            limit: Some(limit),
            exclude_current: false,
            format: None,
            json: false,
            refs: false,
            save: None,
        };
        return match crate::commands::memory::semantic::run(&args) {
            Ok(set) => {
                let records = set.into_records();
                Json(json!({ "records": records })).into_response()
            }
            Err(error) => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("{error:#}") })),
            )
                .into_response(),
        };
    }
    let ranked = query.q.is_some();
    let sort = if ranked {
        Sort::Relevance
    } else {
        Sort::Newest
    };
    let make_filter = || Filter {
        rank: Some(rank.clone()),
        sort,
        limit: Some(limit),
        ..Filter::none()
    };

    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("{error:#}") })),
            )
                .into_response();
        }
    };

    let sources: Vec<String> = match query.source.as_deref().unwrap_or("all") {
        "all" => vec!["all:agent".to_string(), "all:terminal".to_string()],
        source => vec![source.to_string()],
    };

    let mut merged: Vec<sivtr_core::record::WorkRecord> = Vec::new();
    for source in sources {
        let set = match workset::query(&source, make_filter(), Some(&cwd)) {
            Ok(set) => set,
            Err(error) => {
                let message = format!("{error:#}");
                if message.starts_with(NO_RECORD_FOR_SELECTOR) {
                    continue;
                }
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": message })))
                    .into_response();
            }
        };
        merged.extend(set.into_records());
    }
    // A query is already BM25-ordered per source. Re-sorting the merged
    // list by recency would throw that ranking away; `source=all` keeps
    // each source's relevance order (agent, then terminal).
    if !ranked {
        merged.sort_by(|a, b| b.time.primary_at().cmp(&a.time.primary_at()));
    }
    merged.truncate(limit);
    Json(json!({ "records": merged })).into_response()
}

#[derive(Deserialize)]
struct UsageQuery {
    provider: Option<String>,
    session_id: Option<String>,
    since: Option<String>,
    until: Option<String>,
}

async fn usage(AxQuery(query): AxQuery<UsageQuery>) -> Response {
    let query = sivtr_core::usage::UsageQuery {
        provider: query.provider,
        session_id: query.session_id,
        since: query.since,
        until: query.until,
    };
    if let Err(error) = query.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("{error:#}") })),
        )
            .into_response();
    }
    with_archive(|conn| {
        let skipped = sivtr_core::archive::sync::ensure_fresh_with_conn(conn)?;
        let summary = sivtr_core::usage::summarize(conn, &query)?;
        Ok(sivtr_core::usage::UsageResult {
            summary,
            warnings: sivtr_core::usage::warnings_from_sync(&skipped),
        })
    })
}

async fn stats(AxQuery(query): AxQuery<UsageQuery>) -> Response {
    let query = sivtr_core::archive::stats::StatsQuery {
        provider: query.provider,
        since: query.since,
        until: query.until,
    };
    if let Err(error) = query.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("{error:#}") })),
        )
            .into_response();
    }
    with_archive(|conn| {
        let skipped = sivtr_core::archive::sync::ensure_fresh_with_conn(conn)?;
        let report = sivtr_core::archive::stats::compute(conn, &query)?;
        Ok(json!({
            "stats": report,
            "warnings": sivtr_core::usage::warnings_from_sync(&skipped),
        }))
    })
}

async fn index() -> Response {
    serve_asset("index.html")
}

async fn static_asset(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    serve_asset(path)
}

fn serve_asset(path: &str) -> Response {
    match Assets::get(path) {
        Some(asset) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let mut response = (StatusCode::OK, asset.data).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime.as_ref())
                    .unwrap_or(HeaderValue::from_static("text/plain")),
            );
            response
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    // `std::env` is process-global; serialize the env-touching tests the same
    // way sivtr-core's tests do.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn test_router() -> Router {
        router(8080)
    }

    #[test]
    fn bind_addr_brackets_bare_ipv6() {
        assert_eq!(bind_addr("::1", 8080), "[::1]:8080");
        assert_eq!(bind_addr("[::1]", 8080), "[::1]:8080");
        assert_eq!(bind_addr("127.0.0.1", 8080), "127.0.0.1:8080");
    }

    #[test]
    fn execute_rejects_non_loopback_host() {
        let args = crate::cli::WebArgs {
            port: 8080,
            host: "0.0.0.0".into(),
        };
        let error = execute(&args).expect_err("non-loopback bind must fail");
        assert!(
            error.to_string().contains("loopback"),
            "unexpected error: {error:#}"
        );
    }

    #[tokio::test]
    async fn host_guard_rejects_foreign_hosts() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .header(header::HOST, "evil.example.com")
                    .uri("/api/v1/health")
                    .body(axum::body::Body::empty())
                    .expect("build host-guard request"),
            )
            .await
            .expect("dispatch host-guard request");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    // The env var must stay set while the handler runs, and this test has no
    // concurrent sibling that could race the process-global value.
    #[allow(clippy::await_holding_lock)]
    async fn health_serves_json_on_loopback_host() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().expect("create temporary data directory");
        std::env::set_var("SIVTR_DATA_DIR", dir.path());
        let response = test_router()
            .oneshot(
                Request::builder()
                    .header(header::HOST, "localhost:8080")
                    .uri("/api/v1/health")
                    .body(axum::body::Body::empty())
                    .expect("build health request"),
            )
            .await
            .expect("dispatch health request");
        assert_eq!(response.status(), StatusCode::OK);
        std::env::remove_var("SIVTR_DATA_DIR");
    }

    #[tokio::test]
    async fn index_serves_embedded_page() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .header(header::HOST, "localhost:8080")
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .expect("build index request"),
            )
            .await
            .expect("dispatch index request");
        assert_eq!(response.status(), StatusCode::OK);
    }
}
