//! Core real-time engine: SITL/HILS runners (backed by `karhutla-core`),
//! ESP32 watchdog, RTC time sync, and two-phase ESP32 flash backup log state machine.

use crate::mqtt::{spawn_mqtt_bridge, MqttHandle};
use chrono::{Local, SecondsFormat};
use karhutla_core::cfg;
use karhutla_core::FirmwareCore;
use karhutla_protocol::{
    topics, CmdAction, CmdPaths, CmdTarget, EspLogStatus, EspStatus, HilsParity, MobileInfo,
    RunSummary, Scenario, ServerEvent, SimControlRequest, SimStatus, SimTarget, SourceBucket,
    SourcesMap, StateSnapshot, TelemetryRow, Thresholds,
};
use karhutla_sim::esp_log::{
    decode_and_verify_batch, decode_base64_chunk, write_esp_archives_and_batch,
};
use karhutla_sim::forecast::{ForecastOutput, LinearTrendForecaster, PreemptController};
use karhutla_sim::logger::{DailyCsvLogger, DailyMqttLogger};
use karhutla_sim::plant::SmokeChamber;
use karhutla_sim::{cmd_to_core, telemetry_to_row};
use rumqttc::QoS;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, Mutex as AsyncMutex, Notify};
use tokio::task::JoinHandle;

const MAX_HISTORY: usize = 1800;
const MAX_MQTT_LOGS: usize = 300;

pub type PublishHook = Arc<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub broker_host: String,
    pub broker_port: u16,
    pub out_dir: PathBuf,
    pub enable_mqtt: bool,
    pub esp_stale_s: f64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            broker_host: topics::BROKER_HOST_DEFAULT.to_string(),
            broker_port: topics::BROKER_PORT_DEFAULT,
            out_dir: PathBuf::from("logs"),
            enable_mqtt: true,
            esp_stale_s: 5.0,
        }
    }
}

#[derive(Debug, Clone)]
struct HilsMeta {
    pm_true: f64,
    gas_true: f64,
    ref_pm: f64,
    ref_fi: f64,
    ref_state: String,
    scenario: String,
}

#[derive(Debug, Clone, Default)]
struct EspLogInternal {
    count: Option<u32>,
    meta_count: Option<u32>,
    crc32: Option<u32>,
    cap: Option<u32>,
    full: Option<bool>,
    bytes: Option<u64>,
    cap_bytes: Option<u64>,
    synced_key: Option<(u32, u32)>,
    downloaded_key: Option<(u32, u32)>,
    busy: Option<String>,
    progress: Option<f64>,
    last_error: Option<String>,
    last_sync: Option<String>,
    batch_csv: Option<PathBuf>,
    rtc_ok: Option<bool>,
    epoch_s: Option<u64>,
    corrupt_skipped: u32,
}

struct InnerState {
    history: VecDeque<TelemetryRow>,
    latest: TelemetryRow,
    esp_history: VecDeque<TelemetryRow>,
    esp_latest: Option<TelemetryRow>,
    esp_last_rx: Option<Instant>,
    esp_online: bool,
    esp_hw_mode: Option<String>,
    esp_fc: LinearTrendForecaster,
    external_fc: LinearTrendForecaster,

    mqtt_logs: VecDeque<String>,
    mqtt_logger: Option<DailyMqttLogger>,
    broker_connected: bool,
    node_status: String,

    sim_target: SimTarget,
    sim_running: bool,
    sim_paused: bool,
    sim_scenario: Scenario,
    sim_speed: f64,
    sim_duration: f64,
    sim_ai_preempt: bool,
    sim_fault_inject: bool,
    sim_step_idx: u32,
    sim_fw: Option<FirmwareCore>,
    sim_ref_fw: Option<FirmwareCore>,
    sim_fc: LinearTrendForecaster,
    sim_pre: Option<PreemptController>,
    sim_generation: u64,

    hils_by_seq: HashMap<u32, Value>,
    hils_meta: HashMap<u32, HilsMeta>,
    hils_timeouts: u32,
    hils_consec_timeouts: u32,
    hils_parity: HilsParity,

    run_t_bahaya: Option<f64>,
    run_preempt_used: bool,
    last_run_summary: Option<RunSummary>,

    esp_log: EspLogInternal,
    log_meta: Option<Value>,
    log_chunks: HashMap<u32, Value>,
    log_acks: Vec<Value>,
    publish_hook: Option<PublishHook>,
}

pub struct KarhutlaEngine {
    cfg: EngineConfig,
    lan_ip: String,
    ip_is_fallback: bool,
    inner: Mutex<InnerState>,
    tx: broadcast::Sender<ServerEvent>,
    mqtt: Mutex<Option<MqttHandle>>,
    sim_task: Mutex<Option<JoinHandle<()>>>,
    hils_notify: Notify,
    log_notify: Notify,
    log_op_lock: AsyncMutex<()>,
}

pub fn detect_lan_ip_info() -> (String, bool) {
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(local) = sock.local_addr() {
                let ip = local.ip().to_string();
                if !ip.is_empty() && !ip.starts_with("127.") {
                    return (ip, false);
                }
            }
        }
    }
    ("127.0.0.1".to_string(), true)
}

fn should_log_topic(topic: &str) -> bool {
    !(topic.starts_with(&format!("{}/", topics::T_TELEM_SCALAR))
        || topic.starts_with(&format!("{}/", topics::T_FORECAST_SCALAR)))
}

impl KarhutlaEngine {
    pub fn new(cfg: EngineConfig) -> Arc<Self> {
        let (lan_ip, ip_is_fallback) = detect_lan_ip_info();
        let mqtt_logger = DailyMqttLogger::new(&cfg.out_dir, "mqtt").ok();
        let (tx, _) = broadcast::channel(512);
        let state = InnerState {
            history: VecDeque::with_capacity(MAX_HISTORY),
            latest: TelemetryRow::idle(),
            esp_history: VecDeque::with_capacity(MAX_HISTORY),
            esp_latest: None,
            esp_last_rx: None,
            esp_online: false,
            esp_hw_mode: None,
            esp_fc: LinearTrendForecaster::default(),
            external_fc: LinearTrendForecaster::default(),
            mqtt_logs: VecDeque::with_capacity(MAX_MQTT_LOGS),
            mqtt_logger,
            broker_connected: false,
            node_status: "offline".to_string(),
            sim_target: SimTarget::Sitl,
            sim_running: false,
            sim_paused: false,
            sim_scenario: Scenario::Flaming,
            sim_speed: 1.0,
            sim_duration: 900.0,
            sim_ai_preempt: false,
            sim_fault_inject: false,
            sim_step_idx: 0,
            sim_fw: None,
            sim_ref_fw: None,
            sim_fc: LinearTrendForecaster::default(),
            sim_pre: None,
            sim_generation: 0,
            hils_by_seq: HashMap::new(),
            hils_meta: HashMap::new(),
            hils_timeouts: 0,
            hils_consec_timeouts: 0,
            hils_parity: HilsParity::default(),
            run_t_bahaya: None,
            run_preempt_used: false,
            last_run_summary: None,
            esp_log: EspLogInternal::default(),
            log_meta: None,
            log_chunks: HashMap::new(),
            log_acks: Vec::new(),
            publish_hook: None,
        };
        let eng = Arc::new(Self {
            cfg,
            lan_ip,
            ip_is_fallback,
            inner: Mutex::new(state),
            tx,
            mqtt: Mutex::new(None),
            sim_task: Mutex::new(None),
            hils_notify: Notify::new(),
            log_notify: Notify::new(),
            log_op_lock: AsyncMutex::new(()),
        });

        if eng.cfg.enable_mqtt {
            eng.start_mqtt();
        }
        eng.start_watchdog();
        eng
    }

    pub fn set_publish_hook(&self, hook: Option<PublishHook>) {
        self.inner.lock().unwrap().publish_hook = hook;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.tx.subscribe()
    }

    pub fn broker_host(&self) -> &str {
        &self.cfg.broker_host
    }

    pub fn broker_port(&self) -> u16 {
        self.cfg.broker_port
    }

    pub fn lan_ip(&self) -> &str {
        &self.lan_ip
    }

    fn broadcast(&self, ev: ServerEvent) {
        let _ = self.tx.send(ev);
    }

    fn record_mqtt_log(&self, client_id: &str, event: &str, topic: &str, payload: &str) {
        if !should_log_topic(topic) {
            return;
        }
        let line = {
            let mut st = self.inner.lock().unwrap();
            let line = if let Some(logger) = st.mqtt_logger.as_mut() {
                logger
                    .log_event(client_id, event, topic, payload)
                    .unwrap_or_else(|_| format!("[{client_id}] {event} {topic} {payload}"))
            } else {
                format!("[{client_id}] {event} {topic} {payload}")
            };
            if st.mqtt_logs.len() >= MAX_MQTT_LOGS {
                st.mqtt_logs.pop_front();
            }
            st.mqtt_logs.push_back(line.clone());
            line
        };
        self.broadcast(ServerEvent::MqttLog { line });
    }

    fn start_mqtt(self: &Arc<Self>) {
        let eng_conn = Arc::clone(self);
        let eng_msg = Arc::clone(self);
        let handle = spawn_mqtt_bridge(
            self.cfg.broker_host.clone(),
            self.cfg.broker_port,
            move |ok| {
                let node_status = {
                    let mut st = eng_conn.inner.lock().unwrap();
                    st.broker_connected = ok;
                    st.node_status.clone()
                };
                eng_conn.record_mqtt_log(
                    "karhutla-rust",
                    if ok { "CONNECT" } else { "DISCONNECT" },
                    "",
                    if ok { "connected" } else { "disconnected" },
                );
                eng_conn.broadcast(ServerEvent::Status {
                    broker_connected: ok,
                    node_status,
                });
            },
            move |topic, payload| {
                eng_msg.ingest_mqtt(&topic, &payload);
            },
        );
        *self.mqtt.lock().unwrap() = Some(handle);
    }

    fn start_watchdog(self: &Arc<Self>) {
        let eng = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            loop {
                ticker.tick().await;
                let stale_evt = {
                    let mut st = eng.inner.lock().unwrap();
                    if st.esp_online {
                        if let Some(last) = st.esp_last_rx {
                            let age = last.elapsed().as_secs_f64();
                            if age > eng.cfg.esp_stale_s {
                                st.esp_online = false;
                                Some((st.esp_hw_mode.clone(), age))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some((hw_mode, age)) = stale_evt {
                    eng.broadcast(ServerEvent::EspStatus {
                        online: false,
                        hw_mode,
                        age_s: Some((age * 10.0).round() / 10.0),
                    });
                    eng.broadcast(ServerEvent::EspLogStatus {
                        status: eng.get_esp_log_status(),
                    });
                }
            }
        });
    }

    async fn publish_raw(&self, topic: &str, payload: &str, qos: QoS, retain: bool) -> Result<(), String> {
        let hook = self.inner.lock().unwrap().publish_hook.clone();
        if let Some(h) = hook {
            self.record_mqtt_log("karhutla-rust", "PUBLISH", topic, payload);
            return h(topic, payload);
        }
        let handle = self.mqtt.lock().unwrap().clone();
        if let Some(h) = handle {
            self.record_mqtt_log(h.client_id(), "PUBLISH", topic, payload);
            h.publish(topic, payload, qos, retain).await
        } else {
            Err(format!(
                "Broker MQTT :{} tidak terhubung — pastikan Mosquitto aktif.",
                self.cfg.broker_port
            ))
        }
    }

    pub fn ingest_mqtt(&self, topic: &str, payload: &str) {
        if should_log_topic(topic) {
            self.record_mqtt_log("karhutla-rust", "RECV", topic, payload);
        }
        if topic == topics::T_STATUS {
            let (bc, ns) = {
                let mut st = self.inner.lock().unwrap();
                st.node_status = if payload.is_empty() {
                    "offline".to_string()
                } else {
                    payload.to_string()
                };
                (st.broker_connected, st.node_status.clone())
            };
            self.broadcast(ServerEvent::Status {
                broker_connected: bc,
                node_status: ns,
            });
            return;
        }
        if topic == topics::T_CMD {
            if let Some(cmd) = CmdAction::parse(payload) {
                let snap = {
                    let mut st = self.inner.lock().unwrap();
                    if st.sim_running {
                        if let Some(fw) = st.sim_fw.as_mut() {
                            let tel = fw.apply_command(cmd_to_core(cmd));
                            st.latest.mode = tel.mode.as_str().to_string();
                            st.latest.duty = tel.duty as f64;
                            st.latest.state = tel.state.as_str().to_string();
                            let latest_clone = st.latest.clone();
                            if let Some(last) = st.history.back_mut() {
                                *last = latest_clone.clone();
                            }
                            Some(latest_clone)
                        } else if st.sim_target == SimTarget::Hils {
                            if let Some(ref_fw) = st.sim_ref_fw.as_mut() {
                                let _ = ref_fw.apply_command(cmd_to_core(cmd));
                            }
                            None
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(row) = snap {
                    self.broadcast(ServerEvent::Telemetry {
                        source: "sitl".to_string(),
                        row,
                        sim: self.get_sim_status(),
                    });
                }
            }
            return;
        }
        if topic == topics::T_HEARTBEAT {
            let hb: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
            let hw_mode = hb.get("mode").and_then(Value::as_str).map(str::to_string);
            self.mark_esp_alive(hw_mode);
            let need_time_sync = self.apply_esp_heartbeat(&hb);
            if need_time_sync {
                if let Ok(now_s) = SystemTime::now().duration_since(UNIX_EPOCH) {
                    let payload_s = now_s.as_secs().to_string();
                    let handle = self.mqtt.lock().unwrap().clone();
                    let hook = self.inner.lock().unwrap().publish_hook.clone();
                    if let Some(h) = hook {
                        let _ = h(topics::T_TIME_SYNC, &payload_s);
                    } else if let Some(h) = handle {
                        tokio::spawn(async move {
                            let _ = h
                                .publish(topics::T_TIME_SYNC, &payload_s, QoS::AtLeastOnce, false)
                                .await;
                        });
                    }
                }
            }
            return;
        }
        if topic == topics::T_LOG_META || topic == topics::T_LOG_CHUNK || topic == topics::T_LOG_ACK
        {
            if let Ok(msg) = serde_json::from_str::<Value>(payload) {
                if msg.is_object() {
                    let mut st = self.inner.lock().unwrap();
                    if topic == topics::T_LOG_META {
                        st.log_meta = Some(msg);
                    } else if topic == topics::T_LOG_CHUNK {
                        if let Some(start) = msg.get("start").and_then(Value::as_u64) {
                            st.log_chunks.insert(start as u32, msg);
                        }
                    } else {
                        st.log_acks.push(msg);
                    }
                    drop(st);
                    self.log_notify.notify_waiters();
                }
            }
            return;
        }
        if topic == topics::T_TELEM {
            let Ok(tel) = serde_json::from_str::<Value>(payload) else {
                return;
            };
            if !tel.is_object() {
                return;
            }
            if tel.get("src").and_then(Value::as_str) == Some("sitl") {
                let running = self.inner.lock().unwrap().sim_running;
                if !running {
                    self.ingest_external_telemetry(&tel, "sitl");
                }
            } else {
                if let Some(seq) = tel.get("seq").and_then(Value::as_u64) {
                    {
                        let mut st = self.inner.lock().unwrap();
                        st.hils_by_seq.insert(seq as u32, tel.clone());
                    }
                    self.hils_notify.notify_waiters();
                }
                self.ingest_external_telemetry(&tel, "esp32");
            }
        }
    }

    fn mark_esp_alive(&self, hw_mode: Option<String>) {
        let changed = {
            let mut st = self.inner.lock().unwrap();
            st.esp_last_rx = Some(Instant::now());
            if hw_mode.is_some() {
                st.esp_hw_mode = hw_mode;
            }
            if !st.esp_online {
                st.esp_online = true;
                Some(st.esp_hw_mode.clone())
            } else {
                None
            }
        };
        if let Some(mode) = changed {
            self.broadcast(ServerEvent::EspStatus {
                online: true,
                hw_mode: mode,
                age_s: Some(0.0),
            });
            self.broadcast(ServerEvent::EspLogStatus {
                status: self.get_esp_log_status(),
            });
        }
    }

    fn apply_esp_heartbeat(&self, hb: &Value) -> bool {
        let mut changed = false;
        {
            let mut st = self.inner.lock().unwrap();
            let lg = &mut st.esp_log;
            if let Some(cnt) = hb.get("log_count").and_then(Value::as_u64).map(|v| v as u32) {
                if lg.count != Some(cnt) || lg.meta_count != Some(cnt) {
                    if lg.meta_count != Some(cnt) {
                        lg.crc32 = None;
                    }
                    changed = true;
                }
                lg.count = Some(cnt);
            }
            if let Some(cap) = hb.get("log_cap").and_then(Value::as_u64).map(|v| v as u32) {
                if lg.cap != Some(cap) {
                    lg.cap = Some(cap);
                    changed = true;
                }
            }
            if let Some(full) = hb.get("log_full").and_then(Value::as_bool) {
                if lg.full != Some(full) {
                    lg.full = Some(full);
                    changed = true;
                }
            }
            if let Some(b) = hb.get("log_bytes").and_then(Value::as_u64) {
                if lg.bytes != Some(b) {
                    lg.bytes = Some(b);
                    changed = true;
                }
            }
            if let Some(cb) = hb.get("log_cap_bytes").and_then(Value::as_u64) {
                if lg.cap_bytes != Some(cb) {
                    lg.cap_bytes = Some(cb);
                    changed = true;
                }
            }
            if let Some(r_ok) = hb.get("rtc_ok").and_then(Value::as_bool) {
                if lg.rtc_ok != Some(r_ok) {
                    lg.rtc_ok = Some(r_ok);
                    changed = true;
                }
            }
            if let Some(ep) = hb.get("epoch_s").and_then(Value::as_u64) {
                if lg.epoch_s != Some(ep) {
                    lg.epoch_s = Some(ep);
                    changed = true;
                }
            }
        }
        if changed {
            self.broadcast(ServerEvent::EspLogStatus {
                status: self.get_esp_log_status(),
            });
        }
        if let Some(rtc_ok) = hb.get("rtc_ok").and_then(Value::as_bool) {
            let now_s = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let ep = hb.get("epoch_s").and_then(Value::as_f64).unwrap_or(0.0);
            if !rtc_ok || (ep - now_s).abs() > 10.0 {
                return true;
            }
        }
        false
    }

    pub fn ingest_external_telemetry(&self, tel: &Value, source: &str) {
        if source == "esp32" {
            self.mark_esp_alive(None);
        }
        let seq = tel.get("seq").and_then(Value::as_u64).unwrap_or(0) as u32;
        let t = tel
            .get("t")
            .and_then(Value::as_f64)
            .unwrap_or(seq as f64 * cfg::TS_S as f64);
        let state_str = tel
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("NORMAL")
            .to_string();
        let pm = tel.get("pm").and_then(Value::as_f64).unwrap_or(0.0);
        let fi = tel.get("fi").and_then(Value::as_f64).unwrap_or(0.0);

        let row = {
            let mut st = self.inner.lock().unwrap();
            let prev_fc = if source == "esp32" {
                st.esp_latest.as_ref().and_then(|r| r.forecast)
            } else {
                st.latest.forecast
            };
            let prev_mae = if source == "esp32" {
                st.esp_latest.as_ref().and_then(|r| r.mae)
            } else {
                st.latest.mae
            };
            let f_out = if state_str == "FAILSAFE" {
                ForecastOutput {
                    pm_forecast: prev_fc.unwrap_or(pm),
                    slope: 0.0,
                    eta_bahaya_s: None,
                    mae: prev_mae,
                }
            } else if source == "esp32" {
                st.esp_fc.update(t, pm)
            } else {
                st.external_fc.update(t, pm)
            };

            let mut row = TelemetryRow {
                t,
                t_s: t,
                seq,
                scenario: tel
                    .get("scenario")
                    .and_then(Value::as_str)
                    .unwrap_or(if source == "esp32" { "esp32" } else { "external" })
                    .to_string(),
                source: Some(source.to_string()),
                pm_true: tel.get("pm_true").and_then(Value::as_f64),
                gas_true: tel.get("gas_true").and_then(Value::as_f64),
                v_dust: tel.get("v_dust").and_then(Value::as_f64).unwrap_or(0.60),
                v_mq: tel.get("v_mq").and_then(Value::as_f64).unwrap_or(1.0),
                pm_raw: tel.get("pm_raw").and_then(Value::as_f64).unwrap_or(pm),
                pm,
                dcdt: tel.get("dcdt").and_then(Value::as_f64).unwrap_or(0.0),
                gas_ppm: tel
                    .get("gas_ppm")
                    .and_then(Value::as_f64)
                    .unwrap_or(cfg::MQ_ATM_PPM as f64),
                g: tel.get("g").and_then(Value::as_f64).unwrap_or(0.0),
                fi,
                v0: tel
                    .get("v0")
                    .and_then(Value::as_f64)
                    .unwrap_or(cfg::DUST_V0 as f64),
                state: state_str.clone(),
                mode: tel
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("AUTO")
                    .to_string(),
                duty: tel.get("duty").and_then(Value::as_f64).unwrap_or(0.0),
                forecast: Some(f_out.pm_forecast),
                slope: Some(f_out.slope),
                eta_bahaya_s: f_out.eta_bahaya_s,
                mae: f_out.mae,
                sensor_fault: state_str == "FAILSAFE",
                ref_fi: None,
                ref_pm: None,
                ref_state: None,
                parity_delta_fi: None,
            };

            if source == "esp32" {
                if let Some(meta) = st.hils_meta.get(&seq).cloned() {
                    row.pm_true = Some(meta.pm_true);
                    row.gas_true = Some(meta.gas_true);
                    row.scenario = meta.scenario;
                    row.ref_fi = Some(meta.ref_fi);
                    row.ref_pm = Some(meta.ref_pm);
                    row.ref_state = Some(meta.ref_state.clone());
                    let dfi = (row.fi - meta.ref_fi).abs();
                    let dpm = (row.pm - meta.ref_pm).abs();
                    row.parity_delta_fi = Some((dfi * 1000.0).round() / 1000.0);
                    st.hils_parity.samples += 1;
                    if row.state == meta.ref_state {
                        st.hils_parity.state_matches += 1;
                    }
                    if dfi > st.hils_parity.max_delta_fi {
                        st.hils_parity.max_delta_fi = dfi;
                    }
                    st.hils_parity.last_delta_fi = Some(dfi);
                    st.hils_parity.last_delta_pm = Some(dpm);
                }
                if let Some(last) = st.esp_history.back() {
                    if row.t < last.t {
                        st.esp_history.clear();
                    }
                }
                if st.esp_history.len() >= MAX_HISTORY {
                    st.esp_history.pop_front();
                }
                st.esp_history.push_back(row.clone());
                st.esp_latest = Some(row.clone());
            } else {
                if let Some(last) = st.history.back() {
                    if row.t < last.t {
                        st.history.clear();
                    }
                }
                if st.history.len() >= MAX_HISTORY {
                    st.history.pop_front();
                }
                st.history.push_back(row.clone());
                st.latest = row.clone();
            }
            row
        };

        self.broadcast(ServerEvent::Telemetry {
            source: source.to_string(),
            row,
            sim: self.get_sim_status(),
        });
    }

    pub fn get_sim_status(&self) -> SimStatus {
        let st = self.inner.lock().unwrap();
        let has_sitl = st.sim_running && st.sim_fw.is_some();
        let has_hils_ref =
            st.sim_running && st.sim_target == SimTarget::Hils && st.sim_ref_fw.is_some();
        let has_broker = st.broker_connected || st.publish_hook.is_some();
        SimStatus {
            target: st.sim_target.as_str().to_string(),
            running: st.sim_running,
            paused: st.sim_paused,
            scenario: st.sim_scenario.as_str().to_string(),
            speed: st.sim_speed,
            duration: st.sim_duration,
            step: st.sim_step_idx,
            ai_preempt: st.sim_ai_preempt,
            ai_preempt_active: st.sim_pre.as_ref().map(|p| p.active).unwrap_or(false),
            fault_inject: st.sim_fault_inject,
            enable_mqtt: self.cfg.enable_mqtt,
            broker_host: self.cfg.broker_host.clone(),
            broker_port: self.cfg.broker_port,
            broker_connected: st.broker_connected,
            node_status: st.node_status.clone(),
            esp_online: st.esp_online,
            esp_hw_mode: st.esp_hw_mode.clone(),
            hils_timeouts: st.hils_timeouts,
            hils_consec_timeouts: st.hils_consec_timeouts,
            hils_parity: st.hils_parity.clone(),
            has_sitl,
            has_hils_ref,
            has_broker,
            can_send_cmd: has_sitl || has_hils_ref || has_broker,
            log_file: st
                .mqtt_logger
                .as_ref()
                .and_then(|l| l.path.as_ref())
                .map(|p| p.display().to_string()),
            last_summary: st.last_run_summary.clone(),
        }
    }

    pub fn get_esp_log_status(&self) -> EspLogStatus {
        let st = self.inner.lock().unwrap();
        let lg = &st.esp_log;
        let online = st.esp_online;
        let key = match (lg.count, lg.crc32) {
            (Some(c), Some(crc)) => Some((c, crc)),
            _ => None,
        };
        let synced = key.is_some() && lg.synced_key == key;
        let downloaded = synced && lg.downloaded_key == key;
        let can_clear = online
            && lg.count.is_some()
            && lg.busy.is_none()
            && (lg.count == Some(0) || (synced && downloaded));
        EspLogStatus {
            online,
            log_count: lg.count,
            log_cap: lg.cap,
            log_full: lg.full,
            log_bytes: lg.bytes,
            log_cap_bytes: lg.cap_bytes,
            crc32: lg.crc32,
            synced,
            downloaded,
            can_clear,
            busy: lg.busy.clone(),
            progress: lg.progress,
            last_error: lg.last_error.clone(),
            last_sync: lg.last_sync.clone(),
            csv_file: lg
                .batch_csv
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string()),
            rtc_ok: lg.rtc_ok,
            epoch_s: lg.epoch_s,
            corrupt_skipped: lg.corrupt_skipped,
        }
    }

    pub fn get_state_snapshot(&self) -> StateSnapshot {
        let sim = self.get_sim_status();
        let esp_log = self.get_esp_log_status();
        let st = self.inner.lock().unwrap();
        let age_s = st
            .esp_last_rx
            .map(|t| (t.elapsed().as_secs_f64() * 10.0).round() / 10.0);
        let sitl_hist: Vec<TelemetryRow> = st.history.iter().cloned().collect();
        let esp_hist: Vec<TelemetryRow> = st.esp_history.iter().cloned().collect();
        StateSnapshot {
            latest: st.latest.clone(),
            history: sitl_hist.clone(),
            sources: SourcesMap {
                sitl: SourceBucket {
                    history: sitl_hist,
                    latest: Some(st.latest.clone()),
                },
                esp32: SourceBucket {
                    history: esp_hist,
                    latest: st.esp_latest.clone(),
                },
            },
            esp: EspStatus {
                online: st.esp_online,
                hw_mode: st.esp_hw_mode.clone(),
                age_s,
            },
            esp_log,
            mobile: MobileInfo {
                lan_ip: self.lan_ip.clone(),
                ip_is_fallback: self.ip_is_fallback,
                broker_port: self.cfg.broker_port,
            },
            mqtt_logs: st.mqtt_logs.iter().cloned().collect(),
            sim,
            thresholds: Thresholds {
                waspada: cfg::T_W_IN as f64,
                bahaya: cfg::T_B_IN as f64,
                horizon_s: karhutla_sim::forecast::FORECAST_HORIZON_S,
            },
        }
    }

    fn build_summary_locked(st: &InnerState, target: SimTarget, completed: bool) -> Option<RunSummary> {
        let is_hils = target == SimTarget::Hils;
        let hist = if is_hils { &st.esp_history } else { &st.history };
        if hist.is_empty() {
            return None;
        }
        let peak_fi = hist
            .iter()
            .filter(|r| r.state != "FAILSAFE" && !r.sensor_fault)
            .map(|r| r.fi)
            .fold(0.0_f64, f64::max);
        let last_t = hist.back().map(|r| r.t).unwrap_or(0.0);
        Some(RunSummary {
            target: target.as_str().to_string(),
            source: if is_hils { "esp32" } else { "sitl" }.to_string(),
            scenario: st.sim_scenario.as_str().to_string(),
            samples: hist.len(),
            peak_fi: (peak_fi * 10.0).round() / 10.0,
            hils_parity: if is_hils {
                Some(st.hils_parity.clone())
            } else {
                None
            },
            t_bahaya_s: st.run_t_bahaya,
            ai_preempt_used: st.run_preempt_used,
            duration_s: (last_t * 10.0).round() / 10.0,
            stopped_early: !completed,
        })
    }

    pub async fn stop_sim(&self) {
        let handle = {
            let mut st = self.inner.lock().unwrap();
            st.sim_running = false;
            st.sim_paused = false;
            st.hils_consec_timeouts = 0;
            st.sim_generation += 1;
            self.sim_task.lock().unwrap().take()
        };
        self.hils_notify.notify_waiters();
        if let Some(h) = handle {
            h.abort();
            let _ = h.await;
        }
    }

    pub async fn control_sim(self: &Arc<Self>, req: SimControlRequest) -> Result<SimStatus, String> {
        let action = req
            .action
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !action.is_empty()
            && !matches!(action.as_str(), "start" | "stop" | "pause" | "resume" | "clear")
        {
            return Err(format!("Aksi simulasi tidak dikenali: {action}."));
        }

        {
            let mut st = self.inner.lock().unwrap();
            let can_retarget = !st.sim_running || action == "start";
            if can_retarget {
                if let Some(tgt) = req.target.as_deref().and_then(SimTarget::parse) {
                    st.sim_target = tgt;
                }
                if let Some(sc) = req.scenario.as_deref().and_then(Scenario::parse) {
                    st.sim_scenario = sc;
                }
                if let Some(dur) = req.duration {
                    if dur > 0.0 {
                        st.sim_duration = dur;
                    }
                }
            }
            if let Some(sp) = req.speed {
                if sp > 0.0 {
                    st.sim_speed = sp;
                }
            }
            if let Some(pre) = req.ai_preempt {
                st.sim_ai_preempt = pre;
                if pre {
                    if st.sim_pre.is_none() {
                        st.sim_pre = Some(PreemptController::default());
                    }
                } else if let Some(p) = st.sim_pre.as_mut() {
                    p.active = false;
                }
            }
            if let Some(flt) = req.fault_inject {
                st.sim_fault_inject = flt;
            }
        }

        let mut manual_stop_summary = None;
        match action.as_str() {
            "start" => {
                self.stop_sim().await;
                self.start_sim_task();
            }
            "stop" => {
                let (was_running, stop_target) = {
                    let st = self.inner.lock().unwrap();
                    (st.sim_running, st.sim_target)
                };
                self.stop_sim().await;
                if was_running {
                    let mut st = self.inner.lock().unwrap();
                    manual_stop_summary = Self::build_summary_locked(&st, stop_target, false);
                    if let Some(s) = manual_stop_summary.clone() {
                        st.last_run_summary = Some(s);
                    }
                }
            }
            "pause" => {
                self.inner.lock().unwrap().sim_paused = true;
            }
            "resume" => {
                let mut st = self.inner.lock().unwrap();
                st.sim_paused = false;
                st.hils_consec_timeouts = 0;
            }
            "clear" => {
                let src = req.source.as_deref().unwrap_or("sitl");
                let cleared = {
                    let mut st = self.inner.lock().unwrap();
                    if src == "esp32" {
                        st.esp_history.clear();
                        st.esp_latest = None;
                        st.esp_fc.reset();
                        st.hils_meta.clear();
                        st.hils_parity = HilsParity::default();
                        "esp32"
                    } else {
                        st.history.clear();
                        st.latest = TelemetryRow::idle();
                        st.external_fc.reset();
                        "sitl"
                    }
                };
                {
                    let mut st = self.inner.lock().unwrap();
                    st.run_t_bahaya = None;
                    st.run_preempt_used = false;
                    st.last_run_summary = None;
                }
                self.broadcast(ServerEvent::Clear {
                    source: cleared.to_string(),
                });
            }
            _ => {}
        }

        let sim = self.get_sim_status();
        self.broadcast(ServerEvent::Sim { sim: sim.clone() });
        if let Some(summary) = manual_stop_summary {
            self.broadcast(ServerEvent::SimComplete {
                summary,
                sim: sim.clone(),
            });
        }
        Ok(sim)
    }

    fn start_sim_task(self: &Arc<Self>) {
        let (target, scenario, duration, gen) = {
            let mut st = self.inner.lock().unwrap();
            let target = st.sim_target;
            let scenario = st.sim_scenario;
            let duration = st.sim_duration;
            st.run_t_bahaya = None;
            st.run_preempt_used = false;
            st.last_run_summary = None;
            if target == SimTarget::Hils {
                st.esp_history.clear();
                st.esp_latest = None;
                st.esp_fc.reset();
                st.sim_ref_fw = Some(FirmwareCore::new());
                st.sim_fw = None;
                st.hils_meta.clear();
                st.hils_by_seq.clear();
                st.hils_timeouts = 0;
                st.hils_consec_timeouts = 0;
                st.hils_parity = HilsParity::default();
            } else {
                st.sim_ref_fw = None;
                st.sim_fw = Some(FirmwareCore::new());
                st.history.clear();
            }
            st.sim_fc.reset();
            st.sim_pre = if st.sim_ai_preempt {
                Some(PreemptController::default())
            } else {
                None
            };
            st.sim_step_idx = 0;
            st.sim_running = true;
            st.sim_paused = false;
            st.node_status = "online".to_string();
            st.sim_generation += 1;
            (target, scenario, duration, st.sim_generation)
        };

        self.broadcast(ServerEvent::Clear {
            source: if target == SimTarget::Hils {
                "esp32"
            } else {
                "sitl"
            }
            .to_string(),
        });

        let eng = Arc::clone(self);
        let handle = if target == SimTarget::Hils {
            tokio::spawn(async move {
                eng.run_hils_loop(scenario, duration, gen).await;
            })
        } else {
            tokio::spawn(async move {
                eng.run_sitl_loop(scenario, duration, gen).await;
            })
        };
        *self.sim_task.lock().unwrap() = Some(handle);
    }

    async fn run_sitl_loop(self: Arc<Self>, scenario: Scenario, duration: f64, gen: u64) {
        let total_steps = (duration / cfg::TS_S as f64) as u32;
        let mut plant = SmokeChamber::new(scenario, 1, cfg::TS_S as f64);
        let mut csv_logger = DailyCsvLogger::new(&self.cfg.out_dir, "karhutla", "sitl").ok();

        loop {
            let step_res = {
                let mut st = self.inner.lock().unwrap();
                if !st.sim_running || st.sim_generation != gen || st.sim_step_idx >= total_steps {
                    None
                } else if st.sim_paused {
                    Some(Err(()))
                } else {
                    let k = st.sim_step_idx;
                    st.sim_step_idx += 1;
                    let t = k as f64 * cfg::TS_S as f64;
                    let (mut vd, vm) = plant.sensors(t);
                    let fault_now = st.sim_fault_inject || plant.dust_disconnected(t);
                    if st.sim_fault_inject {
                        vd = 0.0;
                    }
                    let mut tel = st
                        .sim_fw
                        .as_mut()
                        .unwrap()
                        .step(vd as f32, vm as f32, k, cfg::TS_S);
                    let f_out = if tel.state.as_str() == "FAILSAFE" {
                        ForecastOutput {
                            pm_forecast: st.latest.forecast.unwrap_or(tel.pm as f64),
                            slope: 0.0,
                            eta_bahaya_s: None,
                            mae: st.latest.mae,
                        }
                    } else {
                        st.sim_fc.update(t, tel.pm as f64)
                    };
                    if st.sim_ai_preempt && tel.state.as_str() != "FAILSAFE" {
                        let mode_now = tel.mode.as_str().to_string();
                        let (pre_cmd, pre_active) = if let Some(pre) = st.sim_pre.as_mut() {
                            let c = pre.decide(t, &mode_now, f_out.pm_forecast);
                            (c, pre.active)
                        } else {
                            (None, false)
                        };
                        if let Some(cmd) = pre_cmd {
                            tel = st.sim_fw.as_mut().unwrap().apply_command(cmd_to_core(cmd));
                        }
                        if pre_active {
                            st.run_preempt_used = true;
                        }
                    }
                    if st.run_t_bahaya.is_none() && tel.state.as_str() == "BAHAYA" {
                        st.run_t_bahaya = Some((t * 10.0).round() / 10.0);
                    }
                    let row = telemetry_to_row(
                        &tel,
                        t,
                        scenario.as_str(),
                        "sitl",
                        Some(plant.c),
                        Some(plant.gas_ppm),
                        Some(f_out),
                        fault_now,
                    );
                    plant.advance(tel.duty as f64 / 100.0, t);
                    st.latest = row.clone();
                    if st.history.len() >= MAX_HISTORY {
                        st.history.pop_front();
                    }
                    st.history.push_back(row.clone());
                    Some(Ok((row, f_out, st.sim_speed)))
                }
            };

            match step_res {
                None => break,
                Some(Err(())) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                Some(Ok((row, f_out, speed))) => {
                    if let Some(logger) = csv_logger.as_mut() {
                        let _ = logger.write(&row);
                    }
                    let mqtt_opt = self.mqtt.lock().unwrap().clone();
                    if let Some(m) = mqtt_opt {
                        if let Ok(val) = serde_json::to_value(&row) {
                            let _ = m.publish_telemetry_bundle(&val).await;
                        }
                        m.publish_forecast_bundle(
                            f_out.pm_forecast,
                            f_out.slope,
                            f_out.eta_bahaya_s,
                            f_out.mae,
                        )
                        .await;
                    }
                    self.broadcast(ServerEvent::Telemetry {
                        source: "sitl".to_string(),
                        row,
                        sim: self.get_sim_status(),
                    });
                    let wait_ms = ((cfg::TS_S as f64 / speed.max(0.1)) * 1000.0).max(5.0) as u64;
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                }
            }
        }

        let summary = {
            let mut st = self.inner.lock().unwrap();
            if st.sim_generation != gen {
                return;
            }
            let natural = st.sim_running && st.sim_step_idx >= total_steps;
            st.sim_running = false;
            if natural {
                let s = Self::build_summary_locked(&st, SimTarget::Sitl, true);
                if let Some(ref sum) = s {
                    st.last_run_summary = Some(sum.clone());
                }
                s
            } else {
                None
            }
        };
        let sim = self.get_sim_status();
        self.broadcast(ServerEvent::Sim { sim: sim.clone() });
        if let Some(s) = summary {
            self.broadcast(ServerEvent::SimComplete { summary: s, sim });
        }
    }

    async fn run_hils_loop(self: Arc<Self>, scenario: Scenario, duration: f64, gen: u64) {
        let total_steps = (duration / cfg::TS_S as f64) as u32;
        let mut plant = SmokeChamber::new(scenario, 1, cfg::TS_S as f64);
        let mut csv_logger = DailyCsvLogger::new(&self.cfg.out_dir, "karhutla", "hils").ok();

        let _ = self
            .publish_raw(topics::T_CMD, "RESET", QoS::AtMostOnce, false)
            .await;
        tokio::time::sleep(Duration::from_millis(350)).await;
        self.inner.lock().unwrap().hils_by_seq.clear();

        loop {
            let step_ctx = {
                let mut st = self.inner.lock().unwrap();
                if !st.sim_running || st.sim_generation != gen || st.sim_step_idx >= total_steps {
                    None
                } else if st.sim_paused {
                    Some(Err(()))
                } else {
                    let k = st.sim_step_idx;
                    st.sim_step_idx += 1;
                    let t = k as f64 * cfg::TS_S as f64;
                    let (mut vd, vm) = plant.sensors(t);
                    let fault_now = st.sim_fault_inject || plant.dust_disconnected(t);
                    if st.sim_fault_inject {
                        vd = 0.0;
                    }
                    let vd_tx = (vd * 10_000.0).round() / 10_000.0;
                    let vm_tx = (vm * 10_000.0).round() / 10_000.0;
                    if !st.hils_meta.contains_key(&k) {
                        if let Some(ref_fw) = st.sim_ref_fw.as_mut() {
                            let ref_tel = ref_fw.step(vd_tx as f32, vm_tx as f32, k, cfg::TS_S);
                            st.hils_meta.insert(
                                k,
                                HilsMeta {
                                    pm_true: plant.c,
                                    gas_true: plant.gas_ppm,
                                    ref_pm: ref_tel.pm as f64,
                                    ref_fi: ref_tel.fi as f64,
                                    ref_state: ref_tel.state.as_str().to_string(),
                                    scenario: scenario.as_str().to_string(),
                                },
                            );
                            st.hils_meta.retain(|&ok, _| ok + 60 >= k);
                        }
                    }
                    Some(Ok((k, t, vd_tx, vm_tx, plant.c, plant.gas_ppm, fault_now, st.sim_speed)))
                }
            };

            let (k, t, vd_tx, vm_tx, pm_true, gas_true, fault_now, speed) = match step_ctx {
                None => break,
                Some(Err(())) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                Some(Ok(v)) => v,
            };

            let payload = format!("{k},{vd_tx:.4},{vm_tx:.4}");
            let _ = self
                .publish_raw(topics::T_HILS_IN, &payload, QoS::AtMostOnce, false)
                .await;

            // Wait up to 4.0s with 1 retry at 2.0s
            let deadline = Instant::now() + Duration::from_secs(4);
            let half_at = Instant::now() + Duration::from_secs(2);
            let mut retried = false;
            let mut reply: Option<Value> = None;

            loop {
                if let Some(v) = self.inner.lock().unwrap().hils_by_seq.remove(&k) {
                    reply = Some(v);
                    break;
                }
                {
                    let st = self.inner.lock().unwrap();
                    if !st.sim_running || st.sim_generation != gen {
                        return;
                    }
                }
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                if !retried && now >= half_at {
                    retried = true;
                    let _ = self
                        .publish_raw(topics::T_HILS_IN, &payload, QoS::AtMostOnce, false)
                        .await;
                }
                let wait_for = (deadline - now).min(Duration::from_millis(200));
                let _ = tokio::time::timeout(wait_for, self.hils_notify.notified()).await;
            }

            if let Some(tel) = reply {
                let (f_out, pre_cmd, duty, row_csv) = {
                    let mut st = self.inner.lock().unwrap();
                    st.hils_consec_timeouts = 0;
                    let tel_state = tel
                        .get("state")
                        .and_then(Value::as_str)
                        .unwrap_or("NORMAL")
                        .to_string();
                    let pm = tel.get("pm").and_then(Value::as_f64).unwrap_or(0.0);
                    let f_out = if tel_state == "FAILSAFE" {
                        ForecastOutput {
                            pm_forecast: st
                                .esp_latest
                                .as_ref()
                                .and_then(|r| r.forecast)
                                .unwrap_or(pm),
                            slope: 0.0,
                            eta_bahaya_s: None,
                            mae: st.esp_latest.as_ref().and_then(|r| r.mae),
                        }
                    } else {
                        st.sim_fc.update(t, pm)
                    };
                    let mut pre_cmd = None;
                    if st.sim_ai_preempt && tel_state != "FAILSAFE" {
                        let mode_now = tel.get("mode").and_then(Value::as_str).unwrap_or("AUTO");
                        if let Some(pre) = st.sim_pre.as_mut() {
                            pre_cmd = pre.decide(t, mode_now, f_out.pm_forecast);
                            if pre.active {
                                st.run_preempt_used = true;
                            }
                        }
                    }
                    if let Some(cmd) = pre_cmd {
                        if let Some(ref_fw) = st.sim_ref_fw.as_mut() {
                            let _ = ref_fw.apply_command(cmd_to_core(cmd));
                        }
                    }
                    if st.run_t_bahaya.is_none() && tel_state == "BAHAYA" {
                        st.run_t_bahaya = Some((t * 10.0).round() / 10.0);
                    }
                    let duty = tel.get("duty").and_then(Value::as_f64).unwrap_or(0.0);
                    let row_csv = TelemetryRow {
                        t,
                        t_s: t,
                        seq: k,
                        scenario: scenario.as_str().to_string(),
                        source: Some("hils".to_string()),
                        pm_true: Some(pm_true),
                        gas_true: Some(gas_true),
                        v_dust: vd_tx,
                        v_mq: vm_tx,
                        pm_raw: tel.get("pm_raw").and_then(Value::as_f64).unwrap_or(pm),
                        pm,
                        dcdt: tel.get("dcdt").and_then(Value::as_f64).unwrap_or(0.0),
                        gas_ppm: tel
                            .get("gas_ppm")
                            .and_then(Value::as_f64)
                            .unwrap_or(cfg::MQ_ATM_PPM as f64),
                        g: tel.get("g").and_then(Value::as_f64).unwrap_or(0.0),
                        fi: tel.get("fi").and_then(Value::as_f64).unwrap_or(0.0),
                        v0: tel
                            .get("v0")
                            .and_then(Value::as_f64)
                            .unwrap_or(cfg::DUST_V0 as f64),
                        state: tel_state,
                        mode: tel
                            .get("mode")
                            .and_then(Value::as_str)
                            .unwrap_or("AUTO")
                            .to_string(),
                        duty,
                        forecast: Some(f_out.pm_forecast),
                        slope: Some(f_out.slope),
                        eta_bahaya_s: f_out.eta_bahaya_s,
                        mae: f_out.mae,
                        sensor_fault: fault_now,
                        ref_fi: None,
                        ref_pm: None,
                        ref_state: None,
                        parity_delta_fi: None,
                    };
                    (f_out, pre_cmd, duty, row_csv)
                };
                plant.advance(duty / 100.0, t);
                if let Some(logger) = csv_logger.as_mut() {
                    let _ = logger.write(&row_csv);
                }
                let mqtt_opt = self.mqtt.lock().unwrap().clone();
                if let Some(m) = mqtt_opt {
                    m.publish_forecast_bundle(
                        f_out.pm_forecast,
                        f_out.slope,
                        f_out.eta_bahaya_s,
                        f_out.mae,
                    )
                    .await;
                    if let Some(cmd) = pre_cmd {
                        let _ = m
                            .publish(topics::T_CMD, cmd.as_str(), QoS::AtMostOnce, false)
                            .await;
                    }
                }
            } else {
                let (timeouts, consec, auto_paused) = {
                    let mut st = self.inner.lock().unwrap();
                    st.hils_timeouts += 1;
                    st.hils_consec_timeouts += 1;
                    st.sim_step_idx = st.sim_step_idx.saturating_sub(1);
                    let ap = st.hils_consec_timeouts >= 3;
                    if ap {
                        st.sim_paused = true;
                    }
                    (st.hils_timeouts, st.hils_consec_timeouts, ap)
                };
                self.broadcast(ServerEvent::HilsTimeout {
                    seq: k,
                    timeouts,
                    consec,
                    auto_paused,
                    sim: self.get_sim_status(),
                });
            }

            self.broadcast(ServerEvent::Sim {
                sim: self.get_sim_status(),
            });
            let wait_ms = ((cfg::TS_S as f64 / speed.max(0.1)) * 1000.0).max(5.0) as u64;
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        }

        let summary = {
            let mut st = self.inner.lock().unwrap();
            if st.sim_generation != gen {
                return;
            }
            let natural = st.sim_running && st.sim_step_idx >= total_steps;
            st.sim_running = false;
            if natural {
                let s = Self::build_summary_locked(&st, SimTarget::Hils, true);
                if let Some(ref sum) = s {
                    st.last_run_summary = Some(sum.clone());
                }
                s
            } else {
                None
            }
        };
        let sim = self.get_sim_status();
        self.broadcast(ServerEvent::Sim { sim: sim.clone() });
        if let Some(s) = summary {
            self.broadcast(ServerEvent::SimComplete { summary: s, sim });
        }
    }

    pub async fn send_cmd(&self, raw_cmd: &str, raw_target: Option<&str>) -> Result<CmdPaths, String> {
        let Some(cmd) = CmdAction::parse(raw_cmd) else {
            return Err("Perintah tidak dikenali.".to_string());
        };
        let target = CmdTarget::parse(raw_target.unwrap_or("auto"));

        let (apply_sitl, apply_hils_ref, publish_needed, did_clear, snap) = {
            let mut st = self.inner.lock().unwrap();
            let has_sitl = st.sim_running && st.sim_fw.is_some();
            let has_broker = st.broker_connected || st.publish_hook.is_some();
            let apply_sitl = has_sitl && matches!(target, CmdTarget::Auto | CmdTarget::Sitl);
            let apply_hils_ref = st.sim_running
                && st.sim_target == SimTarget::Hils
                && st.sim_ref_fw.is_some()
                && matches!(target, CmdTarget::Auto | CmdTarget::Esp32);
            let publish_needed = has_broker && matches!(target, CmdTarget::Auto | CmdTarget::Esp32);
            if !(apply_sitl || apply_hils_ref || publish_needed) {
                return Err(match target {
                    CmdTarget::Sitl => {
                        "Target SITL dipilih tetapi simulasi SITL tidak sedang berjalan."
                            .to_string()
                    }
                    CmdTarget::Esp32 => {
                        "Target ESP32 dipilih tetapi broker MQTT tidak terhubung.".to_string()
                    }
                    CmdTarget::Auto => {
                        "Tidak ada simulasi SITL aktif dan broker MQTT tidak terhubung.".to_string()
                    }
                });
            }

            let mut did_clear = false;
            let mut snap = None;
            if apply_sitl {
                let tel = st.sim_fw.as_mut().unwrap().apply_command(cmd_to_core(cmd));
                if cmd == CmdAction::Reset {
                    st.history.clear();
                    st.sim_step_idx = 0;
                    st.sim_fc.reset();
                    st.run_t_bahaya = None;
                    st.run_preempt_used = false;
                    st.last_run_summary = None;
                    st.latest = TelemetryRow::idle();
                    st.latest.scenario = st.sim_scenario.as_str().to_string();
                    did_clear = true;
                }
                st.latest.mode = tel.mode.as_str().to_string();
                st.latest.duty = tel.duty as f64;
                st.latest.state = tel.state.as_str().to_string();
                st.latest.pm = tel.pm as f64;
                st.latest.pm_raw = tel.pm_raw as f64;
                st.latest.dcdt = tel.dcdt as f64;
                st.latest.gas_ppm = tel.gas_ppm as f64;
                st.latest.g = tel.g as f64;
                st.latest.fi = tel.fi as f64;
                st.latest.v0 = tel.v0 as f64;
                let latest_clone = st.latest.clone();
                if !did_clear {
                    if let Some(last) = st.history.back_mut() {
                        *last = latest_clone.clone();
                    }
                }
                snap = Some(latest_clone);
            }
            if apply_hils_ref {
                let _ = st
                    .sim_ref_fw
                    .as_mut()
                    .unwrap()
                    .apply_command(cmd_to_core(cmd));
                if cmd == CmdAction::Reset {
                    st.esp_history.clear();
                    st.esp_latest = None;
                    st.hils_meta.clear();
                    st.run_t_bahaya = None;
                    st.run_preempt_used = false;
                    st.last_run_summary = None;
                    st.hils_parity = HilsParity::default();
                }
            }
            (apply_sitl, apply_hils_ref, publish_needed, did_clear, snap)
        };

        if publish_needed {
            if let Err(_e) = self
                .publish_raw(topics::T_CMD, cmd.as_str(), QoS::AtMostOnce, false)
                .await
            {
                if !apply_sitl && !apply_hils_ref {
                    return Err(format!(
                        "Broker MQTT :{} tidak dapat dijangkau — periksa layanan Mosquitto dan jaringan.",
                        self.cfg.broker_port
                    ));
                }
            }
        }
        if did_clear {
            self.broadcast(ServerEvent::Clear {
                source: "sitl".to_string(),
            });
        }
        if let Some(row) = snap {
            self.broadcast(ServerEvent::Telemetry {
                source: "sitl".to_string(),
                row,
                sim: self.get_sim_status(),
            });
        }
        Ok(CmdPaths {
            sitl: apply_sitl,
            esp32: publish_needed || apply_hils_ref,
        })
    }

    // ---------- ESP32 Backup Log Two-Phase Sync / Download / Clear ----------

    fn set_esp_log_busy(&self, busy: Option<String>, progress: Option<f64>, last_error: Option<Option<String>>) {
        {
            let mut st = self.inner.lock().unwrap();
            st.esp_log.busy = busy;
            st.esp_log.progress = progress;
            if let Some(err) = last_error {
                st.esp_log.last_error = err;
            }
        }
        self.broadcast(ServerEvent::EspLogStatus {
            status: self.get_esp_log_status(),
        });
    }

    async fn request_esp_log_meta(&self, timeout: Duration) -> Option<(u32, u32, Value)> {
        self.inner.lock().unwrap().log_meta = None;
        self.publish_raw(topics::T_LOG_REQ, "META", QoS::AtLeastOnce, false)
            .await
            .ok()?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(meta) = self.inner.lock().unwrap().log_meta.take() {
                let count = meta.get("count").and_then(Value::as_u64)? as u32;
                let crc32 = meta.get("crc32").and_then(Value::as_u64)? as u32;
                return Some((count, crc32, meta));
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let _ = tokio::time::timeout(deadline - now, self.log_notify.notified()).await;
        }
    }

    fn apply_esp_meta(&self, count: u32, crc32: u32, raw: &Value) {
        let mut st = self.inner.lock().unwrap();
        let lg = &mut st.esp_log;
        lg.count = Some(count);
        lg.meta_count = Some(count);
        lg.crc32 = Some(crc32);
        if let Some(v) = raw.get("cap").and_then(Value::as_u64) {
            lg.cap = Some(v as u32);
        }
        if let Some(v) = raw.get("full").and_then(Value::as_bool) {
            lg.full = Some(v);
        }
        if let Some(v) = raw.get("used_bytes").and_then(Value::as_u64) {
            lg.bytes = Some(v);
        }
        if let Some(v) = raw.get("cap_bytes").and_then(Value::as_u64) {
            lg.cap_bytes = Some(v);
        }
    }

    async fn fetch_esp_chunk(&self, start: u32, want: u32, total: u32) -> Result<Vec<u8>, String> {
        let mut last_err = "tidak ada respons".to_string();
        for _ in 0..3 {
            self.inner.lock().unwrap().log_chunks.remove(&start);
            self.publish_raw(
                topics::T_LOG_REQ,
                &format!("GET:{start}:{want}"),
                QoS::AtLeastOnce,
                false,
            )
            .await?;
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut got = None;
            loop {
                if let Some(msg) = self.inner.lock().unwrap().log_chunks.remove(&start) {
                    got = Some(msg);
                    break;
                }
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let _ = tokio::time::timeout(deadline - now, self.log_notify.notified()).await;
            }
            let Some(msg) = got else {
                last_err = format!("timeout chunk {start}");
                continue;
            };
            let msg_count = msg.get("count").and_then(Value::as_u64).unwrap_or(u64::MAX) as u32;
            let msg_total = msg.get("total").and_then(Value::as_u64).unwrap_or(u64::MAX) as u32;
            let data_b64 = msg.get("data").and_then(Value::as_str).unwrap_or("");
            if msg_count != want || msg_total != total {
                last_err = format!("chunk {start} tidak konsisten");
                continue;
            }
            match decode_base64_chunk(data_b64, want as usize) {
                Ok(bytes) => return Ok(bytes),
                Err(e) => {
                    last_err = format!("chunk {start}: {e}");
                }
            }
        }
        Err(last_err)
    }

    pub async fn sync_esp_logs(&self) -> Result<(usize, u32, u32, String), String> {
        let Ok(_guard) = self.log_op_lock.try_lock() else {
            return Err("operasi log lain sedang berjalan".to_string());
        };
        if !self.inner.lock().unwrap().esp_online {
            let msg = "ESP32 offline".to_string();
            self.set_esp_log_busy(None, None, Some(Some(msg.clone())));
            return Err(msg);
        }
        self.set_esp_log_busy(Some("sync".to_string()), Some(0.0), Some(None));
        let res = async {
            let Some((total, crc32, raw_meta)) =
                self.request_esp_log_meta(Duration::from_secs(5)).await
            else {
                return Err("META tidak dijawab ESP32".to_string());
            };
            let mut raw_all = Vec::with_capacity(total as usize * 64);
            let mut start = 0u32;
            while start < total {
                let want = (total - start).min(16);
                let chunk = self.fetch_esp_chunk(start, want, total).await?;
                raw_all.extend_from_slice(&chunk);
                start += want;
                let prog = ((start as f64 / total.max(1) as f64) * 1000.0).round() / 1000.0;
                self.set_esp_log_busy(Some("sync".to_string()), Some(prog), None);
            }
            let batch = decode_and_verify_batch(&raw_all, total as usize, crc32)?;
            let batch_path = write_esp_archives_and_batch(&self.cfg.out_dir, &batch)
                .map_err(|e| format!("gagal tulis CSV backup: {e}"))?;
            let csv_name = batch_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            self.apply_esp_meta(total, crc32, &raw_meta);
            {
                let mut st = self.inner.lock().unwrap();
                st.esp_log.synced_key = Some((total, crc32));
                st.esp_log.downloaded_key = None;
                st.esp_log.batch_csv = Some(batch_path);
                st.esp_log.last_sync = Some(
                    Local::now().to_rfc3339_opts(SecondsFormat::Secs, false),
                );
                st.esp_log.last_error = None;
                st.esp_log.corrupt_skipped = batch.corrupt_skipped;
            }
            Ok((batch.rows.len(), batch.corrupt_skipped, crc32, csv_name))
        }
        .await;

        match &res {
            Ok(_) => self.set_esp_log_busy(None, None, None),
            Err(e) => self.set_esp_log_busy(None, None, Some(Some(e.clone()))),
        }
        res
    }

    pub fn get_esp_log_download_path(&self) -> Result<(PathBuf, (u32, u32)), String> {
        let st = self.inner.lock().unwrap();
        let lg = &st.esp_log;
        let key = match (lg.count, lg.crc32) {
            (Some(c), Some(crc)) => (c, crc),
            _ => {
                return Err(
                    "Belum sync (atau log berubah sejak sync); jalankan sync dulu.".to_string(),
                )
            }
        };
        if lg.synced_key != Some(key) {
            return Err("Belum sync (atau log berubah sejak sync); jalankan sync dulu.".to_string());
        }
        let Some(ref path) = lg.batch_csv else {
            return Err("File batch CSV belum tersedia; jalankan sync dulu.".to_string());
        };
        if !path.exists() {
            return Err("File batch CSV tidak ditemukan; jalankan sync ulang.".to_string());
        }
        Ok((path.clone(), key))
    }

    pub fn mark_esp_log_downloaded(&self, key: (u32, u32)) -> bool {
        let ok = {
            let mut st = self.inner.lock().unwrap();
            let lg = &mut st.esp_log;
            let cur = match (lg.count, lg.crc32) {
                (Some(c), Some(crc)) => Some((c, crc)),
                _ => None,
            };
            if cur != Some(key) || lg.synced_key != Some(key) {
                false
            } else {
                lg.downloaded_key = Some(key);
                true
            }
        };
        if ok {
            self.broadcast(ServerEvent::EspLogStatus {
                status: self.get_esp_log_status(),
            });
        }
        ok
    }

    pub async fn clear_esp_logs(&self) -> (u16, Result<(), String>) {
        let Ok(_guard) = self.log_op_lock.try_lock() else {
            return (409, Err("operasi log lain sedang berjalan".to_string()));
        };
        let st_now = self.get_esp_log_status();
        if !st_now.online || st_now.log_count.is_none() {
            return (409, Err("ESP32 offline; clear ditolak.".to_string()));
        }
        if st_now.log_count.unwrap_or(0) > 0 && !(st_now.synced && st_now.downloaded) {
            return (
                409,
                Err("Log belum disinkronkan dan diunduh; clear ditolak.".to_string()),
            );
        }

        self.set_esp_log_busy(Some("clear".to_string()), None, Some(None));
        let res = async {
            let Some((count, crc32, raw_meta)) =
                self.request_esp_log_meta(Duration::from_secs(5)).await
            else {
                return (504, Err("META tidak dijawab ESP32".to_string()));
            };
            self.apply_esp_meta(count, crc32, &raw_meta);
            if count > 0 {
                let ok = {
                    let st = self.inner.lock().unwrap();
                    let cur = Some((count, crc32));
                    st.esp_log.synced_key == cur && st.esp_log.downloaded_key == cur
                };
                if !ok {
                    return (
                        409,
                        Err("Isi log berubah sejak sync/unduh; clear ditolak.".to_string()),
                    );
                }
            }
            self.inner.lock().unwrap().log_acks.clear();
            if let Err(e) = self
                .publish_raw(
                    topics::T_LOG_REQ,
                    &format!("CLEAR:{count}:{crc32}"),
                    QoS::AtLeastOnce,
                    false,
                )
                .await
            {
                return (502, Err(e));
            }

            let find_ack = |status_val: &str| -> Option<Value> {
                let st = self.inner.lock().unwrap();
                st.log_acks
                    .iter()
                    .find(|a| {
                        a.get("action").and_then(Value::as_str) == Some("CLEAR")
                            && a.get("status").and_then(Value::as_str) == Some(status_val)
                    })
                    .cloned()
            };

            let deadline1 = Instant::now() + Duration::from_secs(5);
            let mut done = None;
            let mut started = None;
            loop {
                if let Some(d) = find_ack("done") {
                    done = Some(d);
                    break;
                }
                if let Some(s) = find_ack("started") {
                    started = Some(s);
                    break;
                }
                let now = Instant::now();
                if now >= deadline1 {
                    return (504, Err("ESP32 tidak mengakui CLEAR".to_string()));
                }
                let _ = tokio::time::timeout(deadline1 - now, self.log_notify.notified()).await;
            }

            if done.is_none() {
                let sectors = started
                    .as_ref()
                    .and_then(|s| s.get("sectors"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let wait_s = ((sectors as f64 * 0.12).ceil() + 15.0).clamp(20.0, 90.0) as u64;
                let deadline2 = Instant::now() + Duration::from_secs(wait_s);
                loop {
                    if let Some(d) = find_ack("done") {
                        done = Some(d);
                        break;
                    }
                    let now = Instant::now();
                    if now >= deadline2 {
                        return (504, Err("timeout menunggu CLEAR selesai".to_string()));
                    }
                    let _ = tokio::time::timeout(deadline2 - now, self.log_notify.notified()).await;
                }
            }

            let done_val = done.unwrap();
            if done_val.get("ok").and_then(Value::as_bool) != Some(true) {
                let reason = done_val
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("ditolak ESP32");
                return (409, Err(format!("CLEAR ditolak: {reason}")));
            }
            {
                let mut st = self.inner.lock().unwrap();
                st.esp_log.count = Some(0);
                st.esp_log.meta_count = Some(0);
                st.esp_log.crc32 = Some(0);
                st.esp_log.full = Some(false);
                st.esp_log.bytes = Some(0);
                st.esp_log.synced_key = None;
                st.esp_log.downloaded_key = None;
                st.esp_log.last_error = None;
            }
            (200, Ok(()))
        }
        .await;

        match &res.1 {
            Ok(()) => self.set_esp_log_busy(None, None, None),
            Err(e) => self.set_esp_log_busy(None, None, Some(Some(e.clone()))),
        }
        res
    }

    pub fn export_csv_bytes(&self, req_src: &str) -> (String, Vec<u8>) {
        let (chosen_src, rows): (&str, Vec<TelemetryRow>) = {
            let st = self.inner.lock().unwrap();
            match req_src.trim().to_ascii_lowercase().as_str() {
                "esp32" => ("esp32", st.esp_history.iter().cloned().collect()),
                "sitl" => ("sitl", st.history.iter().cloned().collect()),
                _ => {
                    let use_esp =
                        st.esp_online || (!st.esp_history.is_empty() && !st.sim_running);
                    if use_esp {
                        ("esp32", st.esp_history.iter().cloned().collect())
                    } else {
                        ("sitl", st.history.iter().cloned().collect())
                    }
                }
            }
        };
        let mut wtr = csv::Writer::from_writer(Vec::new());
        let cols = [
            "t_s",
            "seq",
            "source",
            "scenario",
            "pm_true",
            "v_dust",
            "v_mq",
            "pm_raw",
            "pm",
            "dcdt",
            "gas_ppm",
            "g",
            "fi",
            "v0",
            "state",
            "mode",
            "duty",
            "forecast",
            "slope",
            "eta_bahaya_s",
            "mae",
            "sensor_fault",
            "ref_fi",
            "parity_delta_fi",
        ];
        let _ = wtr.write_record(cols);
        let opt = |v: Option<f64>| v.map(|x| format!("{x}")).unwrap_or_default();
        for r in rows {
            let rec = [
                format!("{}", r.t_s),
                r.seq.to_string(),
                r.source.unwrap_or_else(|| chosen_src.to_string()),
                r.scenario,
                opt(r.pm_true),
                format!("{}", r.v_dust),
                format!("{}", r.v_mq),
                format!("{}", r.pm_raw),
                format!("{}", r.pm),
                format!("{}", r.dcdt),
                format!("{}", r.gas_ppm),
                format!("{}", r.g),
                format!("{}", r.fi),
                format!("{}", r.v0),
                r.state,
                r.mode,
                format!("{}", r.duty),
                opt(r.forecast),
                opt(r.slope),
                opt(r.eta_bahaya_s),
                opt(r.mae),
                if r.sensor_fault {
                    "True".to_string()
                } else {
                    "False".to_string()
                },
                opt(r.ref_fi),
                opt(r.parity_delta_fi),
            ];
            let _ = wtr.write_record(&rec);
        }
        let data = wtr.into_inner().unwrap_or_default();
        let day = Local::now().format("%Y-%m-%d");
        (format!("karhutla_{chosen_src}_{day}.csv"), data)
    }

    pub fn export_mqtt_log_bytes(&self) -> (String, Vec<u8>) {
        let (path_opt, mem_lines) = {
            let st = self.inner.lock().unwrap();
            let p = st.mqtt_logger.as_ref().and_then(|l| l.path.clone());
            let lines: Vec<String> = st.mqtt_logs.iter().cloned().collect();
            (p, lines)
        };
        let content = path_opt
            .filter(|p| Path::new(p).exists())
            .and_then(|p| std::fs::read(&p).ok())
            .unwrap_or_else(|| mem_lines.join("\n").into_bytes());
        let day = Local::now().format("%Y-%m-%d");
        (format!("karhutla_mqtt_{day}.log"), content)
    }

    pub async fn shutdown(&self) {
        self.stop_sim().await;
        if let Some(m) = self.mqtt.lock().unwrap().take() {
            m.stop_spawned_broker();
        }
    }
}
