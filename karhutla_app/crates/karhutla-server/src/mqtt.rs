//! MQTT client bridge (`rumqttc` TCP :6669) + local Mosquitto auto-starter.

use karhutla_protocol::topics;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde_json::{json, Value};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct MqttHandle {
    client: AsyncClient,
    client_id: String,
    broker_proc: Arc<Mutex<Option<Child>>>,
}

impl MqttHandle {
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub async fn publish(&self, topic: &str, payload: &str, qos: QoS, retain: bool) -> Result<(), String> {
        self.client
            .publish(topic, qos, retain, payload.as_bytes().to_vec())
            .await
            .map_err(|e| format!("{e}"))
    }

    pub async fn publish_telemetry_bundle(&self, tel: &Value) -> Result<(), String> {
        let mut with_src = tel.clone();
        if let Some(obj) = with_src.as_object_mut() {
            obj.insert("src".to_string(), Value::String("sitl".to_string()));
        }
        self.publish(topics::T_TELEM, &with_src.to_string(), QoS::AtMostOnce, false)
            .await?;

        let scalars: [(&str, usize); 10] = [
            ("pm", 2),
            ("pm_raw", 2),
            ("dcdt", 3),
            ("fi", 2),
            ("g", 3),
            ("gas_ppm", 1),
            ("v_dust", 3),
            ("v_mq", 3),
            ("v0", 3),
            ("duty", 1),
        ];
        for (k, prec) in scalars {
            if let Some(v) = tel.get(k).and_then(Value::as_f64) {
                let s = format!("{v:.prec$}");
                let _ = self
                    .publish(&format!("{}/{k}", topics::T_TELEM_SCALAR), &s, QoS::AtMostOnce, false)
                    .await;
            }
        }
        for k in ["state", "mode"] {
            if let Some(s) = tel.get(k).and_then(Value::as_str) {
                let _ = self
                    .publish(&format!("{}/{k}", topics::T_TELEM_SCALAR), s, QoS::AtMostOnce, false)
                    .await;
            }
        }
        if let Some(seq) = tel.get("seq").and_then(Value::as_u64) {
            let _ = self
                .publish(
                    &format!("{}/seq", topics::T_TELEM_SCALAR),
                    &seq.to_string(),
                    QoS::AtMostOnce,
                    false,
                )
                .await;
        }
        Ok(())
    }

    pub async fn publish_forecast_bundle(
        &self,
        pm_forecast: f64,
        slope: f64,
        eta_bahaya_s: Option<f64>,
        mae: Option<f64>,
    ) {
        let payload = json!({
            "pm_forecast": pm_forecast,
            "slope": slope,
            "eta_bahaya_s": eta_bahaya_s,
            "mae": mae,
        });
        let _ = self
            .publish(topics::T_FORECAST, &payload.to_string(), QoS::AtMostOnce, false)
            .await;
        let _ = self
            .publish(
                &format!("{}/pm_forecast", topics::T_FORECAST_SCALAR),
                &format!("{pm_forecast:.1}"),
                QoS::AtMostOnce,
                false,
            )
            .await;
        let eta_str = eta_bahaya_s
            .map(|v| format!("{v:.0}"))
            .unwrap_or_else(|| "-".to_string());
        let _ = self
            .publish(
                &format!("{}/eta_bahaya_s", topics::T_FORECAST_SCALAR),
                &eta_str,
                QoS::AtMostOnce,
                false,
            )
            .await;
        let _ = self
            .publish(
                &format!("{}/slope", topics::T_FORECAST_SCALAR),
                &format!("{slope:.3}"),
                QoS::AtMostOnce,
                false,
            )
            .await;
        if let Some(m) = mae {
            let _ = self
                .publish(
                    &format!("{}/mae", topics::T_FORECAST_SCALAR),
                    &format!("{m:.2}"),
                    QoS::AtMostOnce,
                    false,
                )
                .await;
        }
    }

    pub fn stop_spawned_broker(&self) {
        if let Ok(mut guard) = self.broker_proc.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

pub fn is_port_open(host: &str, port: u16) -> bool {
    let check_host = match host {
        "" | "localhost" | "0.0.0.0" => "127.0.0.1",
        other => other,
    };
    let addr: SocketAddr = match format!("{check_host}:{port}").parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(350)).is_ok()
}

pub fn ensure_local_broker(host: &str, port: u16) -> Option<Child> {
    if is_port_open(host, port) {
        return None;
    }
    let check_host = match host {
        "" | "localhost" | "0.0.0.0" => "127.0.0.1",
        other => other,
    };
    if check_host != "127.0.0.1" && check_host != "::1" {
        return None;
    }
    let conf_candidates = ["mosquitto.conf", "../mosquitto.conf"];
    let conf = conf_candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .copied();
    let mut cmd = Command::new("mosquitto");
    if let Some(c) = conf {
        cmd.arg("-c").arg(c);
    } else {
        cmd.arg("-p").arg(port.to_string());
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    for _ in 0..30 {
        if is_port_open(check_host, port) {
            return Some(child);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    None
}

pub fn spawn_mqtt_bridge<F, C>(
    broker_host: String,
    broker_port: u16,
    on_connect: C,
    on_message: F,
) -> MqttHandle
where
    C: Fn(bool) + Send + Sync + 'static,
    F: Fn(String, String) + Send + Sync + 'static,
{
    let child = ensure_local_broker(&broker_host, broker_port);
    let broker_proc = Arc::new(Mutex::new(child));
    let suffix = chrono::Local::now().timestamp_millis() % 100_000;
    let client_id = format!("karhutla-rust-{suffix}");
    let connect_host = if broker_host.is_empty() || broker_host == "0.0.0.0" {
        "127.0.0.1".to_string()
    } else {
        broker_host
    };
    let mut opts = MqttOptions::new(client_id.clone(), connect_host, broker_port);
    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_max_packet_size(256 * 1024, 256 * 1024);

    let (client, mut eventloop) = AsyncClient::new(opts, 256);
    let sub_client = client.clone();
    tokio::spawn(async move {
        let mut connected = false;
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                    connected = true;
                    on_connect(true);
                    let _ = sub_client.subscribe("karhutla/#", QoS::AtMostOnce).await;
                }
                Ok(Event::Incoming(Incoming::Publish(p))) => {
                    let payload = String::from_utf8_lossy(&p.payload).trim().to_string();
                    on_message(p.topic, payload);
                }
                Ok(_) => {}
                Err(_) => {
                    if connected {
                        connected = false;
                        on_connect(false);
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    });

    MqttHandle {
        client,
        client_id,
        broker_proc,
    }
}
