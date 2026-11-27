//! Shared wire-protocol types, MQTT topic constants, and mobile panel config
//! builder for Karhutla Edge-IoT (WASM-safe, zero OS/runtime dependencies).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub mod topics {
    pub const BROKER_HOST_DEFAULT: &str = "localhost";
    pub const BROKER_PORT_DEFAULT: u16 = 6669;

    pub const T_TELEM: &str = "karhutla/telemetry";
    pub const T_TELEM_SCALAR: &str = "karhutla/t";
    pub const T_CMD: &str = "karhutla/cmd";
    pub const T_HILS_IN: &str = "karhutla/hils/sensor";
    pub const T_FORECAST: &str = "karhutla/forecast";
    pub const T_FORECAST_SCALAR: &str = "karhutla/f";
    pub const T_STATUS: &str = "karhutla/status";
    pub const T_HEARTBEAT: &str = "karhutla/hb";
    pub const T_TIME_SYNC: &str = "karhutla/time/sync";
    pub const T_LOG_REQ: &str = "karhutla/log/req";
    pub const T_LOG_META: &str = "karhutla/log/meta";
    pub const T_LOG_CHUNK: &str = "karhutla/log/chunk";
    pub const T_LOG_ACK: &str = "karhutla/log/ack";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scenario {
    Clean,
    Smolder,
    Flaming,
    Fault,
}

impl Scenario {
    pub const ALL: [Scenario; 4] = [
        Scenario::Clean,
        Scenario::Smolder,
        Scenario::Flaming,
        Scenario::Fault,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Scenario::Clean => "clean",
            Scenario::Smolder => "smolder",
            Scenario::Flaming => "flaming",
            Scenario::Fault => "fault",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "clean" => Some(Scenario::Clean),
            "smolder" => Some(Scenario::Smolder),
            "flaming" => Some(Scenario::Flaming),
            "fault" => Some(Scenario::Fault),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SimTarget {
    Sitl,
    Hils,
}

impl SimTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            SimTarget::Sitl => "sitl",
            SimTarget::Hils => "hils",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sitl" => Some(SimTarget::Sitl),
            "hils" => Some(SimTarget::Hils),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataSource {
    Sitl,
    Esp32,
}

impl DataSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataSource::Sitl => "sitl",
            DataSource::Esp32 => "esp32",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CmdAction {
    Auto,
    ForceMax,
    ForceOff,
    Reset,
}

impl CmdAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            CmdAction::Auto => "AUTO",
            CmdAction::ForceMax => "FORCE_MAX",
            CmdAction::ForceOff => "FORCE_OFF",
            CmdAction::Reset => "RESET",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "AUTO" => Some(CmdAction::Auto),
            "FORCE_MAX" => Some(CmdAction::ForceMax),
            "FORCE_OFF" => Some(CmdAction::ForceOff),
            "RESET" => Some(CmdAction::Reset),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CmdTarget {
    #[default]
    Auto,
    Sitl,
    Esp32,
}

impl CmdTarget {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "sitl" => CmdTarget::Sitl,
            "esp32" => CmdTarget::Esp32,
            _ => CmdTarget::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryRow {
    pub t: f64,
    pub t_s: f64,
    pub seq: u32,
    pub scenario: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub pm_true: Option<f64>,
    #[serde(default)]
    pub gas_true: Option<f64>,
    pub v_dust: f64,
    pub v_mq: f64,
    pub pm_raw: f64,
    pub pm: f64,
    pub dcdt: f64,
    pub gas_ppm: f64,
    pub g: f64,
    pub fi: f64,
    pub v0: f64,
    pub state: String,
    pub mode: String,
    pub duty: f64,
    #[serde(default)]
    pub forecast: Option<f64>,
    #[serde(default)]
    pub slope: Option<f64>,
    #[serde(default)]
    pub eta_bahaya_s: Option<f64>,
    #[serde(default)]
    pub mae: Option<f64>,
    #[serde(default)]
    pub sensor_fault: bool,
    #[serde(default)]
    pub ref_fi: Option<f64>,
    #[serde(default)]
    pub ref_pm: Option<f64>,
    #[serde(default)]
    pub ref_state: Option<String>,
    #[serde(default)]
    pub parity_delta_fi: Option<f64>,
}

impl TelemetryRow {
    pub fn idle() -> Self {
        Self {
            t: 0.0,
            t_s: 0.0,
            seq: 0,
            scenario: "idle".to_string(),
            source: Some("sitl".to_string()),
            pm_true: None,
            gas_true: None,
            v_dust: 0.60,
            v_mq: 1.0,
            pm_raw: 0.0,
            pm: 0.0,
            dcdt: 0.0,
            gas_ppm: 400.0,
            g: 0.0,
            fi: 0.0,
            v0: 0.60,
            state: "NORMAL".to_string(),
            mode: "AUTO".to_string(),
            duty: 0.0,
            forecast: Some(0.0),
            slope: Some(0.0),
            eta_bahaya_s: None,
            mae: None,
            sensor_fault: false,
            ref_fi: None,
            ref_pm: None,
            ref_state: None,
            parity_delta_fi: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HilsParity {
    pub samples: u32,
    pub state_matches: u32,
    pub max_delta_fi: f64,
    pub last_delta_fi: Option<f64>,
    pub last_delta_pm: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub target: String,
    pub source: String,
    pub scenario: String,
    pub samples: usize,
    pub peak_fi: f64,
    pub hils_parity: Option<HilsParity>,
    pub t_bahaya_s: Option<f64>,
    pub ai_preempt_used: bool,
    pub duration_s: f64,
    #[serde(default)]
    pub stopped_early: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimStatus {
    pub target: String,
    pub running: bool,
    pub paused: bool,
    pub scenario: String,
    pub speed: f64,
    pub duration: f64,
    pub step: u32,
    pub ai_preempt: bool,
    pub ai_preempt_active: bool,
    pub fault_inject: bool,
    pub enable_mqtt: bool,
    pub broker_host: String,
    pub broker_port: u16,
    pub broker_connected: bool,
    pub node_status: String,
    pub esp_online: bool,
    pub esp_hw_mode: Option<String>,
    pub hils_timeouts: u32,
    pub hils_consec_timeouts: u32,
    pub hils_parity: HilsParity,
    pub has_sitl: bool,
    pub has_hils_ref: bool,
    pub has_broker: bool,
    pub can_send_cmd: bool,
    pub log_file: Option<String>,
    pub last_summary: Option<RunSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EspStatus {
    pub online: bool,
    pub hw_mode: Option<String>,
    pub age_s: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EspLogStatus {
    pub online: bool,
    pub log_count: Option<u32>,
    pub log_cap: Option<u32>,
    pub log_full: Option<bool>,
    pub log_bytes: Option<u64>,
    pub log_cap_bytes: Option<u64>,
    pub crc32: Option<u32>,
    pub synced: bool,
    pub downloaded: bool,
    pub can_clear: bool,
    pub busy: Option<String>,
    pub progress: Option<f64>,
    pub last_error: Option<String>,
    pub last_sync: Option<String>,
    pub csv_file: Option<String>,
    pub rtc_ok: Option<bool>,
    pub epoch_s: Option<u64>,
    pub corrupt_skipped: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobileInfo {
    pub lan_ip: String,
    pub ip_is_fallback: bool,
    pub broker_port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    pub waspada: f64,
    pub bahaya: f64,
    pub horizon_s: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SourceBucket {
    pub history: Vec<TelemetryRow>,
    pub latest: Option<TelemetryRow>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SourcesMap {
    pub sitl: SourceBucket,
    pub esp32: SourceBucket,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub latest: TelemetryRow,
    pub history: Vec<TelemetryRow>,
    pub sources: SourcesMap,
    pub esp: EspStatus,
    pub esp_log: EspLogStatus,
    pub mobile: MobileInfo,
    pub mqtt_logs: Vec<String>,
    pub sim: SimStatus,
    pub thresholds: Thresholds,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SimControlRequest {
    pub action: Option<String>,
    pub target: Option<String>,
    pub scenario: Option<String>,
    pub speed: Option<f64>,
    pub duration: Option<f64>,
    pub ai_preempt: Option<bool>,
    pub fault_inject: Option<bool>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimControlResponse {
    pub ok: bool,
    pub reason: Option<String>,
    pub sim: SimStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CmdRequest {
    pub cmd: String,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CmdPaths {
    pub sitl: bool,
    pub esp32: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CmdResponse {
    pub ok: bool,
    pub reason: Option<String>,
    pub paths: Option<CmdPaths>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EspLogSyncResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corrupt_skipped: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crc32: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csv_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub status: EspLogStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EspLogClearResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub status: EspLogStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Clear {
        source: String,
    },
    Telemetry {
        source: String,
        row: TelemetryRow,
        sim: SimStatus,
    },
    EspStatus {
        online: bool,
        hw_mode: Option<String>,
        age_s: Option<f64>,
    },
    EspLogStatus {
        #[serde(flatten)]
        status: EspLogStatus,
    },
    HilsTimeout {
        seq: u32,
        timeouts: u32,
        consec: u32,
        auto_paused: bool,
        sim: SimStatus,
    },
    SimComplete {
        summary: RunSummary,
        sim: SimStatus,
    },
    MqttLog {
        line: String,
    },
    Sim {
        sim: SimStatus,
    },
    Status {
        broker_connected: bool,
        node_status: String,
    },
}

/// Generates an importable `IoT MQTT Panel` JSON configuration matching `dashboard.py`.
pub fn build_mobile_dashboard_config(broker_host: &str, broker_port: u16, lan_ip: &str) -> Value {
    let host = if broker_host.is_empty()
        || broker_host == "localhost"
        || broker_host == "127.0.0.1"
        || broker_host == "0.0.0.0"
    {
        lan_ip
    } else {
        broker_host
    };
    json!({
        "app": "IoT MQTT Panel",
        "version": 1,
        "connections": [
            {
                "clientId": "karhutla-mobile-panel",
                "name": "Karhutla Edge-IoT (LAN)",
                "host": host,
                "port": broker_port,
                "protocol": "tcp",
                "keepAlive": 30,
                "cleanSession": true,
                "dashboards": [
                    {
                        "name": "Monitor & Kendali Karhutla",
                        "panels": [
                            {"type": "text", "name": "Status Node", "topic": topics::T_STATUS},
                            {"type": "text", "name": "State FSM", "topic": format!("{}/state", topics::T_TELEM_SCALAR)},
                            {"type": "gauge", "name": "Fusion Index (FI)", "topic": format!("{}/fi", topics::T_TELEM_SCALAR), "min": 0, "max": 250, "unit": "ug/m3"},
                            {"type": "gauge", "name": "PM-proxy Kalman", "topic": format!("{}/pm", topics::T_TELEM_SCALAR), "min": 0, "max": 250, "unit": "ug/m3"},
                            {"type": "gauge", "name": "Gas MQ-135 (ppm)", "topic": format!("{}/gas_ppm", topics::T_TELEM_SCALAR), "min": 400, "max": 1200, "unit": "ppm"},
                            {"type": "gauge", "name": "Duty Kipas PWM", "topic": format!("{}/duty", topics::T_TELEM_SCALAR), "min": 0, "max": 100, "unit": "%"},
                            {"type": "text", "name": "Prakiraan PM AI (+60s)", "topic": format!("{}/pm_forecast", topics::T_FORECAST_SCALAR)},
                            {"type": "text", "name": "ETA Bahaya (s)", "topic": format!("{}/eta_bahaya_s", topics::T_FORECAST_SCALAR)},
                            {"type": "button", "name": "Mode AUTO", "topic": topics::T_CMD, "payload": "AUTO"},
                            {"type": "button", "name": "Paksa MAX 100%", "topic": topics::T_CMD, "payload": "FORCE_MAX"},
                            {"type": "button", "name": "Paksa OFF 0%", "topic": topics::T_CMD, "payload": "FORCE_OFF"},
                            {"type": "button", "name": "Reset Sistem", "topic": topics::T_CMD, "payload": "RESET"}
                        ]
                    }
                ]
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_event_json_tag_and_mobile_config() {
        let ev = ServerEvent::Clear {
            source: "sitl".to_string(),
        };
        let v: Value = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "clear");
        assert_eq!(v["source"], "sitl");

        let cfg = build_mobile_dashboard_config("localhost", 6669, "192.168.1.55");
        assert_eq!(cfg["connections"][0]["host"], "192.168.1.55");
        assert_eq!(cfg["connections"][0]["port"], 6669);
    }
}
