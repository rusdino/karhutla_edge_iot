"""Helper MQTT (kompatibel paho-mqtt 1.x dan 2.x) + auto-logger & auto-broker."""
import json
import math
import os
import socket
import subprocess
import time
import paho.mqtt.client as mqtt
import config as K
from logger import DailyMqttLogger

_DEFAULT_LOGGER = None
_BROKER_PROC = None


def get_default_logger():
    global _DEFAULT_LOGGER
    if _DEFAULT_LOGGER is None:
        _DEFAULT_LOGGER = DailyMqttLogger(folder="logs", prefix="mqtt")
    return _DEFAULT_LOGGER


def _is_port_open(host, port, timeout=0.35):
    try:
        with socket.create_connection((host, int(port)), timeout=timeout):
            return True
    except OSError:
        return False


def ensure_broker(host=None, port=None, conf_path="mosquitto.conf"):
    """Pastikan broker Mosquitto berjalan di port target (auto-start bila lokal)."""
    global _BROKER_PROC
    target_host = host or K.BROKER_HOST
    target_port = int(port or K.BROKER_PORT)
    check_host = "127.0.0.1" if target_host in ("localhost", "0.0.0.0", "") else target_host
    if _is_port_open(check_host, target_port):
        return True
    if check_host not in ("127.0.0.1", "::1"):
        return False
    if not os.path.exists(conf_path):
        base_dir = os.path.dirname(os.path.abspath(__file__))
        alt = os.path.join(base_dir, "mosquitto.conf")
        if os.path.exists(alt):
            conf_path = alt
    try:
        _BROKER_PROC = subprocess.Popen(
            ["mosquitto", "-c", conf_path],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except OSError:
        return False
    for _ in range(30):
        if _is_port_open(check_host, target_port):
            get_default_logger().log_event("mosquitto-broker", "BROKER_START", f"{check_host}:{target_port}", conf_path)
            return True
        time.sleep(0.1)
    return _is_port_open(check_host, target_port)


def make_client(client_id, on_connect=None, on_message=None, publish_status=False, mqtt_logger=None):
    try:
        c = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=client_id)
    except AttributeError:                      # paho 1.x
        c = mqtt.Client(client_id=client_id)
    if K.MQTT_USER:
        c.username_pw_set(K.MQTT_USER, K.MQTT_PASS)

    logger = mqtt_logger or get_default_logger()
    c._karhutla_logger = logger
    c._karhutla_id = client_id
    c._publish_status = publish_status

    if publish_status:
        c.will_set(K.T_STATUS, "offline", qos=0, retain=True)

    orig_publish = c.publish
    orig_subscribe = c.subscribe

    def _should_log_topic(topic):
        return not (topic.startswith(f"{K.T_TELEM_SCALAR}/") or topic.startswith(f"{K.T_FORECAST_SCALAR}/"))

    def logged_publish(topic, payload=None, qos=0, retain=False, *args, **kwargs):
        if _should_log_topic(str(topic)):
            logger.log_event(client_id, "PUBLISH", topic, payload)
        return orig_publish(topic, payload, qos=qos, retain=retain, *args, **kwargs)

    def logged_subscribe(topic, qos=0, *args, **kwargs):
        logger.log_event(client_id, "SUBSCRIBE", str(topic), f"qos={qos}")
        return orig_subscribe(topic, qos, *args, **kwargs)

    c.publish = logged_publish
    c.subscribe = logged_subscribe

    def _wrapped_on_connect(client, userdata, flags, rc, *extra):
        logger.log_event(client_id, "CONNECT", f"rc={rc}", "connected")
        if publish_status:
            client.publish(K.T_STATUS, "online", qos=0, retain=True)
        if on_connect:
            try:
                on_connect(client, userdata, flags, rc, *extra)
            except TypeError:
                on_connect(client, userdata, flags, rc)

    def _wrapped_on_message(client, userdata, msg):
        if _should_log_topic(msg.topic):
            payload_txt = msg.payload.decode("utf-8", errors="ignore")
            logger.log_event(client_id, "RECV", msg.topic, payload_txt)
        if on_message:
            on_message(client, userdata, msg)

    c.on_connect = _wrapped_on_connect
    c.on_message = _wrapped_on_message
    return c


def connect(c, host=None, port=None, auto_start_broker=True):
    target_host = host or K.BROKER_HOST
    target_port = int(port or K.BROKER_PORT)
    if auto_start_broker:
        ensure_broker(target_host, target_port)
    c.connect(target_host, target_port, keepalive=30)
    c.loop_start()


def disconnect(c):
    if c is None:
        return
    logger = getattr(c, "_karhutla_logger", None)
    cid = getattr(c, "_karhutla_id", "mqtt-client")
    try:
        if getattr(c, "_publish_status", False):
            info = c.publish(K.T_STATUS, "offline", qos=0, retain=True)
            try:
                info.wait_for_publish(timeout=0.5)
            except Exception:
                pass
    except Exception:
        pass
    if logger:
        logger.log_event(cid, "DISCONNECT", "", "client disconnected")
    try:
        c.loop_stop()
    except Exception:
        pass
    try:
        c.disconnect()
    except Exception:
        pass


def publish_telemetry(c, tel):
    """Dipakai SITL / Dashboard SITL runner (ESP32 asli mempublikasikan sendiri)."""
    c.publish(K.T_TELEM, json.dumps(dict(tel, src="sitl")))   # penanda sumber: bukan ESP32
    scalar_formats = {
        "pm": "{:.2f}",
        "pm_raw": "{:.2f}",
        "dcdt": "{:.3f}",
        "fi": "{:.2f}",
        "g": "{:.3f}",
        "gas_ppm": "{:.1f}",
        "v_dust": "{:.3f}",
        "v_mq": "{:.3f}",
        "v0": "{:.3f}",
        "duty": "{:.1f}",
        "state": "{}",
        "mode": "{}",
        "seq": "{}",
    }
    for k, fmt in scalar_formats.items():
        if k in tel and tel[k] is not None:
            v = tel[k]
            c.publish(f"{K.T_TELEM_SCALAR}/{k}", fmt.format(v) if isinstance(v, (int, float)) else str(v))


def _is_nan(v):
    return isinstance(v, float) and math.isnan(v)


def publish_forecast(c, f):
    clean = {k: (None if _is_nan(v) else v) for k, v in f.items()}   # NaN -> null
    c.publish(K.T_FORECAST, json.dumps(clean))
    pm_fc = clean.get("pm_forecast")
    if pm_fc is not None:
        c.publish(f"{K.T_FORECAST_SCALAR}/pm_forecast", f"{pm_fc:.1f}")
    eta = clean.get("eta_bahaya_s")
    c.publish(f"{K.T_FORECAST_SCALAR}/eta_bahaya_s", "-" if eta is None else f"{eta:.0f}")
    slope = clean.get("slope")
    if slope is not None:
        c.publish(f"{K.T_FORECAST_SCALAR}/slope", f"{slope:.3f}")
    mae = clean.get("mae")
    if mae is not None:
        c.publish(f"{K.T_FORECAST_SCALAR}/mae", f"{mae:.2f}")
