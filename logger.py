"""Logger CSV harian & Logger MQTT harian (rotasi otomatis tiap ganti hari)."""
import csv
import datetime
import os
import threading

FIELDS = ["timestamp", "t_s", "source", "scenario", "pm_true", "v_dust", "v_mq",
          "pm_raw", "pm", "dcdt", "gas_ppm", "g", "fi", "v0", "state", "mode", "duty",
          "forecast", "slope", "eta_bahaya_s", "mae", "sensor_fault"]


class DailyCsvLogger:
    def __init__(self, folder="logs", prefix="karhutla", source="sitl"):
        self.folder, self.prefix, self.source = folder, prefix, source
        self._day, self._f, self._w = None, None, None
        os.makedirs(folder, exist_ok=True)

    def _open(self, day):
        if self._f:
            self._f.close()
        path = os.path.join(self.folder, f"{self.prefix}_{self.source}_{day}.csv")
        new = not os.path.exists(path) or os.path.getsize(path) == 0
        self._f = open(path, "a", newline="", encoding="utf-8")
        self._w = csv.DictWriter(self._f, fieldnames=FIELDS)
        if new:
            self._w.writeheader()
        self._day = day
        self.path = path

    def write(self, row):
        day = datetime.date.today().isoformat()
        if day != self._day:
            self._open(day)
        out = {k: ("" if row.get(k) is None else row.get(k)) for k in FIELDS}
        out["timestamp"] = datetime.datetime.now().isoformat(timespec="milliseconds")
        out["source"] = self.source
        self._w.writerow(out)
        self._f.flush()

    def close(self):
        if self._f:
            self._f.close()
            self._f = None


class DailyMqttLogger:
    """Pencatat trafik & event MQTT otomatis ke logs/mqtt_YYYY-MM-DD.log."""

    def __init__(self, folder="logs", prefix="mqtt"):
        self.folder, self.prefix = folder, prefix
        self._day, self._f = None, None
        self._lock = threading.Lock()
        self.path = None
        os.makedirs(folder, exist_ok=True)

    def _open(self, day):
        if self._f:
            self._f.close()
        path = os.path.join(self.folder, f"{self.prefix}_{day}.log")
        self._f = open(path, "a", encoding="utf-8")
        self._day = day
        self.path = path

    def log_event(self, client_id, event, topic="", payload=""):
        clean_topic = str(topic or "").replace("\r", " ").replace("\n", " ").strip()
        clean_payload = str(payload if payload is not None else "").replace("\r", " ").replace("\n", " ").strip()
        ts = datetime.datetime.now().isoformat(timespec="milliseconds")
        parts = [f"{ts} [{client_id}] {event}"]
        if clean_topic:
            parts.append(clean_topic)
        if clean_payload:
            parts.append(clean_payload)
        line = " ".join(parts)
        with self._lock:
            day = datetime.date.today().isoformat()
            if day != self._day or self._f is None:
                self._open(day)
            self._f.write(line + "\n")
            self._f.flush()
        return line

    def close(self):
        with self._lock:
            if self._f:
                self._f.close()
                self._f = None
