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
import csv
import datetime
import http.server
import io
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


def detect_lan_ip_info():
    """Deteksi IP LAN laptop beserta status apakah nilai berasal dari fallback."""
    for target_ip in ("8.8.8.8", "192.168.1.1", "10.0.0.1"):
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
                s.connect((target_ip, 80))
                ip = s.getsockname()[0]
                if ip and not ip.startswith("127."):
                    return ip, False
        except OSError:
            pass
    try:
        host_ip = socket.gethostbyname(socket.gethostname())
        if host_ip and not host_ip.startswith("127."):
            return host_ip, False
    except OSError:
        pass
    return "192.168.1.10", True


def detect_lan_ip():
    """Deteksi IP LAN laptop agar mudah diakses dari HP."""
    return detect_lan_ip_info()[0]


def build_mobile_dashboard_config(broker_host=None, broker_port=None):
    """Hasilkan JSON konfigurasi siap-import untuk aplikasi HP (IoT MQTT Panel)."""
    if broker_host and broker_host not in ("localhost", "127.0.0.1", "0.0.0.0"):
        ip, is_fallback = broker_host, False
    else:
        ip, is_fallback = detect_lan_ip_info()
    port = int(broker_port or K.BROKER_PORT)
    return {
        "app": "IoT MQTT Panel",
        "version": 2,
        "generated_at": datetime.datetime.now().isoformat(timespec="seconds"),
        "web_dashboard_lan_url": f"http://{ip}:8080",
        "ip_is_fallback": is_fallback,
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
        self.latest = self._idle_latest()
        self._lan_ip, self._ip_is_fallback = detect_lan_ip_info()
        self.broker_connected = False
        self.node_status = "offline"

        # Embedded SITL / HILS state
        self.sim_target = "sitl"
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
        self.sim_ref_fw = None
        self.sim_fc = None
        self.sim_pre = None
        self.sim_csv = None
        self._sim_stop_evt = threading.Event()
        self._sim_thread = None
        self._hils_cond = threading.Condition()
        self._hils_by_seq = {}
        self._hils_meta = {}
        self.hils_timeouts = 0
        self.hils_consec_timeouts = 0
        self.hils_parity = {
            "samples": 0,
            "state_matches": 0,
            "max_delta_fi": 0.0,
            "last_delta_fi": None,
            "last_delta_pm": None,
        }
        self._last_cmd_error = None
        self._last_sim_error = None
        self._run_t_bahaya = None
        self._run_preempt_used = False
        self._last_run_summary = None

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
            # Topik skalar berfrekuensi tinggi (karhutla/t/* & karhutla/f/*) tidak perlu membanjiri log & SSE browser
            if event == "RECV" and (topic.startswith(f"{K.T_TELEM_SCALAR}/") or topic.startswith(f"{K.T_FORECAST_SCALAR}/")):
                return ""
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
                elif self.sim_running and self.sim_target == "hils" and self.sim_ref_fw is not None:
                    with self.lock:
                        self.sim_ref_fw.apply_command(payload)
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
                    if "seq" in tel:
                        try:
                            s_val = int(tel["seq"])
                            with self._hils_cond:
                                self._hils_by_seq[s_val] = tel
                                self._hils_cond.notify_all()
                        except (TypeError, ValueError):
                            pass
                    self._ingest_external(tel, "esp32")

        self.client = mqtt_io.make_client(
            f"karhutla-dashboard-{int(time.time() * 1000) % 100000}",
            on_connect=on_conn,
            on_message=on_msg,
            publish_status=False,
            mqtt_logger=self.mqtt_logger,
        )
        prev_disc = getattr(self.client, "on_disconnect", None)

        def on_disc(c, u, *args):
            with self.lock:
                self.broker_connected = False
            self._broadcast({"type": "status", "broker_connected": False, "node_status": self.node_status})
            if callable(prev_disc):
                try:
                    prev_disc(c, u, *args)
                except Exception:
                    pass

        self.client.on_disconnect = on_disc
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
        hils_meta = None
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
            if is_esp:
                hils_meta = self._hils_meta.pop(seq_val, None)
            fc = self._esp_fc if is_esp else self._external_fc
        f = fc.update(t_val, float(tel.get("pm", 0.0)))
        tel_pm = float(tel.get("pm", 0.0))
        tel_fi = float(tel.get("fi", 0.0))
        tel_state = str(tel.get("state", "NORMAL"))
        delta_fi = round(abs(tel_fi - float(hils_meta["ref_fi"])), 3) if hils_meta else None
        delta_pm = round(abs(tel_pm - float(hils_meta["ref_pm"])), 3) if hils_meta else None
        parity_match = (tel_state == hils_meta["ref_state"] and abs(tel_fi - float(hils_meta["ref_fi"])) < 1.0) if hils_meta else None
        row = _clean_dict({
            "t": t_val,
            "seq": seq_val,
            "scenario": (f"hils:{hils_meta['scenario']}" if hils_meta else "esp32-live") if is_esp else "mqtt-live",
            "pm_true": hils_meta["pm_true"] if hils_meta else None,
            "gas_true": hils_meta["gas_true"] if hils_meta else None,
            "ref_pm": hils_meta["ref_pm"] if hils_meta else None,
            "ref_fi": hils_meta["ref_fi"] if hils_meta else None,
            "ref_state": hils_meta["ref_state"] if hils_meta else None,
            "parity_delta_fi": delta_fi,
            "parity_delta_pm": delta_pm,
            "parity_match": parity_match,
            "v_dust": float(tel.get("v_dust", K.DUST_V0)),
            "v_mq": float(tel.get("v_mq", 1.0)),
            "pm_raw": float(tel.get("pm_raw", 0.0)),
            "pm": tel_pm,
            "dcdt": float(tel.get("dcdt", 0.0)),
            "gas_ppm": float(tel.get("gas_ppm", K.MQ_ATM_PPM)),
            "g": float(tel.get("g", 0.0)),
            "fi": tel_fi,
            "v0": float(tel.get("v0", K.DUST_V0)),
            "state": tel_state,
            "mode": str(tel.get("mode", "AUTO")),
            "duty": float(tel.get("duty", 0.0)),
            "forecast": _clean_val(f.get("pm_forecast", 0.0)),
            "slope": _clean_val(f.get("slope", 0.0)),
            "eta_bahaya_s": _clean_val(f.get("eta_bahaya_s")),
            "mae": _clean_val(f.get("mae")),
            "sensor_fault": tel_state == "FAILSAFE",
        })
        with self.lock:
            if is_esp:
                self.esp_latest = row
                if hils_meta:
                    self.hils_parity["samples"] += 1
                    if parity_match or tel_state == hils_meta["ref_state"]:
                        self.hils_parity["state_matches"] += 1
                    self.hils_parity["last_delta_fi"] = delta_fi
                    self.hils_parity["last_delta_pm"] = delta_pm
                    self.hils_parity["max_delta_fi"] = round(
                        max(self.hils_parity["max_delta_fi"], delta_fi), 3
                    )
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
                "target": self.sim_target,
                "running": self.sim_running,
                "paused": self.sim_paused,
                "scenario": self.sim_scenario,
                "speed": self.sim_speed,
                "duration": self.sim_duration,
                "ai_preempt": self.sim_ai_preempt,
                "ai_preempt_active": bool(
                    self.sim_running and self.sim_ai_preempt and self.sim_pre and getattr(self.sim_pre, "active", False)
                ),
                "fault_inject": self.sim_fault_inject,
                "step": self.sim_step_idx,
                "enable_mqtt": self.enable_mqtt,
                "broker_host": self.broker_host,
                "broker_port": self.broker_port,
                "broker_connected": self.broker_connected,
                "node_status": self.node_status,
                "hils_timeouts": self.hils_timeouts,
                "hils_consec_timeouts": self.hils_consec_timeouts,
                "hils_parity": dict(self.hils_parity),
                "has_sitl": bool(self.sim_running and self.sim_fw is not None),
                "has_hils_ref": bool(
                    self.sim_running and self.sim_target == "hils" and self.sim_ref_fw is not None
                ),
                "has_broker": bool(self.client is not None and self.broker_connected),
                "can_send_cmd": bool(
                    (self.sim_running and self.sim_fw is not None)
                    or (self.client is not None and self.broker_connected)
                ),
                "last_summary": dict(self._last_run_summary) if self._last_run_summary else None,
                "log_file": self.mqtt_logger.path or f"{self.out_dir}/mqtt_{datetime.date.today().isoformat()}.log",
            }

    def get_esp_status(self):
        with self.lock:
            age = None if self.esp_last_rx is None else round(time.time() - self.esp_last_rx, 1)
            return {"online": self.esp_online, "age_s": age, "hw_mode": self.esp_hw_mode,
                    "stale_s": self.esp_stale_s}

    @staticmethod
    def _idle_latest():
        return {
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
            "mobile": {"ip": self._lan_ip, "ip_is_fallback": self._ip_is_fallback},
            "mqtt_logs": logs,
            "sim": self.get_sim_status(),
            "thresholds": {"waspada": K.T_W_IN, "bahaya": K.T_B_IN, "horizon_s": K.FORECAST_HORIZON_S},
        }

    def send_cmd(self, cmd, target="auto"):
        clean_cmd = (cmd or "").strip().upper()
        if clean_cmd not in ("AUTO", "FORCE_MAX", "FORCE_OFF", "RESET"):
            self._last_cmd_error = "Perintah tidak dikenali."
            return False
        clean_target = (target or "auto").strip().lower()
        if clean_target not in ("auto", "sitl", "esp32"):
            clean_target = "auto"
        snap = None
        did_clear = False
        tel_now = None
        publish_needed = False
        with self.lock:
            has_sitl = bool(self.sim_running and self.sim_fw is not None)
            has_broker = bool(self.client is not None and self.broker_connected)
            apply_sitl = has_sitl and clean_target in ("auto", "sitl")
            apply_hils_ref = bool(
                self.sim_running and self.sim_target == "hils" and self.sim_ref_fw is not None
            ) and clean_target in ("auto", "esp32")
            publish_needed = has_broker and clean_target in ("auto", "esp32")
            if not (apply_sitl or apply_hils_ref or publish_needed):
                if clean_target == "sitl":
                    self._last_cmd_error = "Target SITL dipilih tetapi simulasi SITL tidak sedang berjalan."
                elif clean_target == "esp32":
                    self._last_cmd_error = "Target ESP32 dipilih tetapi broker MQTT tidak terhubung."
                else:
                    self._last_cmd_error = "Tidak ada simulasi SITL aktif dan broker MQTT tidak terhubung."
                return False
            if apply_sitl:
                tel_now = self.sim_fw.apply_command(clean_cmd)
                if tel_now is not None:
                    if clean_cmd == "RESET":
                        self.history.clear()
                        self.sim_step_idx = 0
                        self.sim_fc = LinearTrendForecaster()
                        self._run_t_bahaya = None
                        self._run_preempt_used = False
                        self._last_run_summary = None
                        self.latest = self._idle_latest()
                        self.latest["scenario"] = self.sim_scenario
                        self.latest["t"] = 0.0
                        self.latest["seq"] = 0
                        did_clear = True
                    for k in ("mode", "duty", "state", "pm", "pm_raw", "dcdt", "gas_ppm", "g", "fi", "v0"):
                        if k in tel_now:
                            self.latest[k] = tel_now[k]
                    if self.history and not did_clear:
                        self.history[-1] = dict(self.latest)
                    snap = dict(self.latest)
            if apply_hils_ref:
                self.sim_ref_fw.apply_command(clean_cmd)
                if clean_cmd == "RESET":
                    self.esp_history.clear()
                    self.esp_latest = None
                    self._hils_meta.clear()
                    self._run_t_bahaya = None
                    self._run_preempt_used = False
                    self._last_run_summary = None
                    self.hils_parity = {
                        "samples": 0,
                        "state_matches": 0,
                        "max_delta_fi": 0.0,
                        "last_delta_fi": None,
                        "last_delta_pm": None,
                    }
        if publish_needed:
            try:
                self.client.publish(K.T_CMD, clean_cmd)
                if tel_now is not None:
                    mqtt_io.publish_telemetry(self.client, tel_now)
            except Exception as exc:
                if tel_now is None and not apply_hils_ref:
                    self._last_cmd_error = f"Gagal mengirim ke broker MQTT: {exc}"
                    return False
        self._last_cmd_error = None
        if did_clear:
            self._broadcast({"type": "clear", "source": "sitl"})
        if snap is not None:
            self._broadcast({"type": "telemetry", "source": "sitl", "row": snap, "sim": self.get_sim_status()})
        return True

    def control_sim(self, params):
        action = (params.get("action") or "").strip().lower()
        if action and action not in ("start", "stop", "pause", "resume", "clear"):
            self._last_sim_error = f"Aksi simulasi tidak dikenali: {action}."
            return False
        self._last_sim_error = None
        with self.lock:
            can_retarget = (not self.sim_running) or (action == "start")
        if can_retarget and params.get("target") in ("sitl", "hils"):
            with self.lock:
                self.sim_target = params["target"]
        if can_retarget and "scenario" in params and params["scenario"] in SCENARIOS:
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
        if can_retarget and "duration" in params:
            try:
                dur = float(params["duration"])
                if dur > 0:
                    with self.lock:
                        self.sim_duration = dur
            except (TypeError, ValueError):
                pass
        if "ai_preempt" in params:
            with self.lock:
                self.sim_ai_preempt = bool(params["ai_preempt"])
                if self.sim_ai_preempt:
                    if self.sim_pre is None:
                        self.sim_pre = PreemptController()
                elif self.sim_pre is not None:
                    self.sim_pre.active = False
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
                self.hils_consec_timeouts = 0
        elif action == "clear":
            src = params.get("source", "sitl")
            with self.lock:
                if src == "esp32":
                    self.esp_history.clear()
                    self.esp_latest = None
                    self._esp_fc = LinearTrendForecaster()
                    self._hils_meta.clear()
                    self.hils_parity = {
                        "samples": 0,
                        "state_matches": 0,
                        "max_delta_fi": 0.0,
                        "last_delta_fi": None,
                        "last_delta_pm": None,
                    }
                else:
                    self.history.clear()
                    self.latest = self._idle_latest()
                    self._external_fc = LinearTrendForecaster()
                self._run_t_bahaya = None
                self._run_preempt_used = False
                self._last_run_summary = None
            self._broadcast({"type": "clear", "source": "esp32" if src == "esp32" else "sitl"})
        self._broadcast({"type": "sim", "sim": self.get_sim_status()})
        return True

    def _start_sim_thread(self):
        with self.lock:
            target = self.sim_target
            self._run_t_bahaya = None
            self._run_preempt_used = False
            self._last_run_summary = None
            if target == "hils":
                self.esp_history.clear()
                self.esp_latest = None
                self._esp_fc = LinearTrendForecaster()
                self.sim_ref_fw = Firmware()
                self._hils_meta.clear()
                self.hils_timeouts = 0
                self.hils_consec_timeouts = 0
                self.hils_parity = {
                    "samples": 0,
                    "state_matches": 0,
                    "max_delta_fi": 0.0,
                    "last_delta_fi": None,
                    "last_delta_pm": None,
                }
            else:
                self.sim_ref_fw = None
                self.history.clear()
            self.sim_plant = SmokeChamber(self.sim_scenario, seed=1)
            self.sim_fw = None if target == "hils" else Firmware()
            self.sim_fc = LinearTrendForecaster()
            self.sim_pre = PreemptController() if self.sim_ai_preempt else None
            self.sim_csv = DailyCsvLogger(self.out_dir, source=target)
            self.sim_step_idx = 0
            self.sim_running = True
            self.sim_paused = False
            self.node_status = "online"
            self._sim_stop_evt.clear()

        if target == "hils":
            with self._hils_cond:
                self._hils_by_seq.clear()
            self._broadcast({"type": "clear", "source": "esp32"})
        else:
            self._broadcast({"type": "clear", "source": "sitl"})

        if self.client:
            try:
                self.client.publish(K.T_STATUS, "online", retain=True)
            except Exception:
                pass

        loop_fn = self._hils_loop if target == "hils" else self._sim_loop
        self._sim_thread = threading.Thread(target=loop_fn, daemon=True)
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
                    if self._run_t_bahaya is None and str(tel["state"]) == "BAHAYA":
                        self._run_t_bahaya = round(float(t), 1)
                    if self.sim_pre is not None and getattr(self.sim_pre, "active", False):
                        self._run_preempt_used = True
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

        summary = None
        with self.lock:
            completed_naturally = (not self._sim_stop_evt.is_set()) and (self.sim_step_idx >= total_steps)
            self.sim_running = False
            if self.sim_csv:
                self.sim_csv.close()
                self.sim_csv = None
            if completed_naturally:
                peak_fi = max((float(r.get("fi", 0.0)) for r in self.history), default=0.0)
                last_t = float(self.history[-1].get("t", 0.0)) if self.history else 0.0
                summary = {
                    "target": self.sim_target,
                    "source": "sitl",
                    "scenario": self.sim_scenario,
                    "samples": len(self.history),
                    "peak_fi": round(peak_fi, 1),
                    "hils_parity": None,
                    "t_bahaya_s": self._run_t_bahaya,
                    "ai_preempt_used": bool(self._run_preempt_used),
                    "duration_s": round(float(last_t), 1),
                }
                self._last_run_summary = dict(summary)
        self._broadcast({"type": "sim", "sim": self.get_sim_status()})
        if summary is not None:
            self._broadcast({"type": "sim_complete", "summary": summary, "sim": self.get_sim_status()})

    def _wait_hils_seq(self, seq, vd, vm, timeout=4.0):
        """Tunggu balasan telemetri ESP32 untuk `seq`, dengan 1x retry di tengah timeout."""
        deadline = time.time() + timeout
        retried = False
        half_at = time.time() + timeout * 0.5
        while not self._sim_stop_evt.is_set():
            with self._hils_cond:
                if seq in self._hils_by_seq:
                    return self._hils_by_seq.pop(seq)
                now = time.time()
                if now >= deadline:
                    return None
                if not retried and now >= half_at:
                    retried = True
                    if self.client:
                        try:
                            self.client.publish(K.T_HILS_IN, f"{seq},{vd:.4f},{vm:.4f}")
                        except Exception:
                            pass
                wait_for = min(0.25, deadline - now)
                self._hils_cond.wait(timeout=max(0.01, wait_for))
        return None

    def _hils_loop(self):
        total_steps = int(self.sim_duration / K.TS)
        if self.client:
            try:
                self.client.publish(K.T_CMD, "RESET")
            except Exception:
                pass
            self._sim_stop_evt.wait(0.35)
            with self._hils_cond:
                self._hils_by_seq.clear()

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
                    vd_tx = round(float(vd), 4)
                    vm_tx = round(float(vm), 4)
                    speed = self.sim_speed
                    scenario = self.sim_scenario
                    pm_true_val = float(self.sim_plant.C)
                    gas_true_val = float(self.sim_plant.gas_ppm)
                    if k not in self._hils_meta and self.sim_ref_fw is not None:
                        ref_tel = self.sim_ref_fw.step(vd_tx, vm_tx, seq=k)
                        self._hils_meta[k] = {
                            "pm_true": pm_true_val,
                            "gas_true": gas_true_val,
                            "ref_pm": float(ref_tel["pm"]),
                            "ref_fi": float(ref_tel["fi"]),
                            "ref_state": str(ref_tel["state"]),
                            "ref_duty": float(ref_tel["duty"]),
                            "scenario": scenario,
                        }
                        for old_k in [ok for ok in self._hils_meta if ok < k - 60]:
                            self._hils_meta.pop(old_k, None)

            if paused:
                time.sleep(0.1)
                continue

            if self.client:
                try:
                    self.client.publish(K.T_HILS_IN, f"{k},{vd_tx:.4f},{vm_tx:.4f}")
                except Exception:
                    pass
            tel = self._wait_hils_seq(k, vd_tx, vm_tx, timeout=4.0)
            if self._sim_stop_evt.is_set():
                break

            if tel is not None:
                with self.lock:
                    self.hils_consec_timeouts = 0
                    f = self.sim_fc.update(t, float(tel.get("pm", 0.0)))
                    pre_cmd = None
                    if self.sim_ai_preempt and self.sim_pre is not None:
                        pre_cmd = self.sim_pre.decide(t, str(tel.get("mode", "AUTO")), f["pm_forecast"])
                    if pre_cmd and self.sim_ref_fw is not None:
                        self.sim_ref_fw.apply_command(pre_cmd)
                    if self._run_t_bahaya is None and str(tel.get("state", "")) == "BAHAYA":
                        self._run_t_bahaya = round(float(t), 1)
                    if self.sim_pre is not None and getattr(self.sim_pre, "active", False):
                        self._run_preempt_used = True
                    self.sim_plant.advance(float(tel.get("duty", 0.0)) / 100.0, t)
                    if self.sim_csv:
                        self.sim_csv.write(_clean_dict({
                            "t": t, "t_s": t, "seq": k, "scenario": scenario,
                            "pm_true": pm_true_val, "gas_true": gas_true_val,
                            "v_dust": vd, "v_mq": vm,
                            "pm_raw": tel.get("pm_raw", 0.0), "pm": tel.get("pm", 0.0),
                            "dcdt": tel.get("dcdt", 0.0), "gas_ppm": tel.get("gas_ppm", K.MQ_ATM_PPM),
                            "g": tel.get("g", 0.0), "fi": tel.get("fi", 0.0), "v0": tel.get("v0", K.DUST_V0),
                            "state": tel.get("state", "NORMAL"), "mode": tel.get("mode", "AUTO"),
                            "duty": tel.get("duty", 0.0),
                            "forecast": f.get("pm_forecast", 0.0), "slope": f.get("slope", 0.0),
                            "eta_bahaya_s": f.get("eta_bahaya_s"), "mae": f.get("mae"),
                            "sensor_fault": fault_now,
                        }))
                if self.client:
                    try:
                        mqtt_io.publish_forecast(self.client, f)
                        if pre_cmd:
                            self.client.publish(K.T_CMD, pre_cmd)
                    except Exception:
                        pass
            else:
                with self.lock:
                    self.hils_timeouts += 1
                    self.hils_consec_timeouts += 1
                    self.sim_step_idx = max(0, self.sim_step_idx - 1)
                    auto_paused = self.hils_consec_timeouts >= 3
                    if auto_paused:
                        self.sim_paused = True
                    timeouts = self.hils_timeouts
                    consec = self.hils_consec_timeouts
                self._broadcast({
                    "type": "hils_timeout",
                    "seq": k,
                    "timeouts": timeouts,
                    "consec": consec,
                    "auto_paused": auto_paused,
                    "sim": self.get_sim_status(),
                })

            self._broadcast({"type": "sim", "sim": self.get_sim_status()})
            wait_s = max(0.005, K.TS / max(0.1, speed))
            self._sim_stop_evt.wait(wait_s)

        summary = None
        with self.lock:
            completed_naturally = (not self._sim_stop_evt.is_set()) and (self.sim_step_idx >= total_steps)
            self.sim_running = False
            if self.sim_csv:
                self.sim_csv.close()
                self.sim_csv = None
            if completed_naturally:
                peak_fi = max((float(r.get("fi", 0.0)) for r in self.esp_history), default=0.0)
                last_t = float(self.esp_history[-1].get("t", 0.0)) if self.esp_history else 0.0
                summary = {
                    "target": self.sim_target,
                    "source": "esp32",
                    "scenario": self.sim_scenario,
                    "samples": len(self.esp_history),
                    "peak_fi": round(peak_fi, 1),
                    "hils_parity": dict(self.hils_parity),
                    "t_bahaya_s": self._run_t_bahaya,
                    "ai_preempt_used": bool(self._run_preempt_used),
                    "duration_s": round(float(last_t), 1),
                }
                self._last_run_summary = dict(summary)
        self._broadcast({"type": "sim", "sim": self.get_sim_status()})
        if summary is not None:
            self._broadcast({"type": "sim_complete", "summary": summary, "sim": self.get_sim_status()})

    def stop_sim(self):
        self._sim_stop_evt.set()
        with self._hils_cond:
            self._hils_cond.notify_all()
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
  <title>Karhutla Edge-IoT — Pusat Telemetri &amp; Kendali</title>
  <style>
    :root {
      color-scheme: light dark;
      --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;

      --bg-canvas-light: #f3f4f6;
      --bg-canvas-dark: #0e1114;
      --bg-surface-light: #ffffff;
      --bg-surface-dark: #161a1f;
      --bg-elevated-light: #e8ecf1;
      --bg-elevated-dark: #1f252c;
      --text-main-light: #111827;
      --text-main-dark: #f3f5f7;
      --text-muted-light: #4b5563;
      --text-muted-dark: #9ca7b4;
      --border-subtle-light: #c9d2dc;
      --border-subtle-dark: #333c47;

      --brand-ember-light: #c2410c;
      --brand-ember-dark: #fb923c;
      --ember-fg-light: #ffffff;
      --ember-fg-dark: #111315;
      --accent-light: #0369a1;
      --accent-dark: #38bdf8;
      --accent-fg-light: #ffffff;
      --accent-fg-dark: #0c1117;
      --ok-light: #14532d;
      --ok-dark: #4ade80;
      --warn-light: #92400e;
      --warn-dark: #fbbf24;
      --danger-light: #b91c1c;
      --danger-dark: #f87171;
      --violet-light: #6d28d9;
      --violet-dark: #e879f9;
      --teal-light: #0f766e;
      --teal-dark: #2dd4bf;

      --bg-canvas: var(--bg-canvas-light);
      --bg-surface: var(--bg-surface-light);
      --bg-elevated: var(--bg-elevated-light);
      --text-main: var(--text-main-light);
      --text-muted: var(--text-muted-light);
      --border-subtle: var(--border-subtle-light);
      --brand-ember: var(--brand-ember-light);
      --ember-fg: var(--ember-fg-light);
      --accent: var(--accent-light);
      --accent-fg: var(--accent-fg-light);
      --ok: var(--ok-light);
      --warn: var(--warn-light);
      --danger: var(--danger-light);
      --violet: var(--violet-light);
      --teal: var(--teal-light);

      --scrollbar-thumb: #64748b;
      --scrollbar-track: #e2e8f0;

      @media (prefers-color-scheme: dark) {
        --bg-canvas: var(--bg-canvas-dark);
        --bg-surface: var(--bg-surface-dark);
        --bg-elevated: var(--bg-elevated-dark);
        --text-main: var(--text-main-dark);
        --text-muted: var(--text-muted-dark);
        --border-subtle: var(--border-subtle-dark);
        --brand-ember: var(--brand-ember-dark);
        --ember-fg: var(--ember-fg-dark);
        --accent: var(--accent-dark);
        --accent-fg: var(--accent-fg-dark);
        --ok: var(--ok-dark);
        --warn: var(--warn-dark);
        --danger: var(--danger-dark);
        --violet: var(--violet-dark);
        --teal: var(--teal-dark);
        --scrollbar-thumb: #9ca7b4;
        --scrollbar-track: #161a1f;
      }

      @supports (color: light-dark(white, black)) {
        --bg-canvas: light-dark(var(--bg-canvas-light), var(--bg-canvas-dark));
        --bg-surface: light-dark(var(--bg-surface-light), var(--bg-surface-dark));
        --bg-elevated: light-dark(var(--bg-elevated-light), var(--bg-elevated-dark));
        --text-main: light-dark(var(--text-main-light), var(--text-main-dark));
        --text-muted: light-dark(var(--text-muted-light), var(--text-muted-dark));
        --border-subtle: light-dark(var(--border-subtle-light), var(--border-subtle-dark));
        --brand-ember: light-dark(var(--brand-ember-light), var(--brand-ember-dark));
        --ember-fg: light-dark(var(--ember-fg-light), var(--ember-fg-dark));
        --accent: light-dark(var(--accent-light), var(--accent-dark));
        --accent-fg: light-dark(var(--accent-fg-light), var(--accent-fg-dark));
        --ok: light-dark(var(--ok-light), var(--ok-dark));
        --warn: light-dark(var(--warn-light), var(--warn-dark));
        --danger: light-dark(var(--danger-light), var(--danger-dark));
        --violet: light-dark(var(--violet-light), var(--violet-dark));
        --teal: light-dark(var(--teal-light), var(--teal-dark));
        --scrollbar-thumb: light-dark(#64748b, #9ca7b4);
        --scrollbar-track: light-dark(#e2e8f0, #161a1f);
      }

      accent-color: var(--accent);
      scrollbar-color: var(--scrollbar-thumb) var(--scrollbar-track);
    }

    :root[data-theme="light"] {
      color-scheme: light;
      --bg-canvas: var(--bg-canvas-light);
      --bg-surface: var(--bg-surface-light);
      --bg-elevated: var(--bg-elevated-light);
      --text-main: var(--text-main-light);
      --text-muted: var(--text-muted-light);
      --border-subtle: var(--border-subtle-light);
      --brand-ember: var(--brand-ember-light);
      --ember-fg: var(--ember-fg-light);
      --accent: var(--accent-light);
      --accent-fg: var(--accent-fg-light);
      --ok: var(--ok-light);
      --warn: var(--warn-light);
      --danger: var(--danger-light);
      --violet: var(--violet-light);
      --teal: var(--teal-light);
      --scrollbar-thumb: #64748b;
      --scrollbar-track: #e2e8f0;
    }

    :root[data-theme="dark"] {
      color-scheme: dark;
      --bg-canvas: var(--bg-canvas-dark);
      --bg-surface: var(--bg-surface-dark);
      --bg-elevated: var(--bg-elevated-dark);
      --text-main: var(--text-main-dark);
      --text-muted: var(--text-muted-dark);
      --border-subtle: var(--border-subtle-dark);
      --brand-ember: var(--brand-ember-dark);
      --ember-fg: var(--ember-fg-dark);
      --accent: var(--accent-dark);
      --accent-fg: var(--accent-fg-dark);
      --ok: var(--ok-dark);
      --warn: var(--warn-dark);
      --danger: var(--danger-dark);
      --violet: var(--violet-dark);
      --teal: var(--teal-dark);
      --scrollbar-thumb: #9ca7b4;
      --scrollbar-track: #161a1f;
    }

    @media (forced-colors: active) {
      :root {
        --border-subtle: CanvasText;
        --accent: Highlight;
        --accent-fg: HighlightText;
      }
    }

    * { box-sizing: border-box; margin: 0; padding: 0; }

    ::selection {
      background: color-mix(in oklab, var(--accent), transparent 72%);
      color: var(--text-main);
    }

    body {
      font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      background-color: var(--bg-canvas);
      color: var(--text-main);
      caret-color: var(--accent);
      min-block-size: 100dvb;
      padding: 12px clamp(10px, 2.2vw, 24px);
      line-height: 1.5;
    }

    .sr-only {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip-path: inset(50%);
      white-space: nowrap;
    }

    svg.ico {
      inline-size: 16px;
      block-size: 16px;
      flex-shrink: 0;
      display: block;
    }

    code.topic-tag, kbd.key-hint {
      font-family: var(--font-mono);
      font-size: 0.75rem;
      padding: 1px 6px;
      border-radius: 5px;
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      color: var(--text-main);
    }

    kbd.key-hint {
      font-weight: 700;
      padding: 0 5px;
      line-height: 1.4;
      color: var(--text-muted);
    }

    .btn-primary kbd.key-hint {
      color: var(--text-main);
      background: var(--bg-surface);
      border-color: color-mix(in oklab, var(--brand-ember), transparent 35%);
    }

    kbd.key-hint[hidden] { display: none; }

    .src-btn[aria-pressed="true"] kbd.key-hint,
    .preset-btn[aria-pressed="true"] kbd.key-hint,
    .cmd-btn[aria-pressed="true"]:not(#btn-sim-pause) kbd.key-hint {
      color: var(--accent-fg);
      background: color-mix(in oklab, var(--accent-fg), transparent 85%);
      border-color: color-mix(in oklab, var(--accent-fg), transparent 65%);
    }

    #btn-sim-pause[aria-pressed="true"] kbd.key-hint {
      color: var(--text-main);
      background: var(--bg-surface);
      border-color: var(--border-subtle);
    }

    .page-frame {
      max-inline-size: 1520px;
      margin-inline: auto;
      display: flex;
      flex-direction: column;
      gap: 12px;
    }

    .topbar {
      background: var(--bg-surface);
      border: 1px solid var(--border-subtle);
      border-radius: 14px;
      padding: 10px 16px;
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 10px 14px;
      box-shadow: 0 4px 18px -6px color-mix(in oklab, var(--text-main), transparent 90%);
    }

    .brand-group {
      display: flex;
      align-items: center;
      gap: 12px;
    }

    .brand-badge {
      inline-size: 38px;
      block-size: 38px;
      border-radius: 10px;
      background: color-mix(in oklab, var(--brand-ember), transparent 84%);
      border: 1px solid color-mix(in oklab, var(--brand-ember), transparent 45%);
      color: var(--brand-ember);
      display: grid;
      place-content: center;
      flex-shrink: 0;
    }

    .brand-title {
      font-size: 1.35rem;
      font-weight: 700;
      letter-spacing: -0.02em;
      line-height: 1.2;
    }

    @media (max-width: 767px) {
      .brand-title {
        font-size: 1.3rem;
      }
    }

    .brand-sub {
      font-size: 0.8125rem;
      color: var(--text-muted);
    }

    .status-cluster {
      margin-inline-start: auto;
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 8px;
    }

    .topbar-actions {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 8px;
    }

    .theme-switch {
      display: inline-flex;
      padding: 2px;
      border-radius: 999px;
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      gap: 2px;
    }

    .theme-switch .theme-btn {
      min-block-size: 30px;
      padding: 4px 10px;
      border: none;
      background: transparent;
    }

    .pill-badge {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 5px 11px;
      border-radius: 999px;
      font-size: 0.75rem;
      font-weight: 600;
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      color: var(--text-main);
    }

    .pill-badge[hidden] { display: none; }

    .status-dot {
      inline-size: 8px;
      block-size: 8px;
      border-radius: 50%;
      background: currentColor;
      flex-shrink: 0;
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
      background: color-mix(in oklab, var(--danger), transparent 86%);
      border-color: color-mix(in oklab, var(--danger), transparent 40%);
      color: var(--danger);
    }

    .pill-badge[data-tone="accent"] {
      background: color-mix(in oklab, var(--accent), transparent 86%);
      border-color: color-mix(in oklab, var(--accent), transparent 45%);
      color: var(--accent);
    }

    .safety-callout {
      border-radius: 12px;
      padding: 10px 14px;
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
      font-size: 0.875rem;
      font-weight: 600;
      border: 1px solid var(--border-subtle);
      background: var(--bg-surface);
      color: var(--text-main);
    }

    .safety-callout[hidden] { display: none; }

    .safety-callout[data-tone="danger"] {
      background: color-mix(in oklab, var(--bg-surface), var(--danger) 12%);
      border-color: color-mix(in oklab, var(--danger), transparent 30%);
      color: var(--danger);
    }

    .safety-callout[data-tone="warn"] {
      background: color-mix(in oklab, var(--bg-surface), var(--warn) 12%);
      border-color: color-mix(in oklab, var(--warn), transparent 30%);
      color: var(--warn);
    }

    .safety-callout[data-tone="accent"] {
      background: color-mix(in oklab, var(--bg-surface), var(--accent) 12%);
      border-color: color-mix(in oklab, var(--accent), transparent 30%);
      color: var(--accent);
    }

    .callout-msg {
      display: flex;
      align-items: center;
      gap: 8px;
    }

    .callout-actions {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 6px;
    }

    .dashboard-shell {
      display: grid;
      grid-template-columns: 1fr;
      grid-template-areas:
        "kpis"
        "workspace";
      gap: 12px;
    }

    /* KPI Subgrid Section — State & FI emphasized, symmetric breakpoints 6 -> 3 -> 2 */
    .kpi-grid {
      grid-area: kpis;
      display: grid;
      grid-template-columns: 1.45fr 1.15fr repeat(4, minmax(0, 1fr));
      grid-template-rows: auto 1fr auto;
      gap: 10px;
    }

    article.kpi-card {
      grid-row: span 3;
      display: grid;
      grid-template-rows: subgrid;
      container-type: inline-size;
      background: var(--bg-surface);
      border: 1px solid var(--border-subtle);
      border-radius: 14px;
      padding: 10px 12px;
      gap: 6px;
      box-shadow: 0 2px 10px -4px color-mix(in oklab, var(--text-main), transparent 92%);
    }

    article.kpi-card[data-primary="true"] {
      background: color-mix(in oklab, var(--bg-surface), var(--ok) 5%);
      border-color: color-mix(in oklab, var(--ok), transparent 48%);
    }

    article.kpi-card[data-primary="true"][data-tone="warn"] {
      background: color-mix(in oklab, var(--bg-surface), var(--warn) 8%);
      border-color: color-mix(in oklab, var(--warn), transparent 35%);
    }

    article.kpi-card[data-primary="true"][data-tone="danger"] {
      background: color-mix(in oklab, var(--bg-surface), var(--danger) 11%);
      border-color: color-mix(in oklab, var(--danger), transparent 25%);
    }

    article.kpi-card[data-primary="true"][data-tone="idle"] {
      background: var(--bg-surface);
      border-color: var(--border-subtle);
    }

    article.kpi-card > header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 6px;
      font-size: 0.75rem;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.03em;
      color: var(--text-muted);
    }

    .kpi-value {
      font-family: var(--font-mono);
      font-size: clamp(1.25rem, 9cqi, 1.75rem);
      font-weight: 700;
      letter-spacing: -0.02em;
      font-variant-numeric: tabular-nums;
      display: flex;
      align-items: baseline;
      gap: 5px;
      flex-wrap: wrap;
    }

    .kpi-meter {
      inline-size: 100%;
      block-size: 6px;
      border-radius: 999px;
      background: var(--bg-elevated);
      border: 1px solid color-mix(in oklab, var(--border-subtle), transparent 30%);
      position: relative;
      overflow: hidden;
      margin-block-start: 4px;
    }

    .kpi-meter-fill {
      block-size: 100%;
      border-radius: 999px;
      inline-size: 0%;
      background: var(--ok);
      transition: inline-size 160ms ease, background-color 160ms ease;
    }

    .fi-tick {
      position: absolute;
      inset-block: 0;
      inline-size: 1.5px;
      background: var(--text-muted);
      opacity: 0.7;
    }

    .kpi-parity-tag {
      font-size: 0.75rem;
      font-weight: 700;
    }

    .kpi-parity-tag[hidden] { display: none; }

    .kpi-unit {
      font-family: system-ui, sans-serif;
      font-size: 0.8125rem;
      font-weight: 600;
      color: var(--text-muted);
    }

    article.kpi-card > footer {
      font-family: var(--font-mono);
      font-size: 0.75rem;
      color: var(--text-muted);
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 6px;
      flex-wrap: wrap;
      padding-block-start: 6px;
      border-block-start: 1px dashed color-mix(in oklab, var(--border-subtle), transparent 30%);
      font-variant-numeric: tabular-nums;
    }

    .workspace {
      grid-area: workspace;
      display: grid;
      grid-template-columns: minmax(0, 2.15fr) minmax(350px, 1fr);
      gap: 12px;
      align-items: start;
    }

    .control-column {
      display: flex;
      flex-direction: column;
      gap: 12px;
    }

    .panel-card {
      container-type: inline-size;
      background: var(--bg-surface);
      border: 1px solid var(--border-subtle);
      border-radius: 14px;
      padding: 14px 16px;
      display: flex;
      flex-direction: column;
      gap: 10px;
      box-shadow: 0 4px 18px -6px color-mix(in oklab, var(--text-main), transparent 90%);
    }

    .panel-header {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: space-between;
      gap: 8px 12px;
    }

    .panel-title {
      font-size: 1rem;
      font-weight: 700;
      display: flex;
      align-items: center;
      gap: 8px;
    }

    .panel-subtitle-row {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 8px;
      margin-block-start: 2px;
    }

    .preset-bar {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      padding: 7px 10px;
      border-radius: 10px;
      background: var(--bg-elevated);
      border: 1px solid color-mix(in oklab, var(--border-subtle), transparent 25%);
    }

    .preset-label {
      font-size: 0.75rem;
      font-weight: 700;
      color: var(--text-muted);
    }

    .chart-legend {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 6px 14px;
      padding: 4px 2px;
      font-size: 0.75rem;
      color: var(--text-main);
    }

    .legend-item {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      font-weight: 600;
      white-space: nowrap;
    }

    .legend-swatch {
      inline-size: 18px;
      block-size: 0;
      border-block-start: 3px solid currentColor;
      flex-shrink: 0;
    }

    .legend-swatch[data-style="dashed"] {
      border-block-start-style: dashed;
    }

    .legend-swatch[data-style="dotted"] {
      border-block-start-style: dotted;
    }

    [role="toolbar"] {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 6px;
    }

    [role="toolbar"] [role="group"] {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 5px;
    }

    [role="separator"][aria-orientation="vertical"] {
      inline-size: 0;
      block-size: 18px;
      border-inline-start: 1px solid var(--border-subtle);
      margin-inline: 2px;
    }

    .chip-btn, .cmd-btn {
      min-inline-size: 32px;
      min-block-size: 34px;
      padding: 6px 12px;
      border-radius: 999px;
      border: 1px solid var(--border-subtle);
      background: var(--bg-elevated);
      color: var(--text-main);
      font-size: 0.75rem;
      font-weight: 600;
      cursor: pointer;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 6px;
      white-space: nowrap;
    }

    .chip-btn[hidden], .cmd-btn[hidden] { display: none; }

    .chip-btn[disabled], .cmd-btn[disabled] {
      opacity: 0.45;
      cursor: not-allowed;
    }

    .cmd-btn[aria-busy="true"] {
      opacity: 0.6;
      cursor: wait;
    }

    button:focus-visible, select:focus-visible, input:focus-visible, summary:focus-visible, [tabindex="0"]:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: 2px;
    }

    .src-btn[aria-pressed="true"],
    .spd-btn[aria-pressed="true"],
    .tgt-btn[aria-pressed="true"],
    .preset-btn[aria-pressed="true"],
    .theme-btn[aria-pressed="true"],
    .win-btn[aria-pressed="true"] {
      background: var(--accent);
      border-color: var(--accent);
      color: var(--accent-fg);
      font-weight: 700;
    }

    .layer-btn[aria-pressed="true"],
    #btn-toggle-layers[aria-expanded="true"],
    #btn-sim-pause[aria-pressed="true"] {
      background: color-mix(in oklab, var(--chip-color, var(--accent)), transparent 74%);
      border-color: var(--chip-color, var(--accent));
      color: var(--text-main);
      box-shadow: inset 0 0 0 1px var(--chip-color, var(--accent));
      font-weight: 700;
    }

    .chip-dot {
      inline-size: 8px;
      block-size: 8px;
      border-radius: 50%;
      background: var(--chip-color, var(--accent));
      flex-shrink: 0;
    }

    .compact-drawer {
      border-radius: 10px;
      border: 1px solid var(--border-subtle);
      background: var(--bg-elevated);
      padding: 10px 12px;
      display: flex;
      flex-direction: column;
      gap: 10px;
    }

    .compact-drawer[hidden] { display: none; }

    details.drawer-card > summary {
      cursor: pointer;
      list-style: none;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      font-size: 0.875rem;
      font-weight: 700;
      color: var(--text-main);
      user-select: none;
    }

    details.drawer-card > summary::-webkit-details-marker {
      display: none;
    }

    .drawer-chev {
      transition: transform 150ms ease;
      color: var(--text-muted);
    }

    details[open] .drawer-chev {
      transform: rotate(180deg);
    }

    .drawer-body {
      margin-block-start: 8px;
      padding-block-start: 10px;
      border-block-start: 1px dashed var(--border-subtle);
      display: flex;
      flex-direction: column;
      gap: 10px;
    }

    .canvas-wrap {
      position: relative;
    }

    .offline-banner {
      position: absolute;
      inset-block-start: 10px;
      inset-inline: 10px;
      z-index: 3;
      padding: 8px 14px;
      border-radius: 10px;
      font-size: 0.8125rem;
      font-weight: 700;
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      pointer-events: auto;
      color: var(--danger);
      background: color-mix(in oklab, var(--bg-surface), var(--danger) 14%);
      border: 1px solid color-mix(in oklab, var(--danger), transparent 35%);
    }

    .offline-banner[hidden] { display: none; }
    .canvas-wrap.is-stale canvas { opacity: 0.58; }

    canvas.chart-stage {
      display: block;
      inline-size: 100%;
      block-size: 320px;
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      border-radius: 12px;
      cursor: crosshair;
      touch-action: pan-y;
    }

    canvas.subchart-stage {
      display: block;
      inline-size: 100%;
      block-size: 150px;
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      border-radius: 12px;
      cursor: crosshair;
      touch-action: pan-y;
    }

    .cmd-grid {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 8px;
    }

    .cmd-btn {
      border-radius: 10px;
      min-block-size: 40px;
      padding: 8px 10px;
      font-size: 0.75rem;
    }

    .cmd-btn[aria-pressed="true"] {
      background: var(--accent);
      border-color: var(--accent);
      color: var(--accent-fg);
      font-weight: 700;
    }

    .btn-primary {
      background: var(--brand-ember);
      border-color: var(--brand-ember);
      color: var(--ember-fg);
      font-weight: 700;
    }

    .cmd-btn[data-variant="danger"],
    .chip-btn[data-variant="danger"] {
      border-color: color-mix(in oklab, var(--danger), transparent 35%);
      color: var(--danger);
      background: color-mix(in oklab, var(--bg-elevated), var(--danger) 8%);
    }

    .reset-zone {
      padding-block-start: 10px;
      border-block-start: 1px dashed var(--border-subtle);
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
    }

    .exec-top {
      display: grid;
      grid-template-columns: 1.3fr 1fr 1fr;
      gap: 8px;
    }

    .sim-zone {
      display: flex;
      flex-direction: column;
      gap: 6px;
      padding: 8px 10px;
      border-radius: 10px;
      background: var(--bg-elevated);
      border: 1px solid color-mix(in oklab, var(--border-subtle), transparent 20%);
    }

    .sim-active-bar {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      padding: 6px 8px;
      border-radius: 8px;
      background: var(--bg-surface);
      border: 1px solid var(--border-subtle);
      font-size: 0.8125rem;
      font-weight: 600;
    }

    .sim-active-bar[hidden], #sim-config-fields[hidden] { display: none; }

    .shortcut-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(215px, 1fr));
      gap: 6px 12px;
      margin-block-start: 8px;
      font-size: 0.8125rem;
      color: var(--text-muted);
    }

    .shortcut-grid strong {
      color: var(--text-main);
    }

    .sim-zone-label {
      font-size: 0.75rem;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.04em;
      color: var(--text-muted);
    }

    .op-list {
      display: flex;
      flex-direction: column;
      gap: 8px;
    }

    .op-row {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      padding: 6px 8px;
      border-radius: 8px;
      background: var(--bg-surface);
      border: 1px solid color-mix(in oklab, var(--border-subtle), transparent 35%);
      font-size: 0.8125rem;
    }

    .op-select {
      background: var(--bg-surface);
      color: var(--text-main);
      border: 1px solid var(--border-subtle);
      border-radius: 8px;
      padding: 6px 10px;
      font-size: 0.75rem;
      font-weight: 600;
      min-block-size: 34px;
      max-inline-size: 100%;
    }

    .hils-hint {
      font-size: 0.75rem;
      color: var(--warn);
      background: color-mix(in oklab, var(--warn), transparent 88%);
      border: 1px solid color-mix(in oklab, var(--warn), transparent 50%);
      border-radius: 8px;
      padding: 6px 10px;
    }

    .hils-hint[hidden] { display: none; }

    .touch-label {
      font-size: 0.75rem;
      display: inline-flex;
      align-items: center;
      gap: 8px;
      cursor: pointer;
    }

    /* Custom accessible switch pill preserving input[type="checkbox"][switch] */
    input[type="checkbox"][switch] {
      appearance: none;
      -webkit-appearance: none;
      position: relative;
      inline-size: 40px;
      block-size: 22px;
      border-radius: 999px;
      background: color-mix(in oklab, var(--text-muted), transparent 62%);
      border: 1px solid var(--border-subtle);
      cursor: pointer;
      flex-shrink: 0;
    }

    input[type="checkbox"][switch]::before {
      content: "";
      position: absolute;
      inset-block-start: 2px;
      inset-inline-start: 2px;
      inline-size: 16px;
      block-size: 16px;
      border-radius: 50%;
      background: #ffffff;
      box-shadow: 0 1px 3px rgba(0, 0, 0, 0.35);
    }

    input[type="checkbox"][switch]:checked {
      background: var(--accent);
      border-color: var(--accent);
    }

    input[type="checkbox"][switch]:checked::before {
      inset-inline-start: 20px;
      background: var(--accent-fg);
    }

    @media (forced-colors: active) {
      input[type="checkbox"][switch] {
        appearance: auto;
      }
      input[type="checkbox"][switch]::before {
        display: none;
      }
    }

    .mqtt-stream {
      max-block-size: 200px;
      overflow-y: auto;
      scrollbar-gutter: stable;
      overscroll-behavior: contain;
      scrollbar-width: thin;
      scrollbar-color: var(--scrollbar-thumb) var(--scrollbar-track);
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      border-radius: 10px;
      padding: 10px;
      font-family: var(--font-mono);
      font-size: 0.75rem;
      display: flex;
      flex-direction: column;
      gap: 4px;
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
      border-block-end: 1px solid color-mix(in oklab, var(--border-subtle), transparent 65%);
      padding-block-end: 3px;
    }

    .glossary-list {
      display: grid;
      grid-template-columns: 1fr;
      gap: 8px;
      font-size: 0.8125rem;
      color: var(--text-muted);
      margin-block-start: 10px;
    }

    .glossary-list strong {
      color: var(--text-main);
    }

    dialog.confirm-modal {
      margin: auto;
      max-inline-size: min(440px, calc(100vw - 32px));
      border-radius: 14px;
      border: 1px solid var(--border-subtle);
      background: var(--bg-surface);
      color: var(--text-main);
      padding: 20px;
      box-shadow: 0 20px 40px -12px rgba(0, 0, 0, 0.38);
    }

    dialog.help-modal {
      max-inline-size: min(620px, calc(100vw - 32px));
      max-block-size: min(84dvh, 720px);
      overflow-y: auto;
    }

    dialog.confirm-modal::backdrop {
      background: rgba(14, 17, 20, 0.65);
    }

    .modal-actions {
      display: flex;
      justify-content: flex-end;
      gap: 8px;
      margin-block-start: 16px;
    }

    .toast-region {
      position: fixed;
      inset-block-end: 16px;
      inset-inline-end: 16px;
      z-index: 30;
      display: flex;
      flex-direction: column;
      gap: 8px;
      max-inline-size: min(360px, calc(100vw - 32px));
      pointer-events: none;
    }

    .toast {
      pointer-events: auto;
      padding: 10px 14px;
      border-radius: 10px;
      font-size: 0.8125rem;
      font-weight: 600;
      background: var(--bg-surface);
      color: var(--text-main);
      border: 1px solid var(--border-subtle);
      box-shadow: 0 8px 24px -6px rgba(0, 0, 0, 0.24);
    }

    .toast[data-tone="ok"] {
      border-color: var(--ok);
      color: var(--ok);
    }

    .toast[data-tone="danger"] {
      border-color: var(--danger);
      color: var(--danger);
    }

    .toast[data-tone="warn"] {
      border-color: var(--warn);
      color: var(--warn);
    }

    .mobile-dock {
      display: none;
    }

    .brand-title-ext { font-weight: 600; color: var(--text-muted); }

    .tone-muted { color: var(--text-muted); }
    .tone-accent { color: var(--accent); }
    .tone-teal { color: var(--teal); }

    .fi-tick-waspada { inset-inline-start: 22%; }
    .fi-tick-bahaya { inset-inline-start: 60%; }

    .kpi-abbr-btn {
      background: transparent;
      border: none;
      padding: 0;
      font: inherit;
      color: inherit;
      cursor: help;
      text-decoration: underline dotted;
      text-decoration-color: color-mix(in oklab, var(--text-muted), transparent 40%);
      text-underline-offset: 3px;
    }

    .sim-adv-drawer {
      padding: 8px 10px;
      border-radius: 10px;
      background: var(--bg-elevated);
      border: 1px solid color-mix(in oklab, var(--border-subtle), transparent 20%);
    }

    .sim-adv-drawer > summary {
      cursor: pointer;
      list-style: none;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      font-size: 0.8125rem;
      font-weight: 700;
      color: var(--text-main);
      user-select: none;
    }

    .sim-adv-drawer > summary::-webkit-details-marker {
      display: none;
    }

    .sim-adv-body {
      margin-block-start: 8px;
      display: flex;
      flex-direction: column;
      gap: 8px;
    }

    .hover-dismiss-btn {
      position: absolute;
      inset-block-start: 8px;
      inset-inline-end: 10px;
      z-index: 4;
      box-shadow: 0 4px 12px -2px rgba(0, 0, 0, 0.28);
    }

    .shortcut-toggle-row {
      margin-block-start: 10px;
      padding: 8px 10px;
      border-radius: 8px;
      background: var(--bg-elevated);
      border: 1px solid var(--border-subtle);
      inline-size: 100%;
      justify-content: space-between;
    }

    /* Dominant State card: icon + label, full border & tint on active tones */
    .kpi-state-wrap {
      align-items: center;
      flex-wrap: nowrap;
      gap: 8px;
    }

    svg.kpi-state-ico {
      inline-size: 1em;
      block-size: 1em;
    }

    #card-state[data-tone="idle"] svg.kpi-state-ico {
      inline-size: 18px;
      block-size: 18px;
    }

    #card-state[data-tone="idle"] #kpi-state {
      font-size: clamp(0.95rem, 6.5cqi, 1.15rem);
      line-height: 1.25;
    }

    #card-state[data-tone="ok"] .kpi-state-wrap,
    #card-state[data-tone="warn"] .kpi-state-wrap,
    #card-state[data-tone="danger"] .kpi-state-wrap {
      font-size: clamp(1.45rem, 11.5cqi, 2.05rem);
    }

    article.kpi-card#card-state[data-tone="ok"] {
      background: color-mix(in oklab, var(--bg-surface), var(--ok) 9%);
      border-color: color-mix(in oklab, var(--ok), transparent 30%);
    }

    article.kpi-card#card-state[data-tone="warn"] {
      background: color-mix(in oklab, var(--bg-surface), var(--warn) 13%);
      border-color: color-mix(in oklab, var(--warn), transparent 20%);
    }

    article.kpi-card#card-state[data-tone="danger"] {
      background: color-mix(in oklab, var(--bg-surface), var(--danger) 16%);
      border-color: color-mix(in oklab, var(--danger), transparent 10%);
    }

    .preset-actions {
      display: flex;
      align-items: center;
      gap: 6px;
      flex-wrap: wrap;
    }

    .panel-head-row {
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 8px;
      flex-wrap: wrap;
    }

    .summary-trail {
      display: inline-flex;
      align-items: center;
      gap: 6px;
    }

    .btn-block { justify-content: center; }

    .confirm-desc {
      margin-block-start: 8px;
      font-size: 0.875rem;
      color: var(--text-main);
    }

    .help-head {
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 12px;
    }

    .help-subhead { margin-block-start: 14px; }

    .override-target-line {
      font-weight: 600;
      color: var(--text-main);
    }

    /* Layer chip swatches mirror chart series colors (JS syncs exact palette per theme) */
    .layer-btn[data-layer="fi"] { --chip-color: var(--brand-ember); }
    .layer-btn[data-layer="pm"] { --chip-color: var(--accent); }
    .layer-btn[data-layer="forecast"] { --chip-color: var(--violet); }
    .layer-btn[data-layer="thresholds"] { --chip-color: var(--warn); }
    .layer-btn[data-layer="pm_true"] { --chip-color: var(--ok); }
    .layer-btn[data-layer="pm_raw"] { --chip-color: #475569; }
    .layer-btn[data-layer="gas_ppm"] { --chip-color: var(--teal); }
    .layer-btn[data-layer="g_scaled"] { --chip-color: #854d0e; }
    .layer-btn[data-layer="voltages"] { --chip-color: #2563eb; }

    .chip-dot-vd { background: var(--chip-color-vd, #60a5fa); }
    .chip-dot-vm { background: var(--chip-color-vm, #a78bfa); margin-inline-start: -3px; }

    /* Persistent run summary strip */
    .run-summary {
      display: flex;
      flex-direction: column;
      gap: 8px;
      padding: 8px 12px;
      border-radius: 10px;
      background: color-mix(in oklab, var(--bg-surface), var(--ok) 7%);
      border: 1px solid color-mix(in oklab, var(--ok), transparent 55%);
    }

    .run-summary[hidden] { display: none; }

    .run-summary-head {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      font-size: 0.8125rem;
    }

    .run-summary-actions {
      display: flex;
      flex-wrap: wrap;
      gap: 6px;
    }

    .run-summary-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(118px, 1fr));
      gap: 6px 14px;
    }

    .run-summary-grid dt {
      font-size: 0.75rem;
      font-weight: 600;
      color: var(--text-muted);
    }

    .run-summary-grid dd {
      font-family: var(--font-mono);
      font-size: 0.875rem;
      font-weight: 700;
      font-variant-numeric: tabular-nums;
    }

    .run-summary-grid [hidden] { display: none; }

    .mob-dock-context {
      font-size: 0.75rem;
      font-weight: 600;
      color: var(--text-muted);
      text-align: center;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .mob-dock-tray {
      display: flex;
      gap: 6px;
      padding-block-end: 4px;
      border-block-end: 1px dashed var(--border-subtle);
    }

    .mob-dock-tray[hidden] { display: none; }

    .mob-dock-tray > .cmd-btn {
      flex: 1 1 0;
      min-inline-size: 0;
    }

    .mob-dock-row {
      display: flex;
      gap: 6px;
    }

    .mob-dock-row > .cmd-btn {
      flex: 1 1 0;
      min-inline-size: 0;
    }

    @media (min-width: 768px) {
      #badge-fsm { display: none; }
    }

    /* Projector 1280x768 fit: compact vertical rhythm without scrolling */
    @media (max-height: 820px) and (min-width: 1280px) {
      body { padding-block: 8px; }
      .page-frame, .dashboard-shell { gap: 8px; }
      .topbar { padding: 6px 14px; gap: 6px 12px; }
      .topbar .brand-sub { display: none; }
      .brand-badge { inline-size: 32px; block-size: 32px; }
      .brand-title { font-size: 1.35rem; line-height: 1.15; }
      .kpi-grid { gap: 8px; }
      article.kpi-card { padding: 7px 10px; gap: 3px; }
      .kpi-value { font-size: clamp(1.15rem, 8cqi, 1.5rem); line-height: 1.2; }
      #card-state[data-tone="ok"] .kpi-state-wrap,
      #card-state[data-tone="warn"] .kpi-state-wrap,
      #card-state[data-tone="danger"] .kpi-state-wrap { font-size: clamp(1.35rem, 10.5cqi, 1.85rem); }
      .kpi-meter { margin-block-start: 2px; }
      article.kpi-card > footer { padding-block-start: 4px; }
      .control-column { gap: 8px; }
      .panel-card { padding: 8px 11px; gap: 5px; }
      .panel-subtitle-row { margin-block-start: 0; }
      .preset-bar { padding: 4px 8px; }
      .chip-btn { min-block-size: 30px; padding: 4px 10px; }
      .cmd-btn { min-block-size: 34px; padding: 5px 8px; }
      .reset-zone { padding-block-start: 6px; }
      .sim-zone, .sim-adv-drawer { padding: 6px 8px; gap: 4px; }
      .op-list, .sim-adv-body { gap: 5px; }
      .op-row, .override-scope-row { padding: 4px 7px; }
      .op-select { min-block-size: 30px; padding: 4px 8px; }
      details.drawer-card > summary, .sim-adv-drawer > summary { font-size: 0.8125rem; }
      .theme-switch .theme-btn { min-block-size: 28px; }
      .chart-legend { padding: 0; gap: 4px 12px; }
      canvas.chart-stage { block-size: clamp(205px, 34dvh, 260px); }
      canvas.subchart-stage { block-size: clamp(108px, 16dvh, 128px); }
    }

    @media (max-width: 1279px) {
      .kpi-grid {
        grid-template-columns: repeat(3, minmax(0, 1fr));
        grid-template-rows: none;
      }
      article.kpi-card {
        grid-row: auto;
        grid-template-rows: auto 1fr auto;
      }
      .workspace {
        grid-template-columns: 1fr;
      }
    }

    @media (max-width: 767px) {
      body {
        padding-block-end: calc(98px + env(safe-area-inset-bottom));
      }
      .topbar {
        padding: 8px 10px;
        gap: 6px 8px;
      }
      .topbar .brand-sub, .brand-title-ext, #override-panel {
        display: none;
      }
      .brand-badge {
        inline-size: 32px;
        block-size: 32px;
      }
      .status-cluster {
        margin-inline-start: 0;
      }
      .kpi-grid {
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 8px;
      }
      article.kpi-card {
        padding: 10px 11px;
      }
      .cmd-grid {
        grid-template-columns: 1fr;
      }
      .exec-top {
        grid-template-columns: 1fr 1fr;
      }
      .exec-top #btn-sim-start {
        grid-column: span 2;
      }
      canvas.chart-stage {
        block-size: 250px;
      }
      canvas.subchart-stage {
        block-size: 132px;
      }
      .mobile-dock {
        display: flex;
        flex-direction: column;
        gap: 4px;
        position: fixed;
        inset-block-end: 0;
        inset-inline: 0;
        z-index: 25;
        padding: 8px 10px max(8px, env(safe-area-inset-bottom));
        background: var(--bg-surface);
        border-block-start: 1px solid var(--border-subtle);
        box-shadow: 0 -6px 20px -6px color-mix(in oklab, var(--text-main), transparent 85%);
      }
      .mobile-dock .cmd-btn {
        min-block-size: 44px;
        padding: 6px 4px;
        font-size: 0.75rem;
      }
      .toast-region {
        inset-block-end: calc(102px + env(safe-area-inset-bottom));
      }
    }

    @media (pointer: coarse), (max-width: 767px) {
      kbd.key-hint {
        display: none;
      }
    }

    @media (pointer: coarse) {
      .chip-btn, .cmd-btn, .op-select, .touch-label, .theme-switch .theme-btn, details.drawer-card > summary, .sim-adv-drawer > summary {
        min-block-size: 44px;
      }
      .chip-btn {
        padding: 8px 12px;
      }
    }

    @media (hover: hover) and (prefers-reduced-motion: no-preference) {
      .chip-btn, .cmd-btn, input[type="checkbox"][switch]::before {
        transition: background-color 140ms ease, border-color 140ms ease, inset-inline-start 140ms ease;
      }
      .chip-btn:hover:not([disabled]), .cmd-btn:hover:not([disabled]) {
        border-color: var(--accent);
      }
    }
  </style>
</head>
<body>
  <div id="safety-live" class="sr-only" role="alert" aria-live="assertive" aria-atomic="true"></div>
  <div id="toast-live" class="sr-only" role="status" aria-live="polite" aria-atomic="true"></div>
  <div id="toast-region" class="toast-region"></div>

  <div class="page-frame">
    <header class="topbar">
      <div class="brand-group">
        <div class="brand-badge" aria-hidden="true">
          <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2c.5 4-3 6-3 10a5 5 0 0 0 10 0c0-3-2-5.5-4-8"/><path d="M9 14a3 3 0 0 0 6 0"/></svg>
        </div>
        <div>
          <h1 class="brand-title">Karhutla Edge-IoT<span class="brand-title-ext"> — Pusat Telemetri &amp; Kendali</span></h1>
          <p class="brand-sub">Monitor Real-time SITL, HILS &amp; Node Lapangan • Broker Port 6669</p>
        </div>
      </div>

      <div class="status-cluster">
        <span id="badge-broker" class="pill-badge" data-tone="warn">
          <span class="status-dot" aria-hidden="true"></span>
          <span id="txt-broker">Broker :6669: memeriksa...</span>
        </span>
        <span id="badge-node" class="pill-badge">
          <span class="status-dot" aria-hidden="true"></span>
          <span id="txt-node">ESP32: belum terdeteksi</span>
        </span>
        <span id="badge-fsm" class="pill-badge">
          <svg id="ico-fsm-svg" class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><line x1="8" y1="12" x2="16" y2="12"/></svg>
          <span id="txt-fsm">FSM: —</span>
        </span>
      </div>

      <div class="topbar-actions">
        <div class="theme-switch" role="group" aria-label="Pilih Tema Tampilan">
          <button type="button" class="chip-btn theme-btn" data-theme-val="light" aria-pressed="false" title="Tema Proyektor Terang">
            <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/></svg>
            Terang
          </button>
          <button type="button" class="chip-btn theme-btn" data-theme-val="dark" aria-pressed="false" title="Tema Gelap Laboratorium">
            <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>
            Gelap
          </button>
          <button type="button" class="chip-btn theme-btn" data-theme-val="system" aria-pressed="true" title="Ikuti tema perangkat (prefers-color-scheme)">
            <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>
            Sistem
          </button>
        </div>
        <button type="button" class="chip-btn" id="btn-open-help" title="Buka Glosarium Metrik dan Pintasan Keyboard">
          <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>
          Glosarium <kbd class="key-hint">?</kbd>
        </button>
      </div>
    </header>

    <div id="safety-callout" class="safety-callout" role="status" hidden>
      <div class="callout-msg">
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>
        <span id="safety-callout-text"></span>
      </div>
      <div id="safety-callout-actions" class="callout-actions"></div>
    </div>

    <main class="dashboard-shell">
      <!-- 6 KPI Cards aligned via CSS Subgrid with Domain-Accurate Sensor Grouping & Micro-Instruments -->
      <section class="kpi-grid" aria-label="Indikator Utama Sensor dan Aktuator">
        <article class="kpi-card" id="card-state" data-primary="true" data-tone="idle">
          <header>
            <span>State FSM &amp; Mode</span>
            <span class="pill-badge" id="kpi-src-tag">SITL</span>
          </header>
          <div id="kpi-state-wrap" class="kpi-value kpi-state-wrap tone-muted">
            <svg id="kpi-state-ico" class="ico kpi-state-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><line x1="8" y1="12" x2="16" y2="12"/></svg>
            <span id="kpi-state">BELUM ADA DATA</span>
          </div>
          <footer>
            <span>Mode: <strong id="kpi-mode">--</strong> • t=<span id="kpi-t">0s</span><span id="kpi-t-dur" hidden></span> (#<span id="kpi-seq">0</span>)</span>
            <span id="kpi-parity-wrap" class="kpi-parity-tag" hidden>Paritas: <strong id="kpi-parity">-</strong></span>
          </footer>
        </article>

        <article class="kpi-card" id="card-fi" data-primary="true" data-tone="idle">
          <header>
            <span>Fusion Index (FI)</span>
            <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
          </header>
          <div>
            <div id="kpi-fi-wrap" class="kpi-value tone-muted">
              <span id="kpi-fi">--</span> <span class="kpi-unit">µg/m³</span>
            </div>
            <div class="kpi-meter" aria-hidden="true">
              <div id="fi-meter-fill" class="kpi-meter-fill"></div>
              <span class="fi-tick fi-tick-waspada"></span>
              <span class="fi-tick fi-tick-bahaya"></span>
            </div>
          </div>
          <footer>
            <span>Waspada ≥ 55</span>
            <span>Bahaya ≥ 150</span>
          </footer>
        </article>

        <article class="kpi-card">
          <header>
            <span>PM-proxy Kalman</span>
            <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242"/><path d="M8 19h8"/><path d="M6 22h10"/></svg>
          </header>
          <div class="kpi-value tone-accent">
            <span id="kpi-pm">--</span> <span class="kpi-unit">µg/m³</span>
          </div>
          <footer>
            <span><button type="button" class="kpi-abbr-btn" data-abbr-help="Mentah: PM setelah filter Median3, sebelum Kalman (µg/m³)" title="Mentah: PM setelah filter Median3, sebelum Kalman (µg/m³)">Mentah:</button><span id="kpi-pmraw">--</span> • <button type="button" class="kpi-abbr-btn" data-abbr-help="dC/dt: laju kenaikan konsentrasi asap (µg/m³ per detik)" title="dC/dt: laju kenaikan konsentrasi asap (µg/m³ per detik)">dC/dt:</button><span id="kpi-dcdt">0.00</span></span>
            <span><button type="button" class="kpi-abbr-btn" data-abbr-help="Vd: tegangan keluaran sensor debu Sharp (volt)" title="Vd: tegangan keluaran sensor debu Sharp (volt)">Vd:</button><span id="kpi-vd">--</span>V (<button type="button" class="kpi-abbr-btn" data-abbr-help="V0: tegangan baseline udara bersih sensor debu (volt)" title="V0: tegangan baseline udara bersih sensor debu (volt)">V0:</button><span id="kpi-v0">--</span>)</span>
          </footer>
        </article>

        <article class="kpi-card">
          <header>
            <span>Gas MQ-135 &amp; Skor g</span>
            <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M10 2v7.527a2 2 0 0 1-.211.896L4.72 20.55a1 1 0 0 0 .9 1.45h12.76a1 1 0 0 0 .9-1.45l-5.069-10.127A2 2 0 0 1 14 9.527V2"/><path d="M8.5 2h7"/></svg>
          </header>
          <div class="kpi-value tone-teal">
            <span id="kpi-ppm">--</span> <span class="kpi-unit">ppm</span>
          </div>
          <footer>
            <span><button type="button" class="kpi-abbr-btn" data-abbr-help="g: skor gas ternormalisasi 0,00–1,00 (0 = udara bersih, 1 = gas pembakaran penuh)" title="g: skor gas ternormalisasi 0,00–1,00 (0 = udara bersih, 1 = gas pembakaran penuh)">Skor g:</button> <span id="kpi-g">--</span></span>
            <span><button type="button" class="kpi-abbr-btn" data-abbr-help="Vm: tegangan ADC sensor gas MQ-135 (volt)" title="Vm: tegangan ADC sensor gas MQ-135 (volt)">Vm:</button> <span id="kpi-vm">--</span>V</span>
          </footer>
        </article>

        <article class="kpi-card">
          <header>
            <span>Prakiraan AI (t+60s)</span>
            <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>
          </header>
          <div id="kpi-fc-wrap" class="kpi-value tone-muted">
            <span id="kpi-fc">--</span> <span class="kpi-unit">µg/m³</span>
          </div>
          <footer>
            <span><button type="button" class="kpi-abbr-btn" data-abbr-help="ETA: perkiraan waktu tersisa sampai FI mencapai ambang BAHAYA (150)" title="ETA: perkiraan waktu tersisa sampai FI mencapai ambang BAHAYA (150)">ETA Bahaya:</button> <strong id="kpi-eta">-</strong></span>
            <span><button type="button" class="kpi-abbr-btn" data-abbr-help="MAE: rata-rata galat absolut prakiraan AI terhadap PM aktual (µg/m³)" title="MAE: rata-rata galat absolut prakiraan AI terhadap PM aktual (µg/m³)">MAE:</button> <span id="kpi-mae">-</span></span>
          </footer>
        </article>

        <article class="kpi-card">
          <header>
            <span>Aktuator Kipas PWM</span>
            <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 12c-2-2.67-6-2.67-6 0a4 4 0 0 0 6 0Zm0 0c2.67-2 2.67-6 0-6a4 4 0 0 0 0 6Zm0 0c2 2.67 6 2.67 6 0a4 4 0 0 0-6 0Zm0 0c-2.67 2-2.67 6 0 6a4 4 0 0 0 0-6Z"/></svg>
          </header>
          <div>
            <div id="kpi-duty-wrap" class="kpi-value tone-muted">
              <span id="kpi-duty">--</span> <span class="kpi-unit">%</span>
            </div>
            <div class="kpi-meter" aria-hidden="true">
              <div id="pwm-meter-fill" class="kpi-meter-fill"></div>
            </div>
          </div>
          <footer>
            <span>Kendali: <strong id="kpi-fan-ctrl">--</strong></span>
            <span>Status: <strong id="kpi-fan-state">--</strong></span>
          </footer>
        </article>
      </section>

      <section class="workspace">
        <!-- Left: Multi-Overlay Chart -->
        <div class="panel-card">
          <div class="panel-header">
            <div>
              <h2 class="panel-title">Grafik Multi-Overlay Real-Time</h2>
              <div class="panel-subtitle-row">
                <p class="brand-sub">Sumber: <strong id="src-label">SITL (simulasi)</strong></p>
                <span id="parity-badge" class="pill-badge" data-tone="ok" hidden>Paritas HILS: -</span>
              </div>
            </div>

            <div role="toolbar" focusgroup="toolbar wrap" aria-label="Sumber Data, Rentang Waktu, dan Ekspor">
              <div role="group" aria-label="Sumber Data">
                <button type="button" class="chip-btn src-btn" data-src="auto" aria-pressed="true">Otomatis</button>
                <button type="button" class="chip-btn src-btn" data-src="esp32" aria-pressed="false">ESP32</button>
                <button type="button" class="chip-btn src-btn" data-src="sitl" aria-pressed="false">SITL</button>
              </div>
              <select id="sel-window" class="op-select" aria-label="Rentang Waktu Grafik">
                <option value="120">120 dtk</option>
                <option value="300" selected>300 dtk</option>
                <option value="900">900 dtk</option>
                <option value="0">Semua</option>
              </select>
              <button type="button" class="chip-btn" id="btn-export-png" title="Unduh gambar grafik lengkap PNG" disabled>
                <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
                PNG
              </button>
              <button type="button" class="chip-btn" id="btn-export-csv" title="Unduh data histori grafik CSV" disabled>
                <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="16" y2="17"/></svg>
                CSV
              </button>
            </div>
          </div>

          <div class="preset-bar">
            <div role="toolbar" aria-label="Preset Layer Grafik" id="preset-toolbar">
              <span class="preset-label">Preset Kurva:</span>
              <button type="button" class="chip-btn preset-btn" data-preset="demo" aria-pressed="true">Demo Sidang <kbd class="key-hint">1</kbd></button>
              <button type="button" class="chip-btn preset-btn" data-preset="dsp" aria-pressed="false">Validasi DSP <kbd class="key-hint">2</kbd></button>
              <button type="button" class="chip-btn preset-btn" data-preset="all" aria-pressed="false">Semua Layer <kbd class="key-hint">3</kbd></button>
              <span id="badge-preset-custom" class="pill-badge" data-tone="accent" hidden>Kustom</span>
            </div>
            <div class="preset-actions">
              <button type="button" class="chip-btn" id="btn-toggle-layers" aria-expanded="false" aria-controls="layers-drawer">
                <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="4" y1="21" x2="4" y2="14"/><line x1="4" y1="10" x2="4" y2="3"/><line x1="12" y1="21" x2="12" y2="12"/><line x1="12" y1="8" x2="12" y2="3"/><line x1="20" y1="21" x2="20" y2="16"/><line x1="20" y1="12" x2="20" y2="3"/><line x1="1" y1="14" x2="7" y2="14"/><line x1="9" y1="8" x2="15" y2="8"/><line x1="17" y1="16" x2="23" y2="16"/></svg>
                <span id="lbl-toggle-layers">Atur Kurva</span>
              </button>
              <button type="button" class="chip-btn" id="btn-sim-clear" data-variant="danger" title="Bersihkan seluruh kurva pada sumber grafik aktif" disabled>Bersihkan</button>
            </div>
          </div>

          <div id="layers-drawer" class="compact-drawer" hidden>
            <div role="toolbar" focusgroup="toolbar wrap" aria-label="Filter Layer Overlay Grafik" id="overlay-toolbar">
              <div role="group" aria-label="Kurva Utama">
                <button type="button" class="chip-btn layer-btn" data-layer="fi" aria-pressed="true">
                  <span class="chip-dot" aria-hidden="true"></span> Fusion Index (FI)
                </button>
                <button type="button" class="chip-btn layer-btn" data-layer="pm" aria-pressed="true">
                  <span class="chip-dot" aria-hidden="true"></span> PM-proxy Kalman
                </button>
                <button type="button" class="chip-btn layer-btn" data-layer="forecast" aria-pressed="true">
                  <span class="chip-dot" aria-hidden="true"></span> Prakiraan AI (+60s)
                </button>
                <button type="button" class="chip-btn layer-btn" data-layer="thresholds" aria-pressed="true">
                  <span class="chip-dot" aria-hidden="true"></span> Ambang 55 / 150
                </button>
              </div>
              <span role="separator" aria-orientation="vertical"></span>
              <div role="group" aria-label="Validasi DSP">
                <button type="button" class="chip-btn layer-btn" data-layer="pm_true" aria-pressed="false">
                  <span class="chip-dot" aria-hidden="true"></span> PM Ruang Asap (True)
                </button>
                <button type="button" class="chip-btn layer-btn" data-layer="pm_raw" aria-pressed="false">
                  <span class="chip-dot" aria-hidden="true"></span> PM Mentah (Median)
                </button>
              </div>
              <span role="separator" aria-orientation="vertical"></span>
              <div role="group" aria-label="Gas dan Tegangan">
                <button type="button" class="chip-btn layer-btn" data-layer="gas_ppm" aria-pressed="false">
                  <span class="chip-dot" aria-hidden="true"></span> Gas MQ-135 (×0.2)
                </button>
                <button type="button" class="chip-btn layer-btn" data-layer="g_scaled" aria-pressed="false">
                  <span class="chip-dot" aria-hidden="true"></span> Skor g (×100)
                </button>
                <button type="button" class="chip-btn layer-btn" data-layer="voltages" aria-pressed="false">
                  <span class="chip-dot chip-dot-vd" aria-hidden="true"></span><span class="chip-dot chip-dot-vm" aria-hidden="true"></span> Tegangan Vd/Vm (×50)
                </button>
              </div>
            </div>
            <label class="touch-label" for="sw-zoom-y">
              <input id="sw-zoom-y" type="checkbox" switch>
              <span>Skala Y Ketat (Auto-zoom untuk skenario udara bersih)</span>
            </label>
          </div>

          <div id="run-summary-card" class="run-summary" role="region" aria-labelledby="rs-title" hidden>
            <div class="run-summary-head">
              <strong id="rs-title">Ringkasan Simulasi Terakhir</strong>
              <div class="run-summary-actions">
                <button type="button" class="chip-btn" id="rs-export">Unduh PNG</button>
                <button type="button" class="chip-btn" id="rs-export-csv">Unduh CSV</button>
                <button type="button" class="chip-btn" id="rs-close">Tutup</button>
              </div>
            </div>
            <dl class="run-summary-grid">
              <div><dt>Skenario</dt><dd id="rs-scenario">-</dd></div>
              <div><dt>Target</dt><dd id="rs-target">-</dd></div>
              <div><dt>FI puncak</dt><dd id="rs-peak">-</dd></div>
              <div><dt>Waktu ke BAHAYA</dt><dd id="rs-bahaya">-</dd></div>
              <div><dt>AI Preempt</dt><dd id="rs-preempt">-</dd></div>
              <div id="rs-parity-wrap" hidden><dt>Paritas HILS</dt><dd id="rs-parity">-</dd></div>
              <div><dt>Sampel</dt><dd id="rs-samples">-</dd></div>
            </dl>
          </div>

          <div id="chart-legend" class="chart-legend" aria-label="Legenda Kurva Aktif"></div>

          <div class="canvas-wrap" id="main-wrap">
            <div id="esp-banner" class="offline-banner" hidden>
              <span id="esp-banner-text"></span>
              <button type="button" class="chip-btn" id="btn-fallback-sitl">Beralih ke SITL</button>
            </div>
            <button type="button" id="btn-clear-hover" class="chip-btn hover-dismiss-btn" hidden>Tutup Inspeksi</button>
            <canvas id="main-chart" class="chart-stage" width="960" height="320" tabindex="0" role="img" aria-label="Grafik Utama Multi-Overlay (gunakan panah kiri atau kanan untuk inspeksi titik)" aria-describedby="chart-sr-summary"></canvas>
            <p id="chart-sr-summary" class="sr-only">Belum ada data telemetri.</p>
            <div id="chart-live-readout" class="sr-only" role="status" aria-live="polite" aria-atomic="true"></div>
          </div>

          <div class="canvas-wrap">
            <canvas id="sub-chart" class="subchart-stage" width="960" height="150" tabindex="0" role="img" aria-label="Sub-Grafik Duty Kipas dan Pita State FSM (gunakan panah kiri atau kanan untuk inspeksi)" aria-describedby="subchart-sr-summary"></canvas>
            <p id="subchart-sr-summary" class="sr-only">Belum ada histori duty kipas dan state FSM.</p>
          </div>
        </div>

        <!-- Right: Override, Simulation, Collapsible MQTT Log & Mobile Export -->
        <aside class="control-column" aria-label="Panel Kontrol dan Operasi">
          <section class="panel-card" id="override-panel" aria-label="Kontrol Override Aktuator">
            <div>
              <h2 class="panel-title">Kontrol Override Aktuator <code class="topic-tag">karhutla/cmd</code></h2>
              <p id="override-target-line" class="brand-sub override-target-line">Memeriksa jalur kendali...</p>
            </div>

            <div class="op-row override-scope-row">
              <label for="sel-cmd-target">Sasaran Perintah</label>
              <select id="sel-cmd-target" class="op-select" aria-label="Target Perintah Override">
                <option value="auto" selected>Otomatis (semua jalur aktif)</option>
                <option value="sitl">SITL (Python) saja</option>
                <option value="esp32">ESP32 (Broker :6669) saja</option>
              </select>
            </div>

            <div role="toolbar" aria-label="Perintah Override Kipas" class="cmd-grid" id="cmd-toolbar">
              <button type="button" class="cmd-btn" data-cmd="AUTO" aria-pressed="false" disabled>
                <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.6.77 1.02 1.4 1.15.2.04.41.06.62.06H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"/></svg>
                AUTO <kbd class="key-hint">Alt+⇧A</kbd>
              </button>
              <button type="button" class="cmd-btn" data-cmd="FORCE_MAX" aria-pressed="false" disabled>
                <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>
                MAX 100% <kbd class="key-hint">Alt+⇧M</kbd>
              </button>
              <button type="button" class="cmd-btn" data-cmd="FORCE_OFF" aria-pressed="false" disabled>
                <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>
                OFF 0%
              </button>
            </div>

            <div class="reset-zone">
              <span class="brand-sub">Mengatur ulang Kalman, FSM &amp; histori</span>
              <button type="button" class="cmd-btn" id="btn-cmd-reset" data-cmd="RESET" data-variant="danger" disabled>
                <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10"/></svg>
                Reset Sistem
              </button>
            </div>
          </section>

          <section class="panel-card" aria-label="Operasi Simulasi SITL dan HILS">
            <div class="panel-head-row">
              <div>
                <h2 class="panel-title">Operasi Simulasi SITL / HILS Terpadu</h2>
                <p class="brand-sub">Jalankan skenario ke SITL Python atau HILS ESP32</p>
              </div>
              <span id="badge-sim-state" class="pill-badge">Tidak aktif</span>
            </div>

            <div role="toolbar" aria-label="Kontrol Runner Simulasi" class="exec-top">
              <button type="button" class="cmd-btn btn-primary" id="btn-sim-start">
                <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="6 3 20 12 6 21 6 3"/></svg>
                <span id="lbl-sim-start">Mulai</span>
                <kbd class="key-hint" id="kbd-sim-start">S</kbd>
              </button>
              <button type="button" class="cmd-btn" id="btn-sim-pause" aria-pressed="false" disabled>
                <span id="lbl-sim-pause">Jeda</span> <kbd class="key-hint">P</kbd>
              </button>
              <button type="button" class="cmd-btn" id="btn-sim-stop" disabled>Berhenti</button>
            </div>

            <div id="sim-active-bar" class="sim-active-bar" hidden>
              <span id="sim-active-text">SITL • Api Cepat</span>
              <button type="button" class="chip-btn" id="btn-toggle-sim-cfg" aria-expanded="false" aria-controls="sim-config-fields">Ubah Target/Skenario</button>
            </div>

            <div class="sim-zone">
              <span class="sim-zone-label">Konfigurasi Simulasi</span>
              <div id="sim-config-fields" class="op-list">
                <div class="op-row">
                  <span>Target Eksekusi</span>
                  <div role="toolbar" aria-label="Target Runner Simulasi" id="target-toolbar">
                    <button type="button" class="chip-btn tgt-btn" data-target="sitl" aria-pressed="true">SITL (Python)</button>
                    <button type="button" class="chip-btn tgt-btn" data-target="hils" aria-pressed="false">HILS (ESP32)</button>
                  </div>
                </div>

                <div id="hils-concurrency-hint" class="hils-hint" hidden>
                  Pastikan ESP32 terhubung dan jangan menjalankan <code class="topic-tag">hils.py</code> di terminal secara bersamaan.
                </div>

                <div class="op-row">
                  <label for="sel-scenario">Skenario Ruang Asap</label>
                  <select id="sel-scenario" class="op-select">
                    <option value="flaming" selected>Api Cepat (flaming)</option>
                    <option value="smolder">Membara Lambat (smolder)</option>
                    <option value="clean">Udara Bersih (clean)</option>
                    <option value="fault">Putus Sensor (fault)</option>
                  </select>
                </div>
              </div>
            </div>

            <details id="sim-advanced-drawer" class="sim-adv-drawer">
              <summary>
                <span>Setelan Simulasi Lanjut</span>
                <span class="summary-trail">
                  <span id="sim-adv-summary" class="pill-badge">1x • Standar</span>
                  <svg class="ico drawer-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 9 12 15 18 9"/></svg>
                </span>
              </summary>
              <div class="sim-adv-body">
                <div class="op-row">
                  <span>Kecepatan</span>
                  <div role="toolbar" aria-label="Kecepatan Simulasi" id="speed-toolbar">
                    <button type="button" class="chip-btn spd-btn" data-speed="1" aria-pressed="true">1x</button>
                    <button type="button" class="chip-btn spd-btn" data-speed="5" aria-pressed="false">5x</button>
                    <button type="button" class="chip-btn spd-btn" data-speed="10" aria-pressed="false">10x</button>
                    <button type="button" class="chip-btn spd-btn" data-speed="20" aria-pressed="false">20x</button>
                  </div>
                </div>

                <label class="op-row touch-label" for="sw-preempt">
                  <span>AI Preempt (FORCE MAX Proaktif Dini)</span>
                  <input id="sw-preempt" type="checkbox" switch>
                </label>

                <label class="op-row touch-label" for="sw-fault">
                  <span>Injeksi Putus Kabel Sensor Debu (FAILSAFE) — manual, kapan saja</span>
                  <input id="sw-fault" type="checkbox" switch>
                </label>
              </div>
            </details>
          </section>

          <details id="drawer-mqtt" class="panel-card drawer-card">
            <summary>
              <span>Aliran Log MQTT <code class="topic-tag">:6669</code></span>
              <span class="summary-trail">
                <span id="badge-log-count" class="pill-badge">0 baris</span>
                <svg class="ico drawer-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 9 12 15 18 9"/></svg>
              </span>
            </summary>
            <div class="drawer-body">
              <div class="panel-head-row">
                <span class="brand-sub">File log: <code id="txt-logfile" class="topic-tag">logs/mqtt.log</code></span>
                <div class="summary-trail">
                  <button type="button" class="chip-btn" id="btn-export-log" title="Unduh file log trafik MQTT (.log)">Unduh Log</button>
                  <label class="touch-label" for="sw-autoscroll">
                    <span>Gulir otomatis</span>
                    <input id="sw-autoscroll" type="checkbox" switch checked>
                  </label>
                </div>
              </div>
              <div id="mqtt-log-box" class="mqtt-stream" tabindex="0" aria-label="Aliran Log MQTT"></div>
            </div>
          </details>

          <details id="drawer-mobile" class="panel-card drawer-card">
            <summary>
              <span>Koneksi Aplikasi HP (IoT MQTT Panel)</span>
              <svg class="ico drawer-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 9 12 15 18 9"/></svg>
            </summary>
            <div class="drawer-body">
              <button type="button" class="chip-btn btn-block" id="btn-export-mobile">
                <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="2" width="14" height="20" rx="2" ry="2"/><line x1="12" y1="18" x2="12.01" y2="18"/></svg>
                Unduh Konfigurasi IoT MQTT Panel (.json)
              </button>
              <p id="mobile-config-hint" class="brand-sub">Buka aplikasi <strong>IoT MQTT Panel</strong> di HP, pilih menu Backup/Restore, lalu impor file JSON (port 6669).</p>
            </div>
          </details>
        </aside>
      </section>
    </main>
  </div>

  <!-- Sticky Mobile Thumb-Zone Quick Controls -->
  <nav class="mobile-dock" aria-label="Kendali Cepat Mobile">
    <div id="mob-dock-context" class="mob-dock-context">SITL • Api Cepat • 1x</div>
    <div id="mob-dock-tray" class="mob-dock-tray" hidden>
      <button type="button" class="cmd-btn mob-cmd-btn" data-cmd="FORCE_OFF" data-variant="danger" aria-pressed="false" disabled>OFF 0%</button>
      <button type="button" class="cmd-btn" id="mob-btn-stop" disabled>Berhenti</button>
      <button type="button" class="cmd-btn" id="btn-mob-reset" data-cmd="RESET" data-variant="danger" disabled>Reset Sistem</button>
    </div>
    <div class="mob-dock-row">
      <button type="button" class="cmd-btn mob-cmd-btn" data-cmd="AUTO" aria-pressed="false" disabled>AUTO</button>
      <button type="button" class="cmd-btn mob-cmd-btn" data-cmd="FORCE_MAX" aria-pressed="false" disabled>MAX 100%</button>
      <button type="button" class="cmd-btn btn-primary" id="mob-btn-playpause">Mulai</button>
      <button type="button" class="cmd-btn" id="mob-btn-more" aria-expanded="false" aria-controls="mob-dock-tray">
        <span>Lainnya</span>
        <svg class="ico drawer-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 9 12 15 18 9"/></svg>
      </button>
    </div>
  </nav>

  <!-- Native Confirmation Dialog for Destructive / High-Stakes Actions -->
  <dialog id="confirm-dialog" class="confirm-modal" aria-labelledby="confirm-title" aria-describedby="confirm-desc">
    <h2 id="confirm-title" class="panel-title">Konfirmasi Aksi</h2>
    <p id="confirm-desc" class="brand-sub confirm-desc"></p>
    <div class="modal-actions">
      <button type="button" class="chip-btn" id="confirm-cancel">Batal</button>
      <button type="button" class="cmd-btn" id="confirm-ok" data-variant="danger">Ya, Lanjutkan</button>
    </div>
  </dialog>

  <!-- Dedicated Glossary, Troubleshooting & Keyboard Shortcuts Help Dialog -->
  <dialog id="help-dialog" class="confirm-modal help-modal" aria-labelledby="help-title">
    <div class="help-head">
      <h2 id="help-title" class="panel-title">Glosarium, Pemecahan Masalah &amp; Pintasan</h2>
      <button type="button" class="chip-btn" id="help-close">Tutup</button>
    </div>
    <div class="glossary-list">
      <div><strong>FI (Fusion Index):</strong> Indeks gabungan dalam µg/m³: <code class="topic-tag">FI = PM_Kalman + W_gas × GAS_FULL_PM × g + max(0, dC/dt) × t_lead</code> (W_gas = 0,30; GAS_FULL_PM = 100; t_lead = 15 s). Memicu FSM WASPADA ≥ 55 dan BAHAYA ≥ 150 dengan histeresis pemulihan.</div>
      <div><strong>PM-proxy Kalman, Mentah &amp; dC/dt:</strong> PM tersaring Median3 + Kalman 2-state; <em>Mentah</em> adalah nilai setelah Median3 sebelum Kalman; dC/dt adalah laju kenaikan asap (µg/m³/s); <code class="topic-tag">Vd</code> tegangan debu Sharp; <code class="topic-tag">V0</code> baseline udara bersih.</div>
      <div><strong>Gas MQ-135 (<code class="topic-tag">ppm</code>, Skor <code class="topic-tag">g</code>, <code class="topic-tag">Vm</code>):</strong> Estimasi gas pembakaran, skor ternormalisasi 0,00–1,00, dan tegangan ADC MQ-135.</div>
      <div><strong>Prakiraan AI (<code class="topic-tag">t+60s</code>, <code class="topic-tag">ETA</code>, <code class="topic-tag">MAE</code>):</strong> Regresi linear 60 detik ke depan; ETA = perkiraan waktu menuju BAHAYA; MAE = galat absolut rata-rata prakiraan. Memicu <strong>AI Preempt</strong> (FORCE MAX 100% dini).</div>
      <div><strong>State FSM &amp; FAILSAFE:</strong> <code class="topic-tag">NORMAL</code>, <code class="topic-tag">WASPADA</code>, <code class="topic-tag">BAHAYA</code>, dan <code class="topic-tag">FAILSAFE</code> (kipas terkunci aman 70% bila kabel debu terputus, kecuali override manual). Sebelum ada data, kartu State menampilkan <em>BELUM ADA DATA</em>.</div>
      <div><strong>Skenario <em>Putus Sensor (fault)</em> vs Saklar <em>Injeksi Putus Kabel (FAILSAFE)</em>:</strong> skenario fault otomatis memutus sensor debu pada <code class="topic-tag">t=200–230s</code> di dalam simulasi; saklar injeksi memaksa <code class="topic-tag">Vd=0V</code> secara manual kapan saja, tanpa menunggu skenario.</div>
      <div><strong>SITL vs HILS &amp; Paritas (<code class="topic-tag">ΔFI</code>):</strong> SITL memakai firmware virtual Python; HILS menguji ESP32 fisik <em>lock-step</em> via MQTT :6669 dan membandingkannya dengan firmware referensi. Paritas dinyatakan <strong>OK</strong> bila maks |ΔFI| &lt; 1,0 dan 100% state cocok; selain itu tampil <strong>Cek</strong>.</div>
      <div><strong>Tujuan Perintah Override:</strong> panel Override memiliki pengunci sasaran (Otomatis, SITL saja, atau ESP32 via broker :6669 saja) agar pengujian satu jalur tidak mengganggu jalur lain. Tombol dinonaktifkan bila jalur yang dipilih tidak aktif.</div>
      <div><strong>Skala Kurva &amp; Pemulihan Cepat:</strong> <code class="topic-tag">Gas × 0.2</code>, <code class="topic-tag">g × 100</code>, <code class="topic-tag">Vd/Vm × 50</code> (sumbu kanan tampil otomatis saat layer diaktifkan). Bila HILS timeout, cek USB/Wi-Fi ESP32 dan pastikan <code class="topic-tag">hils.py</code> tidak berjalan ganda di terminal.</div>
    </div>
    <h3 class="sim-zone-label help-subhead">Pintasan Keyboard</h3>
    <label class="touch-label shortcut-toggle-row" for="sw-shortcuts">
      <input id="sw-shortcuts" type="checkbox" switch checked>
      <span>Aktifkan pintasan satu tombol (S, P, 1–3, Space)</span>
    </label>
    <div class="shortcut-grid">
      <div><kbd class="key-hint">S</kbd> Mulai simulasi (saat tidak aktif)</div>
      <div><kbd class="key-hint">P</kbd> / <kbd class="key-hint">Space</kbd> Jeda atau lanjutkan</div>
      <div><kbd class="key-hint">1</kbd> <kbd class="key-hint">2</kbd> <kbd class="key-hint">3</kbd> Pilih preset kurva</div>
      <div><kbd class="key-hint">Alt+⇧A</kbd> Mode AUTO • <kbd class="key-hint">Alt+⇧M</kbd> MAX 100%</div>
      <div><kbd class="key-hint">←</kbd> / <kbd class="key-hint">→</kbd> Geser inspeksi pada grafik</div>
      <div><kbd class="key-hint">Esc</kbd> Lepas crosshair • <kbd class="key-hint">?</kbd> Bantuan</div>
    </div>
  </dialog>

  <script>
    (() => {
      // Accessible roving-tabindex fallback skipping disabled buttons
      const toolbarSyncFns = [];
      function syncAllToolbars() {
        toolbarSyncFns.forEach((fn) => fn());
      }
      document.querySelectorAll('[role="toolbar"][focusgroup]').forEach((bar) => {
        if ('focusGroup' in HTMLElement.prototype) return;
        const getButtons = () => Array.from(bar.querySelectorAll('button'));
        const getEnabled = () => getButtons().filter((b) => !b.disabled);
        function syncBar() {
          const all = getButtons();
          const enabled = all.filter((b) => !b.disabled);
          let current = enabled.find((b) => b === document.activeElement || b.tabIndex === 0);
          if (!current) current = enabled[0] || null;
          all.forEach((b) => {
            b.tabIndex = (!b.disabled && b === current) ? 0 : -1;
          });
        }
        toolbarSyncFns.push(syncBar);
        syncBar();
        bar.addEventListener('focusin', ({ target }) => {
          const enabled = getEnabled();
          if (enabled.includes(target)) {
            getButtons().forEach((b) => (b.tabIndex = b === target ? 0 : -1));
          }
        });
        bar.addEventListener('keydown', (e) => {
          const enabled = getEnabled();
          if (!enabled.length) return;
          const idx = enabled.indexOf(document.activeElement);
          if (idx < 0) return;
          let next = idx;
          if (e.key === 'ArrowRight' || e.key === 'ArrowDown') next = (idx + 1) % enabled.length;
          else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') next = (idx - 1 + enabled.length) % enabled.length;
          else if (e.key === 'Home') next = 0;
          else if (e.key === 'End') next = enabled.length - 1;
          else return;
          e.preventDefault();
          enabled[next].focus();
        });
      });

      const PRESETS = {
        demo: { fi: true, pm: true, pm_true: false, pm_raw: false, forecast: true, gas_ppm: false, g_scaled: false, voltages: false, thresholds: true },
        dsp: { fi: true, pm: true, pm_true: true, pm_raw: true, forecast: false, gas_ppm: false, g_scaled: true, voltages: false, thresholds: true },
        all: { fi: true, pm: true, pm_true: true, pm_raw: true, forecast: true, gas_ppm: true, g_scaled: true, voltages: true, thresholds: true },
      };

      const SCENARIO_LABELS = {
        flaming: 'Api Cepat',
        smolder: 'Membara Lambat',
        clean: 'Udara Bersih',
        fault: 'Putus Sensor',
      };

      const PRESET_LABELS = {
        demo: 'Demo Sidang',
        dsp: 'Validasi DSP',
        all: 'Semua Layer',
        custom: 'Kustom',
      };

      const defaultLayers = { ...PRESETS.demo };
      let savedPrefs = null;
      try {
        savedPrefs = JSON.parse(localStorage.getItem('karhutla_dash_prefs') || 'null');
      } catch (e) {}

      const state = {
        history: [],
        src: {
          sitl: { history: [], latest: null },
          esp32: { history: [], latest: null },
        },
        esp: { online: false, age_s: null, hw_mode: null, lastRx: null },
        sseConnected: false,
        sseHadDisconnect: false,
        bootError: false,
        hilsTimeoutInfo: null,
        wasEspStale: false,
        cmdBusy: false,
        simConfigExpanded: false,
        lastActiveSource: null,
        lastRunSummary: null,
        lastCalloutText: '',
        lastCalloutActionKey: '',
        lastLegendKey: '',
        hoverFromKey: false,
        hoverFromTouch: false,
        totalLogs: 0,
        theme: (savedPrefs && (savedPrefs.theme === 'light' || savedPrefs.theme === 'dark')) ? savedPrefs.theme : null,
        zoomY: !!(savedPrefs && savedPrefs.zoomY),
        viewMode: (savedPrefs && savedPrefs.viewMode) || 'auto',
        cmdTarget: (savedPrefs && ['auto', 'sitl', 'esp32'].includes(savedPrefs.cmdTarget)) ? savedPrefs.cmdTarget : 'auto',
        singleKeyShortcuts: (savedPrefs && typeof savedPrefs.singleKeyShortcuts === 'boolean') ? savedPrefs.singleKeyShortcuts : true,
        simTarget: 'sitl',
        latest: {},
        sim: {},
        windowSec: (savedPrefs && typeof savedPrefs.windowSec === 'number') ? savedPrefs.windowSec : 300,
        preset: (savedPrefs && savedPrefs.preset) || 'demo',
        hoverT: null,
        hoverIdx: null,
        lastAnnouncedSafety: '',
        layers: (savedPrefs && savedPrefs.layers) ? { ...defaultLayers, ...savedPrefs.layers } : defaultLayers,
      };

      function savePrefs() {
        try {
          localStorage.setItem('karhutla_dash_prefs', JSON.stringify({
            viewMode: state.viewMode,
            cmdTarget: state.cmdTarget,
            singleKeyShortcuts: state.singleKeyShortcuts,
            windowSec: state.windowSec,
            preset: state.preset,
            layers: state.layers,
            theme: state.theme,
            zoomY: state.zoomY,
          }));
        } catch (e) {}
      }

      const mainCanvas = document.getElementById('main-chart');
      const subCanvas = document.getElementById('sub-chart');
      const mCtx = mainCanvas.getContext('2d');
      const sCtx = subCanvas.getContext('2d');
      const logBox = document.getElementById('mqtt-log-box');
      const toastRegion = document.getElementById('toast-region');
      const toastLive = document.getElementById('toast-live');
      const safetyLive = document.getElementById('safety-live');

      function formatTime(sec) {
        const s = Math.max(0, Math.round(Number(sec) || 0));
        if (s < 60) return `${s}s`;
        const m = Math.floor(s / 60);
        const rem = s % 60;
        return `${String(m).padStart(2, '0')}:${String(rem).padStart(2, '0')}`;
      }

      function describeFanState(row) {
        if (!row) return '--';
        if (row.mode === 'FORCE_MAX') return 'Dipaksa Aktif 100%';
        if (row.mode === 'FORCE_OFF') return 'Dipaksa Mati 0%';
        if (row.state === 'FAILSAFE') return 'Terkunci Aman 70%';
        if ((row.duty ?? 0) > 0.5) return 'Modulasi PWM';
        return 'Diam';
      }

      function isDarkTheme() {
        if (state.theme === 'dark') return true;
        if (state.theme === 'light') return false;
        return window.matchMedia('(prefers-color-scheme: dark)').matches;
      }

      function chartPalette(dark = isDarkTheme()) {
        return dark ? {
          grid: 'rgba(156,167,180,0.18)',
          text: '#cbd5e1',
          muted: '#9ca7b4',
          fi: '#fb923c',
          pm: '#38bdf8',
          pm_true: '#4ade80',
          pm_raw: '#9ca7b4',
          forecast: '#e879f9',
          forecastFill: 'rgba(232,121,249,0.08)',
          nowLine: 'rgba(232,121,249,0.65)',
          gas_ppm: '#2dd4bf',
          g_scaled: '#facc15',
          v_dust: '#60a5fa',
          v_mq: '#a78bfa',
          waspada: '#fbbf24',
          bahaya: 'rgba(248,113,113,0.85)',
          fsm: '#fbbf24',
          duty: '#4ade80',
          tipBg: 'rgba(22,26,31,0.96)',
          tipFg: '#f3f5f7',
          exportBg: '#161a1f',
        } : {
          grid: 'rgba(75,85,99,0.16)',
          text: '#374151',
          muted: '#4b5563',
          fi: '#c2410c',
          pm: '#0369a1',
          pm_true: '#14532d',
          pm_raw: '#475569',
          forecast: '#6d28d9',
          forecastFill: 'rgba(109,40,217,0.06)',
          nowLine: 'rgba(109,40,217,0.55)',
          gas_ppm: '#0f766e',
          g_scaled: '#854d0e',
          v_dust: '#2563eb',
          v_mq: '#7c3aed',
          waspada: '#92400e',
          bahaya: 'rgba(185,28,28,0.82)',
          fsm: '#92400e',
          duty: '#14532d',
          tipBg: 'rgba(255,255,255,0.97)',
          tipFg: '#111827',
          exportBg: '#ffffff',
        };
      }

      function syncLayerChipColors(pal = chartPalette()) {
        const map = {
          fi: pal.fi,
          pm: pal.pm,
          pm_true: pal.pm_true,
          pm_raw: pal.pm_raw,
          forecast: pal.forecast,
          thresholds: pal.waspada,
          gas_ppm: pal.gas_ppm,
          g_scaled: pal.g_scaled,
          voltages: pal.v_dust,
        };
        document.querySelectorAll('.layer-btn').forEach((btn) => {
          const layer = btn.getAttribute('data-layer');
          if (map[layer]) btn.style.setProperty('--chip-color', map[layer]);
          if (layer === 'voltages') {
            btn.style.setProperty('--chip-color-vd', pal.v_dust);
            btn.style.setProperty('--chip-color-vm', pal.v_mq);
          }
        });
      }

      function getLegendSpecs(pal = chartPalette()) {
        const list = [];
        if (state.layers.fi) list.push({ label: 'Fusion Index (FI)', color: pal.fi, style: 'solid' });
        if (state.layers.pm) list.push({ label: 'PM-proxy Kalman', color: pal.pm, style: 'solid' });
        if (state.layers.pm_true && !document.querySelector('.layer-btn[data-layer="pm_true"]').disabled) {
          list.push({ label: 'PM Ruang Asap (True)', color: pal.pm_true, style: 'solid' });
        }
        if (state.layers.pm_raw) list.push({ label: 'PM Mentah (Median)', color: pal.pm_raw, style: 'solid' });
        if (state.layers.forecast) list.push({ label: 'Prakiraan AI (+60s)', color: pal.forecast, style: 'dotted' });
        if (state.layers.gas_ppm) list.push({ label: 'Gas MQ-135 (×0.2)', color: pal.gas_ppm, style: 'dashed' });
        if (state.layers.g_scaled) list.push({ label: 'Skor g (×100)', color: pal.g_scaled, style: 'dotted' });
        if (state.layers.voltages) {
          list.push({ label: 'Vd Sharp (×50)', color: pal.v_dust, style: 'dashed' });
          list.push({ label: 'Vm MQ-135 (×50)', color: pal.v_mq, style: 'dashed' });
        }
        if (state.layers.thresholds) {
          list.push({ label: 'Ambang Waspada (55)', color: pal.waspada, style: 'dashed' });
          list.push({ label: 'Ambang Bahaya (150)', color: pal.bahaya, style: 'dashed' });
        }
        return list;
      }

      function renderChartLegend() {
        const el = document.getElementById('chart-legend');
        if (!el) return;
        const specs = getLegendSpecs();
        const key = specs.map((s) => `${s.label}|${s.color}|${s.style}`).join(';');
        if (key === state.lastLegendKey) return;
        state.lastLegendKey = key;
        el.innerHTML = '';
        if (!specs.length) {
          const empty = document.createElement('span');
          empty.className = 'brand-sub';
          empty.textContent = 'Tidak ada kurva aktif — buka Atur Kurva untuk memilih.';
          el.appendChild(empty);
          return;
        }
        specs.forEach((s) => {
          const item = document.createElement('span');
          item.className = 'legend-item';
          const sw = document.createElement('span');
          sw.className = 'legend-swatch';
          sw.setAttribute('data-style', s.style);
          sw.style.color = s.color;
          const lbl = document.createElement('span');
          lbl.textContent = s.label;
          item.appendChild(sw);
          item.appendChild(lbl);
          el.appendChild(item);
        });
      }

      function applyTheme(theme, persist = false) {
        const norm = (theme === 'light' || theme === 'dark') ? theme : null;
        state.theme = norm;
        if (norm) {
          document.documentElement.dataset.theme = norm;
        } else {
          delete document.documentElement.dataset.theme;
        }
        document.querySelectorAll('.theme-btn').forEach((btn) => {
          const val = btn.getAttribute('data-theme-val');
          const active = (val === 'system' && norm === null) || (val === norm);
          btn.setAttribute('aria-pressed', active ? 'true' : 'false');
        });
        if (persist) savePrefs();
        syncLayerChipColors();
        renderChartLegend();
        scheduleDraw();
      }

      function showToast(message, tone = 'ok') {
        if (!message) return;
        const el = document.createElement('div');
        el.className = 'toast';
        el.setAttribute('data-tone', tone);
        el.textContent = message;
        toastRegion.appendChild(el);
        while (toastRegion.children.length > 4) {
          toastRegion.removeChild(toastRegion.firstChild);
        }
        if (toastLive) {
          toastLive.textContent = '';
          requestAnimationFrame(() => {
            toastLive.textContent = message;
          });
        }
        setTimeout(() => {
          if (el.parentNode) el.parentNode.removeChild(el);
        }, 3600);
      }

      function announceSafety(msg) {
        if (!msg || msg === state.lastAnnouncedSafety) return;
        state.lastAnnouncedSafety = msg;
        safetyLive.textContent = msg;
      }

      function syncCanvasSize(canvas, fallbackW, fallbackH) {
        const rect = canvas.getBoundingClientRect();
        const cw = Math.max(260, Math.round(rect.width || fallbackW));
        const ch = Math.max(110, Math.round(rect.height || fallbackH));
        const dpr = Math.min(window.devicePixelRatio || 1, 2);
        const pw = Math.round(cw * dpr);
        const ph = Math.round(ch * dpr);
        if (canvas.width !== pw || canvas.height !== ph) {
          canvas.width = pw;
          canvas.height = ph;
        }
        canvas._cw = cw;
        canvas._ch = ch;
        canvas._dpr = dpr;
      }

      if ('ResizeObserver' in window) {
        const ro = new ResizeObserver(() => scheduleDraw());
        ro.observe(mainCanvas);
        ro.observe(subCanvas);
      }

      function appendMqttLog(line, countAsLog = true) {
        if (!line) return;
        const div = document.createElement('div');
        div.className = 'log-line';
        div.textContent = line;
        logBox.appendChild(div);
        while (logBox.children.length > 200) {
          logBox.removeChild(logBox.firstChild);
        }
        if (countAsLog) state.totalLogs += 1;
        const shown = logBox.children.length;
        document.getElementById('badge-log-count').textContent =
          state.totalLogs > shown ? `${state.totalLogs} (${shown} tampil)` : `${state.totalLogs} baris`;
        if (document.getElementById('sw-autoscroll').checked) {
          logBox.scrollTop = logBox.scrollHeight;
        }
      }

      function activeSource() {
        if (state.viewMode === 'sitl' || state.viewMode === 'esp32') return state.viewMode;
        if (state.sim && state.sim.running) return state.sim.target === 'hils' ? 'esp32' : 'sitl';
        if (state.lastActiveSource && state.src[state.lastActiveSource] && state.src[state.lastActiveSource].history.length) {
          return state.lastActiveSource;
        }
        if (state.esp.online) return 'esp32';
        if (state.src.esp32.history.length) return 'esp32';
        return 'sitl';
      }

      function switchFallbackToSitl() {
        state.hilsTimeoutInfo = null;
        const finishSwitch = () => {
          state.simTarget = 'sitl';
          state.viewMode = 'sitl';
          state.lastActiveSource = 'sitl';
          savePrefs();
          applyView();
          showToast('Beralih ke mode & sumber grafik SITL.', 'ok');
        };
        if (state.sim && state.sim.running && state.sim.target === 'hils') {
          postSim({ action: 'stop', target: 'sitl' }).then(finishSwitch);
        } else {
          finishSwitch();
        }
      }

      function updateBrokerBadge() {
        const bBroker = document.getElementById('badge-broker');
        const txtBroker = document.getElementById('txt-broker');
        const sim = state.sim || {};
        const port = sim.broker_port || 6669;
        if (!state.sseConnected && state.sseHadDisconnect) {
          bBroker.setAttribute('data-tone', 'danger');
          txtBroker.textContent = 'Koneksi Server Terputus';
        } else if (sim.enable_mqtt === false) {
          bBroker.setAttribute('data-tone', 'warn');
          txtBroker.textContent = 'MQTT dinonaktifkan';
        } else if (sim.broker_connected === true) {
          bBroker.setAttribute('data-tone', 'ok');
          txtBroker.textContent = `Broker :${port} Terhubung`;
        } else if (sim.broker_connected === false) {
          bBroker.setAttribute('data-tone', 'danger');
          txtBroker.textContent = `Broker :${port} Terputus`;
        } else {
          bBroker.setAttribute('data-tone', 'warn');
          txtBroker.textContent = `Broker :${port}: memeriksa...`;
        }
      }

      function updateEspBadge() {
        const bNode = document.getElementById('badge-node');
        const txt = document.getElementById('txt-node');
        const mode = state.esp.hw_mode ? ` (${state.esp.hw_mode})` : '';
        const src = activeSource();
        const needsEsp = (src === 'esp32') || (state.simTarget === 'hils') || (state.sim && state.sim.target === 'hils');
        if (state.esp.online) {
          txt.textContent = `ESP32: ONLINE${mode}`;
          bNode.setAttribute('data-tone', 'ok');
        } else if (state.esp.lastRx) {
          const age = Math.max(0, Math.round((Date.now() - state.esp.lastRx) / 1000));
          txt.textContent = `ESP32: TERPUTUS (${age} dtk)`;
          bNode.setAttribute('data-tone', 'danger');
        } else {
          txt.textContent = needsEsp ? 'ESP32: menunggu' : 'ESP32: belum terdeteksi';
          if (needsEsp) bNode.setAttribute('data-tone', 'warn');
          else bNode.removeAttribute('data-tone');
        }
        const banner = document.getElementById('esp-banner');
        const bannerText = document.getElementById('esp-banner-text');
        const wrap = document.getElementById('main-wrap');
        const hilsTimeoutShown = !!(state.sim && state.sim.target === 'hils' && (state.sim.hils_consec_timeouts > 0 || state.hilsTimeoutInfo));
        const stale = src === 'esp32' && !state.esp.online;
        wrap.classList.toggle('is-stale', stale);
        banner.hidden = !stale || hilsTimeoutShown;
        if (stale) {
          bannerText.textContent = state.esp.lastRx
            ? `ESP32 TERPUTUS — tidak ada data sejak ${Math.max(0, Math.round((Date.now() - state.esp.lastRx) / 1000))} detik lalu (grafik dibekukan)`
            : 'Menunggu ESP32 — belum ada telemetri atau heartbeat dari broker MQTT';
          if (!state.wasEspStale) {
            state.wasEspStale = true;
            announceSafety('ESP32 terputus, grafik dibekukan.');
          }
        } else if (state.wasEspStale) {
          state.wasEspStale = false;
          if (state.esp.online) announceSafety('ESP32 tersambung kembali.');
        }
      }

      function makeActionBtn(label, onClick) {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'chip-btn';
        btn.textContent = label;
        btn.addEventListener('click', onClick);
        return btn;
      }

      function updateSafetyCallout(row, sim) {
        const box = document.getElementById('safety-callout');
        const txt = document.getElementById('safety-callout-text');
        const act = document.getElementById('safety-callout-actions');

        let verdict = { hidden: true, tone: 'accent', text: '', actionKey: '', buildActions: null, announce: '' };

        if (state.bootError) {
          verdict = {
            hidden: false,
            tone: 'danger',
            text: 'Gagal memuat data awal dari server dashboard.',
            actionKey: 'boot-error',
            buildActions: (c) => c.appendChild(makeActionBtn('Coba Muat Ulang', () => loadStateSnapshot(false))),
            announce: '',
          };
        } else if (!state.sseConnected && state.sseHadDisconnect) {
          verdict = {
            hidden: false,
            tone: 'danger',
            text: 'Koneksi live ke server dashboard terputus. Mencoba menyambung ulang...',
            actionKey: 'sse-lost',
            buildActions: (c) => c.appendChild(makeActionBtn('Muat Snapshot', () => loadStateSnapshot(true))),
            announce: '',
          };
        } else if (sim && sim.target === 'hils' && (sim.hils_consec_timeouts > 0 || state.hilsTimeoutInfo)) {
          const info = state.hilsTimeoutInfo || {};
          const consec = info.consec || sim.hils_consec_timeouts || 0;
          const canResume = !!(sim.paused && sim.running);
          verdict = {
            hidden: false,
            tone: sim.paused ? 'danger' : 'warn',
            text: sim.paused
              ? `HILS dijeda otomatis: ESP32 timeout ${consec}x beruntun pada langkah #${info.seq ?? sim.step ?? 0}. Periksa kabel/Wi-Fi ESP32.`
              : `Peringatan HILS: ESP32 belum membalas langkah #${info.seq ?? sim.step ?? 0} (timeout ${sim.hils_timeouts || info.timeouts || consec}x).`,
            actionKey: canResume ? 'hils-paused-actions' : 'hils-warn-actions',
            buildActions: (c) => {
              if (canResume) {
                c.appendChild(makeActionBtn('Lanjutkan HILS', () => {
                  state.hilsTimeoutInfo = null;
                  postSim({ action: 'resume' }, 'Melanjutkan HILS...');
                }));
              }
              c.appendChild(makeActionBtn('Beralih ke SITL', switchFallbackToSitl));
            },
            announce: sim.paused ? 'HILS dijeda otomatis karena ESP32 timeout.' : '',
          };
        } else if (row && row.state === 'FAILSAFE') {
          const dutyTxt = (row.duty ?? 0).toFixed(0);
          const faultOn = !!(sim && sim.fault_inject);
          const forcedOff = row.mode === 'FORCE_OFF';
          verdict = {
            hidden: false,
            tone: 'danger',
            text: forcedOff
              ? `Mode FAILSAFE dengan override FORCE OFF: sensor debu terputus dan kipas dipaksa mati (${dutyTxt}%). Kembalikan ke AUTO untuk pengamanan 70%.`
              : `Mode FAILSAFE aktif: pembacaan sensor debu tidak valid atau terputus. Kipas dikunci aman pada ${dutyTxt}%.`,
            actionKey: `failsafe-${forcedOff ? 'off' : 'safe'}-${faultOn ? 'inj' : 'hw'}`,
            buildActions: (c) => {
              if (forcedOff) {
                c.appendChild(makeActionBtn('Kembalikan ke AUTO', () => sendActuatorCmd('AUTO')));
              }
              if (faultOn) {
                c.appendChild(makeActionBtn('Matikan Injeksi Fault', () => {
                  document.getElementById('sw-fault').checked = false;
                  postSim({ fault_inject: false }, 'Injeksi fault dimatikan.');
                }));
              }
            },
            announce: forcedOff
              ? 'Mode FAILSAFE dengan FORCE OFF, kipas mati 0 persen.'
              : `Mode FAILSAFE aktif, kipas pada ${dutyTxt} persen.`,
          };
        } else if (row && row.mode === 'FORCE_OFF' && (row.state === 'BAHAYA' || row.state === 'WASPADA')) {
          verdict = {
            hidden: false,
            tone: 'warn',
            text: `Perhatian: Override manual FORCE OFF (0%) menahan kipas mati saat kondisi ${row.state}.`,
            actionKey: 'force-off-hazard',
            buildActions: (c) => c.appendChild(makeActionBtn('Kembalikan ke AUTO', () => sendActuatorCmd('AUTO'))),
            announce: '',
          };
        } else if (row && row.state === 'BAHAYA') {
          const fiVal = row.fi ?? 0;
          const dcdt = row.dcdt ?? 0;
          const dutyTxt = (row.duty ?? 0).toFixed(0);
          const trendTxt = dcdt < -0.15 ? 'tren asap mulai menurun' : (dcdt > 0.15 ? 'tren asap masih naik' : 'laju asap datar');
          const fiClause = fiVal >= 150
            ? 'Fusion Index melampaui 150 µg/m³'
            : `Fusion Index (${fiVal.toFixed(1)} µg/m³) masih tertahan histeresis pemulihan BAHAYA`;
          const preClause = (sim && sim.ai_preempt_active) ? ' • AI Preempt (FORCE MAX) aktif.' : '';
          verdict = {
            hidden: false,
            tone: 'danger',
            text: `Kondisi BAHAYA: ${fiClause}. Kipas ${dutyTxt}% aktif (${trendTxt}, dC/dt ${dcdt >= 0 ? '+' : ''}${dcdt.toFixed(2)}/s).${preClause}`,
            actionKey: 'none',
            buildActions: null,
            announce: 'Kondisi BAHAYA aktif.',
          };
        } else if (sim && sim.ai_preempt_active) {
          verdict = {
            hidden: false,
            tone: 'accent',
            text: 'AI Preempt Aktif: Prakiraan t+60s melampaui ambang, kipas dinaikkan proaktif ke FORCE MAX (100%).',
            actionKey: 'none',
            buildActions: null,
            announce: 'AI Preempt aktif, kipas dinaikkan proaktif ke 100 persen.',
          };
        }

        box.hidden = verdict.hidden;
        if (verdict.hidden) {
          state.lastCalloutText = '';
          if (state.lastCalloutActionKey !== '') {
            state.lastCalloutActionKey = '';
            act.innerHTML = '';
          }
          return;
        }
        if (box.getAttribute('data-tone') !== verdict.tone) {
          box.setAttribute('data-tone', verdict.tone);
        }
        if (verdict.text !== state.lastCalloutText) {
          txt.textContent = verdict.text;
          state.lastCalloutText = verdict.text;
        }
        if (verdict.actionKey !== state.lastCalloutActionKey) {
          state.lastCalloutActionKey = verdict.actionKey;
          act.innerHTML = '';
          if (verdict.buildActions) verdict.buildActions(act);
        }
        if (verdict.announce) announceSafety(verdict.announce);
      }

      let rafPending = false;
      function scheduleDraw() {
        if (rafPending) return;
        rafPending = true;
        requestAnimationFrame(() => {
          rafPending = false;
          drawCharts();
        });
      }

      function canShowPmTrue() {
        if (activeSource() === 'sitl') return true;
        if (state.sim && state.sim.target === 'hils') return true;
        return state.src.esp32.history.some((r) => r.pm_true != null);
      }

      function syncLayerButtons() {
        const allowPmTrue = canShowPmTrue();
        const layerBtns = Array.from(document.querySelectorAll('.layer-btn'));
        let activeCount = 0;
        layerBtns.forEach((btn) => {
          const layer = btn.getAttribute('data-layer');
          const pressed = !!state.layers[layer];
          btn.setAttribute('aria-pressed', pressed ? 'true' : 'false');
          if (layer === 'pm_true') {
            btn.disabled = !allowPmTrue;
            btn.title = allowPmTrue ? 'Kurva referensi konsentrasi ruang asap (ground truth)' : 'Hanya tersedia pada simulasi SITL atau HILS';
          }
          if (pressed && !btn.disabled) activeCount += 1;
        });
        const lblLayers = document.getElementById('lbl-toggle-layers');
        if (lblLayers) lblLayers.textContent = `Atur Kurva (${activeCount}/${layerBtns.length})`;
        document.querySelectorAll('.preset-btn').forEach((b) => {
          b.setAttribute('aria-pressed', b.getAttribute('data-preset') === state.preset ? 'true' : 'false');
        });
        const customBadge = document.getElementById('badge-preset-custom');
        if (customBadge) customBadge.hidden = state.preset !== 'custom';
        const selWin = document.getElementById('sel-window');
        if (selWin) selWin.value = String(state.windowSec);
        const swZoom = document.getElementById('sw-zoom-y');
        if (swZoom) swZoom.checked = !!state.zoomY;
        syncLayerChipColors();
        renderChartLegend();
        syncAllToolbars();
      }

      const FSM_ICON_PATHS = {
        IDLE: '<circle cx="12" cy="12" r="9"/><line x1="8" y1="12" x2="16" y2="12"/>',
        NORMAL: '<path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/>',
        WASPADA: '<path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>',
        BAHAYA: '<path d="M12 2c.5 4-3 6-3 10a5 5 0 0 0 10 0c0-3-2-5.5-4-8"/><path d="M9 14a3 3 0 0 0 6 0"/>',
        FAILSAFE: '<circle cx="12" cy="12" r="9"/><line x1="4.93" y1="4.93" x2="19.07" y2="19.07"/>',
      };

      function setFsmBadge(st, tone) {
        const badge = document.getElementById('badge-fsm');
        const svg = document.getElementById('ico-fsm-svg');
        const kpiSvg = document.getElementById('kpi-state-ico');
        const txt = document.getElementById('txt-fsm');
        if (tone) badge.setAttribute('data-tone', tone);
        else badge.removeAttribute('data-tone');
        const key = FSM_ICON_PATHS[st] ? st : 'IDLE';
        if (svg) svg.innerHTML = FSM_ICON_PATHS[key];
        if (kpiSvg) kpiSvg.innerHTML = FSM_ICON_PATHS[key];
        txt.textContent = key === 'IDLE' ? 'FSM: —' : `FSM: ${st}`;
      }

      function renderRunSummary(summary) {
        const card = document.getElementById('run-summary-card');
        if (!card) return;
        if (!summary) {
          card.hidden = true;
          return;
        }
        state.lastRunSummary = summary;
        document.getElementById('rs-scenario').textContent = SCENARIO_LABELS[summary.scenario] || summary.scenario || '-';
        document.getElementById('rs-target').textContent = (summary.target || 'sitl').toUpperCase();
        document.getElementById('rs-peak').textContent = `${Number(summary.peak_fi || 0).toFixed(1)} µg/m³`;
        document.getElementById('rs-bahaya').textContent =
          summary.t_bahaya_s != null ? `t=${Number(summary.t_bahaya_s).toFixed(1)}s` : 'Tidak tercapai';
        document.getElementById('rs-preempt').textContent = summary.ai_preempt_used ? 'Aktif' : 'Tidak';
        document.getElementById('rs-samples').textContent = String(summary.samples ?? 0);
        const parWrap = document.getElementById('rs-parity-wrap');
        const parVal = document.getElementById('rs-parity');
        const hp = summary.hils_parity;
        if (hp && hp.samples > 0) {
          const ok = hp.max_delta_fi < 1.0 && hp.state_matches === hp.samples;
          parWrap.hidden = false;
          parVal.textContent = `ΔFI maks ${Number(hp.max_delta_fi || 0).toFixed(2)} (${ok ? 'OK' : 'Cek'})`;
        } else {
          parWrap.hidden = true;
        }
        card.hidden = false;
      }

      function hideRunSummary() {
        state.lastRunSummary = null;
        const card = document.getElementById('run-summary-card');
        if (card) card.hidden = true;
      }

      function updateOverrideControlState() {
        const sim = state.sim || {};
        const port = sim.broker_port || 6669;
        const hasSitl = !!sim.has_sitl;
        const hasHilsRef = !!sim.has_hils_ref;
        const hasBroker = !!sim.has_broker;
        const targetLock = state.cmdTarget || 'auto';
        const selCmd = document.getElementById('sel-cmd-target');
        if (selCmd && selCmd.value !== targetLock) {
          selCmd.value = targetLock;
        }
        let canSend = !!sim.can_send_cmd;
        if (targetLock === 'sitl') {
          canSend = hasSitl;
        } else if (targetLock === 'esp32') {
          canSend = hasBroker || hasHilsRef;
        }
        const line = document.getElementById('override-target-line');
        if (line) {
          let dest = `Belum ada jalur kendali aktif — mulai simulasi SITL atau hubungkan broker :${port}`;
          if (targetLock === 'sitl') {
            dest = hasSitl
              ? 'Tujuan terkunci: SITL (Python) saja — broker MQTT tidak disentuh'
              : 'Target SITL dipilih — mulai simulasi SITL untuk mengaktifkan tombol';
          } else if (targetLock === 'esp32') {
            dest = hasBroker
              ? (hasHilsRef
                  ? `Tujuan terkunci: ESP32 + Referensi HILS via :${port} (SITL diabaikan)`
                  : `Tujuan terkunci: ESP32 via Broker :${port} saja (SITL diabaikan)`)
              : `Target ESP32 dipilih — broker MQTT :${port} belum terhubung`;
          } else {
            if (hasHilsRef && hasBroker) {
              dest = `Tujuan aktif: HILS (ESP32 + Referensi via :${port})`;
            } else if (hasSitl && hasBroker) {
              dest = `Tujuan aktif: SITL (Python) + Broker MQTT :${port}`;
            } else if (hasSitl) {
              dest = 'Tujuan aktif: SITL (Python)';
            } else if (hasBroker) {
              dest = `Tujuan aktif: Broker MQTT :${port} (ESP32)`;
            }
            if (activeSource() === 'esp32' && hasSitl) {
              dest += ' (perhatian: simulasi SITL juga ikut terpengaruh)';
            }
          }
          line.textContent = dest;
        }
        document.querySelectorAll('#cmd-toolbar .cmd-btn[data-cmd], #btn-cmd-reset, #btn-mob-reset, .mob-cmd-btn[data-cmd]').forEach((btn) => {
          btn.disabled = !canSend;
        });
        syncAllToolbars();
      }

      function updateSimAdvancedSummary() {
        const el = document.getElementById('sim-adv-summary');
        if (!el) return;
        const sim = state.sim || {};
        const spdBtn = document.querySelector('.spd-btn[aria-pressed="true"]');
        const spd = sim.speed || (spdBtn ? Number(spdBtn.getAttribute('data-speed') || 1) : 1);
        const swPre = document.getElementById('sw-preempt');
        const swFlt = document.getElementById('sw-fault');
        const pre = swPre ? swPre.checked : !!sim.ai_preempt;
        const flt = swFlt ? swFlt.checked : !!sim.fault_inject;
        const parts = [`${spd}x`];
        if (pre) parts.push('AI Preempt');
        if (flt) parts.push('Fault');
        if (!pre && !flt) parts.push('Standar');
        el.textContent = parts.join(' • ');
        if (flt) el.setAttribute('data-tone', 'danger');
        else if (pre) el.setAttribute('data-tone', 'accent');
        else el.removeAttribute('data-tone');
      }

      function updateMobileDockContext() {
        const el = document.getElementById('mob-dock-context');
        if (!el) return;
        const sim = state.sim || {};
        const tgt = ((sim.running ? sim.target : state.simTarget) || 'sitl').toUpperCase();
        const selEl = document.getElementById('sel-scenario');
        const scenKey = (sim.running && sim.scenario) ? sim.scenario : ((selEl && selEl.value) || sim.scenario || 'flaming');
        const scenLabel = SCENARIO_LABELS[scenKey] || scenKey;
        const spdBtn = document.querySelector('.spd-btn[aria-pressed="true"]');
        const spd = sim.speed || (spdBtn ? Number(spdBtn.getAttribute('data-speed') || 1) : 1);
        const statusSuffix = sim.running ? (sim.paused ? ' • Jeda' : ' • Berjalan') : '';
        el.textContent = `${tgt} • ${scenLabel} • ${spd}x${statusSuffix}`;
      }

      function updateParityUI(sim, row) {
        const parity = sim && sim.hils_parity;
        const badge = document.getElementById('parity-badge');
        const kpiWrap = document.getElementById('kpi-parity-wrap');
        const kpiParity = document.getElementById('kpi-parity');
        const showParity = parity && parity.samples > 0 && (activeSource() === 'esp32' || (sim && sim.target === 'hils'));
        if (showParity && parity.last_delta_fi != null) {
          const ok = parity.max_delta_fi < 1.0 && parity.state_matches === parity.samples;
          badge.hidden = false;
          badge.setAttribute('data-tone', ok ? 'ok' : 'warn');
          badge.textContent = `Paritas HILS: ΔFI ${parity.last_delta_fi.toFixed(2)} (${parity.state_matches}/${parity.samples} sama)`;
          kpiWrap.hidden = false;
          kpiParity.textContent = `Δ${parity.last_delta_fi.toFixed(2)} (${ok ? 'OK' : 'Cek'})`;
          kpiParity.style.color = ok ? 'var(--ok)' : 'var(--warn)';
        } else {
          badge.hidden = true;
          const waitingHils = !!(sim && sim.running && sim.target === 'hils');
          kpiWrap.hidden = !waitingHils;
          kpiParity.textContent = waitingHils ? 'menunggu' : '-';
          kpiParity.style.color = '';
        }
      }

      function resetKpis() {
        const cardState = document.getElementById('card-state');
        const cardFi = document.getElementById('card-fi');
        cardState.setAttribute('data-tone', 'idle');
        cardFi.setAttribute('data-tone', 'idle');
        document.getElementById('kpi-state').textContent = 'BELUM ADA DATA';
        document.getElementById('kpi-state-wrap').style.color = 'var(--text-muted)';
        document.getElementById('kpi-mode').textContent = '--';
        document.getElementById('kpi-t').textContent = '0s';
        const durEl = document.getElementById('kpi-t-dur');
        if (durEl) {
          durEl.textContent = '';
          durEl.hidden = true;
        }
        document.getElementById('kpi-seq').textContent = '0';
        document.getElementById('kpi-fi').textContent = '--';
        document.getElementById('kpi-fi-wrap').style.color = 'var(--text-muted)';
        document.getElementById('fi-meter-fill').style.inlineSize = '0%';
        document.getElementById('kpi-pm').textContent = '--';
        document.getElementById('kpi-pmraw').textContent = '--';
        document.getElementById('kpi-dcdt').textContent = '0.00';
        document.getElementById('kpi-vd').textContent = '--';
        document.getElementById('kpi-v0').textContent = '--';
        document.getElementById('kpi-ppm').textContent = '--';
        document.getElementById('kpi-g').textContent = '--';
        document.getElementById('kpi-vm').textContent = '--';
        document.getElementById('kpi-fc').textContent = '--';
        document.getElementById('kpi-fc-wrap').style.color = 'var(--text-muted)';
        document.getElementById('kpi-eta').textContent = '-';
        document.getElementById('kpi-mae').textContent = '-';
        document.getElementById('kpi-duty').textContent = '--';
        document.getElementById('kpi-duty-wrap').style.color = 'var(--text-muted)';
        document.getElementById('pwm-meter-fill').style.inlineSize = '0%';
        document.getElementById('kpi-fan-ctrl').textContent = '--';
        document.getElementById('kpi-fan-state').textContent = '--';
        document.querySelectorAll('#cmd-toolbar .cmd-btn[data-cmd], .mob-cmd-btn[data-cmd]').forEach((btn) => {
          if (btn.getAttribute('data-cmd') !== 'RESET') {
            btn.setAttribute('aria-pressed', 'false');
          }
        });
        setFsmBadge('IDLE', null);
        updateParityUI(state.sim, null);
        updateSafetyCallout(null, state.sim);
      }

      function applyView() {
        const src = activeSource();
        state.history = state.src[src].history;
        const srcTag = src === 'esp32' ? 'ESP32 (hardware)' : 'SITL (simulasi)';
        document.getElementById('src-label').textContent = srcTag + (state.viewMode === 'auto' ? ' — otomatis' : '');
        document.getElementById('kpi-src-tag').textContent = src === 'esp32' ? 'ESP32' : 'SITL';
        document.querySelectorAll('.src-btn').forEach((b) =>
          b.setAttribute('aria-pressed', b.getAttribute('data-src') === state.viewMode ? 'true' : 'false'));
        const btnExport = document.getElementById('btn-export-png');
        const btnExportCsv = document.getElementById('btn-export-csv');
        const btnClear = document.getElementById('btn-sim-clear');
        if (btnExport) btnExport.disabled = state.history.length === 0;
        if (btnExportCsv) btnExportCsv.disabled = state.history.length === 0;
        if (btnClear) btnClear.disabled = state.history.length === 0;
        syncLayerButtons();
        const lat = state.src[src].latest;
        if (lat && (state.history.length > 0 || lat.scenario !== 'idle')) {
          updateKpis(lat, state.sim);
        } else {
          resetKpis();
        }
        updateOverrideControlState();
        updateBrokerBadge();
        updateEspBadge();
        scheduleDraw();
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
        state.lastActiveSource = source;
      }

      function updateKpis(row, sim) {
        if (!row) {
          resetKpis();
          return;
        }
        const st = row.state || 'NORMAL';
        const toneMap = { NORMAL: 'ok', WASPADA: 'warn', BAHAYA: 'danger', FAILSAFE: 'danger' };
        const colorMap = { NORMAL: 'var(--ok)', WASPADA: 'var(--warn)', BAHAYA: 'var(--danger)', FAILSAFE: 'var(--danger)' };
        const tone = toneMap[st] || 'ok';

        document.getElementById('card-state').setAttribute('data-tone', tone);
        document.getElementById('card-fi').setAttribute('data-tone', tone);
        document.getElementById('kpi-state').textContent = st;
        document.getElementById('kpi-state-wrap').style.color = colorMap[st] || 'var(--ok)';
        document.getElementById('kpi-mode').textContent = row.mode || 'AUTO';
        document.getElementById('kpi-t').textContent = formatTime(row.t ?? 0);
        const activeSim = sim || state.sim;
        const durEl = document.getElementById('kpi-t-dur');
        if (durEl) {
          const dur = activeSim && Number(activeSim.duration);
          const showDur = dur > 0 && (activeSim.running || activeSource() === 'sitl' || activeSim.target === 'hils');
          if (showDur) {
            durEl.textContent = ` / ${formatTime(dur)}`;
            durEl.hidden = false;
          } else {
            durEl.textContent = '';
            durEl.hidden = true;
          }
        }
        document.getElementById('kpi-seq').textContent = row.seq ?? 0;

        const fiVal = row.fi ?? 0;
        const fiColor = fiVal >= 150 ? 'var(--danger)' : (fiVal >= 55 ? 'var(--warn)' : 'var(--ok)');
        document.getElementById('kpi-fi').textContent = fiVal.toFixed(1);
        document.getElementById('kpi-fi-wrap').style.color = fiVal >= 55 ? fiColor : 'var(--text-main)';
        const fiFill = document.getElementById('fi-meter-fill');
        fiFill.style.inlineSize = `${Math.min(100, Math.max(0, (fiVal / 250) * 100)).toFixed(1)}%`;
        fiFill.style.backgroundColor = fiColor;

        document.getElementById('kpi-pm').textContent = (row.pm ?? 0).toFixed(1);
        document.getElementById('kpi-pmraw').textContent = (row.pm_raw ?? 0).toFixed(1);
        const dcdt = row.dcdt ?? 0;
        document.getElementById('kpi-dcdt').textContent = (dcdt >= 0 ? '+' : '') + dcdt.toFixed(2);
        document.getElementById('kpi-vd').textContent = (row.v_dust ?? 0).toFixed(2);
        document.getElementById('kpi-v0').textContent = (row.v0 ?? 0.6).toFixed(3);

        document.getElementById('kpi-ppm').textContent = (row.gas_ppm ?? 400).toFixed(0);
        document.getElementById('kpi-g').textContent = (row.g ?? 0).toFixed(2);
        document.getElementById('kpi-vm').textContent = (row.v_mq ?? 0).toFixed(2);

        const fcVal = row.forecast;
        document.getElementById('kpi-fc').textContent = fcVal != null ? fcVal.toFixed(1) : '-';
        document.getElementById('kpi-fc-wrap').style.color =
          fcVal == null ? 'var(--text-muted)'
          : (fcVal >= 150 ? 'var(--danger)' : (fcVal >= 55 ? 'var(--warn)' : 'var(--violet)'));
        document.getElementById('kpi-eta').textContent = row.eta_bahaya_s != null ? `${row.eta_bahaya_s.toFixed(0)} s` : '-';
        document.getElementById('kpi-mae').textContent = row.mae != null ? row.mae.toFixed(2) : '-';

        const dutyVal = row.duty ?? 0;
        const dutyColor = row.mode === 'FORCE_OFF' ? 'var(--warn)'
          : (st === 'BAHAYA' ? 'var(--danger)' : (st === 'FAILSAFE' ? 'var(--warn)' : (dutyVal > 0 ? 'var(--accent)' : 'var(--ok)')));
        document.getElementById('kpi-duty').textContent = dutyVal.toFixed(1);
        document.getElementById('kpi-duty-wrap').style.color = dutyColor;
        const pwmFill = document.getElementById('pwm-meter-fill');
        pwmFill.style.inlineSize = `${Math.min(100, Math.max(0, dutyVal)).toFixed(1)}%`;
        pwmFill.style.backgroundColor = dutyColor;

        const ctrlLabel = (activeSim && activeSim.ai_preempt_active && row.mode === 'FORCE_MAX') ? 'AI PREEMPT' : (row.mode || 'AUTO');
        document.getElementById('kpi-fan-ctrl').textContent = ctrlLabel;
        document.getElementById('kpi-fan-state').textContent = describeFanState(row);

        setFsmBadge(st, tone);

        document.querySelectorAll('#cmd-toolbar .cmd-btn[data-cmd], .mob-cmd-btn[data-cmd]').forEach((btn) => {
          const c = btn.getAttribute('data-cmd');
          if (c !== 'RESET') {
            btn.setAttribute('aria-pressed', c === row.mode ? 'true' : 'false');
          }
        });

        updateParityUI(activeSim, row);
        updateSafetyCallout(row, activeSim);
        if (sim) updateSimBadges(sim);
      }

      function updateSimBadges(sim) {
        if (!sim) return;
        state.sim = sim;
        if (sim.target) {
          state.simTarget = sim.target;
          document.querySelectorAll('.tgt-btn').forEach((b) => {
            b.setAttribute('aria-pressed', b.getAttribute('data-target') === sim.target ? 'true' : 'false');
          });
          document.getElementById('hils-concurrency-hint').hidden = sim.target !== 'hils';
        }
        if (sim.scenario && (!dlg || !dlg.open)) {
          document.getElementById('sel-scenario').value = sim.scenario;
        }
        if (sim.speed != null) {
          document.querySelectorAll('.spd-btn').forEach((b) => {
            b.setAttribute('aria-pressed', Number(b.getAttribute('data-speed')) === Number(sim.speed) ? 'true' : 'false');
          });
        }
        if (typeof sim.ai_preempt === 'boolean') {
          document.getElementById('sw-preempt').checked = sim.ai_preempt;
        }
        if (typeof sim.fault_inject === 'boolean') {
          document.getElementById('sw-fault').checked = sim.fault_inject;
        }
        if (sim.running && sim.hils_consec_timeouts === 0) {
          state.hilsTimeoutInfo = null;
        }

        const tgtLabel = (sim.target || 'sitl').toUpperCase();
        const bSim = document.getElementById('badge-sim-state');
        if (bSim) {
          bSim.hidden = !!sim.running;
          if (!sim.running) {
            bSim.textContent = 'Tidak aktif';
            bSim.removeAttribute('data-tone');
          }
        }

        const btnStart = document.getElementById('btn-sim-start');
        const lblStart = document.getElementById('lbl-sim-start');
        const kbdStart = document.getElementById('kbd-sim-start');
        const btnPause = document.getElementById('btn-sim-pause');
        const lblPause = document.getElementById('lbl-sim-pause');
        const btnStop = document.getElementById('btn-sim-stop');
        const mobPlay = document.getElementById('mob-btn-playpause');
        const mobStop = document.getElementById('mob-btn-stop');
        btnStart.classList.toggle('btn-primary', !sim.running);
        lblStart.textContent = sim.running ? 'Ulang' : 'Mulai';
        if (kbdStart) kbdStart.hidden = !!sim.running;
        btnPause.disabled = !sim.running;
        btnPause.setAttribute('aria-pressed', sim.paused ? 'true' : 'false');
        lblPause.textContent = sim.paused ? 'Lanjut' : 'Jeda';
        btnStop.disabled = !sim.running;
        if (mobPlay) {
          mobPlay.classList.toggle('btn-primary', !sim.running);
          mobPlay.textContent = !sim.running ? 'Mulai' : (sim.paused ? 'Lanjut' : 'Jeda');
        }
        if (mobStop) mobStop.disabled = !sim.running;

        const activeBar = document.getElementById('sim-active-bar');
        const activeTxt = document.getElementById('sim-active-text');
        const cfgFields = document.getElementById('sim-config-fields');
        const toggleBtn = document.getElementById('btn-toggle-sim-cfg');
        if (sim.running) {
          activeBar.hidden = false;
          const scenLbl = SCENARIO_LABELS[sim.scenario] || sim.scenario;
          if (sim.target === 'hils' && !state.esp.online) {
            activeTxt.textContent = `HILS • Menunggu ESP32 (${scenLbl})`;
          } else {
            activeTxt.textContent = `${tgtLabel} • ${scenLbl} (${sim.speed || 1}x${sim.paused ? ', Jeda' : ''})`;
          }
          cfgFields.hidden = !state.simConfigExpanded;
          toggleBtn.setAttribute('aria-expanded', state.simConfigExpanded ? 'true' : 'false');
        } else {
          state.simConfigExpanded = false;
          activeBar.hidden = true;
          cfgFields.hidden = false;
          toggleBtn.setAttribute('aria-expanded', 'false');
        }

        if (sim.log_file) {
          document.getElementById('txt-logfile').textContent = sim.log_file;
        }
        updateSimAdvancedSummary();
        updateOverrideControlState();
        updateMobileDockContext();
        updateBrokerBadge();
        syncAllToolbars();
      }

      function niceStep(rawStep) {
        if (rawStep <= 0) return 50;
        const pow = Math.pow(10, Math.floor(Math.log10(rawStep)));
        const frac = rawStep / pow;
        let nice = 1;
        if (frac <= 1) nice = 1;
        else if (frac <= 2) nice = 2;
        else if (frac <= 2.5) nice = 2.5;
        else if (frac <= 5) nice = 5;
        else nice = 10;
        return nice * pow;
      }

      function syncHoverDismissBtn() {
        const btn = document.getElementById('btn-clear-hover');
        if (btn) btn.hidden = !(state.hoverFromTouch && state.hoverT != null);
      }

      function clearHover() {
        state.hoverT = null;
        state.hoverIdx = null;
        state.hoverFromKey = false;
        state.hoverFromTouch = false;
        syncHoverDismissBtn();
      }

      function drawCharts() {
        syncCanvasSize(mainCanvas, 960, 320);
        syncCanvasSize(subCanvas, 960, 150);

        const w = mainCanvas._cw;
        const h = mainCanvas._ch;
        const dpr = mainCanvas._dpr || 1;
        const sw = subCanvas._cw;
        const sh = subCanvas._ch;
        const sdpr = subCanvas._dpr || 1;

        mCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
        sCtx.setTransform(sdpr, 0, 0, sdpr, 0, 0);
        mCtx.clearRect(0, 0, w, h);
        sCtx.clearRect(0, 0, sw, sh);

        const dark = isDarkTheme();
        const palette = chartPalette(dark);
        const monoFont = 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace';

        const rows = state.history;
        const compact = w < 520;
        const fsmCompact = w < 420;
        const padL = compact ? 44 : 54;
        const padR = fsmCompact ? 46 : 74;
        const padT = 26;
        const padB = 28;
        const plotW = Math.max(40, w - padL - padR);
        const plotH = Math.max(40, h - padT - padB);

        const sPadT = 22, sPadB = 10;
        const sInnerH = Math.max(54, sh - sPadT - sPadB);
        const dutyTop = sPadT;
        const dutyH = Math.max(28, Math.round(sInnerH * 0.60));
        const dutyBot = dutyTop + dutyH;
        const sepY = dutyBot + Math.max(3, Math.round(sInnerH * 0.06));
        const ribbonTop = dutyBot + Math.max(6, Math.round(sInnerH * 0.12));
        const ribbonH = Math.max(16, sh - sPadB - ribbonTop);
        const syOf = (v) => dutyTop + dutyH - (Math.max(0, Math.min(100, v)) / 100) * dutyH;

        function drawSubBase() {
          sCtx.strokeStyle = palette.grid;
          sCtx.fillStyle = palette.text;
          sCtx.font = `11px ${monoFont}`;
          sCtx.lineWidth = 1;
          for (const pct of [0, 50, 100]) {
            const y = syOf(pct);
            sCtx.beginPath(); sCtx.moveTo(padL, y); sCtx.lineTo(sw - padR, y); sCtx.stroke();
            sCtx.fillText(`${pct}%`, 6, y + 4);
          }

          // Subtle 70% FAILSAFE duty mark in upper lane
          const y70 = syOf(70);
          sCtx.save();
          sCtx.strokeStyle = palette.waspada;
          sCtx.globalAlpha = 0.55;
          sCtx.setLineDash([4, 4]);
          sCtx.beginPath(); sCtx.moveTo(padL, y70); sCtx.lineTo(sw - padR, y70); sCtx.stroke();
          sCtx.restore();
          sCtx.font = `600 10px ${monoFont}`;
          sCtx.fillStyle = palette.waspada;
          sCtx.fillText(fsmCompact ? '70%' : '70% FS', sw - padR + 5, y70 + 3);

          // Hairline separator between upper duty lane and lower FSM state ribbon lane
          sCtx.strokeStyle = palette.grid;
          sCtx.beginPath(); sCtx.moveTo(padL, sepY); sCtx.lineTo(sw - padR, sepY); sCtx.stroke();

          // Lower FSM ribbon lane label
          sCtx.font = `600 10.5px ${monoFont}`;
          sCtx.fillStyle = palette.text;
          sCtx.fillText('FSM', 6, ribbonTop + ribbonH / 2 + 4);

          sCtx.font = '700 11.5px system-ui, sans-serif';
          sCtx.fillStyle = palette.duty;
          sCtx.fillRect(padL, 8, 14, 3);
          sCtx.fillStyle = palette.text;
          sCtx.fillText('Duty Kipas (0–100%)', padL + 18, 13);

          const fsmLegX = padL + (compact ? 140 : 156);
          sCtx.fillStyle = palette.fsm;
          sCtx.fillRect(fsmLegX, 8, 14, 3);
          sCtx.fillStyle = palette.text;
          sCtx.fillText('Pita State FSM', fsmLegX + 18, 13);
        }

        drawSubBase();

        if (!rows.length) {
          mCtx.fillStyle = palette.text;
          mCtx.font = '600 13px system-ui, sans-serif';
          const msg = activeSource() === 'esp32'
            ? 'Menunggu telemetri ESP32 dari broker MQTT (:6669)...'
            : 'Belum ada data SITL — tekan "Mulai" di panel kanan atau di bawah.';
          mCtx.fillText(msg, Math.min(padL + 10, 24), h / 2, w - 36);

          sCtx.fillStyle = palette.muted;
          sCtx.font = '600 11.5px system-ui, sans-serif';
          sCtx.fillText('Belum ada histori duty kipas & pita state FSM.', Math.min(padL + 10, 24), dutyTop + dutyH / 2 + 4, sw - padL - padR - 12);
          document.getElementById('chart-sr-summary').textContent = msg;
          const subSr = document.getElementById('subchart-sr-summary');
          if (subSr) subSr.textContent = 'Belum ada histori duty kipas dan state FSM.';
          return;
        }

        const monotonic = [];
        for (const r of rows) {
          const rt = r.t ?? 0;
          if (monotonic.length && rt < (monotonic[monotonic.length - 1].t ?? 0)) monotonic.length = 0;
          if (monotonic.length && rt === (monotonic[monotonic.length - 1].t ?? -1)) monotonic[monotonic.length - 1] = r;
          else monotonic.push(r);
        }

        const maxT = monotonic[monotonic.length - 1].t ?? 0;
        const minT = state.windowSec > 0 ? Math.max(0, maxT - state.windowSec) : (monotonic[0].t ?? 0);
        const spanT = Math.max(60, (maxT + (state.layers.forecast ? 60 : 5)) - minT);
        const visible = monotonic.filter((r) => (r.t ?? 0) >= minT);

        let dataMax = 0;
        visible.forEach((r) => {
          if (state.layers.fi && r.fi != null) dataMax = Math.max(dataMax, r.fi * 1.12);
          if (state.layers.pm && r.pm != null) dataMax = Math.max(dataMax, r.pm * 1.12);
          if (state.layers.pm_true && r.pm_true != null) dataMax = Math.max(dataMax, r.pm_true * 1.12);
          if (state.layers.pm_raw && r.pm_raw != null) dataMax = Math.max(dataMax, r.pm_raw * 1.12);
          if (state.layers.forecast && r.forecast != null) dataMax = Math.max(dataMax, r.forecast * 1.12);
          if (state.layers.gas_ppm && r.gas_ppm != null) dataMax = Math.max(dataMax, r.gas_ppm * 0.22);
          if (state.layers.g_scaled && r.g != null) dataMax = Math.max(dataMax, r.g * 105);
          if (state.layers.voltages) {
            dataMax = Math.max(dataMax, (r.v_dust ?? 0) * 55, (r.v_mq ?? 0) * 55);
          }
        });

        const baseFloor = state.zoomY ? Math.max(20, dataMax) : (state.layers.thresholds ? 170 : Math.max(40, dataMax));
        const rawMaxY = Math.max(baseFloor, dataMax);
        const yStep = niceStep(rawMaxY / 4);
        const maxY = Math.ceil(rawMaxY / yStep) * yStep;

        const xOf = (t) => padL + ((t - minT) / spanT) * plotW;
        const yOf = (v) => padT + plotH - (Math.max(0, Math.min(maxY, v)) / maxY) * plotH;

        // Future +60s horizon shading & vertical "SEKARANG" boundary when forecast active
        if (state.layers.forecast) {
          const xNow = xOf(maxT);
          const xRight = w - padR;
          if (xNow < xRight) {
            mCtx.save();
            mCtx.fillStyle = palette.forecastFill;
            mCtx.fillRect(xNow, padT, xRight - xNow, plotH);
            mCtx.strokeStyle = palette.nowLine;
            mCtx.lineWidth = 1.25;
            mCtx.setLineDash([4, 3]);
            mCtx.beginPath();
            mCtx.moveTo(xNow, padT);
            mCtx.lineTo(xNow, h - padB);
            mCtx.stroke();
            mCtx.fillStyle = palette.forecast;
            mCtx.font = `700 10.5px ${monoFont}`;
            const nowLbl = compact ? 'KINI' : 'SEKARANG → +60s';
            const txtW = mCtx.measureText(nowLbl).width;
            mCtx.fillText(nowLbl, Math.min(xRight - txtW - 4, xNow + 5), padT + 12);
            mCtx.restore();
          }
        }

        // Right-axis scale definitions for active scaled layers
        const rightAxes = [];
        if (state.layers.gas_ppm) rightAxes.push({ unit: 'ppm', color: palette.gas_ppm, fmt: (v) => `${Math.round(v / 0.2)}` });
        if (state.layers.g_scaled) rightAxes.push({ unit: 'g', color: palette.g_scaled, fmt: (v) => (v / 100).toFixed(1) });
        if (state.layers.voltages) rightAxes.push({ unit: 'V', color: palette.v_dust, fmt: (v) => (v / 50).toFixed(1) });

        // Horizontal Y grid + rounded labels + optional right-axis ticks
        mCtx.strokeStyle = palette.grid;
        mCtx.fillStyle = palette.text;
        mCtx.font = `11.5px ${monoFont}`;
        mCtx.lineWidth = 1;
        for (let val = 0; val <= maxY + 0.001; val += yStep) {
          const y = yOf(val);
          mCtx.beginPath();
          mCtx.moveTo(padL, y);
          mCtx.lineTo(w - padR, y);
          mCtx.stroke();
          mCtx.fillStyle = palette.text;
          mCtx.fillText(Math.round(val).toString(), 6, y + 4);
          if (rightAxes.length === 1) {
            mCtx.fillStyle = rightAxes[0].color;
            mCtx.font = `600 10.5px ${monoFont}`;
            mCtx.fillText(`${rightAxes[0].fmt(val)}${fsmCompact ? '' : ' ' + rightAxes[0].unit}`, w - padR + 4, y + 4);
            mCtx.font = `11.5px ${monoFont}`;
          } else if (rightAxes.length > 1) {
            const primary = rightAxes[0];
            mCtx.fillStyle = primary.color;
            mCtx.font = `600 10px ${monoFont}`;
            const composite = fsmCompact
              ? primary.fmt(val)
              : rightAxes.map((ax) => ax.fmt(val)).join('/');
            mCtx.fillText(composite, w - padR + 4, y + 4, padR - 6);
            mCtx.font = `11.5px ${monoFont}`;
          }
        }
        mCtx.fillStyle = palette.text;
        mCtx.font = '700 11.5px system-ui, sans-serif';
        mCtx.fillText('µg/m³ / FI', 6, 15);
        if (rightAxes.length > 0) {
          mCtx.font = `700 10.5px ${monoFont}`;
          mCtx.fillStyle = rightAxes[0].color;
          const unitHdr = rightAxes.map((a) => a.unit).join('/');
          mCtx.fillText(unitHdr, w - padR + 4, 15, padR - 6);
        }

        const xStep = niceStep(spanT / (compact ? 4 : 6));
        const firstTick = Math.ceil(minT / xStep) * xStep;
        mCtx.font = `11.5px ${monoFont}`;
        mCtx.fillStyle = palette.text;
        let lastTickX = -Infinity;
        for (let tv = firstTick; tv <= minT + spanT + 0.001; tv += xStep) {
          const x = xOf(tv);
          mCtx.beginPath();
          mCtx.moveTo(x, padT);
          mCtx.lineTo(x, h - padB);
          mCtx.stroke();
          const tx = Math.max(padL, Math.min(w - padR - 26, x - 10));
          if (tx - lastTickX >= 38) {
            mCtx.fillText(`${Math.round(tv)}s`, tx, h - 9);
            lastTickX = tx;
          }
        }

        // Threshold lines (when within current Y range) + tight-zoom note when < 55
        if (state.layers.thresholds) {
          mCtx.save();
          mCtx.font = '700 11px system-ui, sans-serif';
          mCtx.setLineDash([7, 5]);
          if (55 <= maxY) {
            const yW = yOf(55);
            mCtx.strokeStyle = palette.waspada;
            mCtx.beginPath(); mCtx.moveTo(padL, yW); mCtx.lineTo(w - padR, yW); mCtx.stroke();
            mCtx.fillStyle = palette.waspada;
            mCtx.fillText('WASPADA (55)', padL + 6, yW - 5);
          }
          if (150 <= maxY) {
            const yB = yOf(150);
            mCtx.strokeStyle = palette.bahaya;
            mCtx.beginPath(); mCtx.moveTo(padL, yB); mCtx.lineTo(w - padR, yB); mCtx.stroke();
            mCtx.fillStyle = palette.bahaya;
            mCtx.fillText('BAHAYA (150)', padL + 6, yB - 5);
          }
          if (state.zoomY && maxY < 55) {
            mCtx.setLineDash([]);
            mCtx.font = '600 10.5px system-ui, sans-serif';
            mCtx.fillStyle = palette.muted;
            const zoomNote = 'Ambang 55/150 di luar skala ketat';
            const nw = mCtx.measureText(zoomNote).width;
            const nx = state.layers.forecast ? (padL + 8) : Math.max(padL + 8, w - padR - nw - 8);
            mCtx.fillText(zoomNote, nx, padT + 13);
          }
          mCtx.restore();
        }

        function plotSeries(extractFn, color, width = 2, dash = [], xOffset = 0, drawTipDot = false) {
          mCtx.save();
          mCtx.strokeStyle = color;
          mCtx.lineWidth = width;
          mCtx.setLineDash(dash);
          mCtx.beginPath();
          let started = false;
          let prevT = -Infinity;
          let lastX = null, lastY = null;
          for (const r of visible) {
            const v = extractFn(r);
            if (v == null || Number.isNaN(v)) continue;
            const rt = (r.t ?? 0) + xOffset;
            const x = xOf(rt);
            const y = yOf(v);
            if (!started || rt <= prevT) { mCtx.moveTo(x, y); started = true; }
            else mCtx.lineTo(x, y);
            prevT = rt;
            lastX = x;
            lastY = y;
          }
          mCtx.stroke();
          if (drawTipDot && lastX != null) {
            mCtx.setLineDash([]);
            mCtx.fillStyle = color;
            mCtx.beginPath();
            mCtx.arc(lastX, lastY, 3.5, 0, Math.PI * 2);
            mCtx.fill();
          }
          mCtx.restore();
        }

        if (state.layers.pm_raw) plotSeries((r) => r.pm_raw, palette.pm_raw, 1.4);
        if (state.layers.pm_true) plotSeries((r) => r.pm_true, palette.pm_true, 2.0);
        if (state.layers.voltages) {
          plotSeries((r) => (r.v_dust ?? 0) * 50, palette.v_dust, 1.6, [4, 3]);
          plotSeries((r) => (r.v_mq ?? 0) * 50, palette.v_mq, 1.6, [6, 3]);
        }
        if (state.layers.gas_ppm) plotSeries((r) => (r.gas_ppm ?? 400) * 0.2, palette.gas_ppm, 1.8, [4, 3]);
        if (state.layers.g_scaled) plotSeries((r) => (r.g ?? 0) * 100, palette.g_scaled, 1.8, [2, 3]);
        if (state.layers.pm) plotSeries((r) => r.pm, palette.pm, 2.4);
        if (state.layers.fi) plotSeries((r) => r.fi, palette.fi, 2.8);
        if (state.layers.forecast) plotSeries((r) => r.forecast, palette.forecast, 2.4, [3, 3], 60, true);

        // Sub-chart Lane 1 (Upper): Duty Kipas (%) step function
        function plotStepSub(extractYFn, color, width, dash = []) {
          sCtx.save();
          sCtx.strokeStyle = color;
          sCtx.lineWidth = width;
          sCtx.setLineDash(dash);
          sCtx.beginPath();
          let prevT = -Infinity;
          let prevY = null;
          visible.forEach((r, idx) => {
            const rt = r.t ?? 0;
            const x = xOf(rt);
            const y = extractYFn(r);
            if (idx === 0 || rt <= prevT || prevY === null) {
              sCtx.moveTo(x, y);
            } else {
              sCtx.lineTo(x, prevY);
              sCtx.lineTo(x, y);
            }
            prevT = rt;
            prevY = y;
          });
          sCtx.stroke();
          sCtx.restore();
        }
        plotStepSub((r) => syOf(r.duty ?? 0), palette.duty, 2.5);

        // Sub-chart Lane 2 (Lower): Discrete State FSM color-coded ribbon (zero overlap with Duty %)
        const ribbonStyles = {
          NORMAL: {
            fill: dark ? 'rgba(74,222,128,0.22)' : 'rgba(20,83,45,0.15)',
            stroke: palette.pm_true,
            short: 'NORM',
          },
          WASPADA: {
            fill: dark ? 'rgba(251,191,36,0.26)' : 'rgba(146,64,14,0.18)',
            stroke: palette.waspada,
            short: 'WASP',
          },
          BAHAYA: {
            fill: dark ? 'rgba(248,113,113,0.30)' : 'rgba(185,28,28,0.20)',
            stroke: palette.bahaya,
            short: 'BHY',
          },
          FAILSAFE: {
            fill: dark ? 'rgba(232,121,249,0.28)' : 'rgba(109,40,217,0.18)',
            stroke: palette.forecast,
            short: 'FAIL',
          },
        };
        if (visible.length) {
          const segments = [];
          for (let i = 0; i < visible.length; i++) {
            const r = visible[i];
            const stName = r.state || 'NORMAL';
            const rt = r.t ?? 0;
            const nextT = (i + 1 < visible.length) ? (visible[i + 1].t ?? rt) : Math.max(rt + 1, minT + spanT * 0.015);
            if (segments.length && segments[segments.length - 1].state === stName) {
              segments[segments.length - 1].endT = Math.max(segments[segments.length - 1].endT, nextT);
            } else {
              segments.push({ state: stName, startT: rt, endT: Math.max(rt + 0.5, nextT) });
            }
          }
          sCtx.save();
          segments.forEach((seg) => {
            const stStyle = ribbonStyles[seg.state] || ribbonStyles.NORMAL;
            const x1 = Math.max(padL, xOf(seg.startT));
            const x2 = Math.min(sw - padR, Math.max(x1 + 3, xOf(seg.endT)));
            const segW = Math.max(2, x2 - x1);
            sCtx.fillStyle = stStyle.fill;
            sCtx.fillRect(x1, ribbonTop, segW, ribbonH);
            sCtx.strokeStyle = stStyle.stroke;
            sCtx.lineWidth = 1.2;
            sCtx.strokeRect(x1, ribbonTop, segW, ribbonH);
            if (segW >= 28) {
              const lbl = segW >= 58 ? seg.state : stStyle.short;
              sCtx.fillStyle = palette.tipFg;
              sCtx.font = `700 10px ${monoFont}`;
              const tw = sCtx.measureText(lbl).width;
              if (tw + 6 <= segW) {
                sCtx.fillText(lbl, x1 + (segW - tw) / 2, ribbonTop + ribbonH / 2 + 3.5);
              }
            }
          });
          const lastSeg = segments[segments.length - 1];
          if (lastSeg) {
            const stStyle = ribbonStyles[lastSeg.state] || ribbonStyles.NORMAL;
            sCtx.fillStyle = stStyle.stroke;
            sCtx.font = `700 10px ${monoFont}`;
            sCtx.fillText(fsmCompact ? stStyle.short : lastSeg.state, sw - padR + 5, ribbonTop + ribbonH / 2 + 3.5);
          }
          sCtx.restore();
        }

        const lastVisible = visible[visible.length - 1];
        if (lastVisible) {
          document.getElementById('chart-sr-summary').textContent =
            `Sampel terbaru t=${(lastVisible.t ?? 0).toFixed(0)}s, State ${lastVisible.state}, FI ${(lastVisible.fi ?? 0).toFixed(1)}, PM ${(lastVisible.pm ?? 0).toFixed(1)}, Kipas ${(lastVisible.duty ?? 0).toFixed(0)}%.`;
          const subSr = document.getElementById('subchart-sr-summary');
          if (subSr) {
            subSr.textContent =
              `Sub-grafik t=${(lastVisible.t ?? 0).toFixed(0)}s: Duty Kipas ${(lastVisible.duty ?? 0).toFixed(0)}%, Pita State FSM ${lastVisible.state} (${lastVisible.mode || 'AUTO'}).`;
          }
        }

        // Synchronized Crosshair + Tooltip across both canvases anchored to timestamp state.hoverT
        if (state.hoverT != null && visible.length) {
          if (state.hoverT < minT || state.hoverT > minT + spanT) {
            clearHover();
          } else {
            let nearestIdx = 0;
            for (let i = 1; i < visible.length; i++) {
              if (Math.abs((visible[i].t ?? 0) - state.hoverT) < Math.abs((visible[nearestIdx].t ?? 0) - state.hoverT)) {
                nearestIdx = i;
              }
            }
            state.hoverIdx = nearestIdx;
            const nearest = visible[nearestIdx];
            if (nearest) {
              if (state.hoverFromKey) {
                state.hoverFromKey = false;
                const lr = document.getElementById('chart-live-readout');
                if (lr) {
                  lr.textContent = `Inspeksi t=${(nearest.t ?? 0).toFixed(0)} detik, ${nearest.state}, FI ${(nearest.fi ?? 0).toFixed(1)}, PM ${(nearest.pm ?? 0).toFixed(1)}, Kipas ${(nearest.duty ?? 0).toFixed(0)} persen.`;
                }
              }
              const cx = xOf(nearest.t ?? 0);
              const lines = [
                { font: `700 11px ${monoFont}`, text: `t=${(nearest.t ?? 0).toFixed(0)}s | FSM:${nearest.state}` },
                { font: `11px ${monoFont}`, text: `FI:${(nearest.fi ?? 0).toFixed(1)} | PM:${(nearest.pm ?? 0).toFixed(1)}` },
                { font: `11px ${monoFont}`, text: `Gas:${(nearest.gas_ppm ?? 400).toFixed(0)}ppm (g=${(nearest.g ?? 0).toFixed(2)})` },
                { font: `11px ${monoFont}`, text: `Duty:${(nearest.duty ?? 0).toFixed(0)}% | Fc:${nearest.forecast != null ? nearest.forecast.toFixed(1) : '-'}` },
              ];
              if (state.layers.pm_true || state.layers.pm_raw) {
                const pt = nearest.pm_true != null ? nearest.pm_true.toFixed(1) : '-';
                lines.push({ font: `11px ${monoFont}`, text: `True:${pt} | Mentah:${(nearest.pm_raw ?? 0).toFixed(1)} | dC:${(nearest.dcdt ?? 0).toFixed(2)}` });
              }
              if (state.layers.voltages) {
                lines.push({ font: `11px ${monoFont}`, text: `Vd:${(nearest.v_dust ?? 0).toFixed(2)}V | Vm:${(nearest.v_mq ?? 0).toFixed(2)}V` });
              }
              if (nearest.parity_delta_fi != null && nearest.ref_fi != null) {
                lines.push({ font: `11px ${monoFont}`, text: `RefFI:${nearest.ref_fi.toFixed(1)} | dFI:${nearest.parity_delta_fi.toFixed(2)}` });
              }

              const tipW = Math.min(238, w - 16);
              const tipH = 14 + lines.length * 17;
              let boxX = cx + 10;
              if (boxX + tipW > w - padR - 4) boxX = cx - tipW - 10;
              boxX = Math.max(padL + 4, Math.min(w - tipW - 6, boxX));

              mCtx.save();
              mCtx.strokeStyle = palette.text;
              mCtx.setLineDash([3, 3]);
              mCtx.beginPath(); mCtx.moveTo(cx, padT); mCtx.lineTo(cx, h - padB); mCtx.stroke();
              mCtx.fillStyle = palette.tipBg;
              mCtx.strokeStyle = palette.grid;
              mCtx.setLineDash([]);
              mCtx.fillRect(boxX, padT + 4, tipW, tipH);
              mCtx.strokeRect(boxX, padT + 4, tipW, tipH);
              mCtx.fillStyle = palette.tipFg;
              lines.forEach((ln, idx) => {
                mCtx.font = ln.font;
                mCtx.fillText(ln.text, boxX + 8, padT + 20 + idx * 17, tipW - 14);
              });
              mCtx.restore();

              sCtx.save();
              sCtx.strokeStyle = palette.text;
              sCtx.setLineDash([3, 3]);
              sCtx.beginPath(); sCtx.moveTo(cx, sPadT); sCtx.lineTo(cx, sh - sPadB); sCtx.stroke();
              sCtx.restore();
            }
          }
        }
      }

      function applyPreset(name) {
        if (!PRESETS[name]) return;
        state.preset = name;
        state.layers = { ...PRESETS[name] };
        savePrefs();
        syncLayerButtons();
        scheduleDraw();
      }

      document.querySelectorAll('.theme-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          applyTheme(btn.getAttribute('data-theme-val'), true);
        });
      });

      document.getElementById('btn-toggle-layers').addEventListener('click', () => {
        const drawer = document.getElementById('layers-drawer');
        const btn = document.getElementById('btn-toggle-layers');
        const willOpen = drawer.hidden;
        drawer.hidden = !willOpen;
        btn.setAttribute('aria-expanded', willOpen ? 'true' : 'false');
        syncAllToolbars();
      });

      document.getElementById('btn-toggle-sim-cfg').addEventListener('click', () => {
        state.simConfigExpanded = !state.simConfigExpanded;
        document.getElementById('sim-config-fields').hidden = !state.simConfigExpanded;
        document.getElementById('btn-toggle-sim-cfg').setAttribute('aria-expanded', state.simConfigExpanded ? 'true' : 'false');
        syncAllToolbars();
      });

      document.getElementById('sw-zoom-y').addEventListener('change', (e) => {
        state.zoomY = !!e.target.checked;
        savePrefs();
        scheduleDraw();
      });

      document.querySelectorAll('.preset-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          applyPreset(btn.getAttribute('data-preset'));
        });
      });

      // Wire layer toggle buttons
      document.querySelectorAll('.layer-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          if (btn.disabled) return;
          const layer = btn.getAttribute('data-layer');
          state.layers[layer] = !state.layers[layer];
          state.preset = 'custom';
          savePrefs();
          syncLayerButtons();
          scheduleDraw();
        });
      });

      document.getElementById('sel-window').addEventListener('change', (e) => {
        state.windowSec = Number(e.target.value || 0);
        savePrefs();
        scheduleDraw();
      });

      // Synchronized Pointer/Touch & Keyboard crosshair inspection anchored to timestamp state.hoverT
      function updateHoverFromPointer(e, canvas) {
        const rect = canvas.getBoundingClientRect();
        if (!rect.width || !state.history.length) return;
        const w = canvas._cw || rect.width || 960;
        const compact = w < 520;
        const fsmCompact = w < 420;
        const padL = compact ? 44 : 54;
        const padR = fsmCompact ? 46 : 74;
        const plotW = Math.max(40, w - padL - padR);
        const px = e.clientX - rect.left;
        if (px < padL || px > w - padR) {
          if (e.pointerType === 'mouse') {
            clearHover();
            scheduleDraw();
          }
          return;
        }
        const lastT = state.history[state.history.length - 1].t ?? 0;
        const firstT = state.history[0].t ?? 0;
        const minT = state.windowSec > 0 ? Math.max(0, lastT - state.windowSec) : firstT;
        const spanT = Math.max(60, (lastT + (state.layers.forecast ? 60 : 5)) - minT);
        state.hoverFromKey = false;
        state.hoverFromTouch = (e.pointerType === 'touch' || e.pointerType === 'pen');
        state.hoverT = minT + ((px - padL) / plotW) * spanT;
        syncHoverDismissBtn();
        scheduleDraw();
      }
      [mainCanvas, subCanvas].forEach((cvs) => {
        cvs.addEventListener('pointermove', (e) => updateHoverFromPointer(e, cvs));
        cvs.addEventListener('pointerdown', (e) => updateHoverFromPointer(e, cvs));
        cvs.addEventListener('pointerleave', (e) => {
          if (e.pointerType === 'mouse') {
            clearHover();
            scheduleDraw();
          }
        });
      });
      document.getElementById('btn-clear-hover').addEventListener('click', () => {
        clearHover();
        scheduleDraw();
      });
      document.addEventListener('pointerdown', (e) => {
        const clearBtn = document.getElementById('btn-clear-hover');
        if (state.hoverT != null && e.target !== mainCanvas && e.target !== subCanvas && e.target !== clearBtn) {
          clearHover();
          scheduleDraw();
        }
        const mobDock = document.querySelector('.mobile-dock');
        const mobTray = document.getElementById('mob-dock-tray');
        if (mobTray && !mobTray.hidden && mobDock && !mobDock.contains(e.target)) {
          closeMobileDockTray();
        }
      });

      function handleChartKeydown(e) {
        if (!state.history.length) return;
        const lastT = state.history[state.history.length - 1].t ?? 0;
        const firstT = state.history[0].t ?? 0;
        const minT = state.windowSec > 0 ? Math.max(0, lastT - state.windowSec) : firstT;
        const visible = state.history.filter((r) => (r.t ?? 0) >= minT);
        if (!visible.length) return;
        if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
          e.preventDefault();
          let idx = visible.length - 1;
          if (state.hoverT != null) {
            let best = 0;
            for (let i = 1; i < visible.length; i++) {
              if (Math.abs((visible[i].t ?? 0) - state.hoverT) < Math.abs((visible[best].t ?? 0) - state.hoverT)) {
                best = i;
              }
            }
            idx = e.key === 'ArrowLeft' ? Math.max(0, best - 1) : Math.min(visible.length - 1, best + 1);
          }
          state.hoverIdx = idx;
          state.hoverT = visible[idx].t ?? 0;
          state.hoverFromKey = true;
          state.hoverFromTouch = false;
          syncHoverDismissBtn();
          scheduleDraw();
        } else if (e.key === 'Escape') {
          clearHover();
          scheduleDraw();
        }
      }
      [mainCanvas, subCanvas].forEach((cvs) => {
        cvs.addEventListener('keydown', handleChartKeydown);
      });

      // Accessible KPI abbreviation explanation buttons
      document.querySelectorAll('.kpi-abbr-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          const msg = btn.getAttribute('data-abbr-help') || btn.getAttribute('title');
          if (msg) showToast(msg, 'ok');
        });
      });

      // Browser CSV & MQTT Log exports
      function triggerCsvExport(srcOverride = null) {
        const src = srcOverride || activeSource();
        const a = document.createElement('a');
        a.href = `/api/export-csv?source=${encodeURIComponent(src)}`;
        a.download = '';
        a.click();
        showToast(`Data CSV (${src.toUpperCase()}) diunduh.`, 'ok');
      }

      document.getElementById('btn-export-csv').addEventListener('click', () => {
        if (!state.history.length) {
          showToast('Belum ada data grafik untuk diekspor ke CSV.', 'warn');
          return;
        }
        triggerCsvExport(activeSource());
      });

      document.getElementById('rs-export-csv').addEventListener('click', () => {
        const src = (state.lastRunSummary && state.lastRunSummary.source) || activeSource();
        triggerCsvExport(src);
      });

      document.getElementById('btn-export-log').addEventListener('click', () => {
        const a = document.createElement('a');
        a.href = '/api/export-log';
        a.download = '';
        a.click();
        showToast('Log trafik MQTT (.log) diunduh.', 'ok');
      });

      // Composite opaque PNG export with multi-row wrapped legend
      document.getElementById('btn-export-png').addEventListener('click', () => {
        if (!state.history.length) {
          showToast('Belum ada data grafik untuk diekspor.', 'warn');
          return;
        }
        drawCharts();
        const dark = isDarkTheme();
        const pal = chartPalette(dark);
        const specs = getLegendSpecs(pal);
        const cw = mainCanvas._cw || 960;
        const mh = mainCanvas._ch || 320;
        const sh = subCanvas._ch || 150;
        const dpr = Math.min(window.devicePixelRatio || 1, 2);

        const tmpCanvas = document.createElement('canvas');
        const tCtx = tmpCanvas.getContext('2d');
        tCtx.font = '600 11px system-ui, sans-serif';
        const legendRows = [];
        let curRow = [];
        let curW = 0;
        const maxRowW = Math.max(220, cw - 8);
        for (const s of specs) {
          const itemW = 30 + Math.ceil(tCtx.measureText(s.label).width);
          if (curRow.length && curW + itemW > maxRowW) {
            legendRows.push(curRow);
            curRow = [s];
            curW = itemW;
          } else {
            curRow.push(s);
            curW += itemW;
          }
        }
        if (curRow.length) legendRows.push(curRow);

        const headerH = specs.length ? (48 + legendRows.length * 18) : 44;
        const gap = 10;
        const out = document.createElement('canvas');
        out.width = Math.round((cw + 24) * dpr);
        out.height = Math.round((headerH + mh + gap + sh + 16) * dpr);
        const ctx = out.getContext('2d');
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.fillStyle = pal.exportBg;
        ctx.fillRect(0, 0, cw + 24, headerH + mh + gap + sh + 16);
        ctx.fillStyle = pal.tipFg;
        ctx.font = '700 14px system-ui, sans-serif';
        const srcText = document.getElementById('src-label').textContent;
        ctx.fillText(`Karhutla Edge-IoT — ${srcText}`, 14, 22);
        ctx.fillStyle = pal.muted;
        ctx.font = '11px system-ui, sans-serif';
        const parityStr = document.getElementById('parity-badge').hidden ? '' : ` • ${document.getElementById('parity-badge').textContent}`;
        const presetLabel = PRESET_LABELS[state.preset] || state.preset;
        ctx.fillText(`Diekspor: ${new Date().toLocaleString('id-ID')} • Preset: ${presetLabel}${parityStr}`, 14, 38);

        if (legendRows.length) {
          ctx.font = '600 11px system-ui, sans-serif';
          legendRows.forEach((rSpecs, rIdx) => {
            let lx = 14;
            const ly = 58 + rIdx * 18;
            for (const s of rSpecs) {
              const tw = ctx.measureText(s.label).width;
              ctx.save();
              ctx.strokeStyle = s.color;
              ctx.lineWidth = 2.5;
              if (s.style === 'dashed') ctx.setLineDash([5, 3]);
              else if (s.style === 'dotted') ctx.setLineDash([2, 3]);
              ctx.beginPath(); ctx.moveTo(lx, ly - 4); ctx.lineTo(lx + 16, ly - 4); ctx.stroke();
              ctx.restore();
              ctx.fillStyle = pal.tipFg;
              ctx.fillText(s.label, lx + 20, ly);
              lx += 30 + tw;
            }
          });
        }

        ctx.drawImage(mainCanvas, 12, headerH, cw, mh);
        ctx.drawImage(subCanvas, 12, headerH + mh + gap, cw, sh);

        const a = document.createElement('a');
        a.download = `karhutla_${activeSource()}_${Date.now()}.png`;
        a.href = out.toDataURL('image/png');
        a.click();
        showToast('Grafik PNG berlegenda lengkap berhasil diunduh.', 'ok');
      });

      document.getElementById('btn-export-mobile').addEventListener('click', () => {
        const a = document.createElement('a');
        a.download = 'karhutla_iot_mqtt_panel.json';
        a.href = '/api/mobile-config';
        a.click();
        showToast('File konfigurasi HP (.json) diunduh.', 'ok');
      });

      document.getElementById('btn-fallback-sitl').addEventListener('click', switchFallbackToSitl);

      // Native <dialog> confirmation & help handlers
      const dlg = document.getElementById('confirm-dialog');
      const dlgTitle = document.getElementById('confirm-title');
      const dlgDesc = document.getElementById('confirm-desc');
      const dlgOk = document.getElementById('confirm-ok');
      const dlgCancel = document.getElementById('confirm-cancel');
      const helpDlg = document.getElementById('help-dialog');
      let pendingConfirm = null;
      let pendingCancel = null;

      function askConfirm(title, desc, okLabel, onConfirm, tone = 'danger', onCancel = null) {
        if (typeof dlg.showModal !== 'function') {
          onConfirm();
          return;
        }
        dlgTitle.textContent = title;
        dlgDesc.textContent = desc;
        dlgOk.textContent = okLabel;
        if (tone === 'danger') dlgOk.setAttribute('data-variant', 'danger');
        else dlgOk.removeAttribute('data-variant');
        pendingConfirm = onConfirm;
        pendingCancel = onCancel;
        dlg.showModal();
      }

      function dismissConfirm() {
        const cancelFn = pendingCancel;
        pendingConfirm = null;
        pendingCancel = null;
        if (dlg.open) dlg.close();
        if (cancelFn) cancelFn();
      }

      dlgCancel.addEventListener('click', dismissConfirm);
      dlg.addEventListener('cancel', (e) => {
        e.preventDefault();
        dismissConfirm();
      });
      dlg.addEventListener('click', (e) => {
        if (e.target === dlg) dismissConfirm();
      });
      dlg.addEventListener('close', () => {
        if (pendingCancel) {
          const fn = pendingCancel;
          pendingConfirm = null;
          pendingCancel = null;
          fn();
        } else {
          pendingConfirm = null;
        }
      });
      dlgOk.addEventListener('click', () => {
        const fn = pendingConfirm;
        pendingConfirm = null;
        pendingCancel = null;
        dlg.close();
        if (fn) fn();
      });

      document.getElementById('btn-open-help').addEventListener('click', () => {
        if (typeof helpDlg.showModal === 'function') helpDlg.showModal();
      });
      document.getElementById('help-close').addEventListener('click', () => helpDlg.close());
      helpDlg.addEventListener('click', (e) => {
        if (e.target === helpDlg) helpDlg.close();
      });

      const swShortcuts = document.getElementById('sw-shortcuts');
      if (swShortcuts) {
        swShortcuts.checked = !!state.singleKeyShortcuts;
        swShortcuts.addEventListener('change', (e) => {
          state.singleKeyShortcuts = !!e.target.checked;
          savePrefs();
        });
      }

      const selCmdTarget = document.getElementById('sel-cmd-target');
      if (selCmdTarget) {
        selCmdTarget.value = state.cmdTarget || 'auto';
        selCmdTarget.addEventListener('change', (e) => {
          state.cmdTarget = e.target.value || 'auto';
          savePrefs();
          updateOverrideControlState();
        });
      }

      function setCmdButtonsBusy(busy) {
        state.cmdBusy = busy;
        document.querySelectorAll('#cmd-toolbar .cmd-btn, .mob-cmd-btn, #btn-cmd-reset, #btn-mob-reset').forEach((b) => {
          b.setAttribute('aria-busy', busy ? 'true' : 'false');
        });
      }

      async function sendActuatorCmd(cmd) {
        if (state.cmdBusy) return;
        setCmdButtonsBusy(true);
        try {
          const r = await fetch('/api/cmd', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ cmd, target: state.cmdTarget || 'auto' }),
          });
          if (!r.ok) throw new Error(`HTTP ${r.status}`);
          const res = await r.json();
          if (!res.ok) {
            showToast(res.reason || `Perintah ${cmd} ditolak.`, 'danger');
            return;
          }
          if (cmd === 'RESET') {
            clearHover();
            hideRunSummary();
          }
          showToast(`Perintah ${cmd} dikirim.`, 'ok');
        } catch (err) {
          showToast(`Gagal mengirim ${cmd}: server tidak merespons.`, 'danger');
        } finally {
          setCmdButtonsBusy(false);
        }
      }

      function requestActuatorCmd(cmd) {
        const activeRow = state.src[activeSource()].latest;
        if (cmd === 'FORCE_OFF' && activeRow && (activeRow.state === 'BAHAYA' || activeRow.state === 'WASPADA' || activeRow.state === 'FAILSAFE')) {
          askConfirm(
            'Matikan Kipas Saat Kondisi Kritis?',
            `Sistem sedang berada pada status ${activeRow.state} (FI ${(activeRow.fi ?? 0).toFixed(1)} µg/m³). Menyetel FORCE OFF (0%) akan memutus ventilasi pengaman.`,
            'Tetap Matikan (OFF 0%)',
            () => sendActuatorCmd('FORCE_OFF'),
            'danger',
          );
          return;
        }
        sendActuatorCmd(cmd);
      }

      function setSimButtonsBusy(busy) {
        ['btn-sim-start', 'btn-sim-stop', 'btn-sim-pause', 'mob-btn-playpause', 'mob-btn-stop'].forEach((id) => {
          const el = document.getElementById(id);
          if (el) el.setAttribute('aria-busy', busy ? 'true' : 'false');
        });
      }

      async function postSim(payload, label = '') {
        setSimButtonsBusy(true);
        try {
          const r = await fetch('/api/sim', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
          });
          if (!r.ok) throw new Error('Server simulasi menolak permintaan');
          const res = await r.json();
          if (res.sim) {
            updateSimBadges(res.sim);
            updateSafetyCallout(state.src[activeSource()].latest, res.sim);
          }
          if (!res.ok) {
            showToast(res.reason || 'Operasi simulasi ditolak.', 'danger');
            return null;
          }
          if (label) showToast(label, 'ok');
          return res;
        } catch (err) {
          showToast('Perintah simulasi gagal diproses — menyinkronkan ulang status.', 'danger');
          loadStateSnapshot(true, true);
          return null;
        } finally {
          setSimButtonsBusy(false);
        }
      }

      function closeMobileDockTray() {
        const tray = document.getElementById('mob-dock-tray');
        const btnMore = document.getElementById('mob-btn-more');
        if (tray) tray.hidden = true;
        if (btnMore) btnMore.setAttribute('aria-expanded', 'false');
      }

      const mobBtnMore = document.getElementById('mob-btn-more');
      if (mobBtnMore) {
        mobBtnMore.addEventListener('click', () => {
          const tray = document.getElementById('mob-dock-tray');
          if (!tray) return;
          const willOpen = tray.hidden;
          tray.hidden = !willOpen;
          mobBtnMore.setAttribute('aria-expanded', willOpen ? 'true' : 'false');
        });
      }

      // Wire Actuator Override Commands (Desktop + Mobile Dock)
      document.querySelectorAll('#cmd-toolbar .cmd-btn, .mob-cmd-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          if (btn.disabled) return;
          const tray = document.getElementById('mob-dock-tray');
          if (tray && tray.contains(btn)) closeMobileDockTray();
          requestActuatorCmd(btn.getAttribute('data-cmd'));
        });
      });

      document.querySelectorAll('#btn-cmd-reset, #btn-mob-reset').forEach((btn) => {
        btn.addEventListener('click', () => {
          if (btn.disabled) return;
          if (btn.id === 'btn-mob-reset') closeMobileDockTray();
          askConfirm(
            'Reset Sistem & Filter?',
            'Aksi ini mengatur ulang estimator Kalman, state FSM menjadi NORMAL, mode AUTO, serta menghapus histori kurva yang sedang berjalan.',
            'Ya, Reset Sistem',
            () => sendActuatorCmd('RESET'),
            'danger',
          );
        });
      });

      document.getElementById('rs-export').addEventListener('click', () => {
        document.getElementById('btn-export-png').click();
      });
      document.getElementById('rs-close').addEventListener('click', hideRunSummary);

      async function launchSim(tgt, overrideScenario = null) {
        const bucket = tgt === 'hils' ? 'esp32' : 'sitl';
        const scenario = overrideScenario || document.getElementById('sel-scenario').value;
        const ai_preempt = document.getElementById('sw-preempt').checked;
        const fault_inject = document.getElementById('sw-fault').checked;
        const speedBtn = document.querySelector('.spd-btn[aria-pressed="true"]');
        const speed = speedBtn ? Number(speedBtn.getAttribute('data-speed') || 1) : 1;

        hideRunSummary();
        const res = await postSim(
          { action: 'start', target: tgt, scenario, speed, ai_preempt, fault_inject },
          `Simulasi ${tgt.toUpperCase()} (${SCENARIO_LABELS[scenario] || scenario}) dimulai.`,
        );
        if (!res) return;

        state.src[bucket].history = [];
        state.src[bucket].latest = null;
        clearHover();
        state.simTarget = tgt;
        state.lastActiveSource = bucket;
        if (tgt === 'hils' && state.viewMode === 'sitl') state.viewMode = 'esp32';
        else if (tgt === 'sitl' && state.viewMode === 'esp32') state.viewMode = 'sitl';
        document.getElementById('sel-scenario').value = scenario;
        state.hilsTimeoutInfo = null;
        savePrefs();
        applyView();
      }

      function startTargetWithCheck(tgt, overrideScenario = null) {
        if (tgt === 'hils' && !state.esp.online) {
          askConfirm(
            'ESP32 Belum Terdeteksi',
            'Node ESP32 saat ini sedang offline di broker :6669. Menjalankan HILS tanpa ESP32 akan mengalami timeout dan jeda otomatis setelah 3x gagal. Tetap mulai?',
            'Tetap Mulai HILS',
            () => launchSim('hils', overrideScenario),
            'warn',
            () => updateSimBadges(state.sim),
          );
          return;
        }
        launchSim(tgt, overrideScenario);
      }

      document.querySelectorAll('.tgt-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          const tgt = btn.getAttribute('data-target');
          if (tgt === state.simTarget) return;
          if (state.sim && state.sim.running) {
            askConfirm(
              `Beralih ke Target ${tgt.toUpperCase()}?`,
              `Simulasi ${state.simTarget.toUpperCase()} sedang berjalan. Beralih ke ${tgt.toUpperCase()} akan memulai ulang simulasi dari t=0s.`,
              `Mulai Ulang di ${tgt.toUpperCase()}`,
              () => startTargetWithCheck(tgt),
              'warn',
              () => updateSimBadges(state.sim),
            );
            return;
          }
          state.simTarget = tgt;
          document.querySelectorAll('.tgt-btn').forEach((b) => {
            b.setAttribute('aria-pressed', b.getAttribute('data-target') === tgt ? 'true' : 'false');
          });
          document.getElementById('hils-concurrency-hint').hidden = tgt !== 'hils';
          updateMobileDockContext();
          postSim({ target: tgt });
        });
      });

      document.getElementById('btn-sim-start').addEventListener('click', () => {
        const tgt = state.simTarget || 'sitl';
        if (state.sim && state.sim.running && state.history.length > 0) {
          askConfirm(
            'Mulai Ulang Simulasi?',
            `Simulasi ${tgt.toUpperCase()} sedang berjalan (${state.history.length} sampel). Mengulang akan menghapus histori kurva berjalan dan memulai dari t=0s.`,
            'Ya, Mulai Ulang',
            () => startTargetWithCheck(tgt),
            'warn',
          );
          return;
        }
        startTargetWithCheck(tgt);
      });

      function togglePauseResume() {
        if (!state.sim || !state.sim.running) return;
        const paused = state.sim.paused;
        if (paused) state.hilsTimeoutInfo = null;
        postSim({ action: paused ? 'resume' : 'pause' }, paused ? 'Simulasi dilanjutkan.' : 'Simulasi dijeda.');
      }

      document.getElementById('btn-sim-pause').addEventListener('click', togglePauseResume);

      document.getElementById('mob-btn-playpause').addEventListener('click', () => {
        if (state.sim && state.sim.running) togglePauseResume();
        else document.getElementById('btn-sim-start').click();
      });

      document.getElementById('mob-btn-stop').addEventListener('click', () => {
        closeMobileDockTray();
        document.getElementById('btn-sim-stop').click();
      });

      document.getElementById('btn-sim-stop').addEventListener('click', () => {
        if (state.history.length > 0) {
          askConfirm(
            'Hentikan Simulasi?',
            `Simulasi ${state.simTarget.toUpperCase()} yang sedang berjalan akan dihentikan (data grafik terekam tetap dipertahankan).`,
            'Ya, Hentikan',
            () => postSim({ action: 'stop' }, 'Simulasi dihentikan.'),
            'warn',
          );
          return;
        }
        postSim({ action: 'stop' }, 'Simulasi dihentikan.');
      });

      document.getElementById('btn-sim-clear').addEventListener('click', () => {
        const src = activeSource();
        askConfirm(
          `Bersihkan Grafik ${src.toUpperCase()}?`,
          `Seluruh histori kurva dan KPI sumber ${src.toUpperCase()} akan dikosongkan dari layar saat ini.`,
          'Ya, Bersihkan',
          () => {
            postSim({ action: 'clear', source: src }, `Grafik ${src.toUpperCase()} dibersihkan.`).then((res) => {
              if (!res) return;
              state.src[src].history = [];
              state.src[src].latest = null;
              clearHover();
              hideRunSummary();
              applyView();
            });
          },
          'danger',
        );
      });

      document.querySelectorAll('.src-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          state.viewMode = btn.getAttribute('data-src');
          if (state.viewMode === 'sitl' || state.viewMode === 'esp32') {
            state.lastActiveSource = state.viewMode;
          }
          clearHover();
          savePrefs();
          applyView();
        });
      });

      setInterval(updateEspBadge, 1000);

      document.getElementById('sel-scenario').addEventListener('change', (e) => {
        const newScen = e.target.value;
        if (state.sim && state.sim.running) {
          askConfirm(
            'Ganti Skenario & Mulai Ulang?',
            `Simulasi sedang menjalankan "${SCENARIO_LABELS[state.sim.scenario] || state.sim.scenario}". Mengganti skenario ke "${SCENARIO_LABELS[newScen] || newScen}" perlu memulai ulang ruang asap dari t=0s.`,
            'Mulai Ulang dengan Skenario Ini',
            () => launchSim(state.simTarget || 'sitl', newScen),
            'warn',
            () => { document.getElementById('sel-scenario').value = state.sim.scenario || 'flaming'; },
          );
          return;
        }
        updateMobileDockContext();
        postSim({ scenario: newScen });
      });

      document.querySelectorAll('.spd-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
          document.querySelectorAll('.spd-btn').forEach((b) => b.setAttribute('aria-pressed', 'false'));
          btn.setAttribute('aria-pressed', 'true');
          updateSimAdvancedSummary();
          updateMobileDockContext();
          postSim({ speed: Number(btn.getAttribute('data-speed')) });
        });
      });

      document.getElementById('sw-preempt').addEventListener('change', (e) => {
        updateSimAdvancedSummary();
        postSim({ ai_preempt: e.target.checked });
      });

      document.getElementById('sw-fault').addEventListener('change', (e) => {
        updateSimAdvancedSummary();
        postSim({ fault_inject: e.target.checked });
      });

      // Global keyboard shortcuts (S start idle, 1/2/3 presets, P/Space pause, Alt+Shift+A AUTO, Alt+Shift+M FORCE_MAX, ? Help)
      document.addEventListener('keydown', (e) => {
        if (helpDlg.open) {
          if (e.key === '?') helpDlg.close();
          return;
        }
        if (dlg.open) return;
        const tag = (e.target && e.target.tagName) || '';
        if (tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA') return;

        if (e.altKey && e.shiftKey && !e.ctrlKey && !e.metaKey) {
          const k = (e.key || '').toLowerCase();
          const autoBtn = document.querySelector('#cmd-toolbar .cmd-btn[data-cmd="AUTO"]');
          const canCmd = autoBtn ? !autoBtn.disabled : !!(state.sim && state.sim.can_send_cmd);
          if (k === 'a' || e.code === 'KeyA') {
            e.preventDefault();
            if (canCmd) requestActuatorCmd('AUTO');
            return;
          }
          if (k === 'm' || e.code === 'KeyM') {
            e.preventDefault();
            if (canCmd) requestActuatorCmd('FORCE_MAX');
            return;
          }
          return;
        }

        if (e.metaKey || e.ctrlKey || e.altKey) return;

        if (e.key === '?') {
          e.preventDefault();
          if (typeof helpDlg.showModal === 'function') helpDlg.showModal();
          return;
        }

        if (!state.singleKeyShortcuts) return;

        const onCanvas = (document.activeElement === mainCanvas || document.activeElement === subCanvas);

        if ((e.key === 's' || e.key === 'S') && !onCanvas && (!state.sim || !state.sim.running)) {
          e.preventDefault();
          document.getElementById('btn-sim-start').click();
        } else if (e.key === '1') {
          applyPreset('demo');
        } else if (e.key === '2') {
          applyPreset('dsp');
        } else if (e.key === '3') {
          applyPreset('all');
        } else if ((e.key === 'p' || e.key === 'P') && state.sim && state.sim.running) {
          e.preventDefault();
          togglePauseResume();
        } else if (e.code === 'Space' && !onCanvas && tag !== 'BUTTON' && tag !== 'A' && tag !== 'SUMMARY' && state.sim && state.sim.running) {
          e.preventDefault();
          togglePauseResume();
        }
      });

      function loadStateSnapshot(isReconnect = false, silent = false) {
        return fetch('/api/state')
          .then((r) => {
            if (!r.ok) throw new Error('Server status tidak merespons');
            return r.json();
          })
          .then((d) => {
            state.bootError = false;
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
            if (d.mobile && d.mobile.ip_is_fallback) {
              const hint = document.getElementById('mobile-config-hint');
              if (hint) {
                hint.textContent = 'Hotspot/LAN offline terdeteksi: pastikan alamat host pada file JSON yang diunduh sesuai IP laptop Anda (port 6669).';
              }
            }
            state.latest = d.latest || {};
            if (!isReconnect) {
              (d.mqtt_logs || []).forEach((ln) => appendMqttLog(ln, true));
            } else if (!silent) {
              appendMqttLog('--- Koneksi live ke server tersambung kembali ---', false);
            }
            updateSimBadges(d.sim);
            applyView();
            const ls = d.sim && d.sim.last_summary;
            if (ls && !(d.sim && d.sim.running)) {
              const bucket = ls.source === 'esp32' ? 'esp32' : 'sitl';
              if (state.src[bucket] && state.src[bucket].history.length > 0) {
                renderRunSummary(ls);
              }
            } else if (d.sim && d.sim.running) {
              hideRunSummary();
            }
            if (isReconnect && !silent) showToast('Koneksi live tersambung kembali & data disinkronkan.', 'ok');
          })
          .catch(() => {
            state.bootError = true;
            updateSafetyCallout(null, state.sim);
            if (!silent) showToast('Gagal memuat status dari server.', 'danger');
          });
      }

      applyTheme(state.theme, false);
      loadStateSnapshot(false, false);

      const es = new EventSource('/api/stream');
      es.onopen = () => {
        const reconnected = state.sseHadDisconnect;
        state.sseConnected = true;
        state.sseHadDisconnect = false;
        updateBrokerBadge();
        if (reconnected) {
          loadStateSnapshot(true, false);
        } else {
          updateSafetyCallout(state.src[activeSource()].latest, state.sim);
        }
      };
      es.onerror = () => {
        state.sseConnected = false;
        state.sseHadDisconnect = true;
        updateBrokerBadge();
        updateSafetyCallout(state.src[activeSource()].latest, state.sim);
      };
      es.onmessage = (ev) => {
        state.sseConnected = true;
        try {
          const msg = JSON.parse(ev.data);
          if (msg.type === 'clear') {
            const src = msg.source === 'esp32' ? 'esp32' : 'sitl';
            state.src[src].history = [];
            state.src[src].latest = null;
            clearHover();
            hideRunSummary();
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
          } else if (msg.type === 'hils_timeout') {
            state.hilsTimeoutInfo = msg;
            if (msg.sim) updateSimBadges(msg.sim);
            updateSafetyCallout(state.src[activeSource()].latest, state.sim);
            if (msg.auto_paused) {
              showToast(`HILS dijeda otomatis setelah ${msg.consec}x timeout ESP32.`, 'danger');
            }
          } else if (msg.type === 'sim_complete' && msg.summary) {
            const s = msg.summary;
            if (s.source) state.lastActiveSource = s.source;
            if (msg.sim) updateSimBadges(msg.sim);
            applyView();
            renderRunSummary(s);
            const parNote = (s.hils_parity && s.hils_parity.samples > 0)
              ? `, paritas ΔFI maks ${s.hils_parity.max_delta_fi.toFixed(2)}`
              : '';
            showToast(`Simulasi ${SCENARIO_LABELS[s.scenario] || s.scenario} selesai (${s.samples} sampel, FI puncak ${Number(s.peak_fi || 0).toFixed(1)} µg/m³${parNote}).`, 'ok');
          } else if (msg.type === 'mqtt_log') {
            appendMqttLog(msg.line, true);
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

      window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
        if (!state.theme) applyTheme(null, false);
      });
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
            if parsed.path == "/api/export-csv":
                qs = urllib.parse.parse_qs(parsed.query)
                req_src = (qs.get("source", ["auto"])[0] or "auto").strip().lower()
                with engine.lock:
                    sitl_hist = list(engine.history)
                    esp_hist = list(engine.esp_history)
                    esp_online = engine.esp_online
                    sim_running = engine.sim_running
                if req_src == "esp32":
                    chosen_src = "esp32"
                    rows = esp_hist
                elif req_src == "sitl":
                    chosen_src = "sitl"
                    rows = sitl_hist
                else:
                    use_esp = esp_online or (bool(esp_hist) and not sim_running)
                    chosen_src = "esp32" if use_esp else "sitl"
                    rows = esp_hist if use_esp else sitl_hist
                cols = [
                    "t_s", "seq", "source", "scenario", "pm_true", "v_dust", "v_mq",
                    "pm_raw", "pm", "dcdt", "gas_ppm", "g", "fi", "v0", "state", "mode",
                    "duty", "forecast", "slope", "eta_bahaya_s", "mae", "sensor_fault",
                    "ref_fi", "parity_delta_fi",
                ]
                buf = io.StringIO()
                writer = csv.DictWriter(buf, fieldnames=cols)
                writer.writeheader()
                for r in rows:
                    out_row = {}
                    for c in cols:
                        if c == "t_s":
                            v = r.get("t_s", r.get("t", ""))
                        elif c == "source":
                            v = r.get("source") or chosen_src
                        else:
                            v = r.get(c)
                        out_row[c] = "" if v is None else v
                    writer.writerow(out_row)
                raw = buf.getvalue().encode("utf-8")
                day = datetime.date.today().isoformat()
                self.send_response(200)
                self.send_header("Content-Type", "text/csv; charset=utf-8")
                self.send_header("Content-Disposition", f'attachment; filename="karhutla_{chosen_src}_{day}.csv"')
                self.send_header("Cache-Control", "no-store")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)
                return
            if parsed.path == "/api/export-log":
                log_path = getattr(engine.mqtt_logger, "path", None) if engine.mqtt_logger else None
                if log_path and os.path.exists(log_path):
                    with open(log_path, encoding="utf-8", errors="ignore") as f:
                        content = f.read()
                else:
                    with engine.lock:
                        content = "\n".join(list(engine.mqtt_logs))
                raw = content.encode("utf-8")
                day = datetime.date.today().isoformat()
                self.send_response(200)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Content-Disposition", f'attachment; filename="karhutla_mqtt_{day}.log"')
                self.send_header("Cache-Control", "no-store")
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
                ok = engine.send_cmd(data.get("cmd"), target=data.get("target", "auto"))
                self._send_json({"ok": ok, "reason": None if ok else getattr(engine, "_last_cmd_error", "Perintah ditolak.")})
                return
            if parsed.path == "/api/sim":
                ok = engine.control_sim(data)
                self._send_json({
                    "ok": ok,
                    "reason": None if ok else getattr(engine, "_last_sim_error", "Operasi simulasi ditolak."),
                    "sim": engine.get_sim_status(),
                })
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
