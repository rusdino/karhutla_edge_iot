use karhutla_protocol::{
    EspLogStatus, EspStatus, HilsParity, MobileInfo, RunSummary, ServerEvent, SimStatus,
    StateSnapshot, TelemetryRow, Thresholds,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_HISTORY: usize = 1800;
pub const MAX_LOG_DISPLAY: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    Light,
    Dark,
    System,
}

impl ThemePref {
    pub fn next_cycle(self) -> Self {
        match self {
            ThemePref::Light => ThemePref::Dark,
            ThemePref::Dark => ThemePref::System,
            ThemePref::System => ThemePref::Light,
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            ThemePref::Light => "Terang",
            ThemePref::Dark => "Gelap",
            ThemePref::System => "Sistem",
        }
    }

    pub fn full_label(&self) -> &'static str {
        match self {
            ThemePref::Light => "Proyektor",
            ThemePref::Dark => "Gelap",
            ThemePref::System => "Sistem",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    Auto,
    Sitl,
    Esp32,
}

impl ViewMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ViewMode::Auto => "auto",
            ViewMode::Sitl => "sitl",
            ViewMode::Esp32 => "esp32",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "sitl" => ViewMode::Sitl,
            "esp32" => ViewMode::Esp32,
            _ => ViewMode::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetKind {
    Demo,
    Dsp,
    All,
    Custom,
}

impl PresetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PresetKind::Demo => "demo",
            PresetKind::Dsp => "dsp",
            PresetKind::All => "all",
            PresetKind::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dsp" => PresetKind::Dsp,
            "all" => PresetKind::All,
            "custom" => PresetKind::Custom,
            _ => PresetKind::Demo,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PresetKind::Demo => "Demo Sidang",
            PresetKind::Dsp => "Validasi DSP",
            PresetKind::All => "Semua Kurva",
            PresetKind::Custom => "Kustom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerVisibility {
    pub fi: bool,
    pub pm: bool,
    pub forecast: bool,
    pub thresholds: bool,
    pub pm_true: bool,
    pub pm_raw: bool,
    pub gas_ppm: bool,
    pub g_scaled: bool,
    pub voltages: bool,
}

impl LayerVisibility {
    pub fn for_preset(preset: PresetKind) -> Self {
        match preset {
            PresetKind::Demo => Self {
                fi: true,
                pm: true,
                forecast: true,
                thresholds: true,
                pm_true: false,
                pm_raw: false,
                gas_ppm: false,
                g_scaled: false,
                voltages: false,
            },
            PresetKind::Dsp => Self {
                fi: false,
                pm: true,
                forecast: false,
                thresholds: false,
                pm_true: true,
                pm_raw: true,
                gas_ppm: false,
                g_scaled: false,
                voltages: false,
            },
            PresetKind::All | PresetKind::Custom => Self {
                fi: true,
                pm: true,
                forecast: true,
                thresholds: true,
                pm_true: true,
                pm_raw: true,
                gas_ppm: true,
                g_scaled: true,
                voltages: true,
            },
        }
    }

    pub fn active_count(&self, allow_pm_true: bool) -> usize {
        let mut count = 0;
        if self.fi {
            count += 1;
        }
        if self.pm {
            count += 1;
        }
        if self.forecast {
            count += 1;
        }
        if self.thresholds {
            count += 1;
        }
        if self.pm_true && allow_pm_true {
            count += 1;
        }
        if self.pm_raw {
            count += 1;
        }
        if self.gas_ppm {
            count += 1;
        }
        if self.g_scaled {
            count += 1;
        }
        if self.voltages {
            count += 1;
        }
        count
    }

    pub fn has_right_axis(&self) -> bool {
        self.gas_ppm || self.g_scaled || self.voltages
    }

    pub fn toggle(&mut self, key: &str) {
        match key {
            "fi" => self.fi = !self.fi,
            "pm" => self.pm = !self.pm,
            "forecast" => self.forecast = !self.forecast,
            "thresholds" => self.thresholds = !self.thresholds,
            "pm_true" => self.pm_true = !self.pm_true,
            "pm_raw" => self.pm_raw = !self.pm_raw,
            "gas_ppm" => self.gas_ppm = !self.gas_ppm,
            "g_scaled" => self.g_scaled = !self.g_scaled,
            "voltages" => self.voltages = !self.voltages,
            _ => {}
        }
    }
}

impl Default for LayerVisibility {
    fn default() -> Self {
        Self::for_preset(PresetKind::Demo)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredPrefs {
    pub theme: ThemePref,
    pub preset: PresetKind,
    pub layers: LayerVisibility,
    pub view_mode: ViewMode,
    pub window_sec: u32,
    pub zoom_y: bool,
    pub single_key_shortcuts: bool,
    pub cmd_target: String,
    pub autoscroll_mqtt: bool,
}

impl Default for StoredPrefs {
    fn default() -> Self {
        Self {
            theme: ThemePref::System,
            preset: PresetKind::Demo,
            layers: LayerVisibility::for_preset(PresetKind::Demo),
            view_mode: ViewMode::Auto,
            window_sec: 300,
            zoom_y: false,
            single_key_shortcuts: true,
            cmd_target: "auto".to_string(),
            autoscroll_mqtt: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HilsTimeoutInfo {
    pub seq: u32,
    pub timeouts: u32,
    pub consec: u32,
    pub auto_paused: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToastItem {
    pub id: u64,
    pub message: String,
    pub tone: String,
    pub action_label: Option<String>,
    pub action_cmd: Option<String>,
    pub expires_at_ms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmAction {
    ForceOff,
    Reset,
    RestartSim {
        target: String,
        scenario: String,
    },
    SwitchTarget {
        target: String,
    },
    SwitchScenario {
        target: String,
        scenario: String,
    },
    StartHilsOffline {
        scenario: String,
    },
    ClearChart {
        source: String,
    },
    ClearEspLog,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmModalState {
    pub title: String,
    pub desc: String,
    pub ok_label: String,
    pub tone: String,
    pub action: ConfirmAction,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AbbrPopoverState {
    pub title: String,
    pub body: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverrideRouting {
    pub can_send: bool,
    pub hits_sitl: bool,
    pub hits_esp: bool,
    pub show_scope: bool,
    pub tone: &'static str,
    pub desktop_text: String,
    pub mobile_text: String,
    pub mismatch_fix_target: Option<&'static str>,
    pub mismatch_fix_label: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalloutState {
    pub tone: &'static str,
    pub text: String,
    pub kind: CalloutKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CalloutKind {
    BootError,
    LiveLost,
    HilsTimeout { can_resume: bool },
    Failsafe { forced_off: bool, fault_inject: bool },
    ForceOffHazard,
    Bahaya,
    AiPreempt,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub sitl_history: Vec<TelemetryRow>,
    pub sitl_latest: Option<TelemetryRow>,
    pub esp_history: Vec<TelemetryRow>,
    pub esp_latest: Option<TelemetryRow>,
    pub last_active_source: String,
    pub esp: EspStatus,
    pub esp_last_rx_ms: Option<f64>,
    pub esp_log: EspLogStatus,
    pub mobile: MobileInfo,
    pub mqtt_logs: Vec<String>,
    pub total_logs: usize,
    pub sim: SimStatus,
    pub thresholds: Thresholds,
    pub last_run_summary: Option<RunSummary>,
    pub hils_timeout_info: Option<HilsTimeoutInfo>,

    pub view_mode: ViewMode,
    pub sim_target: String,
    pub selected_scenario: String,
    pub selected_speed: f64,
    pub cmd_target: String,
    pub preset: PresetKind,
    pub layers: LayerVisibility,
    pub window_sec: u32,
    pub zoom_y: bool,
    pub theme: ThemePref,
    pub system_dark: bool,
    pub single_key_shortcuts: bool,
    pub autoscroll_mqtt: bool,

    pub layers_drawer_open: bool,
    pub sim_config_expanded: bool,
    pub mob_tray_open: bool,
    pub help_open: bool,
    pub conn_modal_open: bool,

    pub is_embedded_host: bool,
    pub embedded_lan_url: String,
    pub conn_mode: String,
    pub conn_draft_mode: String,
    pub remote_url_input: String,
    pub using_remote_url: Option<String>,
    pub mqtt_host_input: String,
    pub mqtt_port_input: String,
    pub direct_mqtt_endpoint: Option<String>,

    pub live_connected: bool,
    pub had_disconnect: bool,
    pub boot_error: bool,
    pub cmd_busy: bool,
    pub sim_busy: bool,
    pub esp_sync_busy: bool,
    pub esp_clear_busy: bool,

    pub hover_t: Option<f64>,
    pub hover_from_touch: bool,
    pub hover_from_key: bool,
    pub safety_announce: String,

    pub main_svg_size: (f64, f64),
    pub sub_svg_size: (f64, f64),

    pub toasts: Vec<ToastItem>,
    pub next_toast_id: u64,
    pub confirm_modal: Option<ConfirmModalState>,
    pub abbr_popover: Option<AbbrPopoverState>,
    pub pending_esp_cmd: Option<String>,
    pub now_tick_ms: f64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            sitl_history: Vec::new(),
            sitl_latest: None,
            esp_history: Vec::new(),
            esp_latest: None,
            last_active_source: "sitl".to_string(),
            esp: EspStatus {
                online: false,
                hw_mode: None,
                age_s: None,
            },
            esp_last_rx_ms: None,
            esp_log: EspLogStatus::default(),
            mobile: MobileInfo {
                lan_ip: "127.0.0.1".to_string(),
                ip_is_fallback: true,
                broker_port: 6669,
            },
            mqtt_logs: Vec::new(),
            total_logs: 0,
            sim: default_sim_status(),
            thresholds: Thresholds {
                waspada: 55.0,
                bahaya: 150.0,
                horizon_s: 60.0,
            },
            last_run_summary: None,
            hils_timeout_info: None,

            view_mode: ViewMode::Auto,
            sim_target: "sitl".to_string(),
            selected_scenario: "flaming".to_string(),
            selected_speed: 1.0,
            cmd_target: "auto".to_string(),
            preset: PresetKind::Demo,
            layers: LayerVisibility::for_preset(PresetKind::Demo),
            window_sec: 300,
            zoom_y: false,
            theme: ThemePref::System,
            system_dark: false,
            single_key_shortcuts: true,
            autoscroll_mqtt: true,

            layers_drawer_open: false,
            sim_config_expanded: false,
            mob_tray_open: false,
            help_open: false,
            conn_modal_open: false,

            is_embedded_host: cfg!(feature = "desktop"),
            embedded_lan_url: "http://127.0.0.1:8080".to_string(),
            conn_mode: if cfg!(feature = "desktop") {
                "embedded".to_string()
            } else {
                "remote".to_string()
            },
            conn_draft_mode: if cfg!(feature = "desktop") {
                "embedded".to_string()
            } else {
                "remote".to_string()
            },
            remote_url_input: String::new(),
            using_remote_url: None,
            mqtt_host_input: "192.168.4.1".to_string(),
            mqtt_port_input: "6669".to_string(),
            direct_mqtt_endpoint: None,

            live_connected: false,
            had_disconnect: false,
            boot_error: false,
            cmd_busy: false,
            sim_busy: false,
            esp_sync_busy: false,
            esp_clear_busy: false,

            hover_t: None,
            hover_from_touch: false,
            hover_from_key: false,
            safety_announce: String::new(),

            main_svg_size: (960.0, 310.0),
            sub_svg_size: (960.0, 148.0),

            toasts: Vec::new(),
            next_toast_id: 1,
            confirm_modal: None,
            abbr_popover: None,
            pending_esp_cmd: None,
            now_tick_ms: now_ms(),
        }
    }
}

pub fn default_sim_status() -> SimStatus {
    SimStatus {
        target: "sitl".to_string(),
        running: false,
        paused: false,
        scenario: "flaming".to_string(),
        speed: 1.0,
        duration: 420.0,
        step: 0,
        ai_preempt: false,
        ai_preempt_active: false,
        fault_inject: false,
        enable_mqtt: true,
        broker_host: "localhost".to_string(),
        broker_port: 6669,
        broker_connected: false,
        node_status: "OFFLINE".to_string(),
        esp_online: false,
        esp_hw_mode: None,
        hils_timeouts: 0,
        hils_consec_timeouts: 0,
        hils_parity: HilsParity::default(),
        has_sitl: false,
        has_hils_ref: false,
        has_broker: false,
        can_send_cmd: false,
        log_file: Some("logs/mqtt.log".to_string()),
        last_summary: None,
    }
}

pub fn now_ms() -> f64 {
    #[cfg(feature = "web")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(feature = "web"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0)
    }
}

impl AppState {
    pub fn to_prefs(&self) -> StoredPrefs {
        StoredPrefs {
            theme: self.theme,
            preset: self.preset,
            layers: self.layers.clone(),
            view_mode: self.view_mode,
            window_sec: self.window_sec,
            zoom_y: self.zoom_y,
            single_key_shortcuts: self.single_key_shortcuts,
            cmd_target: self.cmd_target.clone(),
            autoscroll_mqtt: self.autoscroll_mqtt,
        }
    }

    pub fn apply_prefs(&mut self, p: StoredPrefs) {
        self.theme = p.theme;
        self.preset = p.preset;
        self.layers = p.layers;
        self.view_mode = p.view_mode;
        self.window_sec = p.window_sec;
        self.zoom_y = p.zoom_y;
        self.single_key_shortcuts = p.single_key_shortcuts;
        self.cmd_target = if matches!(p.cmd_target.as_str(), "sitl" | "esp32") {
            p.cmd_target
        } else {
            "auto".to_string()
        };
        self.autoscroll_mqtt = p.autoscroll_mqtt;
    }

    pub fn is_direct_mqtt(&self) -> bool {
        self.conn_mode == "direct" || self.direct_mqtt_endpoint.is_some()
    }

    pub fn open_conn_modal(&mut self) {
        self.conn_draft_mode = self.conn_mode.clone();
        self.conn_modal_open = true;
    }

    pub fn is_dark(&self) -> bool {
        match self.theme {
            ThemePref::Dark => true,
            ThemePref::Light => false,
            ThemePref::System => self.system_dark,
        }
    }

    pub fn active_source(&self) -> &'static str {
        match self.view_mode {
            ViewMode::Sitl => "sitl",
            ViewMode::Esp32 => "esp32",
            ViewMode::Auto => {
                if self.is_direct_mqtt() {
                    "esp32"
                } else if self.sim.running {
                    if self.sim.target == "hils" {
                        "esp32"
                    } else {
                        "sitl"
                    }
                } else if self.sim_target == "hils" {
                    "esp32"
                } else if self.last_active_source == "esp32" && !self.esp_history.is_empty() {
                    "esp32"
                } else if self.esp.online && self.sitl_history.is_empty() {
                    "esp32"
                } else {
                    "sitl"
                }
            }
        }
    }

    pub fn active_history(&self) -> &[TelemetryRow] {
        if self.active_source() == "esp32" {
            &self.esp_history
        } else {
            &self.sitl_history
        }
    }

    pub fn active_latest(&self) -> Option<&TelemetryRow> {
        let row = if self.active_source() == "esp32" {
            self.esp_latest.as_ref()
        } else {
            self.sitl_latest.as_ref()
        };
        let has_hist = !self.active_history().is_empty();
        row.filter(|r| has_hist || r.scenario != "idle")
    }

    pub fn can_show_pm_true(&self) -> bool {
        if self.active_source() == "sitl" || self.sim.target == "hils" {
            return true;
        }
        self.esp_history.iter().any(|r| r.pm_true.is_some())
    }

    pub fn clear_hover(&mut self) {
        self.hover_t = None;
        self.hover_from_touch = false;
        self.hover_from_key = false;
    }

    pub fn push_toast(
        &mut self,
        message: impl Into<String>,
        tone: impl Into<String>,
        action_label: Option<String>,
        action_cmd: Option<String>,
    ) -> u64 {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        let msg_str: String = message.into();
        let tone_str: String = tone.into();
        if action_label.is_none() && tone_str == "ok" {
            self.toasts.retain(|t| t.action_label.is_some() || t.tone != "ok");
        }
        let ttl_ms = if action_label.is_some() { 6500.0 } else { 3600.0 };
        self.toasts.push(ToastItem {
            id,
            message: msg_str,
            tone: tone_str,
            action_label,
            action_cmd,
            expires_at_ms: now_ms() + ttl_ms,
        });
        while self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
        id
    }

    pub fn prune_expired_toasts(&mut self, now: f64) {
        self.toasts.retain(|t| t.expires_at_ms > now);
    }

    pub fn remove_toast(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    pub fn push_row(&mut self, source: &str, row: TelemetryRow) {
        let bucket = if source == "esp32" {
            self.esp_latest = Some(row.clone());
            &mut self.esp_history
        } else {
            self.sitl_latest = Some(row.clone());
            &mut self.sitl_history
        };
        if let Some(last) = bucket.last_mut() {
            if row.t < last.t {
                bucket.clear();
                bucket.push(row);
            } else if (row.t - last.t).abs() < 1e-9 {
                *last = row;
            } else {
                bucket.push(row);
            }
        } else {
            bucket.push(row);
        }
        while bucket.len() > MAX_HISTORY {
            bucket.remove(0);
        }
        self.last_active_source = if source == "esp32" {
            "esp32".to_string()
        } else {
            "sitl".to_string()
        };
    }

    pub fn append_mqtt_log(&mut self, line: String, count_as_log: bool) {
        if line.is_empty() {
            return;
        }
        self.mqtt_logs.push(line);
        while self.mqtt_logs.len() > MAX_LOG_DISPLAY {
            self.mqtt_logs.remove(0);
        }
        if count_as_log {
            self.total_logs += 1;
        }
    }

    pub fn apply_sim_status(&mut self, sim: SimStatus) {
        if !sim.target.is_empty() {
            self.sim_target = sim.target.clone();
        }
        if !sim.scenario.is_empty() {
            self.selected_scenario = sim.scenario.clone();
        }
        if sim.speed > 0.0 {
            self.selected_speed = sim.speed;
        }
        if !sim.running || sim.hils_consec_timeouts == 0 {
            self.hils_timeout_info = None;
        }
        if !sim.running {
            self.sim_config_expanded = false;
        }
        self.sim = sim;
    }

    pub fn apply_snapshot(&mut self, snap: StateSnapshot, is_reconnect: bool) {
        self.boot_error = false;
        self.sitl_history = snap.sources.sitl.history;
        self.sitl_latest = snap.sources.sitl.latest;
        self.esp_history = snap.sources.esp32.history;
        self.esp_latest = snap.sources.esp32.latest;
        self.esp = snap.esp.clone();
        self.esp_last_rx_ms = snap
            .esp
            .age_s
            .map(|age| now_ms() - age * 1000.0)
            .or_else(|| if snap.esp.online { Some(now_ms()) } else { None });
        self.esp_log = snap.esp_log;
        self.mobile = snap.mobile;
        self.thresholds = snap.thresholds;
        if !is_reconnect {
            self.mqtt_logs.clear();
            self.total_logs = 0;
            for line in snap.mqtt_logs {
                self.append_mqtt_log(line, true);
            }
        } else {
            self.append_mqtt_log(
                "--- Koneksi live ke server tersambung kembali ---".to_string(),
                false,
            );
        }
        let running = snap.sim.running;
        let last_summary = snap.sim.last_summary.clone();
        self.apply_sim_status(snap.sim);
        if running {
            self.last_run_summary = None;
        } else if let Some(summary) = last_summary {
            let has_hist = if summary.source == "esp32" {
                !self.esp_history.is_empty()
            } else {
                !self.sitl_history.is_empty()
            };
            if has_hist {
                self.last_run_summary = Some(summary);
            }
        }
    }

    pub fn apply_event(&mut self, ev: ServerEvent) {
        self.live_connected = true;
        self.had_disconnect = false;
        match ev {
            ServerEvent::Clear { source } => {
                if source == "esp32" {
                    self.esp_history.clear();
                    self.esp_latest = None;
                } else {
                    self.sitl_history.clear();
                    self.sitl_latest = None;
                }
                self.clear_hover();
                self.last_run_summary = None;
            }
            ServerEvent::Telemetry { source, row, sim } => {
                let src = if source == "esp32" { "esp32" } else { "sitl" };
                if src == "esp32" {
                    self.esp.online = true;
                    self.esp_last_rx_ms = Some(now_ms());
                    if let Some(pending) = self.pending_esp_cmd.as_deref() {
                        let matched = if pending == "RESET" {
                            row.mode == "AUTO" && row.state == "NORMAL"
                        } else {
                            row.mode == pending
                        };
                        if matched {
                            let cmd_name = pending.to_string();
                            self.pending_esp_cmd = None;
                            self.push_toast(
                                format!("✓ ESP32 menerapkan {cmd_name}."),
                                "ok",
                                None,
                                None,
                            );
                        }
                    }
                }
                self.push_row(src, row);
                self.apply_sim_status(sim);
            }
            ServerEvent::EspStatus {
                online,
                hw_mode,
                age_s,
            } => {
                self.esp.online = online;
                if let Some(m) = hw_mode {
                    self.esp.hw_mode = Some(m);
                }
                if online {
                    self.esp_last_rx_ms = Some(now_ms());
                } else if let Some(age) = age_s {
                    self.esp_last_rx_ms = Some(now_ms() - age * 1000.0);
                }
            }
            ServerEvent::EspLogStatus { status } => {
                self.esp_log = status;
            }
            ServerEvent::HilsTimeout {
                seq,
                timeouts,
                consec,
                auto_paused,
                sim,
            } => {
                self.hils_timeout_info = Some(HilsTimeoutInfo {
                    seq,
                    timeouts,
                    consec,
                    auto_paused,
                });
                self.apply_sim_status(sim);
                if auto_paused {
                    self.push_toast(
                        format!("HILS dijeda otomatis setelah {consec}x timeout ESP32."),
                        "danger",
                        None,
                        None,
                    );
                }
            }
            ServerEvent::SimComplete { summary, sim } => {
                self.apply_sim_status(sim);
                let title = if summary.stopped_early {
                    "Simulasi dihentikan."
                } else {
                    "Simulasi selesai."
                };
                self.last_run_summary = Some(summary);
                self.push_toast(title, "ok", None, None);
            }
            ServerEvent::MqttLog { line } => {
                self.append_mqtt_log(line, true);
            }
            ServerEvent::Sim { sim } => {
                let stopped = !sim.running;
                let maybe_summary = sim.last_summary.clone();
                self.apply_sim_status(sim);
                if stopped {
                    if let Some(s) = maybe_summary {
                        self.last_run_summary = Some(s);
                    }
                }
            }
            ServerEvent::Status {
                broker_connected,
                node_status,
            } => {
                self.sim.broker_connected = broker_connected;
                self.sim.has_broker = broker_connected;
                self.sim.can_send_cmd =
                    self.sim.has_sitl || self.sim.has_hils_ref || broker_connected;
                self.sim.node_status = node_status;
            }
        }
    }

    pub fn override_routing(&self) -> OverrideRouting {
        let port = self.sim.broker_port;
        let has_sitl = self.sim.has_sitl;
        let has_hils_ref = self.sim.has_hils_ref;
        let has_broker = self.sim.has_broker;
        let lock = self.cmd_target.as_str();
        let hits_sitl = has_sitl && (lock == "auto" || lock == "sitl");
        let hits_esp = (has_broker || has_hils_ref) && (lock == "auto" || lock == "esp32");
        let can_send = hits_sitl || hits_esp;
        let show_scope = (has_sitl && (has_broker || has_hils_ref)) || lock != "auto";
        let viewed = self.active_source();

        if !can_send {
            let mqtt_off = !self.sim.enable_mqtt;
            let (desktop_text, mobile_text) = if lock == "sitl" {
                (
                    "Perintah nonaktif — mulai simulasi SITL".to_string(),
                    "Cmd: SITL mati".to_string(),
                )
            } else if lock == "esp32" {
                (
                    if mqtt_off {
                        "Perintah nonaktif — broker MQTT sesi ini nonaktif".to_string()
                    } else {
                        format!("Perintah nonaktif — broker :{port} belum terhubung")
                    },
                    if mqtt_off {
                        "Cmd: MQTT mati".to_string()
                    } else {
                        "Cmd: Broker putus".to_string()
                    },
                )
            } else {
                (
                    if mqtt_off {
                        "Perintah nonaktif — mulai SITL (MQTT nonaktif)".to_string()
                    } else {
                        format!("Perintah nonaktif — mulai SITL atau sambungkan broker :{port}")
                    },
                    "Cmd: Nonaktif".to_string(),
                )
            };
            return OverrideRouting {
                can_send,
                hits_sitl,
                hits_esp,
                show_scope,
                tone: "idle",
                desktop_text,
                mobile_text,
                mismatch_fix_target: None,
                mismatch_fix_label: None,
            };
        }

        let (mut desktop_text, mut mobile_text) = if hits_sitl && hits_esp {
            (
                format!("Perintah → SITL + ESP32 :{port}"),
                "Cmd: SITL+ESP32".to_string(),
            )
        } else if hits_sitl {
            (
                "Perintah → Hanya SITL (Rust)".to_string(),
                "Cmd: Hanya SITL".to_string(),
            )
        } else if has_hils_ref {
            (
                format!("Perintah → HILS (ESP32 + Ref :{port})"),
                "Cmd: HILS".to_string(),
            )
        } else {
            (
                format!("Perintah → Hanya ESP32 (:{port})"),
                "Cmd: ESP32".to_string(),
            )
        };

        let mut mismatch = false;
        let mut fix_target = None;
        let mut fix_label = None;

        if viewed == "sitl" && !hits_sitl {
            mismatch = true;
            desktop_text.push_str(" — tidak mencakup grafik SITL");
            mobile_text.push_str(" !");
            if has_sitl {
                fix_target = Some("sitl");
                fix_label = Some("Arahkan ke SITL");
            }
        } else if viewed == "esp32" && !hits_esp {
            mismatch = true;
            desktop_text.push_str(" — tidak mencakup grafik ESP32");
            mobile_text.push_str(" !");
            if has_broker || has_hils_ref {
                fix_target = Some("esp32");
                fix_label = Some("Arahkan ke ESP32");
            }
        } else if lock == "auto" && viewed == "esp32" && hits_sitl {
            mismatch = true;
            desktop_text.push_str(" — SITL juga terpengaruh");
            mobile_text.push_str(" !");
            if has_broker || has_hils_ref {
                fix_target = Some("esp32");
                fix_label = Some("Kunci ke ESP32");
            }
        }

        OverrideRouting {
            can_send,
            hits_sitl,
            hits_esp,
            show_scope,
            tone: if mismatch { "warn" } else { "ok" },
            desktop_text,
            mobile_text,
            mismatch_fix_target: fix_target,
            mismatch_fix_label: fix_label,
        }
    }

    pub fn compute_callout(&self) -> Option<CalloutState> {
        let row = self.active_latest();
        let sim = &self.sim;
        if self.boot_error {
            return Some(CalloutState {
                tone: "danger",
                text: "Gagal memuat data awal dari server Karhutla.".to_string(),
                kind: CalloutKind::BootError,
            });
        }
        if !self.live_connected && self.had_disconnect {
            return Some(CalloutState {
                tone: "danger",
                text: "Koneksi live ke server terputus. Mencoba menyambung ulang...".to_string(),
                kind: CalloutKind::LiveLost,
            });
        }
        if sim.running && sim.target == "hils" && (sim.hils_consec_timeouts > 0 || self.hils_timeout_info.is_some()) {
            let info = self.hils_timeout_info.as_ref();
            let consec = info.map(|i| i.consec).unwrap_or(sim.hils_consec_timeouts);
            let seq = info.map(|i| i.seq).unwrap_or(sim.step);
            let timeouts = info.map(|i| i.timeouts).unwrap_or(sim.hils_timeouts).max(consec);
            let can_resume = sim.paused && sim.running;
            let text = if sim.paused {
                format!("HILS dijeda otomatis: ESP32 timeout {consec}x beruntun pada langkah #{seq}. Periksa kabel/Wi-Fi ESP32.")
            } else {
                format!("Peringatan HILS: ESP32 belum membalas langkah #{seq} (timeout {timeouts}x).")
            };
            return Some(CalloutState {
                tone: if sim.paused { "danger" } else { "warn" },
                text,
                kind: CalloutKind::HilsTimeout { can_resume },
            });
        }
        if let Some(r) = row {
            if r.state == "FAILSAFE" {
                let duty_txt = format!("{:.0}", r.duty);
                let forced_off = r.mode == "FORCE_OFF";
                let text = if forced_off {
                    format!("Mode FAILSAFE dengan override FORCE OFF: sensor debu terputus dan kipas dipaksa mati ({duty_txt}%). Kembalikan ke AUTO untuk pengamanan 70%.")
                } else {
                    format!("Mode FAILSAFE aktif: pembacaan sensor debu tidak valid atau terputus. Kipas pengaman aktif pada {duty_txt}%.")
                };
                return Some(CalloutState {
                    tone: if forced_off { "danger" } else { "fault" },
                    text,
                    kind: CalloutKind::Failsafe {
                        forced_off,
                        fault_inject: sim.fault_inject,
                    },
                });
            }
            if r.mode == "FORCE_OFF" && (r.state == "BAHAYA" || r.state == "WASPADA") {
                return Some(CalloutState {
                    tone: "warn",
                    text: format!("Perhatian: Override manual FORCE OFF (0%) menahan kipas mati saat kondisi {}.", r.state),
                    kind: CalloutKind::ForceOffHazard,
                });
            }
            if r.state == "BAHAYA" {
                let dcdt = r.dcdt;
                let trend_txt = if dcdt < -0.15 {
                    "tren asap mulai menurun"
                } else if dcdt > 0.15 {
                    "tren asap masih naik"
                } else {
                    "laju asap datar"
                };
                let fi_clause = if r.fi >= 150.0 {
                    "Fusion Index melampaui 150 µg/m³".to_string()
                } else {
                    format!(
                        "Fusion Index ({:.1} µg/m³) masih tertahan histeresis pemulihan BAHAYA",
                        r.fi
                    )
                };
                let pre_clause = if sim.ai_preempt_active {
                    " • AI Preempt (FORCE MAX) aktif."
                } else {
                    ""
                };
                return Some(CalloutState {
                    tone: "danger",
                    text: format!(
                        "Kondisi BAHAYA: {fi_clause}. Kipas {:.0}% aktif ({trend_txt}, dC/dt {dcdt:+.2}/s).{pre_clause}",
                        r.duty
                    ),
                    kind: CalloutKind::Bahaya,
                });
            }
        }
        if sim.ai_preempt_active {
            return Some(CalloutState {
                tone: "accent",
                text: "AI Preempt Aktif: Prakiraan t+60s melampaui ambang, kipas dinaikkan proaktif ke FORCE MAX (100%).".to_string(),
                kind: CalloutKind::AiPreempt,
            });
        }
        None
    }

    pub fn mob_dock_context(&self) -> (String, Option<&'static str>) {
        let sim = &self.sim;
        let tgt = if sim.running {
            sim.target.to_ascii_uppercase()
        } else {
            self.sim_target.to_ascii_uppercase()
        };
        let status_suffix = if sim.running {
            if sim.paused {
                " • Jeda"
            } else {
                " • Jalan"
            }
        } else {
            ""
        };
        if let Some(lat) = self.active_latest() {
            let st = lat.state.as_str();
            let fsm_tag = if st == "FAILSAFE" {
                format!("FAILSAFE ({:.0}%)", lat.duty)
            } else {
                format!("{st} • FI {:.0}", lat.fi)
            };
            let tone = match st {
                "BAHAYA" => "danger",
                "FAILSAFE" => "fault",
                "WASPADA" => "warn",
                _ => "ok",
            };
            (format!("{fsm_tag}{status_suffix}"), Some(tone))
        } else {
            let scen = scenario_short_label(&self.selected_scenario);
            (format!("IDLE • {tgt} • {scen}"), None)
        }
    }

    pub fn esp_stale_age_s(&self) -> Option<u64> {
        let rx = self.esp_last_rx_ms?;
        let diff_s = ((self.now_tick_ms - rx) / 1000.0).max(0.0).round() as u64;
        Some(diff_s)
    }

    pub fn multi_target_force_off_desc(&self) -> Option<String> {
        let sim = &self.sim;
        let target_lock = self.cmd_target.as_str();
        let hits_sitl = sim.has_sitl && (target_lock == "auto" || target_lock == "sitl");
        let hits_esp = (sim.has_broker || sim.has_hils_ref)
            && (target_lock == "auto" || target_lock == "esp32");
        let mut candidates: Vec<(&'static str, Option<&TelemetryRow>)> = Vec::new();
        if hits_sitl {
            candidates.push(("SITL", self.sitl_latest.as_ref()));
        }
        if hits_esp {
            candidates.push(("ESP32", self.esp_latest.as_ref()));
        }
        if candidates.is_empty() {
            let act = self.active_source();
            candidates.push((
                if act == "esp32" { "ESP32" } else { "SITL" },
                self.active_latest(),
            ));
        }
        let mut critical = Vec::new();
        for (label, row_opt) in candidates {
            if let Some(r) = row_opt {
                if matches!(r.state.as_str(), "BAHAYA" | "WASPADA" | "FAILSAFE") || r.sensor_fault {
                    if r.state == "FAILSAFE" || r.sensor_fault {
                        critical.push(format!(
                            "{label}: FAILSAFE (sensor debu putus, FI tidak valid — kipas pengaman {:.0}%)",
                            r.duty
                        ));
                    } else {
                        critical.push(format!("{label}: {} (FI {:.1} µg/m³)", r.state, r.fi));
                    }
                }
            }
        }
        if critical.is_empty() {
            None
        } else {
            Some(format!(
                "Jalur target berada pada kondisi kritis ({}). Menyetel FORCE OFF (0%) akan memutus ventilasi pengaman.",
                critical.join("; ")
            ))
        }
    }
}

pub fn scenario_label(key: &str) -> &'static str {
    match key {
        "flaming" => "Api Cepat",
        "smolder" => "Membara Lambat",
        "clean" => "Udara Bersih",
        "fault" => "Putus Sensor",
        _ => "Kustom",
    }
}

pub fn scenario_short_label(key: &str) -> &'static str {
    match key {
        "flaming" => "Api Cepat",
        "smolder" => "Membara",
        "clean" => "Bersih",
        "fault" => "Putus",
        _ => "Kustom",
    }
}

pub fn format_time_s(t: f64) -> String {
    format!("{}s", t.round() as i64)
}

pub fn format_clock(sec: f64) -> String {
    let total = sec.max(0.0).round() as u64;
    let m = total / 60;
    let s = total % 60;
    if m == 0 {
        format!("{s}s")
    } else {
        format!("{m}m{s:02}s")
    }
}

pub fn format_int_id(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + 4);
    for (idx, ch) in s.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

/// Pure-Rust UTC+7 (WIB) civil date-time formatting matching `DD/MM/YYYY HH.MM.SS WIB`.
pub fn format_wib_clock(epoch_s: Option<u64>) -> String {
    let Some(ep) = epoch_s.filter(|&e| e >= 1_700_000_000) else {
        return "--:--:-- WIB".to_string();
    };
    let wib = ep + 7 * 3600;
    let days = (wib / 86_400) as i64;
    let rem = wib % 86_400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;

    // Howard Hinnant civil-from-days algorithm
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{d:02}/{m:02}/{year:04} {hour:02}.{min:02}.{sec:02} WIB")
}

pub fn describe_fan_state(row: &TelemetryRow) -> &'static str {
    let duty = row.duty;
    if row.mode == "FORCE_OFF" && row.state == "FAILSAFE" {
        return "FORCE OFF (Putus Sensor!)";
    }
    if row.mode == "FORCE_OFF" {
        return "FORCE OFF (Dimatikan)";
    }
    if row.mode == "FORCE_MAX" {
        return "FORCE MAX (Penuh)";
    }
    if row.state == "FAILSAFE" {
        return "Pengaman 70%";
    }
    if duty >= 99.0 {
        return "Penuh (BAHAYA)";
    }
    if duty > 0.5 {
        return "Proporsional";
    }
    "Siaga (Mati)"
}

pub fn speed_is_preset(spd: f64) -> bool {
    [1.0, 5.0, 10.0, 20.0]
        .iter()
        .any(|p| (spd - p).abs() < 0.05)
}

pub fn abbr_help_map() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (
            "Mentah",
            "Mentah: PM setelah filter Median3, sebelum Kalman (µg/m³)",
        ),
        (
            "dC/dt",
            "dC/dt: laju kenaikan konsentrasi asap (µg/m³ per detik)",
        ),
        ("Vd", "Vd: tegangan keluaran sensor debu Sharp (volt)"),
        ("V0", "V0: tegangan baseline udara bersih sensor debu (volt)"),
        (
            "Skor g",
            "g: skor gas ternormalisasi 0,00–1,00 (0 = udara bersih, 1 = gas pembakaran penuh)",
        ),
        ("Vm", "Vm: tegangan ADC sensor gas MQ-135 (volt)"),
        (
            "ETA",
            "ETA: perkiraan waktu tersisa sampai FI mencapai ambang BAHAYA (150)",
        ),
        (
            "MAE",
            "MAE: rata-rata galat absolut prakiraan AI terhadap PM aktual (µg/m³)",
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wib_clock_and_id_formatting() {
        assert_eq!(format_int_id(43008), "43.008");
        assert_eq!(format_int_id(950), "950");
        assert_eq!(format_wib_clock(Some(100)), "--:--:-- WIB");
        // 2026-11-27T05:17:02Z -> 2026-11-27 12.17.02 WIB (approx epoch check: known date 2024-01-01 00:00:00Z = 1704067200 -> 07.00.00 WIB)
        assert_eq!(
            format_wib_clock(Some(1_704_067_200)),
            "01/01/2024 07.00.00 WIB"
        );
    }

    #[test]
    fn test_override_routing_mismatch_offers_one_click_fix() {
        let mut st = AppState::default();
        st.sim.has_sitl = true;
        st.sim.has_broker = true;
        st.view_mode = ViewMode::Esp32;
        st.cmd_target = "sitl".to_string();
        let routing = st.override_routing();
        assert_eq!(routing.tone, "warn");
        assert_eq!(routing.mismatch_fix_target, Some("esp32"));
        assert_eq!(routing.mismatch_fix_label, Some("Arahkan ke ESP32"));
    }
}
