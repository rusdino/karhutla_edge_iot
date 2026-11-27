use crate::state::now_ms;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use futures_util::StreamExt;
#[cfg_attr(feature = "web", allow(unused_imports))]
use karhutla_protocol::{
    CmdPaths, CmdRequest, CmdResponse, EspLogClearResponse, EspLogStatus, EspLogSyncResponse,
    ServerEvent, SimControlRequest, SimControlResponse, StateSnapshot,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendTarget {
    Embedded,
    Remote(String),
    DirectMqtt { host: String, port: u16 },
}

impl BackendTarget {
    pub fn is_direct_mqtt(&self) -> bool {
        matches!(self, BackendTarget::DirectMqtt { .. })
    }

    pub fn mode_key(&self) -> &'static str {
        match self {
            BackendTarget::Embedded => "embedded",
            BackendTarget::Remote(_) => "remote",
            BackendTarget::DirectMqtt { .. } => "direct",
        }
    }

    pub fn from_prefs(prefs: &ConnectionPrefs) -> Self {
        match prefs.mode.trim() {
            "direct" => {
                #[cfg(feature = "web")]
                {
                    let clean = prefs.remote_url.trim().to_string();
                    Self::Remote(clean)
                }
                #[cfg(not(feature = "web"))]
                {
                    let host = if prefs.mqtt_host.trim().is_empty() {
                        "192.168.4.1".to_string()
                    } else {
                        prefs.mqtt_host.trim().to_string()
                    };
                    let port = if prefs.mqtt_port == 0 {
                        6669
                    } else {
                        prefs.mqtt_port
                    };
                    Self::DirectMqtt { host, port }
                }
            }
            "remote" => {
                let clean = prefs.remote_url.trim().to_string();
                #[cfg(feature = "desktop")]
                {
                    if clean.is_empty() {
                        Self::Embedded
                    } else {
                        Self::Remote(clean)
                    }
                }
                #[cfg(not(feature = "desktop"))]
                {
                    Self::Remote(clean)
                }
            }
            _ => {
                #[cfg(feature = "desktop")]
                {
                    Self::Embedded
                }
                #[cfg(not(feature = "desktop"))]
                {
                    Self::Remote(prefs.remote_url.trim().to_string())
                }
            }
        }
    }

    pub fn api_url(&self, path: &str) -> String {
        match self {
            BackendTarget::Embedded => format!("{}{}", embedded_local_base(), path),
            BackendTarget::Remote(base) => {
                let clean = base.trim().trim_end_matches('/');
                if clean.is_empty() {
                    path.to_string()
                } else {
                    format!("{clean}{path}")
                }
            }
            BackendTarget::DirectMqtt { host, port } => format!("mqtt://{host}:{port}{path}"),
        }
    }
}

pub fn extract_filename(header_val: Option<&str>, fallback: &str) -> String {
    if let Some(raw) = header_val {
        if let Some(idx) = raw.find("filename=") {
            let tail = raw[idx + 9..].trim().trim_matches('"').trim_matches('\'');
            if !tail.is_empty() {
                return tail.to_string();
            }
        }
    }
    fallback.to_string()
}

pub fn trigger_browser_download(filename: &str, mime: &str, bytes: &[u8]) {
    let b64 = B64.encode(bytes);
    let fname_json = serde_json::to_string(filename).unwrap_or_else(|_| "\"export.bin\"".into());
    let mime_json = serde_json::to_string(mime).unwrap_or_else(|_| "\"application/octet-stream\"".into());
    let b64_json = serde_json::to_string(&b64).unwrap_or_else(|_| "\"\"".into());
    let js = format!(
        r#"(function() {{
            const b64 = {b64_json};
            const bin = atob(b64);
            const arr = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
            const blob = new Blob([arr], {{ type: {mime_json} }});
            const url = URL.createObjectURL(blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = {fname_json};
            document.body.appendChild(a);
            a.click();
            a.remove();
            setTimeout(() => URL.revokeObjectURL(url), 1000);
        }})()"#
    );
    let _ = dioxus::document::eval(&js);

    #[cfg(feature = "desktop")]
    {
        let dir = std::path::PathBuf::from("logs").join("exports");
        if std::fs::create_dir_all(&dir).is_ok() {
            let safe_name = std::path::Path::new(filename)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("export.bin");
            let _ = std::fs::write(dir.join(safe_name), bytes);
        }
    }
}

pub fn trigger_svg_to_png_export(svg_markup: &str, filename: &str, width: u32, height: u32) {
    let svg_json = serde_json::to_string(svg_markup).unwrap_or_else(|_| "\"\"".into());
    let fname_json = serde_json::to_string(filename).unwrap_or_else(|_| "\"karhutla.png\"".into());
    let js = format!(
        r#"(function() {{
            const svgText = {svg_json};
            const fname = {fname_json};
            const blob = new Blob([svgText], {{ type: 'image/svg+xml;charset=utf-8' }});
            const url = URL.createObjectURL(blob);
            const img = new Image();
            img.onload = function() {{
                const dpr = Math.min(window.devicePixelRatio || 1, 2);
                const canvas = document.createElement('canvas');
                canvas.width = Math.round({width} * dpr);
                canvas.height = Math.round({height} * dpr);
                const ctx = canvas.getContext('2d');
                ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
                ctx.drawImage(img, 0, 0, {width}, {height});
                URL.revokeObjectURL(url);
                canvas.toBlob(function(pngBlob) {{
                    if (!pngBlob) return;
                    const pUrl = URL.createObjectURL(pngBlob);
                    const a = document.createElement('a');
                    a.href = pUrl;
                    a.download = fname;
                    document.body.appendChild(a);
                    a.click();
                    a.remove();
                    setTimeout(() => URL.revokeObjectURL(pUrl), 1000);
                }}, 'image/png');
            }};
            img.onerror = function() {{ URL.revokeObjectURL(url); }};
            img.src = url;
        }})()"#
    );
    let _ = dioxus::document::eval(&js);
}

pub fn apply_dom_theme(theme_str: Option<&str>) {
    let js = match theme_str {
        Some(t) => format!(
            "document.documentElement.lang = 'id'; document.documentElement.dataset.theme = {};",
            serde_json::to_string(t).unwrap_or_else(|_| "\"light\"".into())
        ),
        None => "document.documentElement.lang = 'id'; delete document.documentElement.dataset.theme;".to_string(),
    };
    let _ = dioxus::document::eval(&js);
}

// =========================================================================
// Platform Persistence & Preferences
// =========================================================================

const PREFS_KEY: &str = "karhutla_ui_prefs_v2";
const REMOTE_URL_KEY: &str = "karhutla_remote_url_v1";
const CONN_PREFS_KEY: &str = "karhutla_conn_prefs_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionPrefs {
    pub mode: String,
    pub remote_url: String,
    pub mqtt_host: String,
    pub mqtt_port: u16,
}

impl Default for ConnectionPrefs {
    fn default() -> Self {
        Self {
            mode: if cfg!(feature = "desktop") {
                "embedded".to_string()
            } else {
                "remote".to_string()
            },
            remote_url: String::new(),
            mqtt_host: "192.168.4.1".to_string(),
            mqtt_port: 6669,
        }
    }
}

fn load_legacy_remote_url() -> Option<String> {
    #[cfg(feature = "web")]
    {
        let win = web_sys::window()?;
        let storage = win.local_storage().ok()??;
        storage
            .get_item(REMOTE_URL_KEY)
            .ok()?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = REMOTE_URL_KEY;
        std::fs::read_to_string("logs/ui_remote_url.txt")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

pub fn load_connection_prefs() -> ConnectionPrefs {
    let mut parsed: Option<ConnectionPrefs> = None;
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window() {
            if let Ok(Some(storage)) = win.local_storage() {
                if let Ok(Some(raw)) = storage.get_item(CONN_PREFS_KEY) {
                    parsed = serde_json::from_str::<ConnectionPrefs>(&raw).ok();
                }
            }
        }
    }
    #[cfg(not(feature = "web"))]
    {
        if let Ok(raw) = std::fs::read_to_string(format!("logs/{CONN_PREFS_KEY}.json")) {
            parsed = serde_json::from_str::<ConnectionPrefs>(&raw).ok();
        }
    }

    let mut prefs = parsed.unwrap_or_else(|| {
        let mut def = ConnectionPrefs::default();
        if let Some(legacy) = load_legacy_remote_url() {
            def.mode = "remote".to_string();
            def.remote_url = legacy;
        }
        def
    });

    if prefs.mqtt_host.trim().is_empty() {
        prefs.mqtt_host = "192.168.4.1".to_string();
    }
    if prefs.mqtt_port == 0 {
        prefs.mqtt_port = 6669;
    }
    #[cfg(feature = "web")]
    if prefs.mode == "direct" || prefs.mode == "embedded" {
        prefs.mode = "remote".to_string();
    }
    prefs
}

pub fn save_connection_prefs(prefs: &ConnectionPrefs) {
    let Ok(raw) = serde_json::to_string(prefs) else {
        return;
    };
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window() {
            if let Ok(Some(storage)) = win.local_storage() {
                let _ = storage.set_item(CONN_PREFS_KEY, &raw);
            }
        }
    }
    #[cfg(not(feature = "web"))]
    {
        if std::fs::create_dir_all("logs").is_ok() {
            let _ = std::fs::write(format!("logs/{CONN_PREFS_KEY}.json"), raw);
        }
    }
}

pub fn load_prefs() -> crate::state::StoredPrefs {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window() {
            if let Ok(Some(storage)) = win.local_storage() {
                if let Ok(Some(raw)) = storage.get_item(PREFS_KEY) {
                    if let Ok(parsed) = serde_json::from_str(&raw) {
                        return parsed;
                    }
                }
            }
        }
        crate::state::StoredPrefs::default()
    }
    #[cfg(not(feature = "web"))]
    {
        if let Ok(raw) = std::fs::read_to_string(format!("logs/{PREFS_KEY}.json")) {
            if let Ok(parsed) = serde_json::from_str(&raw) {
                return parsed;
            }
        }
        crate::state::StoredPrefs::default()
    }
}

pub fn save_prefs(prefs: &crate::state::StoredPrefs) {
    let Ok(raw) = serde_json::to_string(prefs) else {
        return;
    };
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window() {
            if let Ok(Some(storage)) = win.local_storage() {
                let _ = storage.set_item(PREFS_KEY, &raw);
            }
        }
    }
    #[cfg(not(feature = "web"))]
    {
        if std::fs::create_dir_all("logs").is_ok() {
            let _ = std::fs::write(format!("logs/{PREFS_KEY}.json"), raw);
        }
    }
}

// =========================================================================
// Desktop Embedded Host (`KarhutlaEngine` + Axum LAN Server)
// =========================================================================

#[cfg(feature = "desktop")]
mod desktop_host {
    use karhutla_server::{build_router_with_web_dir, EngineConfig, KarhutlaEngine};
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::sync::{Arc, OnceLock};

    pub struct EmbeddedHostHandle {
        pub engine: Arc<KarhutlaEngine>,
        pub http_port: u16,
        pub lan_url: String,
        pub rt_handle: tokio::runtime::Handle,
    }

    static EMBEDDED_HOST: OnceLock<EmbeddedHostHandle> = OnceLock::new();

    fn detect_web_dir() -> Option<PathBuf> {
        let candidates = [
            PathBuf::from("web"),
            PathBuf::from("public"),
            PathBuf::from("../web/public"),
            PathBuf::from("../../target/dx/karhutla-ui/release/web/public"),
            PathBuf::from("../../target/dx/karhutla-ui/debug/web/public"),
            PathBuf::from("karhutla_app/target/dx/karhutla-ui/release/web/public"),
            PathBuf::from("karhutla_app/target/dx/karhutla-ui/debug/web/public"),
        ];
        candidates
            .into_iter()
            .find(|p| p.join("index.html").is_file())
    }

    pub fn init_embedded_host() -> &'static EmbeddedHostHandle {
        EMBEDDED_HOST.get_or_init(|| {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime for embedded host");
            let (tx, rx) = std::sync::mpsc::channel::<EmbeddedHostHandle>();

            std::thread::Builder::new()
                .name("karhutla-embedded-host".to_string())
                .spawn(move || {
                    rt.block_on(async move {
                        let out_dir = PathBuf::from("logs");
                        let engine = KarhutlaEngine::new(EngineConfig {
                            out_dir,
                            ..Default::default()
                        });

                        let listener = match tokio::net::TcpListener::bind("0.0.0.0:8080").await {
                            Ok(l) => l,
                            Err(_) => tokio::net::TcpListener::bind("0.0.0.0:0")
                                .await
                                .expect("failed to bind embedded HTTP listener"),
                        };
                        let addr: SocketAddr = listener.local_addr().unwrap();
                        let http_port = addr.port();
                        let lan_url = format!("http://{}:{http_port}", engine.lan_ip());
                        let web_dir = detect_web_dir();
                        let router = build_router_with_web_dir(engine.clone(), web_dir);

                        let _ = tx.send(EmbeddedHostHandle {
                            engine: engine.clone(),
                            http_port,
                            lan_url,
                            rt_handle: tokio::runtime::Handle::current(),
                        });

                        let _ = axum::serve(listener, router).await;
                    });
                })
                .expect("failed to spawn embedded host thread");

            rx.recv().expect("embedded host failed to initialize")
        })
    }

    pub fn embedded_local_base() -> String {
        let host = init_embedded_host();
        format!("http://127.0.0.1:{}", host.http_port)
    }
}

#[cfg(feature = "desktop")]
pub use desktop_host::{embedded_local_base, init_embedded_host};

#[cfg(not(feature = "desktop"))]
pub fn embedded_local_base() -> String {
    String::new()
}

// =========================================================================
// Unified Async Transport Client (Web WASM + Desktop + Mobile)
// =========================================================================

pub async fn fetch_state(target: &BackendTarget) -> Result<StateSnapshot, String> {
    if let BackendTarget::DirectMqtt { host, port } = target {
        return Ok(crate::direct_mqtt::default_direct_snapshot(host, *port));
    }

    #[cfg(feature = "desktop")]
    if matches!(target, BackendTarget::Embedded) {
        let host = init_embedded_host();
        return Ok(host.engine.get_state_snapshot());
    }

    #[cfg(feature = "web")]
    {
        let url = target.api_url("/api/state");
        let resp = gloo_net::http::Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Gagal menghubungi server: {e}"))?;
        if !resp.ok() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.json::<StateSnapshot>()
            .await
            .map_err(|e| format!("JSON tidak valid: {e}"))
    }

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    {
        let url = target.api_url("/api/state");
        let client = reqwest::Client::new();
        let fut = async move {
            let resp = client
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("Gagal menghubungi server: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status().as_u16()));
            }
            resp.json::<StateSnapshot>()
                .await
                .map_err(|e| format!("JSON tidak valid: {e}"))
        };
        #[cfg(feature = "desktop")]
        {
            run_on_host_rt(fut).await
        }
        #[cfg(feature = "mobile")]
        {
            fut.await
        }
    }
}

pub async fn control_sim(
    target: &BackendTarget,
    req: SimControlRequest,
) -> Result<SimControlResponse, String> {
    if target.is_direct_mqtt() {
        let _ = req;
        return Err("Simulasi SITL/HILS dikendalikan melalui Host Lokal atau Server LAN (:8080). Mode Direct MQTT (:6669) terhubung langsung ke broker lapangan.".to_string());
    }

    #[cfg(feature = "desktop")]
    if matches!(target, BackendTarget::Embedded) {
        let host = init_embedded_host();
        let engine = host.engine.clone();
        return run_on_host_rt(async move {
            match engine.control_sim(req).await {
                Ok(sim) => Ok(SimControlResponse {
                    ok: true,
                    reason: None,
                    sim,
                }),
                Err(reason) => Ok(SimControlResponse {
                    ok: false,
                    reason: Some(reason),
                    sim: engine.get_sim_status(),
                }),
            }
        })
        .await;
    }

    #[cfg(feature = "web")]
    {
        let url = target.api_url("/api/sim");
        let resp = gloo_net::http::Request::post(&url)
            .json(&req)
            .map_err(|e| format!("Gagal serialize permintaan: {e}"))?
            .send()
            .await
            .map_err(|e| format!("Koneksi terputus: {e}"))?;
        if !resp.ok() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.json::<SimControlResponse>()
            .await
            .map_err(|e| format!("Respon tidak valid: {e}"))
    }

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    {
        let url = target.api_url("/api/sim");
        let client = reqwest::Client::new();
        let fut = async move {
            let resp = client
                .post(&url)
                .json(&req)
                .send()
                .await
                .map_err(|e| format!("Koneksi terputus: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status().as_u16()));
            }
            resp.json::<SimControlResponse>()
                .await
                .map_err(|e| format!("Respon tidak valid: {e}"))
        };
        #[cfg(feature = "desktop")]
        {
            run_on_host_rt(fut).await
        }
        #[cfg(feature = "mobile")]
        {
            fut.await
        }
    }
}

pub async fn send_cmd(
    target: &BackendTarget,
    cmd: String,
    cmd_target: Option<String>,
) -> Result<CmdResponse, String> {
    let req = CmdRequest {
        cmd,
        target: cmd_target,
    };

    if let BackendTarget::DirectMqtt { host, port } = target {
        if req.target.as_deref() == Some("sitl") {
            return Ok(CmdResponse {
                ok: false,
                reason: Some("Sasaran SITL tidak tersedia pada mode Direct MQTT (:6669).".to_string()),
                paths: None,
            });
        }
        #[cfg(feature = "web")]
        {
            let _ = (host, port);
            return Err("Mode Direct MQTT (:6669) tidak didukung di browser WebAssembly.".to_string());
        }
        #[cfg(any(feature = "desktop", feature = "mobile"))]
        {
            let h = host.clone();
            let p = *port;
            let c = req.cmd.clone();
            let fut = async move { crate::direct_mqtt::publish_direct_cmd(&h, p, &c).await };
            #[cfg(feature = "desktop")]
            let res = run_on_host_rt(fut).await;
            #[cfg(feature = "mobile")]
            let res = fut.await;
            return match res {
                Ok(()) => Ok(CmdResponse {
                    ok: true,
                    reason: None,
                    paths: Some(CmdPaths {
                        sitl: false,
                        esp32: true,
                    }),
                }),
                Err(reason) => Ok(CmdResponse {
                    ok: false,
                    reason: Some(reason),
                    paths: None,
                }),
            };
        }
    }

    #[cfg(feature = "desktop")]
    if matches!(target, BackendTarget::Embedded) {
        let host = init_embedded_host();
        let engine = host.engine.clone();
        return run_on_host_rt(async move {
            match engine.send_cmd(&req.cmd, req.target.as_deref()).await {
                Ok(paths) => Ok(CmdResponse {
                    ok: true,
                    reason: None,
                    paths: Some(paths),
                }),
                Err(reason) => Ok(CmdResponse {
                    ok: false,
                    reason: Some(reason),
                    paths: None,
                }),
            }
        })
        .await;
    }

    #[cfg(feature = "web")]
    {
        let url = target.api_url("/api/cmd");
        let resp = gloo_net::http::Request::post(&url)
            .json(&req)
            .map_err(|e| format!("Gagal serialize perintah: {e}"))?
            .send()
            .await
            .map_err(|e| format!("Server tidak merespons: {e}"))?;
        if !resp.ok() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.json::<CmdResponse>()
            .await
            .map_err(|e| format!("Respon tidak valid: {e}"))
    }

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    {
        let url = target.api_url("/api/cmd");
        let client = reqwest::Client::new();
        let fut = async move {
            let resp = client
                .post(&url)
                .json(&req)
                .send()
                .await
                .map_err(|e| format!("Server tidak merespons: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status().as_u16()));
            }
            resp.json::<CmdResponse>()
                .await
                .map_err(|e| format!("Respon tidak valid: {e}"))
        };
        #[cfg(feature = "desktop")]
        {
            run_on_host_rt(fut).await
        }
        #[cfg(feature = "mobile")]
        {
            fut.await
        }
    }
}

pub async fn fetch_esp_log_status(target: &BackendTarget) -> Result<EspLogStatus, String> {
    if target.is_direct_mqtt() {
        return Err("Status memori flash diperbarui otomatis via heartbeat MQTT.".to_string());
    }

    #[cfg(feature = "desktop")]
    if matches!(target, BackendTarget::Embedded) {
        let host = init_embedded_host();
        return Ok(host.engine.get_esp_log_status());
    }

    #[cfg(feature = "web")]
    {
        let resp = gloo_net::http::Request::get(&target.api_url("/api/esp-log/status"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.ok() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.json::<EspLogStatus>().await.map_err(|e| e.to_string())
    }

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    {
        let url = target.api_url("/api/esp-log/status");
        let client = reqwest::Client::new();
        let fut = async move {
            let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status().as_u16()));
            }
            resp.json::<EspLogStatus>().await.map_err(|e| e.to_string())
        };
        #[cfg(feature = "desktop")]
        {
            run_on_host_rt(fut).await
        }
        #[cfg(feature = "mobile")]
        {
            fut.await
        }
    }
}

pub async fn sync_esp_logs(target: &BackendTarget) -> Result<EspLogSyncResponse, String> {
    if target.is_direct_mqtt() {
        return Err("Sinkronisasi log flash ESP32 memerlukan Host Lokal atau Server LAN (:8080).".to_string());
    }

    #[cfg(feature = "desktop")]
    if matches!(target, BackendTarget::Embedded) {
        let host = init_embedded_host();
        let engine = host.engine.clone();
        return run_on_host_rt(async move {
            match engine.sync_esp_logs().await {
                Ok((count, corrupt_skipped, crc32, csv_file)) => Ok(EspLogSyncResponse {
                    ok: true,
                    count: Some(count),
                    corrupt_skipped: Some(corrupt_skipped),
                    crc32: Some(crc32),
                    csv_file: Some(csv_file),
                    error: None,
                    status: engine.get_esp_log_status(),
                }),
                Err(error) => Ok(EspLogSyncResponse {
                    ok: false,
                    count: None,
                    corrupt_skipped: None,
                    crc32: None,
                    csv_file: None,
                    error: Some(error),
                    status: engine.get_esp_log_status(),
                }),
            }
        })
        .await;
    }

    #[cfg(feature = "web")]
    {
        let resp = gloo_net::http::Request::post(&target.api_url("/api/esp-log/sync"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        resp.json::<EspLogSyncResponse>()
            .await
            .map_err(|e| e.to_string())
    }

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    {
        let url = target.api_url("/api/esp-log/sync");
        let client = reqwest::Client::new();
        let fut = async move {
            let resp = client.post(&url).send().await.map_err(|e| e.to_string())?;
            resp.json::<EspLogSyncResponse>()
                .await
                .map_err(|e| e.to_string())
        };
        #[cfg(feature = "desktop")]
        {
            run_on_host_rt(fut).await
        }
        #[cfg(feature = "mobile")]
        {
            fut.await
        }
    }
}

pub async fn clear_esp_logs(target: &BackendTarget) -> Result<EspLogClearResponse, String> {
    if target.is_direct_mqtt() {
        return Err("Pengosongan memori flash ESP32 memerlukan Host Lokal atau Server LAN (:8080).".to_string());
    }

    #[cfg(feature = "desktop")]
    if matches!(target, BackendTarget::Embedded) {
        let host = init_embedded_host();
        let engine = host.engine.clone();
        return run_on_host_rt(async move {
            let (_code, res) = engine.clear_esp_logs().await;
            match res {
                Ok(()) => Ok(EspLogClearResponse {
                    ok: true,
                    error: None,
                    status: engine.get_esp_log_status(),
                }),
                Err(error) => Ok(EspLogClearResponse {
                    ok: false,
                    error: Some(error),
                    status: engine.get_esp_log_status(),
                }),
            }
        })
        .await;
    }

    #[cfg(feature = "web")]
    {
        let resp = gloo_net::http::Request::post(&target.api_url("/api/esp-log/clear"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        resp.json::<EspLogClearResponse>()
            .await
            .map_err(|e| e.to_string())
    }

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    {
        let url = target.api_url("/api/esp-log/clear");
        let client = reqwest::Client::new();
        let fut = async move {
            let resp = client.post(&url).send().await.map_err(|e| e.to_string())?;
            resp.json::<EspLogClearResponse>()
                .await
                .map_err(|e| e.to_string())
        };
        #[cfg(feature = "desktop")]
        {
            run_on_host_rt(fut).await
        }
        #[cfg(feature = "mobile")]
        {
            fut.await
        }
    }
}

pub async fn download_verified_file(
    target: &BackendTarget,
    path: &str,
    fallback_name: &str,
) -> Result<(String, Vec<u8>), String> {
    if target.is_direct_mqtt() {
        let _ = (path, fallback_name);
        return Err("Gunakan ekspor lokal mode Direct MQTT.".to_string());
    }

    #[cfg(feature = "desktop")]
    if matches!(target, BackendTarget::Embedded) {
        let host = init_embedded_host();
        if path.starts_with("/api/export-csv") {
            let src = path
                .split("source=")
                .nth(1)
                .unwrap_or("auto");
            return Ok(host.engine.export_csv_bytes(src));
        } else if path == "/api/export-log" {
            return Ok(host.engine.export_mqtt_log_bytes());
        } else if path == "/api/mobile-config" {
            let cfg = karhutla_protocol::build_mobile_dashboard_config(
                host.engine.broker_host(),
                host.engine.broker_port(),
                host.engine.lan_ip(),
            );
            let bytes = serde_json::to_vec_pretty(&cfg).unwrap_or_default();
            return Ok(("karhutla_iot_mqtt_panel.json".to_string(), bytes));
        } else if path == "/api/esp-log/download" {
            let (fpath, key) = host.engine.get_esp_log_download_path()?;
            let bytes = std::fs::read(&fpath).map_err(|e| format!("gagal baca CSV: {e}"))?;
            let fname = fpath
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| fallback_name.to_string());
            host.engine.mark_esp_log_downloaded(key);
            return Ok((fname, bytes));
        }
    }

    #[cfg(feature = "web")]
    {
        let url = target.api_url(path);
        let resp = gloo_net::http::Request::get(&url)
            .send()
            .await
            .map_err(|e| format!("Gagal mengunduh berkas: {e}"))?;
        if !resp.ok() {
            return Err(format!("Unduh gagal (HTTP {})", resp.status()));
        }
        let fname = extract_filename(
            resp.headers().get("content-disposition").as_deref(),
            fallback_name,
        );
        let bytes = resp
            .binary()
            .await
            .map_err(|e| format!("Gagal membaca isi berkas: {e}"))?;
        Ok((fname, bytes))
    }

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    {
        let url = target.api_url(path);
        let fallback = fallback_name.to_string();
        let client = reqwest::Client::new();
        let fut = async move {
            let resp = client
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("Gagal mengunduh berkas: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Unduh gagal (HTTP {})", resp.status().as_u16()));
            }
            let cd = resp
                .headers()
                .get(reqwest::header::CONTENT_DISPOSITION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let fname = extract_filename(cd.as_deref(), &fallback);
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| format!("Gagal membaca isi berkas: {e}"))?;
            Ok((fname, bytes.to_vec()))
        };
        #[cfg(feature = "desktop")]
        {
            run_on_host_rt(fut).await
        }
        #[cfg(feature = "mobile")]
        {
            fut.await
        }
    }
}

#[cfg(feature = "desktop")]
async fn run_on_host_rt<F, T>(fut: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let host = init_embedded_host();
    let (tx, rx) = tokio::sync::oneshot::channel();
    host.rt_handle.spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.await
        .map_err(|_| "Runtime embedded host terputus".to_string())?
}

// =========================================================================
// Live Event Subscription (Direct Broadcast on Embedded, WebSocket otherwise)
// =========================================================================

pub async fn sleep_ms(ms: u32) {
    #[cfg(feature = "web")]
    {
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }
    #[cfg(not(feature = "web"))]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        #[cfg(feature = "desktop")]
        {
            let host = init_embedded_host();
            host.rt_handle.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
                let _ = tx.send(());
            });
        }
        #[cfg(feature = "mobile")]
        {
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
                let _ = tx.send(());
            });
        }
        let _ = rx.await;
    }
}

pub enum LiveMessage {
    Connected { is_reconnect: bool },
    Disconnected,
    Event(ServerEvent),
}

pub async fn run_live_loop<F>(target: BackendTarget, mut on_msg: F)
where
    F: FnMut(LiveMessage),
{
    let mut had_disconnect = false;

    #[cfg(feature = "desktop")]
    if matches!(target, BackendTarget::Embedded) {
        let host = init_embedded_host();
        let mut rx = host.engine.subscribe();
        on_msg(LiveMessage::Connected { is_reconnect: false });
        loop {
            match rx.recv().await {
                Ok(ev) => on_msg(LiveMessage::Event(ev)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    on_msg(LiveMessage::Disconnected);
                    break;
                }
            }
        }
        return;
    }

    #[cfg(feature = "web")]
    {
        if target.is_direct_mqtt() {
            let _ = &mut on_msg;
            return;
        }
        loop {
            let ws_url = build_ws_url_web(&target);
            match gloo_net::websocket::futures::WebSocket::open(&ws_url) {
                Ok(mut ws) => {
                    on_msg(LiveMessage::Connected {
                        is_reconnect: had_disconnect,
                    });
                    while let Some(item) = ws.next().await {
                        match item {
                            Ok(gloo_net::websocket::Message::Text(txt)) => {
                                if let Ok(ev) = serde_json::from_str::<ServerEvent>(&txt) {
                                    on_msg(LiveMessage::Event(ev));
                                }
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                    had_disconnect = true;
                    on_msg(LiveMessage::Disconnected);
                }
                Err(_) => {
                    had_disconnect = true;
                    on_msg(LiveMessage::Disconnected);
                }
            }
            sleep_ms(1500).await;
        }
    }

    #[cfg(any(feature = "desktop", feature = "mobile"))]
    {
        if let BackendTarget::DirectMqtt { host, port } = target {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<LiveMessage>(64);
            let mqtt_fut = crate::direct_mqtt::run_direct_mqtt_client(host, port, tx);
            #[cfg(feature = "desktop")]
            init_embedded_host().rt_handle.spawn(mqtt_fut);
            #[cfg(feature = "mobile")]
            tokio::spawn(mqtt_fut);
            while let Some(msg) = rx.recv().await {
                on_msg(msg);
            }
            return;
        }

        loop {
            let ws_url = build_ws_url_native(&target);
            let (tx, mut rx) = tokio::sync::mpsc::channel::<LiveMessage>(64);
            let spawn_connect = async move {
                match tokio_tungstenite::connect_async(&ws_url).await {
                    Ok((ws_stream, _)) => {
                        let _ = tx
                            .send(LiveMessage::Connected {
                                is_reconnect: had_disconnect,
                            })
                            .await;
                        let (_write, mut read) = ws_stream.split();
                        while let Some(Ok(msg)) = read.next().await {
                            if let tokio_tungstenite::tungstenite::Message::Text(txt) = msg {
                                if let Ok(ev) = serde_json::from_str::<ServerEvent>(&txt) {
                                    if tx.send(LiveMessage::Event(ev)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                        let _ = tx.send(LiveMessage::Disconnected).await;
                    }
                    Err(_) => {
                        let _ = tx.send(LiveMessage::Disconnected).await;
                    }
                }
            };
            #[cfg(feature = "desktop")]
            init_embedded_host().rt_handle.spawn(spawn_connect);
            #[cfg(feature = "mobile")]
            tokio::spawn(spawn_connect);

            while let Some(msg) = rx.recv().await {
                if matches!(msg, LiveMessage::Disconnected) {
                    had_disconnect = true;
                }
                on_msg(msg);
            }
            sleep_ms(1500).await;
        }
    }
}

#[cfg(feature = "web")]
fn build_ws_url_web(target: &BackendTarget) -> String {
    match target {
        BackendTarget::Embedded | BackendTarget::DirectMqtt { .. } => "/api/ws".to_string(),
        BackendTarget::Remote(base) => {
            let trimmed = base.trim().trim_end_matches('/');
            if !trimmed.is_empty() {
                if let Some(rest) = trimmed.strip_prefix("https://") {
                    return format!("wss://{rest}/api/ws");
                }
                if let Some(rest) = trimmed.strip_prefix("http://") {
                    return format!("ws://{rest}/api/ws");
                }
                return format!("ws://{trimmed}/api/ws");
            }
            if let Some(win) = web_sys::window() {
                let loc = win.location();
                let proto = loc.protocol().unwrap_or_else(|_| "http:".to_string());
                let host = loc.host().unwrap_or_else(|_| "127.0.0.1:8080".to_string());
                let ws_proto = if proto == "https:" { "wss:" } else { "ws:" };
                return format!("{ws_proto}//{host}/api/ws");
            }
            "ws://127.0.0.1:8080/api/ws".to_string()
        }
    }
}

#[cfg(any(feature = "desktop", feature = "mobile"))]
fn build_ws_url_native(target: &BackendTarget) -> String {
    let http = target.api_url("/api/ws");
    if let Some(rest) = http.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = http.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("ws://{http}")
    }
}

pub fn timestamp_suffix() -> u64 {
    (now_ms() / 1000.0).max(0.0) as u64
}
