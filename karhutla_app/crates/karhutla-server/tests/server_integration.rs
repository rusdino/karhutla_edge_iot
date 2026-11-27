use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::STANDARD as BASE64_STD;
use base64::Engine;
use http_body_util::BodyExt;
use karhutla_core::record::{crc32_ieee, BackupRecord, RECORD_SIZE};
use karhutla_core::FirmwareCore;
use karhutla_protocol::topics;
use karhutla_server::{build_router, EngineConfig, KarhutlaEngine};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

fn tmp_out(tag: &str) -> std::path::PathBuf {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("karhutla_srv_{tag}_{ns}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn test_http_endpoints_sim_cmd_and_exports() {
    let dir = tmp_out("sim");
    let eng = KarhutlaEngine::new(EngineConfig {
        out_dir: dir.clone(),
        enable_mqtt: false,
        ..Default::default()
    });
    let app = build_router(eng.clone());

    // /health & /favicon.ico (204)
    let resp = app
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let fav = app
        .clone()
        .oneshot(Request::get("/favicon.ico").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(fav.status(), StatusCode::NO_CONTENT);

    // /api/state initial idle
    let st0 = body_json(
        app.clone()
            .oneshot(Request::get("/api/state").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st0["sim"]["running"], false);
    assert_eq!(st0["sim"]["can_send_cmd"], false);

    // /api/cmd rejected while idle & offline
    let cmd_idle = body_json(
        app.clone()
            .oneshot(
                Request::post("/api/cmd")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"cmd":"AUTO","target":"auto"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(cmd_idle["ok"], false);

    // Start SITL flaming at 50x, collect samples, apply FORCE_MAX, then manual stop -> summary
    let sim_start = body_json(
        app.clone()
            .oneshot(
                Request::post("/api/sim")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"action":"start","target":"sitl","scenario":"flaming","speed":50})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(sim_start["ok"], true);
    assert_eq!(sim_start["sim"]["running"], true);

    tokio::time::sleep(Duration::from_millis(220)).await;

    let cmd_max = body_json(
        app.clone()
            .oneshot(
                Request::post("/api/cmd")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"cmd":"FORCE_MAX","target":"sitl"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(cmd_max["ok"], true);
    assert_eq!(cmd_max["paths"]["sitl"], true);
    assert_eq!(cmd_max["paths"]["esp32"], false);

    let sim_stop = body_json(
        app.clone()
            .oneshot(
                Request::post("/api/sim")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"action":"stop"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(sim_stop["ok"], true);
    assert_eq!(sim_stop["sim"]["running"], false);
    assert_eq!(sim_stop["sim"]["last_summary"]["stopped_early"], true);
    assert!(sim_stop["sim"]["last_summary"]["samples"].as_u64().unwrap() > 0);

    // CSV export & mobile JSON config
    let csv_resp = app
        .clone()
        .oneshot(
            Request::get("/api/export-csv?source=sitl")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(csv_resp.status(), StatusCode::OK);
    let csv_bytes = csv_resp.into_body().collect().await.unwrap().to_bytes();
    let csv_txt = String::from_utf8(csv_bytes.to_vec()).unwrap();
    assert!(csv_txt.starts_with("t_s,seq,source,scenario,pm_true"));
    assert!(csv_txt.lines().count() >= 2);

    let mob_cfg = body_json(
        app.clone()
            .oneshot(Request::get("/api/mobile-config").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(mob_cfg["connections"][0]["port"], 6669);

    eng.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_esp_backup_log_two_phase_sync_download_and_clear_gate() {
    let dir = tmp_out("esplog");
    let eng = KarhutlaEngine::new(EngineConfig {
        out_dir: dir.clone(),
        enable_mqtt: false,
        ..Default::default()
    });
    let app = build_router(eng.clone());

    // Prepare 2 real 64-byte BackupRecords using karhutla-core
    let mut fw = FirmwareCore::new();
    let t0 = fw.step(0.65, 1.20, 1, 1.0);
    let t1 = fw.step(0.75, 1.25, 2, 1.0);
    let r0 = BackupRecord::from_telemetry(&t0, 1_795_688_200, 1000, true, false);
    let r1 = BackupRecord::from_telemetry(&t1, 1_795_688_201, 2000, true, false);
    let mut raw_all = vec![0u8; 2 * RECORD_SIZE];
    let mut s0 = [0u8; RECORD_SIZE];
    let mut s1 = [0u8; RECORD_SIZE];
    r0.encode(&mut s0);
    r1.encode(&mut s1);
    raw_all[..RECORD_SIZE].copy_from_slice(&s0);
    raw_all[RECORD_SIZE..].copy_from_slice(&s1);
    let total_crc = crc32_ieee(&raw_all);
    let b64_chunk = BASE64_STD.encode(&raw_all);

    // Wire a deterministic mock ESP32 responder on T_LOG_REQ
    let eng_hook = Arc::clone(&eng);
    eng.set_publish_hook(Some(Arc::new(move |topic, payload| {
        if topic == topics::T_LOG_REQ {
            let e = Arc::clone(&eng_hook);
            let p = payload.to_string();
            let b64 = b64_chunk.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                if p == "META" {
                    let meta = json!({
                        "count": 2,
                        "cap": 43008,
                        "full": false,
                        "used_bytes": 128,
                        "cap_bytes": 2752512,
                        "crc32": total_crc,
                    });
                    e.ingest_mqtt(topics::T_LOG_META, &meta.to_string());
                } else if p.starts_with("GET:0:2") {
                    let chunk = json!({
                        "start": 0,
                        "count": 2,
                        "total": 2,
                        "data": b64,
                    });
                    e.ingest_mqtt(topics::T_LOG_CHUNK, &chunk.to_string());
                } else if p == format!("CLEAR:2:{total_crc}") {
                    e.ingest_mqtt(
                        topics::T_LOG_ACK,
                        &json!({"action":"CLEAR","status":"started","sectors":1}).to_string(),
                    );
                    tokio::time::sleep(Duration::from_millis(15)).await;
                    e.ingest_mqtt(
                        topics::T_LOG_ACK,
                        &json!({"action":"CLEAR","status":"done","ok":true}).to_string(),
                    );
                }
            });
        }
        Ok(())
    })));

    // 1. Offline clear must return 409
    let clr_off = app
        .clone()
        .oneshot(Request::post("/api/esp-log/clear").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(clr_off.status(), StatusCode::CONFLICT);

    // 2. Heartbeat brings ESP32 online with 2 records; clear still 409 before sync+download
    eng.ingest_mqtt(
        topics::T_HEARTBEAT,
        &json!({"mode":"RIIL","log_count":2,"log_cap":43008,"log_full":false,"rtc_ok":true,"epoch_s":1795688205})
            .to_string(),
    );
    let clr_unsynced = app
        .clone()
        .oneshot(Request::post("/api/esp-log/clear").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(clr_unsynced.status(), StatusCode::CONFLICT);

    // 3. Sync succeeds (200), still 409 before download
    let sync_resp = app
        .clone()
        .oneshot(Request::post("/api/esp-log/sync").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(sync_resp.status(), StatusCode::OK);
    let sync_json = body_json(sync_resp).await;
    assert_eq!(sync_json["ok"], true);
    assert_eq!(sync_json["count"], 2);
    assert_eq!(sync_json["status"]["synced"], true);
    assert_eq!(sync_json["status"]["downloaded"], false);

    let clr_nodl = app
        .clone()
        .oneshot(Request::post("/api/esp-log/clear").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(clr_nodl.status(), StatusCode::CONFLICT);

    // 4. Download marks downloaded=true, then Clear returns 200 and resets count to 0
    let dl_resp = app
        .clone()
        .oneshot(Request::get("/api/esp-log/download").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(dl_resp.status(), StatusCode::OK);
    let dl_bytes = dl_resp.into_body().collect().await.unwrap().to_bytes();
    let dl_txt = String::from_utf8(dl_bytes.to_vec()).unwrap();
    assert!(dl_txt.starts_with("timestamp,epoch_s,uptime_ms,seq,record_crc32"));
    assert_eq!(dl_txt.lines().count(), 3);

    let clr_ok = app
        .clone()
        .oneshot(Request::post("/api/esp-log/clear").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(clr_ok.status(), StatusCode::OK);
    let clr_json = body_json(clr_ok).await;
    assert_eq!(clr_json["ok"], true);
    assert_eq!(clr_json["status"]["log_count"], 0);

    eng.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}
