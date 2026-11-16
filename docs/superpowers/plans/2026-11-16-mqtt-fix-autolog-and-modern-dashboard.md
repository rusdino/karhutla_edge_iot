# Perbaikan MQTT SITL (Port 6669), Auto-Logger MQTT, dan Modern Web Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Menyeragamkan komunikasi MQTT ke port `6669` dengan paritas topik/perintah penuh (`karhutla/status`, `RESET`, `gas_ppm`, skalar lengkap) di SITL/HILS/ESP32, mengotomatiskan rotasi log harian `logs/mqtt_YYYY-MM-DD.log` beserta auto-start broker, dan membangun Web Dashboard interaktif modern (`dashboard.py`) yang mematuhi pedoman **Modern Web Guidance**.

**Architecture:** Konfigurasi port `6669` dan topik `karhutla/status` disinkronkan antara `config.py`, `mosquitto.conf`, dan `firmware/karhutla_esp32/{config.h,karhutla_esp32.ino}`. Modul `logger.py` menyediakan `DailyMqttLogger` yang diintegrasikan ke `mqtt_io.py` bersama `ensure_broker()` untuk auto-start Mosquitto dan pencatatan trafik otomatis. Modul `dashboard.py` menyajikan server HTTP + Server-Sent Events (SSE) dengan antarmuka HTML5/CSS3 modern (OKLCH `light-dark()`, CSS `subgrid`, `@container`, `<article>` card, `role="toolbar"`, `<input type="checkbox" switch>`) yang mendukung grafik *multi-overlay*, kontrol aktuator (`karhutla/cmd`), dan *embedded SITL runner*.

**Tech Stack:** Python 3.x (`http.server`, `threading`, `paho-mqtt`, `unittest`/`pytest`), HTML5/CSS3/Canvas 2D (Modern Web Guidance compliant), Mosquitto MQTT Broker, C++ (Arduino-ESP32 + FreeRTOS).

**Spec:** `docs/superpowers/specs/2026-11-16-mqtt-fix-autolog-and-modern-dashboard-design.md`

## Global Constraints

- Buka izin tulis (`chmod u+w`) terlebih dahulu pada file yang berstatus *read-only* (`config.py`, `mqtt_io.py`, `logger.py`, `mosquitto.conf`, `field_node.py`, `forecast.py`, `plant.py`).
- Gunakan `BROKER_PORT = 6669` di `config.py`, `#define MQTT_PORT 6669` di `firmware/karhutla_esp32/config.h`, dan `listener 6669 0.0.0.0` di `mosquitto.conf`.
- Pertahankan paritas algoritma 100% antara `core.py` dan `firmware/karhutla_esp32/karhutla_esp32.ino`; jangan mengubah konstanta kalibrasi sensor, Kalman, atau ambang FSM.
- Seluruh komentar kode, label UI, dan dokumentasi menggunakan **Bahasa Indonesia** (gunakan istilah **"PM-proxy"**, bukan PM2,5 tersertifikasi).
- Jangan menambahkan dependensi eksternal di luar `requirements.txt` (`numpy`, `matplotlib`, `paho-mqtt`).
- Antarmuka web di `dashboard.py` wajib mematuhi aturan **Modern Web Guidance**: `<meta name="color-scheme" content="light dark">`, `:root { color-scheme: light dark; }` dengan fallback `@media (prefers-color-scheme: dark)` dan `@supports (color: light-dark(white, black))`, `@media (forced-colors: active)`, indikator non-warna (ikon + teks), `grid-template-rows: subgrid` dengan fallback `grid-template-rows: auto 1fr auto`, `container-type: inline-size`, `<article class="kpi-card">`, `role="toolbar" focusgroup="toolbar wrap"` dengan `[aria-pressed="true"]` + fallback roving-tabindex, `<input type="checkbox" switch>`, serta `scrollbar-color` + `@supports not (scrollbar-color: auto)`.

## Review Focus

- Saat perintah `FORCE_MAX`, `FORCE_OFF`, `AUTO`, atau `RESET` diterima via `karhutla/cmd` di tengah jeda sampel SITL (`--speed 1`), `Firmware.apply_command()` harus langsung memperbarui `mode`/`duty` (atau mereset state) dan segera mem-publish ulang telemetri tanpa menunggu langkah `t` berikutnya.
- Saat `publish_forecast()` menerima nilai `eta_bahaya_s=None` atau `float('nan')`, JSON `karhutla/forecast` harus tetap valid (`null` bukan `NaN`) dan topik skalar `karhutla/f/eta_bahaya_s` berisi `"-"`.
- Saat `ensure_broker()` dipanggil ketika broker Mosquitto sudah berjalan di port `6669`, fungsi tersebut tidak boleh gagal atau menyalakan proses duplikat yang bentrok port.
- Saat `DailyMqttLogger` mencatat payload biner atau string berkarakter newline dari topik MQTT, baris log di `logs/mqtt_YYYY-MM-DD.log` harus tetap bersih satu baris per event dan langsung ter-*flush* ke disk.
- Saat pengguna mengaktifkan saklar injeksi *sensor fault* (`fault_inject`) di `dashboard.py`, simulasi harus menyuntikkan tegangan `v_dust = 0.0` (kabel putus) sehingga `Firmware` masuk ke state `FAILSAFE` (`duty = 70%`) setelah `FS_INVALID_N` (5) sampel, dan pulih otomatis setelah saklar dimatikan selama `FS_RECOVER_N` (5) sampel.

---

### Task 1: Penyeragaman Port 6669, `T_STATUS`, `Firmware.reset()`/`apply_command()`, dan Paritas `gas_ppm` ESP32

**Files:**
- Modify: `mosquitto.conf`
- Modify: `config.py`
- Modify: `core.py`
- Modify: `firmware/karhutla_esp32/config.h`
- Modify: `firmware/karhutla_esp32/karhutla_esp32.ino`
- Create: `tests/test_mqtt_and_dashboard.py`

**Interfaces:**
- Consumes: Konstanta kalibrasi dan FSM di `config.py` & `firmware/karhutla_esp32/config.h`
- Produces:
  - `config.BROKER_PORT = 6669`
  - `config.T_STATUS = "karhutla/status"`
  - `core.Firmware.reset(self) -> dict`
  - `core.Firmware.apply_command(self, cmd: str) -> dict | None`
  - `core.Firmware.current_telemetry(self, v_dust: float = None, v_mq: float = None) -> dict`

- [ ] **Step 1: Buat pengujian gagal untuk konfigurasi port 6669, `T_STATUS`, `Firmware.reset()`/`apply_command()`, dan paritas `config.h`/`karhutla_esp32.ino`**

Buat file `tests/test_mqtt_and_dashboard.py` dengan pengujian berikut:

```python
import unittest
import config as K
from core import Firmware


class TestConfigAndCoreCommands(unittest.TestCase):
    def test_port_6669_and_status_topic(self):
        self.assertEqual(K.BROKER_PORT, 6669)
        self.assertEqual(getattr(K, "T_STATUS", None), "karhutla/status")
        with open("mosquitto.conf", encoding="utf-8") as f:
            conf = f.read()
        self.assertIn("listener 6669 0.0.0.0", conf)
        with open("firmware/karhutla_esp32/config.h", encoding="utf-8") as f:
            ch = f.read()
        self.assertIn("#define MQTT_PORT   6669", ch)
        with open("firmware/karhutla_esp32/karhutla_esp32.ino", encoding="utf-8") as f:
            ino = f.read()
        self.assertIn("gas_ppm", ino)

    def test_firmware_apply_command_and_reset(self):
        fw = Firmware()
        fw.step(1.5, 1.2, seq=1)
        tel_max = fw.apply_command("FORCE_MAX")
        self.assertIsNotNone(tel_max)
        self.assertEqual(fw.mode, "FORCE_MAX")
        self.assertEqual(tel_max["duty"], 100.0)

        tel_off = fw.apply_command("FORCE_OFF")
        self.assertEqual(fw.mode, "FORCE_OFF")
        self.assertEqual(tel_off["duty"], 0.0)

        tel_rst = fw.apply_command("RESET")
        self.assertEqual(fw.mode, "AUTO")
        self.assertEqual(tel_rst["state"], "NORMAL")
        self.assertEqual(tel_rst["pm"], 0.0)
        self.assertEqual(tel_rst["duty"], 0.0)
        self.assertIsNone(fw.apply_command("INVALID_CMD"))


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Jalankan pengujian untuk memverifikasi kegagalan awal**

Run: `./sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: FAIL (`1883 != 6669` dan `'Firmware' object has no attribute 'apply_command'`)

- [ ] **Step 3: Implementasikan perubahan pada `mosquitto.conf`, `config.py`, `core.py`, `config.h`, dan `karhutla_esp32.ino`**

1. Jalankan `chmod u+w mosquitto.conf config.py mqtt_io.py logger.py field_node.py forecast.py plant.py`.
2. Di `mosquitto.conf`, ubah `listener 6669` menjadi `listener 6669 0.0.0.0` dan perbarui komentar port menjadi `6669`.
3. Di `config.py`, ubah `BROKER_PORT = 6669` dan tambahkan `T_STATUS = "karhutla/status"`.
4. Di `core.py`, tambahkan pada kelas `Firmware`:
   - Simpan `self.last_vd = K.DUST_V0` dan `self.last_vm = 1.0` saat `step()`.
   - `current_telemetry(self, v_dust=None, v_mq=None) -> dict`: mengembalikan dictionary telemetri terkini (`seq`, `v_dust`, `v_mq`, `pm_raw`, `pm`, `dcdt`, `gas_ppm`, `g`, `fi`, `v0`, `state`, `mode`, `duty`, `alarm`).
   - `reset(self) -> dict`: mereset `med_d`, `med_m`, `kf`, `fsm`, mengembalikan `v0 = K.DUST_V0`, `pm_raw = 0.0`, `ppm = K.MQ_ATM_PPM`, `g = 0.0`, `fi = 0.0`, `duty = 0.0`, `failsafe = False`, `bad = 0`, `good = 0`, `calib = []`, `r0`, dan `mode = "AUTO"`, lalu mengembalikan `current_telemetry()`.
   - `apply_command(self, cmd: str) -> dict | None`: membersihkan string `cmd`; jika `"RESET"`, panggil `return self.reset()`; jika `cmd in MODES`, set `self.mode = cmd`, hitung ulang `self.duty = self._target_duty(FAILSAFE if self.failsafe else self.fsm.state)`, dan kembalikan `self.current_telemetry()`; selain itu kembalikan `None`.
5. Di `firmware/karhutla_esp32/config.h`, ubah `#define MQTT_PORT   6669`.
6. Di `firmware/karhutla_esp32/karhutla_esp32.ino`, tambahkan field `float gas_ppm = MQ_ATM_PPM;` pada `struct Tel`, isi `tel.gas_ppm = gasPpm;` di `publishState()`, serta sertakan `"gas_ppm":%.1f` pada JSON `T_TELEM` dan publikasikan skalar `T_TELEM_SCALAR "/gas_ppm"` di `publishTel()`.

- [ ] **Step 4: Jalankan pengujian Python dan kompilasi firmware ESP32 untuk memverifikasi keberhasilan**

Run: `./sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: PASS (2 tests OK)

---

### Task 2: Auto-Logger MQTT (`DailyMqttLogger`) di `logger.py`, Perbaikan `mqtt_io.py`, dan Integrasi `sitl.py`

**Files:**
- Modify: `logger.py`
- Modify: `mqtt_io.py`
- Modify: `sitl.py`
- Modify: `tests/test_mqtt_and_dashboard.py`

**Interfaces:**
- Consumes: `config.BROKER_HOST`, `config.BROKER_PORT`, `config.T_STATUS`, `config.T_TELEM`, `config.T_TELEM_SCALAR`, `config.T_FORECAST`, `config.T_FORECAST_SCALAR`, `config.T_CMD`, `core.Firmware.apply_command`
- Produces:
  - `logger.DailyMqttLogger(folder="logs", prefix="mqtt")` dengan metode `log_event(client_id: str, event: str, topic: str = "", payload: str = "") -> str` dan `close() -> None`
  - `mqtt_io.ensure_broker(host=None, port=None, conf_path="mosquitto.conf") -> bool`
  - `mqtt_io.make_client(client_id, on_connect=None, on_message=None, publish_status=False, mqtt_logger=None)`
  - `mqtt_io.connect(c, host=None, port=None, auto_start_broker=True)`
  - `mqtt_io.disconnect(c)`
  - `mqtt_io.publish_telemetry(c, tel)` (mem-publish JSON + 13 sub-topik skalar)
  - `mqtt_io.publish_forecast(c, f)` (mem-publish JSON + 4 sub-topik skalar)

- [ ] **Step 1: Tambahkan pengujian gagal untuk `DailyMqttLogger`, `mqtt_io` (status, skalar lengkap, `ensure_broker`), dan integrasi `sitl.py --mqtt`**

Tambahkan `TestMqttLoggerAndIo` di `tests/test_mqtt_and_dashboard.py`:

```python
import os
import tempfile
from logger import DailyMqttLogger
import mqtt_io


class TestMqttLoggerAndIo(unittest.TestCase):
    def test_daily_mqtt_logger_writes_single_line_flushed(self):
        with tempfile.TemporaryDirectory() as tmp:
            mlog = DailyMqttLogger(folder=tmp, prefix="mqtt")
            line = mlog.log_event("karhutla-sitl", "PUBLISH", "karhutla/cmd", "FORCE_MAX\n")
            mlog.close()
            self.assertIn("[karhutla-sitl] PUBLISH karhutla/cmd FORCE_MAX", line)
            self.assertTrue(os.path.exists(mlog.path))
            with open(mlog.path, encoding="utf-8") as f:
                content = f.read().splitlines()
            self.assertEqual(len(content), 1)

    def test_ensure_broker_and_live_publish_subscribe(self):
        with tempfile.TemporaryDirectory() as tmp:
            ok = mqtt_io.ensure_broker("127.0.0.1", K.BROKER_PORT)
            self.assertTrue(ok)
            received = {}
            mlog = DailyMqttLogger(folder=tmp, prefix="mqtt")

            def on_conn(c, u, flags, rc, props=None):
                c.subscribe([(K.T_STATUS, 0), (f"{K.T_TELEM_SCALAR}/+", 0), (f"{K.T_FORECAST_SCALAR}/+", 0)])

            def on_msg(c, u, msg):
                received[msg.topic] = msg.payload.decode("utf-8", errors="ignore")

            sub = mqtt_io.make_client("test-sub", on_conn, on_msg, mqtt_logger=mlog)
            mqtt_io.connect(sub, "127.0.0.1", K.BROKER_PORT)

            pub = mqtt_io.make_client("test-pub", publish_status=True, mqtt_logger=mlog)
            mqtt_io.connect(pub, "127.0.0.1", K.BROKER_PORT)

            import time
            time.sleep(0.4)
            fw = Firmware()
            tel = fw.step(0.95, 1.25, seq=7)
            mqtt_io.publish_telemetry(pub, tel)
            mqtt_io.publish_forecast(pub, {"pm_forecast": 88.4, "slope": 0.75, "eta_bahaya_s": float("nan"), "mae": 2.1})
            time.sleep(0.4)

            self.assertEqual(received.get(K.T_STATUS), "online")
            self.assertIn(f"{K.T_TELEM_SCALAR}/gas_ppm", received)
            self.assertIn(f"{K.T_TELEM_SCALAR}/pm_raw", received)
            self.assertIn(f"{K.T_TELEM_SCALAR}/v_dust", received)
            self.assertEqual(received.get(f"{K.T_FORECAST_SCALAR}/eta_bahaya_s"), "-")
            self.assertIn(f"{K.T_FORECAST_SCALAR}/slope", received)

            mqtt_io.disconnect(pub)
            time.sleep(0.2)
            self.assertEqual(received.get(K.T_STATUS), "offline")
            mqtt_io.disconnect(sub)
            mlog.close()
```

- [ ] **Step 2: Jalankan pengujian untuk memverifikasi kegagalan**

Run: `./sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: FAIL (`ImportError: cannot import name 'DailyMqttLogger' from 'logger'`)

- [ ] **Step 3: Implementasikan `DailyMqttLogger` di `logger.py`, perbarui `mqtt_io.py`, dan lengkapi `sitl.py`**

1. Di `logger.py`:
   - Tambahkan `"gas_ppm"` ke dalam `FIELDS` (setelah `"dcdt"`).
   - Tambahkan kelas `DailyMqttLogger(folder="logs", prefix="mqtt")` yang membuka `logs/{prefix}_{YYYY-MM-DD}.log` secara *thread-safe* (`threading.Lock()`), menyediakan `log_event(client_id, event, topic="", payload="")` yang menormalkan newline pada payload menjadi spasi, menulis `<ISO-timestamp> [<client_id>] <EVENT> <topic> <payload>\n`, memanggil `flush()`, dan mengembalikan string baris log tersebut.
2. Di `mqtt_io.py`:
   - Tambahkan `ensure_broker(host=None, port=None, conf_path="mosquitto.conf") -> bool`: cek koneksi socket TCP ke `(host or K.BROKER_HOST, port or K.BROKER_PORT)`. Jika gagal dan host adalah lokal (`localhost` / `127.0.0.1`), jalankan `subprocess.Popen(["mosquitto", "-c", conf_path], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)` dan tunggu hingga port terbuka (maks 3 detik).
   - Pada `make_client(client_id, on_connect=None, on_message=None, publish_status=False, mqtt_logger=None)`:
     - Gunakan instance `mqtt_logger or DailyMqttLogger()` yang disimpan di `c._karhutla_logger` dan `c._karhutla_id = client_id` serta `c._publish_status = publish_status`.
     - Jika `publish_status=True`, panggil `c.will_set(K.T_STATUS, "offline", qos=0, retain=True)`.
     - Bungkus callback `on_connect` dan `on_message` agar setiap event `CONNECT`, `RECV`, serta panggilan `c.subscribe` dan `c.publish` otomatis tercatat ke `c._karhutla_logger.log_event(...)` (dan saat `publish_status=True`, `on_connect` otomatis mem-publish `c.publish(K.T_STATUS, "online", retain=True)`).
   - Pada `connect(c, host=None, port=None, auto_start_broker=True)`: panggil `ensure_broker(host, port)` bila `auto_start_broker=True`, lalu `c.connect(...)` dan `c.loop_start()`.
   - Tambahkan `disconnect(c)`: jika `getattr(c, "_publish_status", False)`, publish `K.T_STATUS = "offline"` (`retain=True`); catat event `DISCONNECT`, panggil `c.loop_stop()` dan `c.disconnect()`.
   - Pada `publish_telemetry(c, tel)`: publish JSON ke `K.T_TELEM` dan skalar ke `karhutla/t/{pm,pm_raw,dcdt,fi,g,gas_ppm,v_dust,v_mq,v0,state,mode,duty,seq}`.
   - Pada `publish_forecast(c, f)`: ubah `NaN` menjadi `None` pada JSON `K.T_FORECAST`, serta publish `pm_forecast`, `eta_bahaya_s` (`"-"` jika `None` atau `NaN`), `slope`, dan `mae` ke `karhutla/f/*`.
3. Di `sitl.py`:
   - Saat `args.mqtt` aktif, buat client dengan `publish_status=True`, gunakan `fw.apply_command(msg.payload.decode(errors="ignore").strip())` pada `on_msg` dan jika mengembalikan dict telemetri, langsung panggil `mqtt_io.publish_telemetry(client, tel_now)`.
   - Di akhir `run()`, panggil `mqtt_io.disconnect(client)`.

- [ ] **Step 4: Jalankan pengujian untuk memverifikasi semua lulus**

Run: `./sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: PASS (4 tests OK)

---

### Task 3: Pembuatan Modern Web Dashboard (`dashboard.py`) Sesuai Modern Web Guidance & Pembaruan `README.md`

**Files:**
- Create: `dashboard.py`
- Modify: `sitl.py`
- Modify: `README.md`
- Modify: `tests/test_mqtt_and_dashboard.py`

**Interfaces:**
- Consumes: `config`, `core.Firmware`, `plant.SmokeChamber`, `plant.SCENARIOS`, `forecast.LinearTrendForecaster`, `forecast.PreemptController`, `logger.DailyCsvLogger`, `logger.DailyMqttLogger`, `mqtt_io`
- Produces:
  - `dashboard.DashboardEngine`: mengelola koneksi MQTT `:6669`, *ring buffer* riwayat telemetri & log MQTT, serta *embedded SITL runner thread* (`start`, `stop`, `pause`, `resume`, `set_scenario`, `set_speed`, `set_ai_preempt`, `set_fault_inject`, `send_cmd`).
  - `dashboard.start_dashboard_server(engine: DashboardEngine, host: str = "0.0.0.0", port: int = 8080) -> http.server.ThreadingHTTPServer`
  - CLI `python dashboard.py [--port 8080] [--host localhost] [--broker-host localhost] [--broker-port 6669] [--auto-sim flaming] [--speed 1] [--no-browser]`
  - Flag `--dashboard` pada `sitl.py`

- [ ] **Step 1: Tambahkan pengujian gagal untuk endpoint `dashboard.py`, kepatuhan Modern Web Guidance, dan injeksi *sensor fault***

Tambahkan `TestModernWebDashboard` di `tests/test_mqtt_and_dashboard.py`:

```python
import json
import urllib.request
import dashboard


class TestModernWebDashboard(unittest.TestCase):
    def test_dashboard_html_complies_with_modern_web_guidance(self):
        html = dashboard.RENDERED_HTML
        self.assertIn('<meta name="color-scheme" content="light dark">', html)
        self.assertIn("color-scheme: light dark", html)
        self.assertIn("light-dark(", html)
        self.assertIn("grid-template-rows: subgrid", html)
        self.assertIn("container-type: inline-size", html)
        self.assertIn('<article class="kpi-card"', html)
        self.assertIn('role="toolbar"', html)
        self.assertIn('focusgroup="toolbar wrap"', html)
        self.assertIn('aria-pressed="true"', html)
        self.assertIn('type="checkbox" switch', html)
        self.assertIn("@supports not (scrollbar-color: auto)", html)

    def test_dashboard_server_endpoints_and_fault_injection(self):
        eng = dashboard.DashboardEngine(broker_host="127.0.0.1", broker_port=K.BROKER_PORT, enable_mqtt=True)
        srv = dashboard.start_dashboard_server(eng, host="127.0.0.1", port=0)
        port = srv.server_address[1]
        base = f"http://127.0.0.1:{port}"
        try:
            # 1. GET /
            resp = urllib.request.urlopen(f"{base}/", timeout=3).read().decode("utf-8")
            self.assertIn("Karhutla Edge-IoT", resp)

            # 2. POST /api/sim start with fast speed & fault_inject=True
            req = urllib.request.Request(
                f"{base}/api/sim",
                data=json.dumps({"action": "start", "scenario": "clean", "speed": 50, "fault_inject": True}).encode("utf-8"),
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            res = json.loads(urllib.request.urlopen(req, timeout=3).read().decode("utf-8"))
            self.assertTrue(res["ok"])

            import time
            time.sleep(0.35)

            # 3. GET /api/state -> harus sudah masuk FAILSAFE karena fault_inject=True (>= 5 langkah)
            st = json.loads(urllib.request.urlopen(f"{base}/api/state", timeout=3).read().decode("utf-8"))
            self.assertGreaterEqual(len(st["history"]), 6)
            self.assertEqual(st["latest"]["state"], "FAILSAFE")
            self.assertEqual(st["latest"]["duty"], K.DUTY_FAILSAFE)

            # 4. POST /api/cmd FORCE_OFF -> tetap dihormati saat FAILSAFE
            cmd_req = urllib.request.Request(
                f"{base}/api/cmd",
                data=json.dumps({"cmd": "FORCE_OFF"}).encode("utf-8"),
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            cmd_res = json.loads(urllib.request.urlopen(cmd_req, timeout=3).read().decode("utf-8"))
            self.assertTrue(cmd_res["ok"])
            time.sleep(0.15)
            st2 = json.loads(urllib.request.urlopen(f"{base}/api/state", timeout=3).read().decode("utf-8"))
            self.assertEqual(st2["latest"]["mode"], "FORCE_OFF")
            self.assertEqual(st2["latest"]["duty"], 0.0)
        finally:
            eng.shutdown()
            srv.shutdown()
            srv.server_close()
```

- [ ] **Step 2: Jalankan pengujian untuk memverifikasi kegagalan**

Run: `./sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: FAIL (`ModuleNotFoundError: No module named 'dashboard'`)

- [ ] **Step 3: Implementasikan `dashboard.py`, tambahkan `--dashboard` di `sitl.py`, dan perbarui `README.md`**

1. Buat `dashboard.py`:
   - Implementasikan `DashboardEngine` yang:
     - Memastikan broker MQTT `:6669` menyala via `mqtt_io.ensure_broker()`, menghubungkan client MQTT dashboard (`karhutla-dashboard`) yang men-subscribe `karhutla/#`, mencatat semua event ke `DailyMqttLogger` (`logs/mqtt_YYYY-MM-DD.log`), dan menyimpan riwayat telemetri/forecast (`deque(maxlen=1800)`) serta riwayat log MQTT (`deque(maxlen=300)`).
     - Menyediakan *embedded SITL runner* (`start`, `stop`, `pause`, `resume`, `scenario`, `speed`, `ai_preempt`, `fault_inject`, `clear`) yang juga mem-publish telemetri dan forecast ke broker MQTT `:6669` (sehingga HP dan laptop sinkron secara bersamaan) serta mencatat CSV ke `DailyCsvLogger(source="sitl")`. Saat `fault_inject=True`, paksa `v_dust = 0.0` (simulasi sensor putus) sehingga firmware masuk `FAILSAFE` setelah 5 sampel dan pulih setelah `fault_inject=False` selama 5 sampel.
   - Implementasikan `RENDERED_HTML` dengan antarmuka **Modern Web Guidance**:
     - Header status broker `:6669`, status node (`online`/`offline`), indikator state FSM, dan nama file log aktif (`logs/mqtt_YYYY-MM-DD.log`).
     - 6 Kartu KPI `<article class="kpi-card">` dengan `grid-template-rows: subgrid` dan `container-type: inline-size`.
     - Kanvas grafik utama HTML5 `<canvas>` (dengan *hover crosshair tooltip* nilai per sampel, tombol *Reset Zoom / Jendela Waktu*, dan tombol *Export PNG*) + sub-kanvas sinkron untuk `Duty PWM (%)` & `State FSM`.
     - Toolbar `<div role="toolbar" focusgroup="toolbar wrap">` berisi tombol toggle (`[aria-pressed="true|false"]`) untuk layer: `Fusion Index (FI)`, `PM Kalman`, `PM Plant (True)`, `PM Raw (Median)`, `AI Forecast (t+60s)`, `Gas MQ-135 (ppm)`, `Skor Gas (g×100)`, `Tegangan Sensor (V×50)`, dan `Ambang Waspada/Bahaya`.
     - Panel kontrol *override* (`AUTO`, `FORCE_MAX`, `FORCE_OFF`, `RESET`) dan panel operasi simulasi SITL (pilihan skenario `clean`/`smolder`/`flaming`/`fault`, tombol `Start/Pause/Stop/Clear`, pilihan kecepatan `1x`/`2x`/`5x`/`10x`/`20x`, saklar `<input type="checkbox" switch>` untuk `AI Preempt` dan `Injeksi Sensor Fault`).
     - Panel *Live MQTT Stream* dengan filter topik dan saklar `Auto-scroll`.
2. Di `sitl.py`: tambahkan argumen `--dashboard` dan `--dashboard-port` (default `8080`) yang menyalakan server web `dashboard.py` saat `sitl.py` berjalan dengan `--mqtt`.
3. Di `README.md`: perbarui port `1883` menjadi `6669`, dokumentasikan fitur auto-log `logs/mqtt_YYYY-MM-DD.log`, daftar topik lengkap (`karhutla/t/gas_ppm`, dll.), serta cara menjalankan **Modern Web Dashboard di Laptop** (`python dashboard.py` atau `python sitl.py --scenario flaming --mqtt --speed 1 --dashboard`).

- [ ] **Step 4: Jalankan seluruh pengujian unit, simulasi 4 skenario `sitl.py`, dan kompilasi PlatformIO ESP32**

Run: `./sitl_venv/bin/python -m unittest discover -s tests -v && ./sitl_venv/bin/python sitl.py --scenario all`
Expected: Semua unit test `OK` dan `sitl.py` mencetak `HASIL AKHIR: SEMUA LULUS` (exit code 0).
