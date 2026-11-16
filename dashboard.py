#!/usr/bin/env python3
"""Dashboard Web Modern Karhutla Edge-IoT (Laptop Command & Telemetry Center).

Fitur:
- Terhubung ke broker MQTT (default localhost:6669, auto-start Mosquitto bila belum jalan)
- Mencatat trafik MQTT otomatis ke logs/mqtt_YYYY-MM-DD.log
- Grafik Multi-Overlay interaktif (FI, PM Kalman, PM Plant, PM Raw, AI Forecast t+60s, Gas PPM/g, Tegangan)
- Panel Kontrol Aktuator (AUTO, FORCE_MAX, FORCE_OFF, RESET ke karhutla/cmd)
- Embedded SITL Runner (bisa menjalankan/pause/ubah skenario/kecepatan/injeksi fault langsung dari browser,
  atau memantau sitl.py / hils.py / field_node.py yang berjalan terpisah)

Contoh:
  python dashboard.py                                # buka dashboard di http://localhost:8080
  python dashboard.py --auto-sim flaming --speed 1   # langsung mulai simulasi flaming
"""
import argparse
from collections import deque
import datetime
import http.server
import json
import math
import os
import socket
import socketserver
import subprocess
import threading
import time
import urllib.parse

import config as K
from core import Firmware
from forecast import LinearTrendForecaster, PreemptController
from logger import DailyCsvLogger, DailyMqttLogger
import mqtt_io
from plant import SCENARIOS, SmokeChamber


def _clean_val(v):
    if isinstance(v, float) and (math.isnan(v) or math.isinf(v)):
        return None
    return v


def _clean_dict(d):
    return {k: _clean_val(v) for k, v in d.items()}


def detect_lan_ip():
    """Deteksi IP LAN laptop agar mudah diakses dari HP."""
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.connect(("8.8.8.8", 80))
            ip = s.getsockname()[0]
            if ip and not ip.startswith("127."):
                return ip
    except OSError:
        pass
    return "192.168.1.10"


def build_mobile_dashboard_config(broker_host=None, broker_port=None):
    """Hasilkan JSON konfigurasi siap-import untuk aplikasi HP (IoT MQTT Panel)."""
    ip = broker_host if (broker_host and broker_host not in ("localhost", "127.0.0.1", "0.0.0.0")) else detect_lan_ip()
    port = int(broker_port or K.BROKER_PORT)
    return {
        "app": "IoT MQTT Panel",
        "version": 2,
        "generated_at": datetime.datetime.now().isoformat(timespec="seconds"),
        "web_dashboard_lan_url": f"http://{ip}:8080",
        "connections": [
            {
                "connectionName": "Karhutla Edge-IoT",
                "clientId": "hp-dashboard-karhutla",
                "host": ip,
                "port": port,
                "protocol": "tcp",
                "autoReconnect": True,
                "cleanSession": True,
                "keepAlive": 60,
                "username": K.MQTT_USER or "",
                "password": K.MQTT_PASS or "",
                "dashboards": [
                    {
                        "dashboardName": "Monitor Karhutla",
                        "panels": [
                            {
                                "panelName": "Status Koneksi ESP32 / SITL",
                                "type": "led_status",
                                "topic": K.T_STATUS,
                                "payloadOnline": "online",
                                "payloadOffline": "offline",
                                "qos": 0,
                            },
                            {
                                "panelName": "Gauge Fusion Index (FI)",
                                "type": "gauge",
                                "topic": f"{K.T_TELEM_SCALAR}/fi",
                                "min": 0,
                                "max": 250,
                                "unit": "ug/m3",
                                "qos": 0,
                            },
                            {
                                "panelName": "Gauge PM-proxy Kalman",
                                "type": "gauge",
                                "topic": f"{K.T_TELEM_SCALAR}/pm",
                                "min": 0,
                                "max": 300,
                                "unit": "ug/m3",
                                "qos": 0,
                            },
                            {
                                "panelName": "Kecepatan Kipas (PWM)",
                                "type": "progress",
                                "topic": f"{K.T_TELEM_SCALAR}/duty",
                                "min": 0,
                                "max": 100,
                                "unit": "%",
                                "qos": 0,
                            },
                            {
                                "panelName": "Status Bahaya (FSM)",
                                "type": "text_log",
                                "topic": f"{K.T_TELEM_SCALAR}/state",
                                "qos": 0,
                            },
                            {
                                "panelName": "Mode Operasi",
                                "type": "text_log",
                                "topic": f"{K.T_TELEM_SCALAR}/mode",
                                "qos": 0,
                            },
                            {
                                "panelName": "Gas MQ-135 (ppm)",
                                "type": "gauge",
                                "topic": f"{K.T_TELEM_SCALAR}/gas_ppm",
                                "min": 300,
                                "max": 1500,
                                "unit": "ppm",
                                "qos": 0,
                            },
                            {
                                "panelName": "Skor Gas (g 0-1)",
                                "type": "text",
                                "topic": f"{K.T_TELEM_SCALAR}/g",
                                "qos": 0,
                            },
                            {
                                "panelName": "Prakiraan AI (t+60s)",
                                "type": "gauge",
                                "topic": f"{K.T_FORECAST_SCALAR}/pm_forecast",
                                "min": 0,
                                "max": 300,
                                "unit": "ug/m3",
                                "qos": 0,
                            },
                            {
                                "panelName": "ETA Menuju BAHAYA",
                                "type": "text",
                                "topic": f"{K.T_FORECAST_SCALAR}/eta_bahaya_s",
                                "unit": "s",
                                "qos": 0,
                            },
                            {
                                "panelName": "Grafik Tren PM-proxy vs Forecast",
                                "type": "line_graph",
                                "topic": f"{K.T_TELEM_SCALAR}/pm",
                                "topic2": f"{K.T_FORECAST_SCALAR}/pm_forecast",
                                "unit": "ug/m3",
                                "qos": 0,
                            },
                            {
                                "panelName": "Mode AUTO (FSM)",
                                "type": "button",
                                "topic": K.T_CMD,
                                "payload": "AUTO",
                                "qos": 0,
                            },
                            {
                                "panelName": "KIPAS 100% (FORCE MAX)",
                                "type": "button",
                                "topic": K.T_CMD,
                                "payload": "FORCE_MAX",
                                "qos": 0,
                            },
                            {
                                "panelName": "MATIKAN KIPAS (FORCE OFF)",
                                "type": "button",
                                "topic": K.T_CMD,
                                "payload": "FORCE_OFF",
                                "qos": 0,
                            },
                            {
                                "panelName": "RESET SISTEM",
                                "type": "button",
                                "topic": K.T_CMD,
                                "payload": "RESET",
                                "qos": 0,
                            },
                        ],
                    }
                ],
            }
        ],
    }


class DashboardEngine:
    """Pengelola state real-time, jembatan MQTT, dan embedded SITL runner."""

    def __init__(self, broker_host=None, broker_port=None, out_dir="logs", enable_mqtt=True,
                 esp_stale_s=5.0):
        self.broker_host = broker_host or K.BROKER_HOST
        self.broker_port = int(broker_port or K.BROKER_PORT)
        self.out_dir = out_dir
        self.enable_mqtt = enable_mqtt
        self.esp_stale_s = float(esp_stale_s)

        # Aliran data ESP32 (hardware) dipisah dari aliran SITL agar tidak saling menimpa
        self.esp_history = deque(maxlen=1800)
        self.esp_latest = None
        self.esp_last_rx = None          # waktu dinding terakhir menerima telemetri/heartbeat ESP32
        self.esp_online = False
        self.esp_hw_mode = None          # "HILS" / "RIIL" dari heartbeat firmware
        self._esp_fc = LinearTrendForecaster()
        self._watchdog_stop = threading.Event()
        self._watchdog_thread = None

        self.lock = threading.Lock()
        self.history = deque(maxlen=1800)
        self.mqtt_logs = deque(maxlen=300)
        self.latest = {
            "t": 0.0,
            "seq": 0,
            "scenario": "idle",
            "pm_true": None,
            "gas_true": None,
            "v_dust": K.DUST_V0,
            "v_mq": 1.0,
            "pm_raw": 0.0,
            "pm": 0.0,
            "dcdt": 0.0,
            "gas_ppm": K.MQ_ATM_PPM,
            "g": 0.0,
            "fi": 0.0,
            "v0": K.DUST_V0,
            "state": "NORMAL",
            "mode": "AUTO",
            "duty": 0.0,
            "forecast": 0.0,
            "slope": 0.0,
            "eta_bahaya_s": None,
            "mae": None,
            "sensor_fault": False,
        }
        self.broker_connected = False
        self.node_status = "offline"

        # Embedded SITL state
        self.sim_running = False
        self.sim_paused = False
        self.sim_scenario = "flaming"
        self.sim_speed = 1.0
        self.sim_duration = 900.0
        self.sim_ai_preempt = False
        self.sim_fault_inject = False
        self.sim_step_idx = 0
        self.sim_plant = None
        self.sim_fw = None
        self.sim_fc = None
        self.sim_pre = None
        self.sim_csv = None
        self._sim_stop_evt = threading.Event()
        self._sim_thread = None

        # SSE subscribers
        self._sse_clients = []
        self._sse_lock = threading.Lock()

        # Logger & MQTT client
        self.mqtt_logger = DailyMqttLogger(folder=out_dir, prefix="mqtt")
        self.client = None
        self._external_fc = LinearTrendForecaster()
        self._t0_wall = time.time()

        if self.enable_mqtt:
            self._init_mqtt()
        self._watchdog_thread = threading.Thread(target=self._watchdog_loop, daemon=True)
        self._watchdog_thread.start()

    def _record_mqtt_log(self, line):
        with self.lock:
            self.mqtt_logs.append(line)
        self._broadcast({"type": "mqtt_log", "line": line})

    def _init_mqtt(self):
        orig_log_event = self.mqtt_logger.log_event

        def hooked_log_event(client_id, event, topic="", payload=""):
            line = orig_log_event(client_id, event, topic, payload)
            self._record_mqtt_log(line)
            return line

        self.mqtt_logger.log_event = hooked_log_event

        def on_conn(c, u, flags, rc, props=None):
            with self.lock:
                self.broker_connected = (rc == 0)
            c.subscribe("karhutla/#")
            self._broadcast({"type": "status", "broker_connected": self.broker_connected, "node_status": self.node_status})

        def on_msg(c, u, msg):
            topic = msg.topic
            payload = msg.payload.decode("utf-8", errors="ignore").strip()
            if topic == K.T_STATUS:
                with self.lock:
                    self.node_status = payload or "offline"
                self._broadcast({"type": "status", "broker_connected": self.broker_connected, "node_status": self.node_status})
            elif topic == K.T_CMD:
                if self.sim_running and self.sim_fw is not None:
                    with self.lock:
                        tel_now = self.sim_fw.apply_command(payload)
                        if tel_now is not None:
                            self.latest["mode"] = tel_now["mode"]
                            self.latest["duty"] = tel_now["duty"]
                            self.latest["state"] = tel_now["state"]
                            if self.history:
                                self.history[-1] = dict(self.latest)
                            snap = dict(self.latest)
                        else:
                            snap = None
                    if snap is not None:
                        self._broadcast({"type": "telemetry", "source": "sitl", "row": snap,
                                         "sim": self.get_sim_status()})
            elif topic == K.T_HEARTBEAT:
                try:
                    hb = json.loads(payload)
                except ValueError:
                    hb = {}
                self._esp_alive(hb.get("mode") if isinstance(hb, dict) else None)
            elif topic == K.T_TELEM:
                try:
                    tel = json.loads(payload)
                except ValueError:
                    return
                if not isinstance(tel, dict):
                    return
                if tel.get("src") == "sitl":
                    # Gema dari runner SITL milik dashboard sendiri sudah dicatat langsung; abaikan.
                    # Bila SITL berasal dari proses lain (sitl.py), tampilkan sebagai aliran SITL.
                    if not self.sim_running:
                        self._ingest_external(tel, "sitl")
                else:
                    self._ingest_external(tel, "esp32")

        self.client = mqtt_io.make_client(
            f"karhutla-dashboard-{int(time.time() * 1000) % 100000}",
            on_connect=on_conn,
            on_message=on_msg,
            publish_status=False,
            mqtt_logger=self.mqtt_logger,
        )
        try:
            mqtt_io.connect(self.client, self.broker_host, self.broker_port, auto_start_broker=True)
            with self.lock:
                self.broker_connected = True
        except Exception as exc:
            self._record_mqtt_log(f"{datetime.datetime.now().isoformat(timespec='milliseconds')} [dashboard] ERROR connect failed: {exc}")

    # ---------- Aliran ESP32 (hardware) ----------
    def _esp_alive(self, hw_mode=None):
        """Tandai ESP32 hidup (dipanggil oleh telemetri maupun heartbeat)."""
        changed = False
        with self.lock:
            self.esp_last_rx = time.time()
            if hw_mode:
                self.esp_hw_mode = hw_mode
            if not self.esp_online:
                self.esp_online = True
                changed = True
        if changed:
            self._broadcast({"type": "esp_status", "online": True, "hw_mode": self.esp_hw_mode, "age_s": 0.0})

    def _watchdog_loop(self):
        """Deteksi ESP32 dicabut: tidak ada telemetri/heartbeat > esp_stale_s."""
        while not self._watchdog_stop.wait(0.25):
            went_offline = False
            with self.lock:
                if self.esp_online and self.esp_last_rx is not None \
                        and time.time() - self.esp_last_rx > self.esp_stale_s:
                    self.esp_online = False
                    went_offline = True
                age = None if self.esp_last_rx is None else time.time() - self.esp_last_rx
            if went_offline:
                self._broadcast({"type": "esp_status", "online": False, "hw_mode": self.esp_hw_mode,
                                 "age_s": age})

    def _ingest_external(self, tel, target):
        """Masukkan telemetri dari MQTT ke aliran `target` ('esp32' atau 'sitl')."""
        is_esp = target == "esp32"
        hist = self.esp_history if is_esp else self.history
        t_rel = round(time.time() - self._t0_wall, 1)
        seq_val = int(tel.get("seq", 0))
        t_val = float(tel.get("seq", t_rel))
        did_reset_history = False
        with self.lock:
            if hist and t_val <= hist[-1].get("t", -1.0):
                # Skenario/run baru dimulai dari seq=0 atau re-publish pada detik yang sama
                if seq_val == 0 or t_val < hist[-1].get("t", -1.0):
                    hist.clear()
                    if is_esp:
                        self._esp_fc = LinearTrendForecaster()
                    else:
                        self._external_fc = LinearTrendForecaster()
                    did_reset_history = True
            fc = self._esp_fc if is_esp else self._external_fc
        f = fc.update(t_val, float(tel.get("pm", 0.0)))
        row = _clean_dict({
            "t": t_val,
            "seq": seq_val,
            "scenario": "esp32-live" if is_esp else "mqtt-live",
            "pm_true": None,
            "gas_true": None,
            "v_dust": float(tel.get("v_dust", K.DUST_V0)),
            "v_mq": float(tel.get("v_mq", 1.0)),
            "pm_raw": float(tel.get("pm_raw", 0.0)),
            "pm": float(tel.get("pm", 0.0)),
            "dcdt": float(tel.get("dcdt", 0.0)),
            "gas_ppm": float(tel.get("gas_ppm", K.MQ_ATM_PPM)),
            "g": float(tel.get("g", 0.0)),
            "fi": float(tel.get("fi", 0.0)),
            "v0": float(tel.get("v0", K.DUST_V0)),
            "state": str(tel.get("state", "NORMAL")),
            "mode": str(tel.get("mode", "AUTO")),
            "duty": float(tel.get("duty", 0.0)),
            "forecast": _clean_val(f.get("pm_forecast", 0.0)),
            "slope": _clean_val(f.get("slope", 0.0)),
            "eta_bahaya_s": _clean_val(f.get("eta_bahaya_s")),
            "mae": _clean_val(f.get("mae")),
            "sensor_fault": str(tel.get("state", "")) == "FAILSAFE",
        })
        with self.lock:
            if is_esp:
                self.esp_latest = row
            else:
                self.latest = row
            if hist and hist[-1].get("t") == t_val:
                hist[-1] = row
            else:
                hist.append(row)
            if not is_esp and self.node_status == "offline":
                self.node_status = "online"
        if is_esp:
            self._esp_alive()
        if did_reset_history:
            self._broadcast({"type": "clear", "source": target})
        self._broadcast({"type": "telemetry", "source": target, "row": row, "sim": self.get_sim_status()})

    def _broadcast(self, payload):
        msg = f"data: {json.dumps(payload)}\n\n".encode("utf-8")
        with self._sse_lock:
            dead = []
            for q in self._sse_clients:
                try:
                    q.append(msg)
                except Exception:
                    dead.append(q)
            for d in dead:
                if d in self._sse_clients:
                    self._sse_clients.remove(d)

    def register_sse(self):
        q = deque(maxlen=250)
        with self._sse_lock:
            self._sse_clients.append(q)
        return q

    def unregister_sse(self, q):
        with self._sse_lock:
            if q in self._sse_clients:
                self._sse_clients.remove(q)

    def get_sim_status(self):
        with self.lock:
            return {
                "running": self.sim_running,
                "paused": self.sim_paused,
                "scenario": self.sim_scenario,
                "speed": self.sim_speed,
                "duration": self.sim_duration,
                "ai_preempt": self.sim_ai_preempt,
                "fault_inject": self.sim_fault_inject,
                "step": self.sim_step_idx,
                "broker_host": self.broker_host,
                "broker_port": self.broker_port,
                "broker_connected": self.broker_connected,
                "node_status": self.node_status,
                "log_file": self.mqtt_logger.path or f"{self.out_dir}/mqtt_{datetime.date.today().isoformat()}.log",
            }

    def get_esp_status(self):
        with self.lock:
            age = None if self.esp_last_rx is None else round(time.time() - self.esp_last_rx, 1)
            return {"online": self.esp_online, "age_s": age, "hw_mode": self.esp_hw_mode,
                    "stale_s": self.esp_stale_s}

    def get_state_snapshot(self):
        with self.lock:
            sitl_hist = list(self.history)
            esp_hist = list(self.esp_history)
            logs = list(self.mqtt_logs)
            sitl_latest = dict(self.latest)
            esp_latest = dict(self.esp_latest) if self.esp_latest else None
            esp_online = self.esp_online
            sim_running = self.sim_running
        # Tampilan bawaan (kompatibilitas): ESP32 bila hidup/ada data & SITL tidak jalan, selain itu SITL
        use_esp = esp_online or (esp_hist and not sim_running)
        return {
            "latest": (esp_latest if use_esp and esp_latest else sitl_latest),
            "history": (esp_hist if use_esp else sitl_hist),
            "sources": {
                "sitl": {"history": sitl_hist, "latest": sitl_latest, "online": sim_running},
                "esp32": {"history": esp_hist, "latest": esp_latest, "online": esp_online},
            },
            "esp": self.get_esp_status(),
            "mqtt_logs": logs,
            "sim": self.get_sim_status(),
            "thresholds": {"waspada": K.T_W_IN, "bahaya": K.T_B_IN, "horizon_s": K.FORECAST_HORIZON_S},
        }

    def send_cmd(self, cmd):
        clean_cmd = (cmd or "").strip().upper()
        if clean_cmd not in ("AUTO", "FORCE_MAX", "FORCE_OFF", "RESET"):
            return False
        snap = None
        did_clear = False
        with self.lock:
            if self.sim_running and self.sim_fw is not None:
                tel_now = self.sim_fw.apply_command(clean_cmd)
                if tel_now is not None:
                    if clean_cmd == "RESET":
                        self.history.clear()
                        self.sim_step_idx = 0
                        self.sim_fc = LinearTrendForecaster()
                        self.latest["t"] = 0.0
                        self.latest["seq"] = 0
                        did_clear = True
                    self.latest["mode"] = tel_now["mode"]
                    self.latest["duty"] = tel_now["duty"]
                    self.latest["state"] = tel_now["state"]
                    self.latest["pm"] = tel_now["pm"]
                    self.latest["fi"] = tel_now["fi"]
                    if self.history and not did_clear:
                        self.history[-1] = dict(self.latest)
                    snap = dict(self.latest)
                else:
                    tel_now = None
            else:
                tel_now = None
        if self.client:
            try:
                self.client.publish(K.T_CMD, clean_cmd)
                if tel_now is not None:
                    mqtt_io.publish_telemetry(self.client, tel_now)
            except Exception:
                pass
        if did_clear:
            self._broadcast({"type": "clear", "source": "sitl"})
        if snap is not None:
            self._broadcast({"type": "telemetry", "source": "sitl", "row": snap, "sim": self.get_sim_status()})
        return True

    def control_sim(self, params):
        action = (params.get("action") or "").strip().lower()
        if "scenario" in params and params["scenario"] in SCENARIOS:
            with self.lock:
                self.sim_scenario = params["scenario"]
        if "speed" in params:
            try:
                sp = float(params["speed"])
                if sp > 0:
                    with self.lock:
                        self.sim_speed = sp
            except (TypeError, ValueError):
                pass
        if "ai_preempt" in params:
            with self.lock:
                self.sim_ai_preempt = bool(params["ai_preempt"])
                if self.sim_pre is None and self.sim_ai_preempt:
                    self.sim_pre = PreemptController()
        if "fault_inject" in params:
            with self.lock:
                self.sim_fault_inject = bool(params["fault_inject"])

        if action == "start":
            self.stop_sim()
            self._start_sim_thread()
        elif action == "stop":
            self.stop_sim()
        elif action == "pause":
            with self.lock:
                self.sim_paused = True
        elif action == "resume":
            with self.lock:
                self.sim_paused = False
        elif action == "clear":
            src = params.get("source", "sitl")
            with self.lock:
                if src == "esp32":
                    self.esp_history.clear()
                    self._esp_fc = LinearTrendForecaster()
                else:
                    self.history.clear()
            self._broadcast({"type": "clear", "source": "esp32" if src == "esp32" else "sitl"})
        self._broadcast({"type": "sim", "sim": self.get_sim_status()})
        return True

    def _start_sim_thread(self):
        with self.lock:
            self.history.clear()
            self.sim_plant = SmokeChamber(self.sim_scenario, seed=1)
            self.sim_fw = Firmware()
            self.sim_fc = LinearTrendForecaster()
            self.sim_pre = PreemptController() if self.sim_ai_preempt else None
            self.sim_csv = DailyCsvLogger(self.out_dir, source="sitl")
            self.sim_step_idx = 0
            self.sim_running = True
            self.sim_paused = False
            self.node_status = "online"
            self._sim_stop_evt.clear()

        self._broadcast({"type": "clear", "source": "sitl"})

        if self.client:
            try:
                self.client.publish(K.T_STATUS, "online", retain=True)
            except Exception:
                pass

        self._sim_thread = threading.Thread(target=self._sim_loop, daemon=True)
        self._sim_thread.start()

    def _sim_loop(self):
        total_steps = int(self.sim_duration / K.TS)
        while not self._sim_stop_evt.is_set():
            with self.lock:
                if not self.sim_running or self.sim_step_idx >= total_steps:
                    break
                if self.sim_paused:
                    paused = True
                else:
                    paused = False
                    k = self.sim_step_idx
                    self.sim_step_idx += 1
                    t = k * K.TS
                    vd, vm = self.sim_plant.sensors(t)
                    fault_now = self.sim_fault_inject or self.sim_plant.dust_disconnected(t)
                    if self.sim_fault_inject:
                        vd = 0.0
                    tel = self.sim_fw.step(vd, vm, seq=k)
                    f = self.sim_fc.update(t, tel["pm"])
                    if self.sim_ai_preempt and self.sim_pre is not None:
                        cmd = self.sim_pre.decide(t, self.sim_fw.mode, f["pm_forecast"])
                        if cmd:
                            self.sim_fw.apply_command(cmd)
                            tel["mode"] = self.sim_fw.mode
                            tel["duty"] = self.sim_fw.duty
                    row = _clean_dict({
                        "t": t,
                        "t_s": t,
                        "seq": k,
                        "scenario": self.sim_scenario,
                        "pm_true": self.sim_plant.C,
                        "gas_true": self.sim_plant.gas_ppm,
                        "v_dust": vd,
                        "v_mq": vm,
                        "pm_raw": tel["pm_raw"],
                        "pm": tel["pm"],
                        "dcdt": tel["dcdt"],
                        "gas_ppm": tel["gas_ppm"],
                        "g": tel["g"],
                        "fi": tel["fi"],
                        "v0": tel["v0"],
                        "state": tel["state"],
                        "mode": tel["mode"],
                        "duty": tel["duty"],
                        "forecast": f["pm_forecast"],
                        "slope": f["slope"],
                        "eta_bahaya_s": f["eta_bahaya_s"],
                        "mae": f["mae"],
                        "sensor_fault": fault_now,
                    })
                    self.sim_plant.advance(tel["duty"] / 100.0, t)
                    self.latest = row
                    self.history.append(row)
                    if self.sim_csv:
                        self.sim_csv.write(row)
                    speed = self.sim_speed

            if paused:
                time.sleep(0.1)
                continue

            if self.client:
                try:
                    mqtt_io.publish_telemetry(self.client, tel)
                    mqtt_io.publish_forecast(self.client, f)
                except Exception:
                    pass

            self._broadcast({"type": "telemetry", "source": "sitl", "row": row, "sim": self.get_sim_status()})
            wait_s = max(0.005, K.TS / max(0.1, speed))
            self._sim_stop_evt.wait(wait_s)

        with self.lock:
            self.sim_running = False
            if self.sim_csv:
                self.sim_csv.close()
                self.sim_csv = None
        self._broadcast({"type": "sim", "sim": self.get_sim_status()})

    def stop_sim(self):
        self._sim_stop_evt.set()
        t = self._sim_thread
        if t and t.is_alive():
            t.join(timeout=1.5)
        with self.lock:
            self.sim_running = False
            self.sim_paused = False
            if self.sim_csv:
                self.sim_csv.close()
                self.sim_csv = None

    def shutdown(self):
        self._watchdog_stop.set()
        self.stop_sim()
        if self.client:
            mqtt_io.disconnect(self.client)
            self.client = None
        if self.mqtt_logger:
            self.mqtt_logger.close()


RENDERED_HTML = """<!DOCTYPE html>
<html lang="id">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="light dark">
  <title>Karhutla Edge-IoT — Telemetry &amp; Command Center</title>
  <style>
    :root {
      color-scheme: light dark;

      --bg-canvas-light: #f1f5f9;
      --bg-canvas-dark: #090d16;
      --bg-surface-light: #ffffff;
      --bg-surface-dark: #111827;
      --bg-elevated-light: #f8fafc;
      --bg-elevated-dark: #1e293b;
      --text-main-light: #0f172a;
      --text-main-dark: #f8fafc;
      --text-muted-light: #475569;
      --text-muted-dark: #94a3b8;
      --border-subtle-light: #cbd5e1;
      --border-subtle-dark: #334155;

      --accent-light: #0284c7;
      --accent-dark: #38bdf8;
      --ok-light: #15803d;
      --ok-dark: #4ade80;
      --warn-light: #b45309;
      --warn-dark: #fbbf24;
      --danger-light: #b91c1c;
      --danger-dark: #f87171;
      --violet-light: #6d28d9;
      --violet-dark: #c084fc;

      --bg-canvas: var(--bg-canvas-light);
      --bg-surface: var(--bg-surface-light);
      --bg-elevated: var(--bg-elevated-light);
      --text-main: var(--text-main-light);
      --text-muted: var(--text-muted-light);
      --border-subtle: var(--border-subtle-light);
      --accent: var(--accent-light);
      --ok: var(--ok-light);
      --warn: var(--warn-light);
      --danger: var(--danger-light);
      --violet: var(--violet-light);

      --scrollbar-thumb: #64748b;
      --scrollbar-track: #e2e8f0;

      @media (prefers-color-scheme: dark) {
        --bg-canvas: var(--bg-canvas-dark);
        --bg-surface: var(--bg-surface-dark);
        --bg-elevated: var(--bg-elevated-dark);
        --text-main: var(--text-main-dark);
        --text-muted: var(--text-muted-dark);
        --border-subtle: var(--border-subtle-dark);
        --accent: var(--accent-dark);
        --ok: var(--ok-dark);
        --warn: var(--warn-dark);
        --danger: var(--danger-dark);
        --violet: var(--violet-dark);
        --scrollbar-thumb: #64748b;
        --scrollbar-track: #0f172a;
      }

      @supports (color: light-dark(white, black)) {
        --bg-canvas: light-dark(var(--bg-canvas-light), var(--bg-canvas-dark));
        --bg-surface: light-dark(var(--bg-surface-light), var(--bg-surface-dark));
        --bg-elevated: light-dark(var(--bg-elevated-light), var(--bg-elevated-dark));
        --text-main: light-dark(var(--text-main-light), var(--text-main-dark));
        --text-muted: light-dark(var(--text-muted-light), var(--text-muted-dark));
        --border-subtle: light-dark(var(--border-subtle-light), var(--border-subtle-dark));
        --accent: light-dark(var(--accent-light), var(--accent-dark));
        --ok: light-dark(var(--ok-light), var(--ok-dark));
        --warn: light-dark(var(--warn-light), var(--warn-dark));
        --danger: light-dark(var(--danger-light), var(--danger-dark));
        --violet: light-dark(var(--violet-light), var(--violet-dark));
        --scrollbar-thumb: light-dark(#64748b, #94a3b8);
        --scrollbar-track: light-dark(#e2e8f0, #0f172a);
      }

      accent-color: var(--accent);
      scrollbar-color: var(--scrollbar-thumb) var(--scrollbar-track);
    }

    @media (forced-colors: active) {
      :root {
        --border-subtle: CanvasText;
        --accent: Highlight;
      }
    }

    * { box-sizing: border-box; margin: 0; padding: 0; }

    body {
      font-family: 'Inter', system-ui, -apple-system, BlinkMacSystemFont, sans-serif;
      background-color: var(--bg-canvas);
      color: var(--text-main);
      min-block-size: 100dvb;
      padding-block: 1rem;
      padding-inline: clamp(0.85rem, 2.5vw, 1.75rem);
      line-height: 1.5;
    }

    .dashboard-shell {
      max-inline-size: 1520px;
      margin-inline: auto;
      display: grid;
      grid-template-columns: 1fr;
      grid-template-areas:
        "topbar"
        "kpis"
        "workspace";
      gap: 1rem;
    }

    .topbar {
      grid-area: topbar;
      background: var(--bg-surface);
      border: 1px solid var(--border-subtle);
      border-radius: 16px;
      padding-block: 0.75rem;
      padding-inline: 1.2rem;
      display: flex;
      flex-wrap: wrap;
      align-items: safe center;
      gap: 1rem;
      box-shadow: 0 4px 20px -4px color-mix(in oklab, var(--text-main), transparent 92%);
    }

    .brand-group {
      display: flex;
      align-items: safe center;
      gap: 0.85rem;
    }

    .brand-badge {
      inline-size: 2.5rem;
      block-size: 2.5rem;
      border-radius: 12px;
      background: color-mix(in oklab, var(--accent), transparent 84%);
      border: 1px solid color-mix(in oklab, var(--accent), transparent 55%);
      display: grid;
      place-content: center;
      font-size: 1.2rem;
    }

    .brand-title {
      font-size: 1.05rem;
      font-weight: 700;
      letter-spacing: -0.01em;
    }

    .brand-sub {
      font-size: 0.76rem;
      color: var(--text-muted);
    }

    .status-cluster {
      margin-inline-start: auto;
      display: flex;
      flex-wrap: wrap;
      flex-wrap: balance;
      align-items: safe center;
      gap: 0.55rem;
    }

    .pill-badge {
      display: inline-flex;
      align-items: safe center;
      gap: 0.4rem;
      padding-block: 0.32rem;
      padding-inline: 0.72rem;
      border-radius: 999px;
      font-size: 0.76rem;
      font-weight: 600;
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
    }

    .pill-badge[data-tone="ok"] {
      background: color-mix(in oklab, var(--ok), transparent 86%);
      border-color: color-mix(in oklab, var(--ok), transparent 45%);
      color: var(--ok);
    }

    .pill-badge[data-tone="warn"] {
      background: color-mix(in oklab, var(--warn), transparent 86%);
      border-color: color-mix(in oklab, var(--warn), transparent 45%);
      color: var(--warn);
    }

    .pill-badge[data-tone="danger"] {
      background: color-mix(in oklab, var(--danger), transparent 84%);
      border-color: color-mix(in oklab, var(--danger), transparent 40%);
      color: var(--danger);
    }

    /* KPI Subgrid Section */
    .kpi-grid {
      grid-area: kpis;
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(205px, 1fr));
      grid-template-rows: auto 1fr auto;
      gap: 0.85rem;
    }

    article.kpi-card {
      grid-row: span 3;
      display: grid;
      grid-template-rows: auto 1fr auto;
      grid-template-rows: subgrid;
      container: kpi / inline-size;
      container-type: inline-size;
      background: var(--bg-surface);
      border: 1px solid var(--border-subtle);
      border-radius: 16px;
      padding-block: 0.9rem;
      padding-inline: 1.05rem;
      gap: 0.35rem;
      box-shadow: 0 2px 10px -2px color-mix(in oklab, var(--text-main), transparent 94%);
    }

    article.kpi-card > header {
      display: flex;
      align-items: safe center;
      justify-content: space-between;
      font-size: 0.72rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--text-muted);
    }

    .kpi-value {
      font-size: clamp(1.3rem, 8cqi, 1.8rem);
      font-weight: 700;
      letter-spacing: -0.02em;
      font-variant-numeric: tabular-nums;
      display: flex;
      align-items: baseline;
      gap: 0.35rem;
    }

    .kpi-unit {
      font-size: 0.8rem;
      font-weight: 500;
      color: var(--text-muted);
    }

    article.kpi-card > footer {
      font-size: 0.74rem;
      color: var(--text-muted);
      display: flex;
      align-items: safe center;
      justify-content: space-between;
      gap: 0.5rem;
      padding-block-start: 0.25rem;
      border-block-start: 1px dashed color-mix(in oklab, var(--border-subtle), transparent 35%);
    }

    .workspace {
      grid-area: workspace;
      display: grid;
      grid-template-columns: minmax(0, 2.2fr) minmax(340px, 1fr);
      gap: 1rem;
      align-items: start;
    }

    @media (max-width: 1080px) {
      .workspace {
        grid-template-columns: 1fr;
      }
    }

    .panel-card {
      container: panel / inline-size;
      container-type: inline-size;
      background: var(--bg-surface);
      border: 1px solid var(--border-subtle);
      border-radius: 16px;
      padding: 1.1rem;
      display: flex;
      flex-direction: column;
      gap: 0.9rem;
      box-shadow: 0 4px 20px -4px color-mix(in oklab, var(--text-main), transparent 93%);
    }

    .panel-header {
      display: flex;
      flex-wrap: wrap;
      align-items: safe center;
      justify-content: space-between;
      gap: 0.65rem;
    }

    .panel-title {
      font-size: 0.96rem;
      font-weight: 700;
    }

    [role="toolbar"] {
      display: flex;
      flex-wrap: wrap;
      flex-wrap: balance;
      align-items: safe center;
      gap: 0.4rem;
    }

    [role="toolbar"] [role="group"] {
      display: flex;
      flex-wrap: wrap;
      align-items: safe center;
      gap: 0.35rem;
    }

    [role="separator"][aria-orientation="vertical"] {
      inline-size: 0;
      block-size: 1.3rem;
      border-inline-start: 1px solid var(--border-subtle);
      margin-inline: 0.15rem;
    }

    .chip-btn, .cmd-btn {
      min-inline-size: 24px;
      min-block-size: 30px;
      padding-block: 0.3rem;
      padding-inline: 0.68rem;
      border-radius: 999px;
      border: 1px solid var(--border-subtle);
      background: var(--bg-elevated);
      color: var(--text-main);
      font-size: 0.74rem;
      font-weight: 600;
      cursor: pointer;
      display: inline-flex;
      align-items: safe center;
      gap: 0.38rem;
    }

    .chip-btn:focus-visible, .cmd-btn:focus-visible, select:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: 2px;
    }

    .chip-btn[aria-pressed="true"] {
      background: color-mix(in oklab, var(--chip-color, var(--accent)), transparent 82%);
      border-color: var(--chip-color, var(--accent));
      color: var(--text-main);
      box-shadow: inset 0 0 0 1px var(--chip-color, var(--accent));
    }

    .chip-dot {
      inline-size: 8px;
      block-size: 8px;
      border-radius: 50%;
      background: var(--chip-color, var(--accent));
    }

    .canvas-wrap {
      position: relative;
    }

    .offline-banner {
      position: absolute;
      inset-block-start: 0.6rem;
      inset-inline: 0.6rem;
      z-index: 2;
      padding-block: 0.5rem;
      padding-inline: 0.85rem;
      border-radius: 10px;
      font-size: 0.82rem;
      font-weight: 700;
      text-align: center;
      pointer-events: none;
      color: var(--danger);
      background: color-mix(in oklab, var(--bg-surface), var(--danger) 14%);
      border: 1px solid color-mix(in oklab, var(--danger), transparent 35%);
    }

    .offline-banner[hidden] { display: none; }

    .canvas-wrap.is-stale canvas { opacity: 0.45; }

    canvas.chart-stage {
      display: block;
      inline-size: 100%;
      block-size: 320px;
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      border-radius: 12px;
      cursor: crosshair;
    }

    canvas.subchart-stage {
      display: block;
      inline-size: 100%;
      block-size: 130px;
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      border-radius: 12px;
    }

    .cmd-grid {
      display: grid;
      grid-template-columns: repeat(2, 1fr);
      gap: 0.5rem;
    }

    .cmd-btn {
      border-radius: 10px;
      justify-content: center;
      min-block-size: 38px;
      font-size: 0.8rem;
    }

    .cmd-btn[aria-pressed="true"] {
      background: var(--accent);
      border-color: var(--accent);
      color: #ffffff;
    }

    .cmd-btn[data-variant="danger"] {
      border-color: color-mix(in oklab, var(--danger), transparent 40%);
      color: var(--danger);
    }

    .op-list {
      display: flex;
      flex-direction: column;
      gap: 0.55rem;
    }

    .op-row {
      display: flex;
      align-items: safe center;
      justify-content: space-between;
      gap: 0.65rem;
      padding-block: 0.45rem;
      padding-inline: 0.7rem;
      border-radius: 10px;
      background: var(--bg-elevated);
      border: 1px solid color-mix(in oklab, var(--border-subtle), transparent 35%);
      font-size: 0.8rem;
    }

    .op-select {
      background: var(--bg-surface);
      color: var(--text-main);
      border: 1px solid var(--border-subtle);
      border-radius: 8px;
      padding: 0.28rem 0.55rem;
      font-size: 0.78rem;
      font-weight: 600;
    }

    input[type="checkbox"][switch] {
      accent-color: var(--accent);
      inline-size: 1.15rem;
      block-size: 1.15rem;
      cursor: pointer;
    }

    .mqtt-stream {
      max-block-size: 210px;
      overflow-y: auto;
      scrollbar-gutter: stable;
      overscroll-behavior: contain;
      scrollbar-width: thin;
      scrollbar-color: var(--scrollbar-thumb) var(--scrollbar-track);
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      border-radius: 10px;
      padding: 0.65rem;
      font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      font-size: 0.72rem;
      display: flex;
      flex-direction: column;
      gap: 0.3rem;
    }

    @supports not (scrollbar-color: auto) {
      .mqtt-stream::-webkit-scrollbar {
        width: 8px;
        height: 8px;
      }
      .mqtt-stream::-webkit-scrollbar-thumb {
        background: var(--scrollbar-thumb);
        border-radius: 4px;
      }
      .mqtt-stream::-webkit-scrollbar-track {
        background: var(--scrollbar-track);
      }
    }

    .log-line {
      white-space: pre-wrap;
      word-break: break-all;
      border-block-end: 1px solid color-mix(in oklab, var(--border-subtle), transparent 70%);
      padding-block-end: 0.2rem;
    }

    @media (hover: hover) and (prefers-reduced-motion: no-preference) {
      .chip-btn, .cmd-btn {
        transition: background-color 140ms ease, border-color 140ms ease;
      }
      .chip-btn:hover, .cmd-btn:hover {
        border-color: var(--accent);
      }
    }
  </style>
</head>
<body>
  <main class="dashboard-shell">
    <header class="topbar">
      <div class="brand-group">
        <div class="brand-badge" aria-hidden="true">🔥</div>
        <div>
          <h1 class="brand-title">Karhutla Edge-IoT — Telemetry &amp; Command Center</h1>
          <p class="brand-sub">Monitor Real-time SITL / HILS / Field Node • Broker Port 6669 • Modern Web UI</p>
        </div>
      </div>

      <div class="status-cluster">
        <span id="badge-broker" class="pill-badge" data-tone="ok">
          <span aria-hidden="true">●</span> <span id="txt-broker">Broker :6669 Connected</span>
        </span>
        <span id="badge-node" class="pill-badge" data-tone="warn">
          <span id="ico-node" aria-hidden="true">○</span> <span id="txt-node">ESP32: belum terdeteksi</span>
        </span>
        <span id="badge-fsm" class="pill-badge" data-tone="ok">
          <span id="ico-fsm" aria-hidden="true">✓</span> <span id="txt-fsm">FSM: NORMAL</span>
        </span>
        <span class="pill-badge">
          <span aria-hidden="true">⏺</span> <span id="txt-logfile">logs/mqtt.log</span>
        </span>
      </div>
    </header>

    <!-- 6 KPI Cards aligned via CSS Subgrid -->
    <section class="kpi-grid" aria-label="Indikator Utama Sensor dan Aktuator">
      <article class="kpi-card">
        <header>
          <span>State FSM &amp; Mode</span>
          <span id="kpi-state-ico" aria-hidden="true">✓</span>
        </header>
        <div id="kpi-state" class="kpi-value" style="color:var(--ok);">NORMAL</div>
        <footer>
          <span>Mode: <strong id="kpi-mode">AUTO</strong></span>
          <span>t = <span id="kpi-t">0</span>s (#<span id="kpi-seq">0</span>)</span>
        </footer>
      </article>

      <article class="kpi-card">
        <header>
          <span>Fusion Index (FI)</span>
          <span aria-hidden="true">📈</span>
        </header>
        <div class="kpi-value">
          <span id="kpi-fi">0.0</span> <span class="kpi-unit">µg/m³</span>
        </div>
        <footer>
          <span>Waspada ≥ 55</span>
          <span>Bahaya ≥ 150</span>
        </footer>
      </article>

      <article class="kpi-card">
        <header>
          <span>PM-proxy Kalman</span>
          <span aria-hidden="true">🌫️</span>
        </header>
        <div class="kpi-value" style="color:var(--accent);">
          <span id="kpi-pm">0.0</span> <span class="kpi-unit">µg/m³</span>
        </div>
        <footer>
          <span>Raw: <span id="kpi-pmraw">0.0</span></span>
          <span>dC/dt: <span id="kpi-dcdt">+0.00</span>/s</span>
        </footer>
      </article>

      <article class="kpi-card">
        <header>
          <span>Gas MQ-135 &amp; Skor g</span>
          <span aria-hidden="true">🧪</span>
        </header>
        <div class="kpi-value" style="color:var(--violet);">
          <span id="kpi-ppm">400</span> <span class="kpi-unit">ppm</span>
        </div>
        <footer>
          <span>Skor g: <span id="kpi-g">0.00</span></span>
          <span>V0: <span id="kpi-v0">0.600</span>V</span>
        </footer>
      </article>

      <article class="kpi-card">
        <header>
          <span>Prakiraan AI (t+60s)</span>
          <span aria-hidden="true">🤖</span>
        </header>
        <div class="kpi-value" style="color:var(--danger);">
          <span id="kpi-fc">0.0</span> <span class="kpi-unit">µg/m³</span>
        </div>
        <footer>
          <span>ETA Bahaya: <strong id="kpi-eta">-</strong></span>
          <span>MAE: <span id="kpi-mae">-</span></span>
        </footer>
      </article>

      <article class="kpi-card">
        <header>
          <span>Aktuator Kipas PWM</span>
          <span aria-hidden="true">🌀</span>
        </header>
        <div class="kpi-value" style="color:var(--ok);">
          <span id="kpi-duty">0.0</span> <span class="kpi-unit">%</span>
        </div>
        <footer>
          <span>Vd: <span id="kpi-vd">0.60</span>V</span>
          <span>Vm: <span id="kpi-vm">1.00</span>V</span>
        </footer>
      </article>
    </section>

    <section class="workspace">
      <!-- Left: Multi-Overlay Chart -->
      <div class="panel-card">
        <div class="panel-header">
          <div>
            <h2 class="panel-title">Grafik Multi-Overlay Real-Time (Klik untuk Toggle Layer)</h2>
            <p class="brand-sub">Sumber data: <strong id="src-label">SITL</strong> • Arahkan kursor pada grafik untuk melihat detail semua nilai pada detik tersebut</p>
          </div>

          <div role="toolbar" focusgroup="toolbar wrap" aria-label="Pengaturan Jendela Waktu dan Ekspor">
            <div role="group" aria-label="Sumber Data">
              <button type="button" class="chip-btn src-btn" data-src="auto" aria-pressed="true">Otomatis</button>
              <button type="button" class="chip-btn src-btn" data-src="esp32" aria-pressed="false" style="--chip-color:#4ade80;">ESP32</button>
              <button type="button" class="chip-btn src-btn" data-src="sitl" aria-pressed="false" style="--chip-color:#38bdf8;">SITL</button>
            </div>
            <span role="separator" aria-orientation="vertical"></span>
            <div role="group" aria-label="Rentang Waktu">
              <button type="button" class="chip-btn win-btn" data-win="120" aria-pressed="false">120s</button>
              <button type="button" class="chip-btn win-btn" data-win="300" aria-pressed="true">300s</button>
              <button type="button" class="chip-btn win-btn" data-win="900" aria-pressed="false">900s</button>
              <button type="button" class="chip-btn win-btn" data-win="0" aria-pressed="false">Semua</button>
            </div>
            <span role="separator" aria-orientation="vertical"></span>
            <button type="button" class="chip-btn" id="btn-export-png" title="Unduh gambar grafik PNG">⬇ PNG</button>
            <button type="button" class="chip-btn" id="btn-export-mobile" title="Unduh file JSON konfigurasi siap-import untuk aplikasi HP">📱 Import HP (.json)</button>
          </div>
        </div>

        <!-- Overlay Layer Toggles Toolbar -->
        <div role="toolbar" focusgroup="toolbar wrap" aria-label="Filter Layer Overlay Grafik" id="overlay-toolbar">
          <div role="group" aria-label="Kurva Utama">
            <button type="button" class="chip-btn layer-btn" data-layer="fi" aria-pressed="true" style="--chip-color:#f87171;">
              <span class="chip-dot" aria-hidden="true"></span> Fusion Index (FI)
            </button>
            <button type="button" class="chip-btn layer-btn" data-layer="pm" aria-pressed="true" style="--chip-color:#38bdf8;">
              <span class="chip-dot" aria-hidden="true"></span> PM-proxy Kalman
            </button>
            <button type="button" class="chip-btn layer-btn" data-layer="pm_true" aria-pressed="true" style="--chip-color:#4ade80;">
              <span class="chip-dot" aria-hidden="true"></span> PM Plant (True)
            </button>
            <button type="button" class="chip-btn layer-btn" data-layer="pm_raw" aria-pressed="false" style="--chip-color:#94a3b8;">
              <span class="chip-dot" aria-hidden="true"></span> PM Raw (Median)
            </button>
            <button type="button" class="chip-btn layer-btn" data-layer="forecast" aria-pressed="true" style="--chip-color:#fbbf24;">
              <span class="chip-dot" aria-hidden="true"></span> AI Forecast (t+60s)
            </button>
          </div>
          <span role="separator" aria-orientation="vertical"></span>
          <div role="group" aria-label="Kurva Gas dan Tegangan">
            <button type="button" class="chip-btn layer-btn" data-layer="gas_ppm" aria-pressed="false" style="--chip-color:#c084fc;">
              <span class="chip-dot" aria-hidden="true"></span> Gas MQ-135 (ppm×0.2)
            </button>
            <button type="button" class="chip-btn layer-btn" data-layer="g_scaled" aria-pressed="true" style="--chip-color:#a855f7;">
              <span class="chip-dot" aria-hidden="true"></span> Skor Gas (g×100)
            </button>
            <button type="button" class="chip-btn layer-btn" data-layer="voltages" aria-pressed="false" style="--chip-color:#2dd4bf;">
              <span class="chip-dot" aria-hidden="true"></span> Tegangan Sensor (V×50)
            </button>
            <button type="button" class="chip-btn layer-btn" data-layer="thresholds" aria-pressed="true" style="--chip-color:#fb923c;">
              <span class="chip-dot" aria-hidden="true"></span> Garis Ambang (55 / 150)
            </button>
          </div>
        </div>

        <div class="canvas-wrap" id="main-wrap">
          <div id="esp-banner" class="offline-banner" role="status" hidden></div>
          <canvas id="main-chart" class="chart-stage" width="960" height="320" aria-label="Grafik Utama Multi-Overlay"></canvas>
        </div>

        <div class="canvas-wrap">
          <canvas id="sub-chart" class="subchart-stage" width="960" height="130" aria-label="Sub-Grafik Sinkron Duty Kipas dan State FSM"></canvas>
        </div>
      </div>

      <!-- Right: Actuator Override, Embedded SITL Operations, Live MQTT Log -->
      <aside class="panel-card" aria-label="Panel Kontrol dan Operasi">
        <div>
          <h2 class="panel-title">Kontrol Override Aktuator (`karhutla/cmd`)</h2>
          <p class="brand-sub">Mengirim perintah ke SITL / HILS / ESP32 dan langsung memperbarui kipas</p>
        </div>

        <div role="toolbar" focusgroup="toolbar wrap" aria-label="Perintah Override Kipas" class="cmd-grid" id="cmd-toolbar">
          <button type="button" class="cmd-btn" data-cmd="AUTO" aria-pressed="true">
            <span aria-hidden="true">🤖</span> AUTO (FSM)
          </button>
          <button type="button" class="cmd-btn" data-cmd="FORCE_MAX" aria-pressed="false">
            <span aria-hidden="true">🚀</span> FORCE MAX (100%)
          </button>
          <button type="button" class="cmd-btn" data-cmd="FORCE_OFF" aria-pressed="false">
            <span aria-hidden="true">⏹</span> FORCE OFF (0%)
          </button>
          <button type="button" class="cmd-btn" data-cmd="RESET" data-variant="danger">
            <span aria-hidden="true">↺</span> RESET SISTEM
          </button>
        </div>

        <div style="border-block-start:1px solid var(--border-subtle);padding-block-start:0.75rem;">
          <div style="display:flex;justify-content:space-between;align-items:safe center;">
            <div>
              <h2 class="panel-title">Operasi Simulasi SITL Terpadu</h2>
              <p class="brand-sub">Jalankan atau uji skenario langsung dari panel ini</p>
            </div>
            <span id="badge-sim-state" class="pill-badge">IDLE</span>
          </div>
        </div>

        <div class="op-list">
          <div class="op-row">
            <label for="sel-scenario">Skenario Ruang Asap</label>
            <select id="sel-scenario" class="op-select">
              <option value="flaming" selected>flaming (Api Cepat)</option>
              <option value="smolder">smolder (membara Lambat)</option>
              <option value="clean">clean (Udara Bersih)</option>
              <option value="fault">fault (Uji Putus Sensor t=200-230s)</option>
            </select>
          </div>

          <div class="op-row">
            <span>Kontrol Eksekusi</span>
            <div role="toolbar" focusgroup="toolbar wrap" aria-label="Kontrol Runner SITL">
              <button type="button" class="chip-btn" id="btn-sim-start" style="--chip-color:#4ade80;" aria-pressed="false">▶ Mulai</button>
              <button type="button" class="chip-btn" id="btn-sim-pause" aria-pressed="false">⏸ Jeda</button>
              <button type="button" class="chip-btn" id="btn-sim-stop" aria-pressed="false">⏹ Berhenti</button>
              <button type="button" class="chip-btn" id="btn-sim-clear" aria-pressed="false">🗑 Bersihkan</button>
            </div>
          </div>

          <div class="op-row">
            <span>Kecepatan Simulasi</span>
            <div role="toolbar" focusgroup="toolbar wrap" aria-label="Kecepatan Simulasi" id="speed-toolbar">
              <button type="button" class="chip-btn spd-btn" data-speed="1" aria-pressed="true">1x</button>
              <button type="button" class="chip-btn spd-btn" data-speed="2" aria-pressed="false">2x</button>
              <button type="button" class="chip-btn spd-btn" data-speed="5" aria-pressed="false">5x</button>
              <button type="button" class="chip-btn spd-btn" data-speed="10" aria-pressed="false">10x</button>
              <button type="button" class="chip-btn spd-btn" data-speed="20" aria-pressed="false">20x</button>
            </div>
          </div>

          <label class="op-row" for="sw-preempt" style="cursor:pointer;">
            <span>AI Preempt (`FORCE_MAX` Proaktif Dini)</span>
            <input id="sw-preempt" type="checkbox" switch>
          </label>

          <label class="op-row" for="sw-fault" style="cursor:pointer;">
            <span>Injeksi Putus Kabel Sensor Debu (`FAILSAFE`)</span>
            <input id="sw-fault" type="checkbox" switch>
          </label>
        </div>

        <div style="border-block-start:1px solid var(--border-subtle);padding-block-start:0.75rem;display:flex;justify-content:space-between;align-items:safe center;">
          <div>
            <h2 class="panel-title">Live MQTT Stream (`:6669`)</h2>
            <p class="brand-sub">Tercatat otomatis di `logs/mqtt_YYYY-MM-DD.log`</p>
          </div>
          <label style="font-size:0.74rem;display:flex;align-items:safe center;gap:0.35rem;cursor:pointer;">
            <span>Auto-scroll</span>
            <input id="sw-autoscroll" type="checkbox" switch checked>
          </label>
        </div>

        <div id="mqtt-log-box" class="mqtt-stream" role="log" aria-live="polite" aria-label="Aliran Log MQTT"></div>
      </aside>
    </section>
  </main>

  <script>
    (() => {
      // Roving-tabindex fallback when native focusgroup is unavailable
      document.querySelectorAll('[role="toolbar"][focusgroup]').forEach((bar) => {
        if ('focusGroup' in HTMLElement.prototype) return;
        const items = Array.from(bar.querySelectorAll('button'));
        if (!items.length) return;
        items.forEach((b, i) => (b.tabIndex = i ? -1 : 0));
        bar.addEventListener('focusin', ({ target }) => {
          if (items.includes(target)) items.forEach((b) => (b.tabIndex = b === target ? 0 : -1));
        });
        bar.addEventListener('keydown', (e) => {
          const idx = items.indexOf(document.activeElement);
          if (idx < 0) return;
          let next = idx;
          if (e.key === 'ArrowRight') next = (idx + 1) % items.length;
          else if (e.key === 'ArrowLeft') next = (idx - 1 + items.length) % items.length;
          else if (e.key === 'Home') next = 0;
          else if (e.key === 'End') next = items.length - 1;
          else return;
          e.preventDefault();
          items[next].focus();
        });
      });

      const state = {
        history: [],
        src: {
          sitl: { history: [], latest: null },
          esp32: { history: [], latest: null },
        },
        esp: { online: false, age_s: null, hw_mode: null, lastRx: null },
        viewMode: 'auto',
        latest: {},
        sim: {},
        windowSec: 300,
        hoverX: null,
        layers: {
          fi: true,
          pm: true,
          pm_true: true,
          pm_raw: false,
          forecast: true,
          gas_ppm: false,
          g_scaled: true,
          voltages: false,
          thresholds: true,
        },
      };

      const mainCanvas = document.getElementById('main-chart');
      const subCanvas = document.getElementById('sub-chart');
      const mCtx = mainCanvas.getContext('2d');
      const sCtx = subCanvas.getContext('2d');
      const logBox = document.getElementById('mqtt-log-box');

      function appendMqttLog(line) {
        if (!line) return;
        const div = document.createElement('div');
        div.className = 'log-line';
        div.textContent = line;
        logBox.appendChild(div);
        while (logBox.children.length > 200) {
          logBox.removeChild(logBox.firstChild);
        }
        if (document.getElementById('sw-autoscroll').checked) {
          logBox.scrollTop = logBox.scrollHeight;
        }
      }

      function activeSource() {
        if (state.viewMode === 'sitl' || state.viewMode === 'esp32') return state.viewMode;
        if (state.esp.online) return 'esp32';
        if (state.sim && state.sim.running) return 'sitl';
        if (state.src.esp32.history.length) return 'esp32';
        return 'sitl';
      }

      function updateEspBadge() {
        const bNode = document.getElementById('badge-node');
        const txt = document.getElementById('txt-node');
        const ico = document.getElementById('ico-node');
        const mode = state.esp.hw_mode ? ` (${state.esp.hw_mode})` : '';
        if (state.esp.online) {
          txt.textContent = `ESP32: ONLINE${mode}`;
          ico.textContent = '✓';
          bNode.setAttribute('data-tone', 'ok');
        } else if (state.esp.lastRx) {
          const age = Math.round((Date.now() - state.esp.lastRx) / 1000);
          txt.textContent = `ESP32: TERPUTUS (${age} dtk tanpa data)`;
          ico.textContent = '✕';
          bNode.setAttribute('data-tone', 'danger');
        } else {
          txt.textContent = 'ESP32: belum terdeteksi';
          ico.textContent = '○';
          bNode.setAttribute('data-tone', 'warn');
        }
        const src = activeSource();
        const banner = document.getElementById('esp-banner');
        const wrap = document.getElementById('main-wrap');
        const stale = src === 'esp32' && !state.esp.online;
        wrap.classList.toggle('is-stale', stale);
        banner.hidden = !stale;
        if (stale) {
          banner.textContent = state.esp.lastRx
            ? `⚠ ESP32 TERPUTUS — tidak ada data sejak ${Math.round((Date.now() - state.esp.lastRx) / 1000)} detik lalu (grafik dibekukan)`
            : '⚠ Menunggu ESP32 — belum ada telemetri/heartbeat diterima dari broker';
        }
      }

      function applyView() {
        const src = activeSource();
        state.history = state.src[src].history;
        document.getElementById('src-label').textContent =
          (src === 'esp32' ? 'ESP32 (hardware)' : 'SITL (simulasi)') + (state.viewMode === 'auto' ? ' — otomatis' : '');
        document.querySelectorAll('.src-btn').forEach((b) =>
          b.setAttribute('aria-pressed', b.getAttribute('data-src') === state.viewMode ? 'true' : 'false'));
        const lat = state.src[src].latest;
        if (lat) updateKpis(lat, state.sim);
        updateEspBadge();
        drawCharts();
      }

      function pushRow(source, row) {
        const bucket = state.src[source] || state.src.sitl;
        const rt = row.t ?? 0;
        const hist = bucket.history;
        const last = hist.length ? hist[hist.length - 1] : null;
        if (last && rt < (last.t ?? 0)) {
          bucket.history = [row];
        } else if (last && rt === (last.t ?? -1)) {
          hist[hist.length - 1] = row;
        } else {
          hist.push(row);
        }
        if (bucket.history.length > 1800) bucket.history.shift();
        bucket.latest = row;
      }

      function updateKpis(row, sim) {
        if (!row) return;
        const st = row.state || 'NORMAL';
        const toneMap = { NORMAL: 'ok', WASPADA: 'warn', BAHAYA: 'danger', FAILSAFE: 'danger' };
        const icoMap = { NORMAL: '✓', WASPADA: '⚠', BAHAYA: '🔥', FAILSAFE: '⚡' };
        const colorMap = { NORMAL: 'var(--ok)', WASPADA: 'var(--warn)', BAHAYA: 'var(--danger)', FAILSAFE: 'var(--danger)' };

        document.getElementById('kpi-state').textContent = st;
        document.getElementById('kpi-state').style.color = colorMap[st] || 'var(--ok)';
        document.getElementById('kpi-state-ico').textContent = icoMap[st] || '✓';
        document.getElementById('kpi-mode').textContent = row.mode || 'AUTO';
        document.getElementById('kpi-t').textContent = (row.t ?? 0).toFixed(0);
        document.getElementById('kpi-seq').textContent = row.seq ?? 0;

        document.getElementById('kpi-fi').textContent = (row.fi ?? 0).toFixed(1);
        document.getElementById('kpi-pm').textContent = (row.pm ?? 0).toFixed(1);
        document.getElementById('kpi-pmraw').textContent = (row.pm_raw ?? 0).toFixed(1);
        const dcdt = row.dcdt ?? 0;
        document.getElementById('kpi-dcdt').textContent = (dcdt >= 0 ? '+' : '') + dcdt.toFixed(2);

        document.getElementById('kpi-ppm').textContent = (row.gas_ppm ?? 400).toFixed(0);
        document.getElementById('kpi-g').textContent = (row.g ?? 0).toFixed(2);
        document.getElementById('kpi-v0').textContent = (row.v0 ?? 0.6).toFixed(3);

        document.getElementById('kpi-fc').textContent = row.forecast != null ? row.forecast.toFixed(1) : '-';
        document.getElementById('kpi-eta').textContent = row.eta_bahaya_s != null ? `${row.eta_bahaya_s.toFixed(0)} s` : '-';
        document.getElementById('kpi-mae').textContent = row.mae != null ? row.mae.toFixed(2) : '-';

        document.getElementById('kpi-duty').textContent = (row.duty ?? 0).toFixed(1);
        document.getElementById('kpi-vd').textContent = (row.v_dust ?? 0).toFixed(2);
        document.getElementById('kpi-vm').textContent = (row.v_mq ?? 0).toFixed(2);

        const badgeFsm = document.getElementById('badge-fsm');
        badgeFsm.setAttribute('data-tone', toneMap[st] || 'ok');
        document.getElementById('ico-fsm').textContent = icoMap[st] || '✓';
        document.getElementById('txt-fsm').textContent = `FSM: ${st}`;

        // Sync active mode button aria-pressed
        document.querySelectorAll('#cmd-toolbar .cmd-btn[data-cmd]').forEach((btn) => {
          const c = btn.getAttribute('data-cmd');
          if (c !== 'RESET') {
            btn.setAttribute('aria-pressed', c === row.mode ? 'true' : 'false');
          }
        });

        if (sim) updateSimBadges(sim);
      }

      function updateSimBadges(sim) {
        if (!sim) return;
        state.sim = sim;
        const bSim = document.getElementById('badge-sim-state');
        if (sim.running) {
          bSim.textContent = sim.paused ? `PAUSED (${sim.scenario})` : `RUNNING: ${sim.scenario} (${sim.speed}x)`;
          bSim.setAttribute('data-tone', sim.paused ? 'warn' : 'ok');
        } else {
          bSim.textContent = 'IDLE / MQTT MONITOR';
          bSim.removeAttribute('data-tone');
        }
        document.getElementById('btn-sim-start').setAttribute('aria-pressed', sim.running && !sim.paused ? 'true' : 'false');
        document.getElementById('btn-sim-pause').setAttribute('aria-pressed', sim.paused ? 'true' : 'false');
        if (sim.log_file) {
          document.getElementById('txt-logfile').textContent = sim.log_file;
        }
      }

      function drawCharts() {
        const rows = state.history;
        const w = mainCanvas.width;
        const h = mainCanvas.height;
        const sw = subCanvas.width;
        const sh = subCanvas.height;

        const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
        const gridColor = isDark ? 'rgba(148,163,184,0.18)' : 'rgba(71,85,105,0.16)';
        const textColor = isDark ? '#94a3b8' : '#475569';

        mCtx.clearRect(0, 0, w, h);
        sCtx.clearRect(0, 0, sw, sh);

        const padL = 52, padR = 24, padT = 22, padB = 28;
        const plotW = w - padL - padR;
        const plotH = h - padT - padB;

        if (!rows.length) {
          mCtx.fillStyle = textColor;
          mCtx.font = '600 14px system-ui, sans-serif';
          mCtx.fillText(activeSource() === 'esp32'
            ? 'Menunggu telemetri ESP32 dari broker MQTT (:6669) ...'
            : 'Belum ada data SITL — klik "▶ Mulai" pada panel Operasi Simulasi SITL ...', padL + 20, h / 2);
          return;
        }

        const maxT = rows[rows.length - 1].t ?? 0;
        const minT = state.windowSec > 0 ? Math.max(0, maxT - state.windowSec) : (rows[0].t ?? 0);
        const spanT = Math.max(60, (maxT + (state.layers.forecast ? 60 : 5)) - minT);

        // Hanya ambil segmen waktu monoton terakhir agar tidak pernah ada garis miring balik ke kiri
        const monotonic = [];
        for (const r of rows) {
          const rt = r.t ?? 0;
          if (monotonic.length && rt < (monotonic[monotonic.length - 1].t ?? 0)) {
            monotonic.length = 0;
          }
          if (monotonic.length && rt === (monotonic[monotonic.length - 1].t ?? -1)) {
            monotonic[monotonic.length - 1] = r;
          } else {
            monotonic.push(r);
          }
        }
        const visible = monotonic.filter((r) => (r.t ?? 0) >= minT);
        let maxY = 180;
        visible.forEach((r) => {
          if (state.layers.fi && r.fi != null) maxY = Math.max(maxY, r.fi * 1.1);
          if (state.layers.pm && r.pm != null) maxY = Math.max(maxY, r.pm * 1.1);
          if (state.layers.pm_true && r.pm_true != null) maxY = Math.max(maxY, r.pm_true * 1.1);
          if (state.layers.pm_raw && r.pm_raw != null) maxY = Math.max(maxY, r.pm_raw * 1.1);
          if (state.layers.forecast && r.forecast != null) maxY = Math.max(maxY, r.forecast * 1.1);
          if (state.layers.gas_ppm && r.gas_ppm != null) maxY = Math.max(maxY, r.gas_ppm * 0.22);
        });

        const xOf = (t) => padL + ((t - minT) / spanT) * plotW;
        const yOf = (v) => padT + plotH - (Math.max(0, Math.min(maxY, v)) / maxY) * plotH;

        // Grid & Y labels
        mCtx.strokeStyle = gridColor;
        mCtx.fillStyle = textColor;
        mCtx.font = '11px system-ui, sans-serif';
        mCtx.lineWidth = 1;
        for (let i = 0; i <= 4; i++) {
          const val = (maxY / 4) * i;
          const y = yOf(val);
          mCtx.beginPath();
          mCtx.moveTo(padL, y);
          mCtx.lineTo(w - padR, y);
          mCtx.stroke();
          mCtx.fillText(val.toFixed(0), 10, y + 4);
        }

        // Thresholds
        if (state.layers.thresholds) {
          const yW = yOf(55);
          const yB = yOf(150);
          mCtx.save();
          mCtx.setLineDash([6, 4]);
          mCtx.strokeStyle = '#fbbf24';
          mCtx.beginPath(); mCtx.moveTo(padL, yW); mCtx.lineTo(w - padR, yW); mCtx.stroke();
          mCtx.fillStyle = '#fbbf24';
          mCtx.fillText('WASPADA (55)', padL + 6, yW - 5);

          mCtx.strokeStyle = '#f87171';
          mCtx.beginPath(); mCtx.moveTo(padL, yB); mCtx.lineTo(w - padR, yB); mCtx.stroke();
          mCtx.fillStyle = '#f87171';
          mCtx.fillText('BAHAYA (150)', padL + 6, yB - 5);
          mCtx.restore();
        }

        function plotSeries(extractFn, color, width = 2, dash = [], xOffset = 0) {
          mCtx.save();
          mCtx.strokeStyle = color;
          mCtx.lineWidth = width;
          mCtx.setLineDash(dash);
          mCtx.beginPath();
          let started = false;
          let prevT = -Infinity;
          for (const r of visible) {
            const v = extractFn(r);
            if (v == null || Number.isNaN(v)) continue;
            const rt = (r.t ?? 0) + xOffset;
            const x = xOf(rt);
            const y = yOf(v);
            if (!started || rt <= prevT) { mCtx.moveTo(x, y); started = true; }
            else mCtx.lineTo(x, y);
            prevT = rt;
          }
          mCtx.stroke();
          mCtx.restore();
        }

        if (state.layers.pm_raw) plotSeries((r) => r.pm_raw, '#94a3b8', 1.4);
        if (state.layers.pm_true) plotSeries((r) => r.pm_true, '#4ade80', 1.8);
        if (state.layers.voltages) {
          plotSeries((r) => (r.v_dust ?? 0) * 50, '#2dd4bf', 1.5, [2, 2]);
          plotSeries((r) => (r.v_mq ?? 0) * 50, '#14b8a6', 1.5, [4, 2]);
        }
        if (state.layers.gas_ppm) plotSeries((r) => (r.gas_ppm ?? 400) * 0.2, '#c084fc', 1.8, [3, 3]);
        if (state.layers.g_scaled) plotSeries((r) => (r.g ?? 0) * 100, '#a855f7', 1.8);
        if (state.layers.pm) plotSeries((r) => r.pm, '#38bdf8', 2.4);
        if (state.layers.fi) plotSeries((r) => r.fi, '#f87171', 2.8);
        if (state.layers.forecast) plotSeries((r) => r.forecast, '#fbbf24', 2.2, [6, 4], 60);

        // Hover crosshair inspection
        if (state.hoverX != null && state.hoverX >= padL && state.hoverX <= w - padR) {
          const targetT = minT + ((state.hoverX - padL) / plotW) * spanT;
          let nearest = visible[0];
          for (const r of visible) {
            if (Math.abs((r.t ?? 0) - targetT) < Math.abs((nearest.t ?? 0) - targetT)) nearest = r;
          }
          if (nearest) {
            const cx = xOf(nearest.t ?? 0);
            mCtx.save();
            mCtx.strokeStyle = textColor;
            mCtx.setLineDash([3, 3]);
            mCtx.beginPath(); mCtx.moveTo(cx, padT); mCtx.lineTo(cx, h - padB); mCtx.stroke();
            const boxX = Math.min(w - 205, Math.max(padL + 8, cx + 10));
            mCtx.fillStyle = isDark ? 'rgba(15,23,42,0.92)' : 'rgba(255,255,255,0.94)';
            mCtx.strokeStyle = gridColor;
            mCtx.setLineDash([]);
            mCtx.fillRect(boxX, padT + 6, 190, 88);
            mCtx.strokeRect(boxX, padT + 6, 190, 88);
            mCtx.fillStyle = isDark ? '#f8fafc' : '#0f172a';
            mCtx.font = '600 11px system-ui, sans-serif';
            mCtx.fillText(`t = ${(nearest.t ?? 0).toFixed(0)}s | FSM: ${nearest.state}`, boxX + 8, padT + 22);
            mCtx.font = '11px system-ui, sans-serif';
            mCtx.fillText(`FI: ${(nearest.fi ?? 0).toFixed(1)} | PM: ${(nearest.pm ?? 0).toFixed(1)} µg/m³`, boxX + 8, padT + 40);
            mCtx.fillText(`Gas: ${(nearest.gas_ppm ?? 400).toFixed(0)} ppm (g=${(nearest.g ?? 0).toFixed(2)})`, boxX + 8, padT + 58);
            mCtx.fillText(`Duty: ${(nearest.duty ?? 0).toFixed(0)}% | Fc+60s: ${(nearest.forecast ?? 0).toFixed(1)}`, boxX + 8, padT + 76);
            mCtx.restore();
          }
        }

        // Sub-chart: Duty % (0-100) and State FSM (0, 25, 50, 75)
        const sPadT = 18, sPadB = 20;
        const sPlotH = sh - sPadT - sPadB;
        const syOf = (v) => sPadT + sPlotH - (Math.max(0, Math.min(100, v)) / 100) * sPlotH;

        sCtx.strokeStyle = gridColor;
        sCtx.fillStyle = textColor;
        sCtx.font = '10.5px system-ui, sans-serif';
        for (const pct of [0, 50, 100]) {
          const y = syOf(pct);
          sCtx.beginPath(); sCtx.moveTo(padL, y); sCtx.lineTo(sw - padR, y); sCtx.stroke();
          sCtx.fillText(`${pct}%`, 12, y + 3);
        }

        const stateCode = { NORMAL: 0, WASPADA: 33, BAHAYA: 66, FAILSAFE: 100 };
        sCtx.strokeStyle = '#fbbf24';
        sCtx.lineWidth = 1.8;
        sCtx.beginPath();
        let prevStT = -Infinity;
        visible.forEach((r, idx) => {
          const rt = r.t ?? 0;
          const x = xOf(rt);
          const y = syOf(stateCode[r.state] ?? 0);
          if (idx === 0 || rt <= prevStT) sCtx.moveTo(x, y);
          else sCtx.lineTo(x, y);
          prevStT = rt;
        });
        sCtx.stroke();

        sCtx.strokeStyle = '#4ade80';
        sCtx.lineWidth = 2.3;
        sCtx.beginPath();
        let prevDuT = -Infinity;
        visible.forEach((r, idx) => {
          const rt = r.t ?? 0;
          const x = xOf(rt);
          const y = syOf(r.duty ?? 0);
          if (idx === 0 || rt <= prevDuT) sCtx.moveTo(x, y);
          else sCtx.lineTo(x, y);
          prevDuT = rt;
        });
        sCtx.stroke();
      }

      // Wire layer toggle buttons
      document.querySelectorAll('.layer-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          const layer = btn.getAttribute('data-layer');
          const next = btn.getAttribute('aria-pressed') !== 'true';
          btn.setAttribute('aria-pressed', next ? 'true' : 'false');
          state.layers[layer] = next;
          drawCharts();
        });
      });

      // Wire time window buttons
      document.querySelectorAll('.win-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          document.querySelectorAll('.win-btn').forEach((b) => b.setAttribute('aria-pressed', 'false'));
          btn.setAttribute('aria-pressed', 'true');
          state.windowSec = Number(btn.getAttribute('data-win') || 0);
          drawCharts();
        });
      });

      // Hover crosshair
      mainCanvas.addEventListener('mousemove', (e) => {
        const rect = mainCanvas.getBoundingClientRect();
        state.hoverX = ((e.clientX - rect.left) / rect.width) * mainCanvas.width;
        drawCharts();
      });
      mainCanvas.addEventListener('mouseleave', () => {
        state.hoverX = null;
        drawCharts();
      });

      // Export PNG & Mobile JSON
      document.getElementById('btn-export-png').addEventListener('click', () => {
        const a = document.createElement('a');
        a.download = `karhutla_dashboard_${Date.now()}.png`;
        a.href = mainCanvas.toDataURL('image/png');
        a.click();
      });

      document.getElementById('btn-export-mobile').addEventListener('click', () => {
        const a = document.createElement('a');
        a.download = 'karhutla_iot_mqtt_panel.json';
        a.href = '/api/mobile-config';
        a.click();
      });

      // Wire Actuator Override Commands
      document.querySelectorAll('#cmd-toolbar .cmd-btn').forEach((btn) => {
        btn.addEventListener('click', async () => {
          const cmd = btn.getAttribute('data-cmd');
          await fetch('/api/cmd', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ cmd }),
          });
        });
      });

      // Wire Simulation Operations
      async function postSim(payload) {
        await fetch('/api/sim', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        });
      }

      document.getElementById('btn-sim-start').addEventListener('click', () => {
        state.src.sitl.history = [];
        applyView();
        const scenario = document.getElementById('sel-scenario').value;
        const ai_preempt = document.getElementById('sw-preempt').checked;
        const fault_inject = document.getElementById('sw-fault').checked;
        postSim({ action: 'start', scenario, ai_preempt, fault_inject });
      });

      document.getElementById('btn-sim-pause').addEventListener('click', () => {
        const paused = state.sim && state.sim.paused;
        postSim({ action: paused ? 'resume' : 'pause' });
      });

      document.getElementById('btn-sim-stop').addEventListener('click', () => {
        postSim({ action: 'stop' });
      });

      document.getElementById('btn-sim-clear').addEventListener('click', () => {
        const src = activeSource();
        state.src[src].history = [];
        postSim({ action: 'clear', source: src });
        applyView();
      });

      document.querySelectorAll('.src-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          state.viewMode = btn.getAttribute('data-src');
          applyView();
        });
      });

      setInterval(updateEspBadge, 1000);

      document.getElementById('sel-scenario').addEventListener('change', (e) => {
        postSim({ scenario: e.target.value });
      });

      document.querySelectorAll('.spd-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          document.querySelectorAll('.spd-btn').forEach((b) => b.setAttribute('aria-pressed', 'false'));
          btn.setAttribute('aria-pressed', 'true');
          postSim({ speed: Number(btn.getAttribute('data-speed')) });
        });
      });

      document.getElementById('sw-preempt').addEventListener('change', (e) => {
        postSim({ ai_preempt: e.target.checked });
      });

      document.getElementById('sw-fault').addEventListener('change', (e) => {
        postSim({ fault_inject: e.target.checked });
      });

      // Initial snapshot load + SSE subscription
      fetch('/api/state')
        .then((r) => r.json())
        .then((d) => {
          if (d.sources) {
            state.src.sitl.history = (d.sources.sitl && d.sources.sitl.history) || [];
            state.src.sitl.latest = (d.sources.sitl && d.sources.sitl.latest) || null;
            state.src.esp32.history = (d.sources.esp32 && d.sources.esp32.history) || [];
            state.src.esp32.latest = (d.sources.esp32 && d.sources.esp32.latest) || null;
          } else {
            state.src.sitl.history = d.history || [];
            state.src.sitl.latest = d.latest || null;
          }
          if (d.esp) {
            state.esp.online = !!d.esp.online;
            state.esp.hw_mode = d.esp.hw_mode || null;
            state.esp.lastRx = d.esp.age_s != null ? Date.now() - d.esp.age_s * 1000 : null;
          }
          state.latest = d.latest || {};
          (d.mqtt_logs || []).forEach(appendMqttLog);
          updateSimBadges(d.sim);
          applyView();
        })
        .catch(() => {});

      const es = new EventSource('/api/stream');
      es.onmessage = (ev) => {
        try {
          const msg = JSON.parse(ev.data);
          if (msg.type === 'clear') {
            const src = msg.source === 'esp32' ? 'esp32' : 'sitl';
            state.src[src].history = [];
            applyView();
          } else if (msg.type === 'telemetry' && msg.row) {
            const src = msg.source === 'esp32' ? 'esp32' : 'sitl';
            pushRow(src, msg.row);
            if (src === 'esp32') {
              state.esp.online = true;
              state.esp.lastRx = Date.now();
            }
            if (msg.sim) updateSimBadges(msg.sim);
            applyView();
          } else if (msg.type === 'esp_status') {
            state.esp.online = !!msg.online;
            if (msg.hw_mode) state.esp.hw_mode = msg.hw_mode;
            if (msg.online) {
              state.esp.lastRx = Date.now();
            } else if (msg.age_s != null) {
              state.esp.lastRx = Date.now() - msg.age_s * 1000;
            }
            applyView();
          } else if (msg.type === 'mqtt_log') {
            appendMqttLog(msg.line);
          } else if (msg.type === 'sim') {
            updateSimBadges(msg.sim);
            applyView();
          } else if (msg.type === 'status') {
            if (state.sim) {
              state.sim.broker_connected = msg.broker_connected;
              state.sim.node_status = msg.node_status;
              updateSimBadges(state.sim);
            }
          }
        } catch (e) {}
      };

      window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', drawCharts);
    })();
  </script>
</body>
</html>
"""


class _ThreadingHTTPServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def start_dashboard_server(engine, host="0.0.0.0", port=8080):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):
            pass

        def _send_json(self, data, status=200):
            raw = json.dumps(data).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def do_GET(self):
            parsed = urllib.parse.urlparse(self.path)
            if parsed.path in ("/", "/index.html"):
                raw = RENDERED_HTML.encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)
                return
            if parsed.path == "/api/state":
                self._send_json(engine.get_state_snapshot())
                return
            if parsed.path == "/api/mobile-config":
                cfg = build_mobile_dashboard_config(engine.broker_host, engine.broker_port)
                raw = json.dumps(cfg, indent=2).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Content-Disposition", 'attachment; filename="karhutla_iot_mqtt_panel.json"')
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)
                return
            if parsed.path == "/api/stream":
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Connection", "keep-alive")
                self.end_headers()
                q = engine.register_sse()
                try:
                    while True:
                        if q:
                            chunk = q.popleft()
                            self.wfile.write(chunk)
                            self.wfile.flush()
                        else:
                            self.wfile.write(b": keepalive\n\n")
                            self.wfile.flush()
                            time.sleep(0.15)
                except (BrokenPipeError, ConnectionResetError, OSError):
                    pass
                finally:
                    engine.unregister_sse(q)
                return
            self.send_response(404)
            self.end_headers()

        def do_POST(self):
            parsed = urllib.parse.urlparse(self.path)
            length = int(self.headers.get("Content-Length", 0))
            body_raw = self.rfile.read(length).decode("utf-8", errors="ignore") if length else "{}"
            try:
                data = json.loads(body_raw)
            except ValueError:
                data = {}
            if parsed.path == "/api/cmd":
                ok = engine.send_cmd(data.get("cmd"))
                self._send_json({"ok": ok})
                return
            if parsed.path == "/api/sim":
                ok = engine.control_sim(data)
                self._send_json({"ok": ok, "sim": engine.get_sim_status()})
                return
            self.send_response(404)
            self.end_headers()

    srv = _ThreadingHTTPServer((host, int(port)), Handler)
    t = threading.Thread(target=srv.serve_forever, daemon=True)
    t.start()
    return srv


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--host", default="0.0.0.0", help="alamat bind HTTP server (default: 0.0.0.0)")
    ap.add_argument("--port", type=int, default=8080, help="port HTTP dashboard (default: 8080)")
    ap.add_argument("--broker-host", default=K.BROKER_HOST, help="host broker MQTT")
    ap.add_argument("--broker-port", type=int, default=K.BROKER_PORT, help="port broker MQTT (default: 6669)")
    ap.add_argument("--out", default="logs", help="folder penyimpanan log CSV & MQTT")
    ap.add_argument("--auto-sim", choices=SCENARIOS, default=None, help="langsung mulai simulasi skenario SITL")
    ap.add_argument("--speed", type=float, default=1.0, help="kecepatan simulasi awal (default: 1.0)")
    ap.add_argument("--ai-preempt", action="store_true", help="aktifkan AI Preempt saat auto-sim")
    ap.add_argument("--no-browser", action="store_true", help="jangan buka tab browser secara otomatis")
    args = ap.parse_args()

    eng = DashboardEngine(
        broker_host=args.broker_host,
        broker_port=args.broker_port,
        out_dir=args.out,
        enable_mqtt=True,
    )
    srv = start_dashboard_server(eng, host=args.host, port=args.port)
    actual_port = srv.server_address[1]
    url = f"http://localhost:{actual_port}/"
    print(f"Dashboard Web Modern Karhutla aktif di: {url}")
    print(f"Broker MQTT: {args.broker_host}:{args.broker_port} | Log MQTT otomatis: {eng.get_sim_status()['log_file']}")

    if args.auto_sim:
        eng.control_sim({
            "action": "start",
            "scenario": args.auto_sim,
            "speed": args.speed,
            "ai_preempt": args.ai_preempt,
        })

    if not args.no_browser:
        try:
            subprocess.Popen(["xdg-open", url], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except Exception:
            pass

    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        print("\nMenutup dashboard ...")
    finally:
        eng.shutdown()
        srv.shutdown()
        srv.server_close()


if __name__ == "__main__":
    main()
