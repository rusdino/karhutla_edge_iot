//! Axum HTTP, SSE (`/api/stream`), and WebSocket (`/api/ws`) router
//! matching 100% of `dashboard.py` API routes + WebSocket push for Dioxus clients.

use crate::engine::KarhutlaEngine;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::Stream;
use futures_util::{SinkExt, StreamExt};
use karhutla_protocol::{
    build_mobile_dashboard_config, CmdRequest, CmdResponse, EspLogClearResponse,
    EspLogSyncResponse, SimControlRequest, SimControlResponse,
};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Debug, Deserialize)]
struct CsvQuery {
    source: Option<String>,
}

pub fn build_router(engine: Arc<KarhutlaEngine>) -> Router {
    build_router_with_web_dir(engine, None)
}

pub fn build_router_with_web_dir(engine: Arc<KarhutlaEngine>, web_dir: Option<PathBuf>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let router = Router::new()
        .route("/health", get(health))
        .route("/favicon.ico", get(favicon))
        .route("/api/state", get(get_state))
        .route("/api/sim", post(post_sim))
        .route("/api/cmd", post(post_cmd))
        .route("/api/stream", get(get_sse_stream))
        .route("/api/ws", get(get_ws))
        .route("/api/esp-log/status", get(get_esp_log_status))
        .route("/api/esp-log/sync", post(post_esp_log_sync))
        .route("/api/esp-log/download", get(get_esp_log_download))
        .route("/api/esp-log/clear", post(post_esp_log_clear))
        .route("/api/export-csv", get(get_export_csv))
        .route("/api/export-log", get(get_export_log))
        .route("/api/mobile-config", get(get_mobile_config));

    let router = if let Some(dir) = web_dir.filter(|d| d.join("index.html").is_file()) {
        let index_file = dir.join("index.html");
        router.fallback_service(ServeDir::new(dir).not_found_service(ServeFile::new(index_file)))
    } else {
        router
    };

    router.layer(cors).with_state(engine)
}

async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true, "engine": "karhutla-rust" }))
}

async fn favicon() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn get_state(State(engine): State<Arc<KarhutlaEngine>>) -> impl IntoResponse {
    Json(engine.get_state_snapshot())
}

async fn post_sim(
    State(engine): State<Arc<KarhutlaEngine>>,
    Json(req): Json<SimControlRequest>,
) -> impl IntoResponse {
    match engine.control_sim(req).await {
        Ok(sim) => Json(SimControlResponse {
            ok: true,
            reason: None,
            sim,
        }),
        Err(reason) => Json(SimControlResponse {
            ok: false,
            reason: Some(reason),
            sim: engine.get_sim_status(),
        }),
    }
}

async fn post_cmd(
    State(engine): State<Arc<KarhutlaEngine>>,
    Json(req): Json<CmdRequest>,
) -> impl IntoResponse {
    match engine.send_cmd(&req.cmd, req.target.as_deref()).await {
        Ok(paths) => Json(CmdResponse {
            ok: true,
            reason: None,
            paths: Some(paths),
        }),
        Err(reason) => Json(CmdResponse {
            ok: false,
            reason: Some(reason),
            paths: None,
        }),
    }
}

async fn get_sse_stream(
    State(engine): State<Arc<KarhutlaEngine>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = engine.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(ev) => serde_json::to_string(&ev)
                .ok()
                .map(|s| Ok(Event::default().data(s))),
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(5))
            .text("keepalive"),
    )
}

async fn get_ws(
    ws: WebSocketUpgrade,
    State(engine): State<Arc<KarhutlaEngine>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, engine))
}

async fn handle_ws(socket: WebSocket, engine: Arc<KarhutlaEngine>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = engine.subscribe();

    let send_task = tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            if let Ok(txt) = serde_json::to_string(&ev) {
                if sender.send(Message::Text(txt)).await.is_err() {
                    break;
                }
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        if matches!(msg, Message::Close(_)) {
            break;
        }
    }
    send_task.abort();
}

async fn get_esp_log_status(State(engine): State<Arc<KarhutlaEngine>>) -> impl IntoResponse {
    Json(engine.get_esp_log_status())
}

async fn post_esp_log_sync(State(engine): State<Arc<KarhutlaEngine>>) -> Response {
    match engine.sync_esp_logs().await {
        Ok((count, corrupt_skipped, crc32, csv_file)) => (
            StatusCode::OK,
            Json(EspLogSyncResponse {
                ok: true,
                count: Some(count),
                corrupt_skipped: Some(corrupt_skipped),
                crc32: Some(crc32),
                csv_file: Some(csv_file),
                error: None,
                status: engine.get_esp_log_status(),
            }),
        )
            .into_response(),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(EspLogSyncResponse {
                ok: false,
                count: None,
                corrupt_skipped: None,
                crc32: None,
                csv_file: None,
                error: Some(err),
                status: engine.get_esp_log_status(),
            }),
        )
            .into_response(),
    }
}

async fn get_esp_log_download(State(engine): State<Arc<KarhutlaEngine>>) -> Response {
    match engine.get_esp_log_download_path() {
        Ok((path, key)) => match std::fs::read(&path) {
            Ok(bytes) => {
                let fname = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "karhutla_esp32_sync.csv".to_string());
                engine.mark_esp_log_downloaded(key);
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
                        (
                            header::CONTENT_DISPOSITION,
                            format!("attachment; filename=\"{fname}\""),
                        ),
                        (header::CACHE_CONTROL, "no-store".to_string()),
                    ],
                    bytes,
                )
                    .into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": format!("gagal baca CSV: {e}") })),
            )
                .into_response(),
        },
        Err(err) => (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": err })),
        )
            .into_response(),
    }
}

async fn post_esp_log_clear(State(engine): State<Arc<KarhutlaEngine>>) -> Response {
    let (code, res) = engine.clear_esp_logs().await;
    let status_code = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    match res {
        Ok(()) => (
            status_code,
            Json(EspLogClearResponse {
                ok: true,
                error: None,
                status: engine.get_esp_log_status(),
            }),
        )
            .into_response(),
        Err(err) => (
            status_code,
            Json(EspLogClearResponse {
                ok: false,
                error: Some(err),
                status: engine.get_esp_log_status(),
            }),
        )
            .into_response(),
    }
}

async fn get_export_csv(
    State(engine): State<Arc<KarhutlaEngine>>,
    Query(q): Query<CsvQuery>,
) -> impl IntoResponse {
    let (fname, data) = engine.export_csv_bytes(q.source.as_deref().unwrap_or("auto"));
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{fname}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        data,
    )
}

async fn get_export_log(State(engine): State<Arc<KarhutlaEngine>>) -> impl IntoResponse {
    let (fname, data) = engine.export_mqtt_log_bytes();
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "text/plain; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{fname}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        data,
    )
}

async fn get_mobile_config(State(engine): State<Arc<KarhutlaEngine>>) -> impl IntoResponse {
    let cfg = build_mobile_dashboard_config(
        engine.broker_host(),
        engine.broker_port(),
        engine.lan_ip(),
    );
    let raw = serde_json::to_vec_pretty(&cfg).unwrap_or_default();
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "application/json; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"karhutla_iot_mqtt_panel.json\"".to_string(),
            ),
        ],
        raw,
    )
}
