use karhutla_protocol::{
    topics, EspLogStatus, EspStatus, MobileInfo, ServerEvent, SimStatus, StateSnapshot,
    TelemetryRow, Thresholds,
};
use serde_json::Value;

/// Mem-parsing paket MQTT mentah dari topik `karhutla/#` langsung ke `ServerEvent` UI.
#[cfg_attr(feature = "web", allow(dead_code))]
#[derive(Debug, Clone)]
pub struct DirectMqttParser {
    pub broker_host: String,
    pub broker_port: u16,
    pub broker_connected: bool,
    pub node_status: String,
    pub esp_online: bool,
    pub esp_hw_mode: Option<String>,
    pub last_hb_ts: Option<f64>,
    pub latest_fc_fi: Option<f64>,
    pub latest_fc_pm: Option<f64>,
    pub latest_fc_slope: Option<f64>,
    pub latest_fc_eta: Option<f64>,
    pub latest_fc_mae: Option<f64>,
    pub log_status: EspLogStatus,
    pub step_counter: u32,
}

#[cfg_attr(feature = "web", allow(dead_code))]
impl DirectMqttParser {
    pub fn new(broker_host: impl Into<String>, broker_port: u16) -> Self {
        Self {
            broker_host: broker_host.into(),
            broker_port,
            broker_connected: false,
            node_status: "OFFLINE".to_string(),
            esp_online: false,
            esp_hw_mode: None,
            last_hb_ts: None,
            latest_fc_fi: None,
            latest_fc_pm: None,
            latest_fc_slope: None,
            latest_fc_eta: None,
            latest_fc_mae: None,
            log_status: EspLogStatus::default(),
            step_counter: 0,
        }
    }

    pub fn set_connected(&mut self, connected: bool) -> Vec<ServerEvent> {
        self.broker_connected = connected;
        if !connected {
            self.esp_online = false;
            self.node_status = "OFFLINE".to_string();
            self.log_status.online = false;
        }
        let log_line = if connected {
            format!(
                "[direct-mqtt] CONNECT tcp://{}:{} connected",
                self.broker_host, self.broker_port
            )
        } else {
            format!(
                "[direct-mqtt] DISCONNECT tcp://{}:{} disconnected",
                self.broker_host, self.broker_port
            )
        };
        let mut evs = vec![
            ServerEvent::Status {
                broker_connected: self.broker_connected,
                node_status: self.node_status.clone(),
            },
            ServerEvent::Sim {
                sim: self.build_sim_status(),
            },
            ServerEvent::MqttLog { line: log_line },
        ];
        if !connected {
            evs.push(ServerEvent::EspStatus {
                online: false,
                hw_mode: self.esp_hw_mode.clone(),
                age_s: None,
            });
            evs.push(ServerEvent::EspLogStatus {
                status: self.log_status.clone(),
            });
        }
        evs
    }

    pub fn build_sim_status(&self) -> SimStatus {
        SimStatus {
            target: "hils".to_string(),
            running: false,
            paused: false,
            scenario: "esp32".to_string(),
            speed: 1.0,
            duration: 900.0,
            step: self.step_counter,
            ai_preempt: false,
            ai_preempt_active: false,
            fault_inject: false,
            enable_mqtt: true,
            broker_host: self.broker_host.clone(),
            broker_port: self.broker_port,
            broker_connected: self.broker_connected,
            node_status: self.node_status.clone(),
            esp_online: self.esp_online,
            esp_hw_mode: self.esp_hw_mode.clone(),
            hils_timeouts: 0,
            hils_consec_timeouts: 0,
            hils_parity: Default::default(),
            has_sitl: false,
            has_hils_ref: false,
            has_broker: self.broker_connected,
            can_send_cmd: self.broker_connected,
            log_file: None,
            last_summary: None,
        }
    }

    pub fn ingest(&mut self, topic: &str, payload: &str, now_sec: f64) -> Vec<ServerEvent> {
        let mut events = Vec::new();
        if should_log_topic(topic) {
            let trimmed = payload.trim();
            let preview: String = if trimmed.chars().count() > 220 {
                let mut s: String = trimmed.chars().take(217).collect();
                s.push_str("...");
                s
            } else {
                trimmed.to_string()
            };
            events.push(ServerEvent::MqttLog {
                line: format!("[mqtt] RX {topic} {preview}"),
            });
        }

        if topic == topics::T_STATUS {
            let status_clean = payload.trim().to_ascii_lowercase();
            let is_online = status_clean == "online" || status_clean.contains("\"online\"");
            self.esp_online = is_online;
            self.node_status = if is_online {
                self.last_hb_ts = Some(now_sec);
                "ONLINE".to_string()
            } else {
                "OFFLINE".to_string()
            };
            self.log_status.online = is_online;
            events.push(ServerEvent::Status {
                broker_connected: self.broker_connected,
                node_status: self.node_status.clone(),
            });
            events.push(ServerEvent::EspStatus {
                online: is_online,
                hw_mode: self.esp_hw_mode.clone(),
                age_s: if is_online { Some(0.0) } else { None },
            });
            events.push(ServerEvent::EspLogStatus {
                status: self.log_status.clone(),
            });
            events.push(ServerEvent::Sim {
                sim: self.build_sim_status(),
            });
            return events;
        }

        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return events;
        };

        if topic == topics::T_HEARTBEAT {
            self.esp_online = true;
            self.node_status = "ONLINE".to_string();
            self.last_hb_ts = Some(now_sec);
            if let Some(m) = v
                .get("hw_mode")
                .or_else(|| v.get("mode"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                self.esp_hw_mode = Some(m.to_string());
            }
            self.log_status.online = true;
            if let Some(rtc) = v.get("rtc_ok").and_then(Value::as_bool) {
                self.log_status.rtc_ok = Some(rtc);
            }
            if let Some(ep) = v.get("epoch_s").and_then(Value::as_u64) {
                self.log_status.epoch_s = Some(ep);
            }
            if let Some(cnt) = v.get("log_count").and_then(Value::as_u64) {
                self.log_status.log_count = Some(cnt as u32);
            }
            if let Some(cap) = v.get("log_cap").and_then(Value::as_u64) {
                self.log_status.log_cap = Some(cap as u32);
            }
            if let Some(full) = v.get("log_full").and_then(Value::as_bool) {
                self.log_status.log_full = Some(full);
            }
            if let Some(bytes) = v.get("log_bytes").and_then(Value::as_u64) {
                self.log_status.log_bytes = Some(bytes);
            }
            if let Some(cap_b) = v.get("log_cap_bytes").and_then(Value::as_u64) {
                self.log_status.log_cap_bytes = Some(cap_b);
            }
            if let Some(crc) = v
                .get("crc32")
                .or_else(|| v.get("log_crc32"))
                .and_then(Value::as_u64)
            {
                self.log_status.crc32 = Some(crc as u32);
            }
            events.push(ServerEvent::Status {
                broker_connected: self.broker_connected,
                node_status: self.node_status.clone(),
            });
            events.push(ServerEvent::EspStatus {
                online: true,
                hw_mode: self.esp_hw_mode.clone(),
                age_s: Some(0.0),
            });
            events.push(ServerEvent::EspLogStatus {
                status: self.log_status.clone(),
            });
            events.push(ServerEvent::Sim {
                sim: self.build_sim_status(),
            });
            return events;
        }

        if topic == topics::T_FORECAST {
            self.latest_fc_fi = v
                .get("fi_forecast")
                .or_else(|| v.get("fi"))
                .and_then(Value::as_f64);
            self.latest_fc_pm = v
                .get("pm_forecast")
                .or_else(|| v.get("forecast"))
                .and_then(Value::as_f64);
            self.latest_fc_slope = v.get("slope").and_then(Value::as_f64);
            self.latest_fc_eta = v.get("eta_bahaya_s").and_then(Value::as_f64);
            self.latest_fc_mae = v.get("mae").and_then(Value::as_f64);
            return events;
        }

        if topic == topics::T_TELEM {
            let was_offline = !self.esp_online;
            self.esp_online = true;
            self.node_status = "ONLINE".to_string();
            self.last_hb_ts = Some(now_sec);
            self.log_status.online = true;

            let seq = v
                .get("seq")
                .and_then(Value::as_u64)
                .map(|s| s as u32)
                .unwrap_or_else(|| {
                    self.step_counter = self.step_counter.saturating_add(1);
                    self.step_counter
                });
            if seq > self.step_counter {
                self.step_counter = seq;
            } else if self.step_counter == 0 {
                self.step_counter = 1;
            }
            let t = v
                .get("t")
                .or_else(|| v.get("t_s"))
                .and_then(Value::as_f64)
                .unwrap_or(seq as f64 * 2.0);
            let state_str = v
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("NORMAL")
                .to_string();
            let pm = v.get("pm").and_then(Value::as_f64).unwrap_or(0.0);
            let is_fault = state_str == "FAILSAFE"
                || v.get("sensor_fault").and_then(Value::as_bool).unwrap_or(false);
            let source = v
                .get("source")
                .and_then(Value::as_str)
                .map(|s| if s == "sitl" { "sitl" } else { "esp32" })
                .unwrap_or("esp32")
                .to_string();

            let forecast = if is_fault {
                None
            } else {
                v.get("forecast")
                    .or_else(|| v.get("pm_forecast"))
                    .and_then(Value::as_f64)
                    .or(self.latest_fc_pm)
            };
            let slope = v.get("slope").and_then(Value::as_f64).or(if is_fault {
                Some(0.0)
            } else {
                self.latest_fc_slope
            });
            let eta_bahaya_s = if is_fault {
                None
            } else {
                v.get("eta_bahaya_s")
                    .and_then(Value::as_f64)
                    .or(self.latest_fc_eta)
            };
            let mae = v
                .get("mae")
                .and_then(Value::as_f64)
                .or(self.latest_fc_mae);

            let row = TelemetryRow {
                t,
                t_s: t,
                seq,
                scenario: v
                    .get("scenario")
                    .and_then(Value::as_str)
                    .unwrap_or("esp32")
                    .to_string(),
                source: Some(source.clone()),
                pm_true: v.get("pm_true").and_then(Value::as_f64),
                gas_true: v.get("gas_true").and_then(Value::as_f64),
                v_dust: v.get("v_dust").and_then(Value::as_f64).unwrap_or(0.60),
                v_mq: v.get("v_mq").and_then(Value::as_f64).unwrap_or(1.0),
                pm_raw: v.get("pm_raw").and_then(Value::as_f64).unwrap_or(pm),
                pm,
                dcdt: v.get("dcdt").and_then(Value::as_f64).unwrap_or(0.0),
                gas_ppm: v.get("gas_ppm").and_then(Value::as_f64).unwrap_or(400.0),
                g: v.get("g").and_then(Value::as_f64).unwrap_or(0.0),
                fi: v.get("fi").and_then(Value::as_f64).unwrap_or(0.0),
                v0: v.get("v0").and_then(Value::as_f64).unwrap_or(0.60),
                state: state_str,
                mode: v
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("AUTO")
                    .to_string(),
                duty: v.get("duty").and_then(Value::as_f64).unwrap_or(0.0),
                forecast,
                slope,
                eta_bahaya_s,
                mae,
                sensor_fault: is_fault,
                ref_fi: v.get("ref_fi").and_then(Value::as_f64),
                ref_pm: v.get("ref_pm").and_then(Value::as_f64),
                ref_state: v
                    .get("ref_state")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                parity_delta_fi: v.get("parity_delta_fi").and_then(Value::as_f64),
            };

            if was_offline {
                events.push(ServerEvent::Status {
                    broker_connected: self.broker_connected,
                    node_status: self.node_status.clone(),
                });
                events.push(ServerEvent::EspStatus {
                    online: true,
                    hw_mode: self.esp_hw_mode.clone(),
                    age_s: Some(0.0),
                });
            }
            events.push(ServerEvent::Telemetry {
                source,
                row,
                sim: self.build_sim_status(),
            });
        }

        events
    }

    pub fn check_stale(&mut self, now_sec: f64, threshold_s: f64) -> Option<ServerEvent> {
        if !self.esp_online {
            return None;
        }
        let last = self.last_hb_ts?;
        let age = (now_sec - last).max(0.0);
        if age > threshold_s {
            self.esp_online = false;
            self.node_status = "OFFLINE".to_string();
            self.log_status.online = false;
            return Some(ServerEvent::EspStatus {
                online: false,
                hw_mode: self.esp_hw_mode.clone(),
                age_s: Some((age * 10.0).round() / 10.0),
            });
        }
        None
    }
}

#[cfg_attr(feature = "web", allow(dead_code))]
fn should_log_topic(topic: &str) -> bool {
    !topic.starts_with("karhutla/t/")
        && topic != topics::T_TELEM_SCALAR
        && !topic.starts_with("karhutla/forecast/")
        && topic != topics::T_FORECAST_SCALAR
}

pub fn default_direct_snapshot(host: &str, port: u16) -> StateSnapshot {
    let parser = DirectMqttParser::new(host, port);
    StateSnapshot {
        latest: TelemetryRow::idle(),
        history: Vec::new(),
        sources: Default::default(),
        esp: EspStatus {
            online: false,
            hw_mode: None,
            age_s: None,
        },
        esp_log: EspLogStatus::default(),
        mobile: MobileInfo {
            lan_ip: host.to_string(),
            ip_is_fallback: host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost"),
            broker_port: port,
        },
        mqtt_logs: Vec::new(),
        sim: parser.build_sim_status(),
        thresholds: Thresholds {
            waspada: 55.0,
            bahaya: 150.0,
            horizon_s: 60.0,
        },
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn rows_to_csv_bytes(rows: &[TelemetryRow]) -> Vec<u8> {
    let mut out = String::from(
        "t_s,seq,source,scenario,pm_true,v_dust,v_mq,pm_raw,pm,dcdt,gas_ppm,g,fi,v0,state,mode,duty,forecast,slope,eta_bahaya_s,mae,sensor_fault,ref_fi,parity_delta_fi\n",
    );
    let opt = |v: Option<f64>| v.map(|x| format!("{x}")).unwrap_or_default();
    for r in rows {
        let src = r.source.as_deref().unwrap_or("esp32");
        let fault = if r.sensor_fault { "True" } else { "False" };
        let fields = [
            format!("{}", r.t_s),
            r.seq.to_string(),
            csv_escape(src),
            csv_escape(&r.scenario),
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
            csv_escape(&r.state),
            csv_escape(&r.mode),
            format!("{}", r.duty),
            opt(r.forecast),
            opt(r.slope),
            opt(r.eta_bahaya_s),
            opt(r.mae),
            fault.to_string(),
            opt(r.ref_fi),
            opt(r.parity_delta_fi),
        ];
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out.into_bytes()
}

// =========================================================================
// Native rumqttc TCP Client (Desktop & Mobile APK)
// =========================================================================

#[cfg(any(feature = "desktop", feature = "mobile"))]
mod native_mqtt {
    use super::*;
    use crate::state::now_ms;
    use crate::transport::LiveMessage;
    use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    static ACTIVE_MQTT_CLIENT: OnceLock<Mutex<Option<(String, u16, AsyncClient)>>> = OnceLock::new();

    fn client_slot() -> &'static Mutex<Option<(String, u16, AsyncClient)>> {
        ACTIVE_MQTT_CLIENT.get_or_init(|| Mutex::new(None))
    }

    fn set_active_client(host: &str, port: u16, client: AsyncClient) {
        if let Ok(mut guard) = client_slot().lock() {
            *guard = Some((host.to_string(), port, client));
        }
    }

    fn clear_active_client(host: &str, port: u16) {
        if let Ok(mut guard) = client_slot().lock() {
            let matches = guard
                .as_ref()
                .map(|(h, p, _)| h == host && *p == port)
                .unwrap_or(false);
            if matches {
                *guard = None;
            }
        }
    }

    fn get_active_client(host: &str, port: u16) -> Option<AsyncClient> {
        client_slot().lock().ok().and_then(|guard| {
            guard
                .as_ref()
                .filter(|(h, p, _)| h == host && *p == port)
                .map(|(_, _, c)| c.clone())
        })
    }

    fn now_sec() -> f64 {
        now_ms() / 1000.0
    }

    fn unique_client_id(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        format!("{prefix}-{}-{nanos}", std::process::id())
    }

    fn make_opts(prefix: &str, host: &str, port: u16) -> MqttOptions {
        let mut opts = MqttOptions::new(unique_client_id(prefix), host, port);
        opts.set_keep_alive(Duration::from_secs(30));
        opts.set_max_packet_size(256 * 1024, 256 * 1024);
        opts.set_clean_session(true);
        opts
    }

    async fn emit_all(
        tx: &tokio::sync::mpsc::Sender<LiveMessage>,
        events: Vec<ServerEvent>,
    ) -> bool {
        for ev in events {
            if tx.send(LiveMessage::Event(ev)).await.is_err() {
                return false;
            }
        }
        true
    }

    pub async fn run_direct_mqtt_client(
        host: String,
        port: u16,
        tx: tokio::sync::mpsc::Sender<LiveMessage>,
    ) {
        let mut had_disconnect = false;
        let mut parser = DirectMqttParser::new(host.clone(), port);

        while !tx.is_closed() {
            let opts = make_opts("karhutla-ui-direct", &host, port);
            let (client, mut eventloop) = AsyncClient::new(opts, 64);
            let mut stale_tick = tokio::time::interval(Duration::from_secs(2));
            stale_tick.tick().await;

            loop {
                tokio::select! {
                    _ = tx.closed() => {
                        let _ = client.disconnect().await;
                        clear_active_client(&host, port);
                        return;
                    }
                    _ = stale_tick.tick() => {
                        if let Some(stale_ev) = parser.check_stale(now_sec(), 12.0) {
                            let status_ev = ServerEvent::Status {
                                broker_connected: parser.broker_connected,
                                node_status: parser.node_status.clone(),
                            };
                            let log_ev = ServerEvent::EspLogStatus {
                                status: parser.log_status.clone(),
                            };
                            let sim_ev = ServerEvent::Sim {
                                sim: parser.build_sim_status(),
                            };
                            if !emit_all(&tx, vec![status_ev, stale_ev, log_ev, sim_ev]).await {
                                let _ = client.disconnect().await;
                                clear_active_client(&host, port);
                                return;
                            }
                        }
                    }
                    res = eventloop.poll() => {
                        match res {
                            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                                let _ = client.subscribe("karhutla/#", QoS::AtLeastOnce).await;
                            }
                            Ok(Event::Incoming(Packet::SubAck(_))) => {
                                if !parser.broker_connected {
                                    set_active_client(&host, port, client.clone());
                                    if tx
                                        .send(LiveMessage::Connected {
                                            is_reconnect: had_disconnect,
                                        })
                                        .await
                                        .is_err()
                                    {
                                        let _ = client.disconnect().await;
                                        clear_active_client(&host, port);
                                        return;
                                    }
                                    had_disconnect = false;
                                    let evs = parser.set_connected(true);
                                    if !emit_all(&tx, evs).await {
                                        let _ = client.disconnect().await;
                                        clear_active_client(&host, port);
                                        return;
                                    }
                                }
                            }
                            Ok(Event::Incoming(Packet::Publish(p))) => {
                                let payload = String::from_utf8_lossy(&p.payload);
                                let evs = parser.ingest(&p.topic, &payload, now_sec());
                                if !emit_all(&tx, evs).await {
                                    let _ = client.disconnect().await;
                                    clear_active_client(&host, port);
                                    return;
                                }
                            }
                            Ok(_) => {}
                            Err(_) => {
                                clear_active_client(&host, port);
                                had_disconnect = true;
                                if tx.send(LiveMessage::Disconnected).await.is_err() {
                                    return;
                                }
                                let evs = parser.set_connected(false);
                                if !emit_all(&tx, evs).await {
                                    return;
                                }
                                break;
                            }
                        }
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    }

    pub async fn publish_direct_cmd(host: &str, port: u16, cmd: &str) -> Result<(), String> {
        if let Some(client) = get_active_client(host, port) {
            if let Ok(Ok(())) = tokio::time::timeout(
                Duration::from_secs(5),
                client.publish(topics::T_CMD, QoS::AtLeastOnce, false, cmd.as_bytes()),
            )
            .await
            {
                return Ok(());
            }
        }

        let opts = make_opts("karhutla-ui-cmd", host, port);
        let (client, mut eventloop) = AsyncClient::new(opts, 16);
        let fut = async move {
            let mut published = false;
            loop {
                match eventloop.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        if !published {
                            client
                                .publish(topics::T_CMD, QoS::AtLeastOnce, false, cmd.as_bytes())
                                .await
                                .map_err(|e| format!("Gagal kirim perintah MQTT: {e}"))?;
                            published = true;
                        }
                    }
                    Ok(Event::Incoming(Packet::PubAck(_))) => {
                        let _ = client.disconnect().await;
                        return Ok(());
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return Err(format!("Broker MQTT {host}:{port} tidak merespons ({e})"));
                    }
                }
            }
        };

        tokio::time::timeout(Duration::from_secs(5), fut)
            .await
            .map_err(|_| format!("Batas waktu koneksi MQTT ke {host}:{port} habis (5 dtk)."))?
    }
}

#[cfg(any(feature = "desktop", feature = "mobile"))]
pub use native_mqtt::{publish_direct_cmd, run_direct_mqtt_client};

// =========================================================================
// Unit & Live Local Mosquitto Roundtrip Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_mqtt_parser_telemetry_heartbeat_and_stale() {
        let mut p = DirectMqttParser::new("192.168.4.1", 6669);
        let conn_evs = p.set_connected(true);
        assert!(matches!(
            conn_evs.first(),
            Some(ServerEvent::Status {
                broker_connected: true,
                ..
            })
        ));
        assert!(p.build_sim_status().can_send_cmd);

        let hb = r#"{"mode":"RIIL","rtc_ok":true,"epoch_s":1764200000,"log_count":12,"log_cap":43008,"log_full":false}"#;
        let hb_evs = p.ingest(topics::T_HEARTBEAT, hb, 100.0);
        assert!(hb_evs.iter().any(|e| matches!(
            e,
            ServerEvent::EspStatus {
                online: true,
                hw_mode: Some(m),
                ..
            } if m == "RIIL"
        )));
        assert_eq!(p.log_status.log_count, Some(12));

        let fc = r#"{"pm_forecast":88.5,"slope":0.42,"eta_bahaya_s":36.0,"mae":1.8}"#;
        p.ingest(topics::T_FORECAST, fc, 101.0);

        let tel = r#"{"seq":7,"pm":62.4,"pm_raw":64.1,"gas_ppm":495.0,"g":0.22,"fi":71.2,"state":"WASPADA","mode":"AUTO","duty":40.0}"#;
        let tel_evs = p.ingest(topics::T_TELEM, tel, 102.0);
        let got_row = tel_evs.iter().find_map(|e| match e {
            ServerEvent::Telemetry { source, row, .. } => Some((source.clone(), row.clone())),
            _ => None,
        });
        let (src, row) = got_row.expect("telemetry event expected");
        assert_eq!(src, "esp32");
        assert_eq!(row.seq, 7);
        assert_eq!(row.state, "WASPADA");
        assert_eq!(row.forecast, Some(88.5));

        assert!(p.check_stale(110.0, 12.0).is_none());
        let stale = p.check_stale(120.0, 12.0);
        assert!(matches!(
            stale,
            Some(ServerEvent::EspStatus { online: false, .. })
        ));
        assert!(!p.esp_online);
    }

    #[test]
    fn test_rows_to_csv_bytes_formatting() {
        let mut r = TelemetryRow::idle();
        r.t_s = 4.0;
        r.seq = 2;
        r.source = Some("esp32".to_string());
        r.scenario = "flaming".to_string();
        r.pm = 55.25;
        r.fi = 64.5;
        r.state = "WASPADA".to_string();
        r.mode = "AUTO".to_string();
        r.duty = 35.0;
        r.forecast = Some(72.0);

        let csv_str = String::from_utf8(rows_to_csv_bytes(&[r])).expect("utf8 csv");
        let lines: Vec<&str> = csv_str.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("t_s,seq,source,scenario,pm_true"));
        assert!(lines[1].starts_with("4,2,esp32,flaming,"));
        assert!(lines[1].contains(",WASPADA,AUTO,35,72,"));
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn test_direct_mqtt_live_mosquitto_roundtrip() {
        use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
        use std::net::{TcpListener, TcpStream};
        use std::process::{Child, Command, Stdio};
        use std::time::{Duration, Instant};

        struct MosquittoGuard(Child);
        impl Drop for MosquittoGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local_addr").port();
        drop(listener);

        let child = Command::new("/usr/sbin/mosquitto")
            .args(["-p", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn /usr/sbin/mosquitto");
        let _guard = MosquittoGuard(child);

        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        let rt = tokio::runtime::Runtime::new().expect("tokio rt");
        rt.block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            let client_task = tokio::spawn(run_direct_mqtt_client(
                "127.0.0.1".to_string(),
                port,
                tx,
            ));

            // Wait until direct client's ConnAck + SubAck is active before node publishes
            let conn_res = tokio::time::timeout(Duration::from_secs(5), async {
                while let Some(msg) = rx.recv().await {
                    if matches!(msg, crate::transport::LiveMessage::Connected { .. }) {
                        return true;
                    }
                }
                false
            })
            .await;
            assert_eq!(conn_res, Ok(true), "direct MQTT client did not connect + subscribe");

            // Mock ESP32 node subscribing to karhutla/cmd and publishing hb + telemetry after SubAck
            let mut node_opts = MqttOptions::new("mock-esp32-test", "127.0.0.1", port);
            node_opts.set_keep_alive(Duration::from_secs(15));
            let (node_client, mut node_loop) = AsyncClient::new(node_opts, 32);
            let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<String>(8);

            let node_task = tokio::spawn(async move {
                loop {
                    match node_loop.poll().await {
                        Ok(Event::Incoming(Packet::ConnAck(_))) => {
                            let _ = node_client.subscribe(topics::T_CMD, QoS::AtLeastOnce).await;
                        }
                        Ok(Event::Incoming(Packet::SubAck(_))) => {
                            let hb = r#"{"mode":"RIIL","rtc_ok":true,"epoch_s":1764200001,"log_count":5,"log_cap":43008,"log_full":false}"#;
                            let _ = node_client
                                .publish(topics::T_HEARTBEAT, QoS::AtLeastOnce, false, hb.as_bytes())
                                .await;
                            let tel = r#"{"seq":1,"pm":82.5,"pm_raw":84.0,"gas_ppm":510.0,"g":0.31,"fi":92.0,"state":"WASPADA","mode":"AUTO","duty":48.0}"#;
                            let _ = node_client
                                .publish(topics::T_TELEM, QoS::AtLeastOnce, false, tel.as_bytes())
                                .await;
                        }
                        Ok(Event::Incoming(Packet::Publish(p))) if p.topic == topics::T_CMD => {
                            let _ = cmd_tx
                                .send(String::from_utf8_lossy(&p.payload).to_string())
                                .await;
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            });

            let mut got_telemetry = false;
            let mut got_online = false;
            let ev_res = tokio::time::timeout(Duration::from_secs(5), async {
                while let Some(msg) = rx.recv().await {
                    match msg {
                        crate::transport::LiveMessage::Event(ServerEvent::EspStatus {
                            online: true,
                            ..
                        }) => got_online = true,
                        crate::transport::LiveMessage::Event(ServerEvent::Telemetry {
                            row, ..
                        }) if (row.fi - 92.0).abs() < 1e-6 => {
                            got_telemetry = true;
                        }
                        _ => {}
                    }
                    if got_online && got_telemetry {
                        break;
                    }
                }
            })
            .await;
            assert!(ev_res.is_ok(), "timed out waiting for direct MQTT events");

            publish_direct_cmd("127.0.0.1", port, "FORCE_MAX")
                .await
                .expect("publish_direct_cmd failed");

            let cmd_received = tokio::time::timeout(Duration::from_secs(5), cmd_rx.recv())
                .await
                .expect("timed out waiting for FORCE_MAX on karhutla/cmd")
                .expect("cmd channel closed");
            assert_eq!(cmd_received, "FORCE_MAX");

            node_task.abort();
            drop(rx);
            let _ = tokio::time::timeout(Duration::from_secs(2), client_task).await;
        });
    }
}
