import json
import os
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.request
import io
import config as K
from core import Firmware
from logger import DailyMqttLogger
import mqtt_io
import dashboard


class TestConfigAndCoreCommands(unittest.TestCase):
    def test_port_6669_and_status_topic(self):
        self.assertEqual(K.BROKER_PORT, 6669)
        self.assertEqual(getattr(K, "T_STATUS", None), "karhutla/status")
        with open("mosquitto.conf", encoding="utf-8") as f:
            conf = f.read()
        self.assertIn("listener 6669 0.0.0.0", conf)

    def test_espidf_firmware_structure_no_arduino(self):
        self.assertFalse(os.path.exists("firmware/karhutla_esp32/karhutla_esp32.ino"))
        self.assertTrue(os.path.exists("firmware/karhutla_esp32/platformio.ini"))
        self.assertTrue(os.path.exists("firmware/karhutla_esp32/CMakeLists.txt"))
        self.assertTrue(os.path.exists("firmware/karhutla_esp32/src/CMakeLists.txt"))
        self.assertTrue(os.path.exists("firmware/karhutla_esp32/include/config.h"))
        self.assertTrue(os.path.exists("firmware/karhutla_esp32/src/main.cpp"))

        with open("firmware/karhutla_esp32/platformio.ini", encoding="utf-8") as f:
            pio_ini = f.read()
        self.assertIn("espidf_sdk.py", pio_ini)
        self.assertNotIn("framework = arduino", pio_ini)
        self.assertIn("board = esp32dev", pio_ini)
        self.assertIn("[env:hils]", pio_ini)
        self.assertIn("[env:riil]", pio_ini)

        with open("firmware/karhutla_esp32/include/config.h", encoding="utf-8") as f:
            ch = f.read()
        self.assertIn("#define MQTT_PORT   6669", ch)
        self.assertIn("#ifndef MODE_HILS", ch)

        with open("firmware/karhutla_esp32/src/main.cpp", encoding="utf-8") as f:
            main_cpp = f.read()
        self.assertNotIn("Arduino.h", main_cpp)
        self.assertNotIn("PubSubClient", main_cpp)
        self.assertIn("app_main", main_cpp)
        self.assertIn("mqtt_client.h", main_cpp)
        self.assertIn("driver/gpio.h", main_cpp)
        self.assertIn("driver/ledc.h", main_cpp)
        self.assertIn("esp_wifi.h", main_cpp)
        self.assertIn("gas_ppm", main_cpp)
        self.assertIn("esp_wifi_set_ps(WIFI_PS_NONE)", main_cpp)

    def test_hils_and_field_node_sync_with_espidf_telemetry(self):
        with open("hils.py", encoding="utf-8") as f:
            hils_src = f.read()
        self.assertIn('gas_ppm=tel.get("gas_ppm"', hils_src)
        self.assertIn("gas_true=plant.gas_ppm", hils_src)
        self.assertIn("mqtt_io.disconnect(client)", hils_src)

        with open("field_node.py", encoding="utf-8") as f:
            fn_src = f.read()
        self.assertIn('gas_ppm=tel.get("gas_ppm"', fn_src)
        self.assertIn("mqtt_io.disconnect(client)", fn_src)

    def test_hils_reset_handshake_does_not_wipe_fast_esp32_reply(self):
        import hils
        link = hils.Link()

        class DummyClient:
            def publish(self, topic, payload):
                if topic == K.T_CMD and payload == "RESET":
                    # Simulasikan ESP32 membalas RESET dalam 50 ms
                    class Msg:
                        pass
                    m = Msg()
                    m.topic = K.T_TELEM
                    m.payload = b'{"seq":0,"state":"NORMAL","mode":"AUTO","duty":0.0}'
                    link.on_message(self, None, m)

        tel = hils.reset_and_wait_esp32(DummyClient(), link, timeout=1.0)
        self.assertIsNotNone(tel)
        self.assertEqual(tel.get("state"), "NORMAL")

    def test_hils_wait_seq_with_retry_recovers_dropped_packet(self):
        import hils
        link = hils.Link()
        attempts = []

        class RetryClient:
            def publish(self, topic, payload):
                attempts.append((topic, payload))
                if len(attempts) == 2:
                    class Msg:
                        pass
                    m = Msg()
                    m.topic = K.T_TELEM
                    m.payload = b'{"seq":5,"pm":12.3,"state":"NORMAL","mode":"AUTO","duty":0.0}'
                    link.on_message(self, None, m)

        tel = hils.send_and_wait_seq(RetryClient(), link, 5, 0.61, 1.0, timeout=0.3)
        self.assertIsNotNone(tel)
        self.assertEqual(tel["seq"], 5)
        self.assertEqual(len(attempts), 2)

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


class TestModernWebDashboard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.bind(("127.0.0.1", 0))
            cls.broker_port = s.getsockname()[1]
        cls._broker_proc = subprocess.Popen(
            ["mosquitto", "-p", str(cls.broker_port)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.time() + 5.0
        while time.time() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", cls.broker_port), timeout=0.2):
                    break
            except OSError:
                time.sleep(0.05)

    @classmethod
    def tearDownClass(cls):
        proc = getattr(cls, "_broker_proc", None)
        if proc is not None:
            proc.terminate()
            try:
                proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                proc.kill()

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
        self.assertIn("esp_status", html)
        self.assertIn('id="esp-banner"', html)
        self.assertIn('class="chip-btn src-btn"', html)
        self.assertIn('data-target="hils"', html)
        self.assertIn("requestAnimationFrame", html)

    def test_dashboard_server_endpoints_and_fault_injection(self):
        eng = dashboard.DashboardEngine(broker_host="127.0.0.1", broker_port=self.broker_port, enable_mqtt=True)
        srv = dashboard.start_dashboard_server(eng, host="127.0.0.1", port=0)
        port = srv.server_address[1]
        base = f"http://127.0.0.1:{port}"
        try:
            resp = urllib.request.urlopen(f"{base}/", timeout=3).read().decode("utf-8")
            self.assertIn("Karhutla Edge-IoT", resp)

            req = urllib.request.Request(
                f"{base}/api/sim",
                data=json.dumps({"action": "start", "scenario": "clean", "speed": 50, "fault_inject": True}).encode("utf-8"),
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            res = json.loads(urllib.request.urlopen(req, timeout=3).read().decode("utf-8"))
            self.assertTrue(res["ok"])

            time.sleep(0.35)

            st = json.loads(urllib.request.urlopen(f"{base}/api/state", timeout=3).read().decode("utf-8"))
            self.assertGreaterEqual(len(st["history"]), 6)
            self.assertEqual(st["latest"]["state"], "FAILSAFE")
            self.assertEqual(st["latest"]["duty"], K.DUTY_FAILSAFE)

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

    def test_history_never_jumps_backward_on_external_scenario_restart(self):
        eng = dashboard.DashboardEngine(broker_host="127.0.0.1", broker_port=self.broker_port, enable_mqtt=True)
        try:
            pub = mqtt_io.make_client("test-multi-scenario", publish_status=True)
            mqtt_io.connect(pub, "127.0.0.1", self.broker_port)
            time.sleep(0.3)

            fw = Firmware()
            # Skenario 1 mencapai seq=10, 11, 12
            for seq in (10, 11, 12):
                mqtt_io.publish_telemetry(pub, fw.step(0.9, 1.2, seq=seq))
            time.sleep(0.25)

            # Skenario 2 dimulai ulang dari seq=0, 1, 2
            fw.reset()
            for seq in (0, 1, 2):
                mqtt_io.publish_telemetry(pub, fw.step(0.8, 1.1, seq=seq))
            time.sleep(0.25)

            hist = eng.get_state_snapshot()["history"]
            times = [r["t"] for r in hist]
            self.assertGreaterEqual(len(times), 2)
            # Waktu t pada history harus selalu naik monoton (tidak boleh ada lompatan mundur 12 -> 0)
            for i in range(1, len(times)):
                self.assertGreater(times[i], times[i - 1], f"History time jumped backward or duplicated: {times}")

            mqtt_io.disconnect(pub)
        finally:
            eng.shutdown()

    def test_esp32_telemetry_not_dropped_while_embedded_sitl_runs(self):
        eng = dashboard.DashboardEngine(broker_host="127.0.0.1", broker_port=self.broker_port, enable_mqtt=True)
        try:
            eng.control_sim({"action": "start", "scenario": "clean", "speed": 5})
            esp = mqtt_io.make_client("fake-esp32")
            mqtt_io.connect(esp, "127.0.0.1", self.broker_port)
            time.sleep(0.4)
            for seq in range(5):
                esp.publish(K.T_TELEM, json.dumps({
                    "seq": seq, "src": "esp32", "v_dust": 0.7, "v_mq": 1.2, "pm_raw": 20.0,
                    "pm": 20.0, "dcdt": 0.0, "gas_ppm": 420.0, "g": 0.0, "fi": 77.0, "v0": 0.6,
                    "state": "WASPADA", "mode": "AUTO", "duty": 55.0, "alarm": 1}))
                time.sleep(0.05)
            time.sleep(0.4)
            snap = eng.get_state_snapshot()
            esp_rows = snap["sources"]["esp32"]["history"]
            self.assertGreaterEqual(len(esp_rows), 5)
            self.assertTrue(any(r["fi"] == 77.0 for r in esp_rows))
            # Grafik SITL tidak boleh tercampur dengan baris ESP32
            self.assertTrue(all(r["fi"] != 77.0 for r in snap["sources"]["sitl"]["history"]))
            mqtt_io.disconnect(esp)
        finally:
            eng.shutdown()

    def test_esp32_marked_offline_when_telemetry_stops(self):
        eng = dashboard.DashboardEngine(broker_host="127.0.0.1", broker_port=self.broker_port,
                                        enable_mqtt=True, esp_stale_s=0.6)
        try:
            esp = mqtt_io.make_client("fake-esp32-2")
            mqtt_io.connect(esp, "127.0.0.1", self.broker_port)
            time.sleep(0.3)
            esp.publish(K.T_TELEM, json.dumps({"seq": 1, "src": "esp32", "pm": 10.0, "fi": 10.0,
                                               "state": "NORMAL", "mode": "AUTO", "duty": 0.0}))
            time.sleep(0.3)
            self.assertTrue(eng.get_state_snapshot()["sources"]["esp32"]["online"])
            time.sleep(1.8)  # ESP32 "dicabut": tidak ada data lagi
            self.assertFalse(eng.get_state_snapshot()["sources"]["esp32"]["online"])
            mqtt_io.disconnect(esp)
        finally:
            eng.shutdown()

    def test_mobile_dashboard_import_json_endpoint(self):
        eng = dashboard.DashboardEngine(broker_host="127.0.0.1", broker_port=K.BROKER_PORT, enable_mqtt=False)
        srv = dashboard.start_dashboard_server(eng, host="127.0.0.1", port=0)
        port = srv.server_address[1]
        base = f"http://127.0.0.1:{port}"
        try:
            cfg = json.loads(urllib.request.urlopen(f"{base}/api/mobile-config", timeout=3).read().decode("utf-8"))
            self.assertIn("connections", cfg)
            conn = cfg["connections"][0]
            self.assertEqual(conn["port"], 6669)
            dash_panels = conn["dashboards"][0]["panels"]
            topics = {p.get("topic") for p in dash_panels}
            self.assertIn("karhutla/t/fi", topics)
            self.assertIn("karhutla/t/pm", topics)
            self.assertIn("karhutla/t/duty", topics)
            self.assertIn("karhutla/status", topics)
            self.assertIn("karhutla/cmd", topics)
        finally:
            eng.shutdown()
            srv.shutdown()
            srv.server_close()

    def test_dashboard_hils_runner_drives_esp32_with_speed_and_stop(self):
        eng = dashboard.DashboardEngine(broker_host="127.0.0.1", broker_port=self.broker_port, enable_mqtt=True)
        fw = Firmware()
        esp_seen = []

        def on_msg(c, u, msg):
            if msg.topic == K.T_HILS_IN:
                parts = msg.payload.decode("utf-8").strip().split(",")
                seq = int(parts[0])
                vd, vm = float(parts[1]), float(parts[2])
                esp_seen.append(seq)
                tel = fw.step(vd, vm, seq=seq)
                tel["src"] = "esp32"
                c.publish(K.T_TELEM, json.dumps(tel))
            elif msg.topic == K.T_CMD and msg.payload.decode("utf-8").strip() == "RESET":
                fw.reset()

        def on_conn(c, u, f, rc, p=None):
            c.subscribe([(K.T_HILS_IN, 0), (K.T_CMD, 0)])

        esp = mqtt_io.make_client("fake-esp32-hils", on_connect=on_conn, on_message=on_msg)
        try:
            mqtt_io.connect(esp, "127.0.0.1", self.broker_port)
            time.sleep(0.4)

            eng.control_sim({"action": "start", "target": "hils", "scenario": "clean", "speed": 20})
            time.sleep(1.2)
            snap = eng.get_state_snapshot()
            self.assertEqual(snap["sim"]["target"], "hils")
            self.assertTrue(snap["sim"]["running"])
            self.assertGreaterEqual(len(esp_seen), 4)
            self.assertGreaterEqual(len(snap["sources"]["esp32"]["history"]), 4)

            eng.control_sim({"action": "stop"})
            self.assertFalse(eng.get_state_snapshot()["sim"]["running"])
        finally:
            mqtt_io.disconnect(esp)
            eng.shutdown()

    def test_send_cmd_returns_false_when_idle_and_offline(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            try:
                self.assertFalse(eng.send_cmd("AUTO"))
                self.assertIsNotNone(eng._last_cmd_error)
            finally:
                eng.shutdown()

    def test_ai_preempt_active_resets_when_disabled_and_mid_run_scenario_locked(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            try:
                eng.control_sim({"action": "start", "scenario": "flaming", "speed": 10, "ai_preempt": True})
                time.sleep(0.1)
                with eng.lock:
                    self.assertTrue(eng.sim_running)
                    self.assertIsNotNone(eng.sim_pre)
                    eng.sim_pre.active = True
                self.assertTrue(eng.get_sim_status()["ai_preempt_active"])

                eng.control_sim({"ai_preempt": False})
                self.assertFalse(eng.get_sim_status()["ai_preempt_active"])

                # Changing scenario/target without action="start" while running must not mutate metadata
                eng.control_sim({"scenario": "clean", "target": "hils"})
                st = eng.get_sim_status()
                self.assertEqual(st["scenario"], "flaming")
                self.assertEqual(st["target"], "sitl")
            finally:
                eng.shutdown()

    def test_hils_preserves_pm_true_and_computes_zero_parity_delta(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(
                broker_host="127.0.0.1",
                broker_port=self.broker_port,
                out_dir=tmp,
                enable_mqtt=True,
            )
            fw = Firmware()

            def on_msg(c, u, msg):
                if msg.topic == K.T_HILS_IN:
                    parts = msg.payload.decode("utf-8").strip().split(",")
                    seq = int(parts[0])
                    vd, vm = float(parts[1]), float(parts[2])
                    tel = fw.step(vd, vm, seq=seq)
                    tel["src"] = "esp32"
                    c.publish(K.T_TELEM, json.dumps(tel))
                elif msg.topic == K.T_CMD and msg.payload.decode("utf-8").strip() == "RESET":
                    fw.reset()

            def on_conn(c, u, f, rc, p=None):
                c.subscribe([(K.T_HILS_IN, 0), (K.T_CMD, 0)])

            esp = mqtt_io.make_client("fake-esp32-parity", on_connect=on_conn, on_message=on_msg)
            try:
                mqtt_io.connect(esp, "127.0.0.1", self.broker_port)
                time.sleep(0.4)
                eng.control_sim({"action": "start", "target": "hils", "scenario": "clean", "speed": 20})

                deadline = time.time() + 2.5
                snap = None
                while time.time() < deadline:
                    snap = eng.get_state_snapshot()
                    if len(snap["sources"]["esp32"]["history"]) >= 3:
                        break
                    time.sleep(0.1)

                rows = snap["sources"]["esp32"]["history"]
                self.assertGreaterEqual(len(rows), 3)
                self.assertTrue(all(r.get("pm_true") is not None for r in rows))
                self.assertTrue(all(r.get("ref_fi") is not None for r in rows))
                self.assertTrue(all(r.get("parity_delta_fi") == 0.0 for r in rows))
                self.assertTrue(all(r.get("parity_match") is True for r in rows))
                self.assertGreaterEqual(snap["sim"]["hils_parity"]["samples"], 3)
            finally:
                mqtt_io.disconnect(esp)
                eng.shutdown()

    def test_sitl_clear_resets_latest_to_idle_and_snapshot_has_mobile_flag(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            try:
                snap0 = eng.get_state_snapshot()
                self.assertIn("mobile", snap0)
                self.assertIn("ip_is_fallback", snap0["mobile"])
                self.assertIsInstance(snap0["mobile"]["ip_is_fallback"], bool)

                eng.control_sim({"action": "start", "target": "sitl", "scenario": "flaming", "speed": 20})
                time.sleep(0.35)
                eng.control_sim({"action": "stop"})
                self.assertGreater(len(eng.history), 0)

                eng.control_sim({"action": "clear"})
                snap1 = eng.get_state_snapshot()
                self.assertEqual(len(snap1["history"]), 0)
                self.assertEqual(snap1["latest"]["seq"], 0)
                self.assertEqual(snap1["latest"]["state"], "NORMAL")
                self.assertEqual(snap1["latest"]["fi"], 0.0)
            finally:
                eng.shutdown()

    def test_sim_complete_event_broadcast_on_short_duration(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            q = eng.register_sse()
            try:
                eng.control_sim({
                    "action": "start",
                    "target": "sitl",
                    "scenario": "clean",
                    "speed": 20,
                    "duration": 1.0,
                })
                got_complete = False
                deadline = time.time() + 2.0
                while time.time() < deadline and not got_complete:
                    while q:
                        raw = q.popleft().decode("utf-8", errors="ignore").strip()
                        if raw.startswith("data: "):
                            msg = json.loads(raw[6:])
                            if msg.get("type") == "sim_complete":
                                got_complete = True
                                self.assertEqual(msg["summary"]["scenario"], "clean")
                                break
                    if not got_complete:
                        time.sleep(0.05)
                self.assertTrue(got_complete, "Expected sim_complete SSE event when simulation finishes naturally")
            finally:
                eng.unregister_sse(q)
                eng.shutdown()

    def test_sim_status_exposes_command_target_flags_and_invalid_sim_action_returns_reason(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            srv = dashboard.start_dashboard_server(eng, host="127.0.0.1", port=0)
            port = srv.server_address[1]
            base = f"http://127.0.0.1:{port}"
            try:
                st0 = eng.get_sim_status()
                self.assertFalse(st0["has_sitl"])
                self.assertFalse(st0["has_broker"])
                self.assertFalse(st0["can_send_cmd"])
                self.assertIn("last_summary", st0)

                eng.control_sim({"action": "start", "target": "sitl", "scenario": "clean", "speed": 20})
                time.sleep(0.1)
                st1 = eng.get_sim_status()
                self.assertTrue(st1["has_sitl"])
                self.assertTrue(st1["can_send_cmd"])

                req = urllib.request.Request(
                    f"{base}/api/sim",
                    data=json.dumps({"action": "bogus"}).encode("utf-8"),
                    headers={"Content-Type": "application/json"},
                    method="POST",
                )
                res = json.loads(urllib.request.urlopen(req, timeout=3).read().decode("utf-8"))
                self.assertFalse(res["ok"])
                self.assertTrue(bool(res.get("reason")))
            finally:
                eng.shutdown()
                srv.shutdown()
                srv.server_close()

    def test_dashboard_html_round4_clarity_and_a11y_invariants(self):
        html = dashboard.RENDERED_HTML
        self.assertNotIn("siaga", html.lower())
        self.assertIn('id="safety-live" class="sr-only" role="alert" aria-live="assertive"', html)
        self.assertIn('id="toast-live"', html)
        self.assertIn('id="override-target-line"', html)
        self.assertIn('id="run-summary-card"', html)
        self.assertIn('data-theme-val="system"', html)
        self.assertIn('id="mob-dock-context"', html)
        self.assertIn('id="mob-btn-stop"', html)
        self.assertIn('id="mob-dock-tray"', html)
        self.assertIn('id="btn-mob-reset"', html)
        self.assertIn("@media (max-height: 820px) and (min-width: 1280px)", html)

    def test_send_cmd_target_scope_isolates_sitl_and_esp32(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(
                broker_host="127.0.0.1",
                broker_port=self.broker_port,
                out_dir=tmp,
                enable_mqtt=True,
            )
            received_cmds = []

            def on_conn(c, u, flags, rc, props=None):
                c.subscribe(K.T_CMD)

            def on_msg(c, u, msg):
                if msg.topic == K.T_CMD:
                    received_cmds.append(msg.payload.decode("utf-8", errors="ignore").strip())

            sub = mqtt_io.make_client("test-cmd-scope-sub", on_connect=on_conn, on_message=on_msg)
            try:
                mqtt_io.connect(sub, "127.0.0.1", self.broker_port)
                time.sleep(0.3)

                # SITL not running: target="sitl" must fail cleanly without publishing to broker
                self.assertFalse(eng.send_cmd("FORCE_MAX", target="sitl"))
                self.assertIn("SITL", eng._last_cmd_error or "")
                time.sleep(0.15)
                self.assertEqual(received_cmds, [])

                # Start SITL
                eng.control_sim({"action": "start", "target": "sitl", "scenario": "clean", "speed": 20})
                time.sleep(0.2)

                # target="sitl" mutates SITL firmware only, does not publish to broker
                self.assertTrue(eng.send_cmd("FORCE_MAX", target="sitl"))
                self.assertEqual(eng.get_state_snapshot()["sources"]["sitl"]["latest"]["mode"], "FORCE_MAX")
                time.sleep(0.15)
                self.assertEqual(received_cmds, [])

                # target="esp32" publishes to broker without mutating running SITL firmware directly
                with eng.lock:
                    eng.sim_fw.apply_command("AUTO")
                    eng.latest["mode"] = "AUTO"
                self.assertTrue(eng.send_cmd("FORCE_OFF", target="esp32"))
                self.assertEqual(eng.sim_fw.mode, "AUTO")
                time.sleep(0.2)
                self.assertIn("FORCE_OFF", received_cmds)
            finally:
                mqtt_io.disconnect(sub)
                eng.shutdown()

    def test_send_cmd_reset_clears_stale_latest_fields(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            try:
                eng.control_sim({"action": "start", "target": "sitl", "scenario": "flaming", "speed": 50})
                time.sleep(0.25)
                eng.control_sim({"action": "pause"})
                with eng.lock:
                    eng.latest["pm_true"] = 222.5
                    eng.latest["forecast"] = 188.0
                    eng.latest["eta_bahaya_s"] = 14.0
                    eng.latest["mae"] = 3.5
                    eng.latest["gas_ppm"] = 980.0
                    eng._run_t_bahaya = 42.0
                    eng._run_preempt_used = True
                    eng._last_run_summary = {"scenario": "flaming"}

                self.assertTrue(eng.send_cmd("RESET", target="sitl"))
                lat = eng.get_state_snapshot()["sources"]["sitl"]["latest"]
                self.assertEqual(lat["t"], 0.0)
                self.assertEqual(lat["seq"], 0)
                self.assertEqual(lat["state"], "NORMAL")
                self.assertEqual(lat["mode"], "AUTO")
                self.assertEqual(lat["duty"], 0.0)
                self.assertEqual(lat["pm"], 0.0)
                self.assertEqual(lat["fi"], 0.0)
                self.assertEqual(lat["gas_ppm"], K.MQ_ATM_PPM)
                self.assertIsNone(lat["pm_true"])
                self.assertEqual(lat["forecast"], 0.0)
                self.assertIsNone(lat["eta_bahaya_s"])
                self.assertIsNone(lat["mae"])
                self.assertEqual(lat["scenario"], "flaming")
                self.assertIsNone(eng.get_sim_status()["last_summary"])
            finally:
                eng.shutdown()

    def test_export_csv_and_export_log_endpoints(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            srv = dashboard.start_dashboard_server(eng, host="127.0.0.1", port=0)
            port = srv.server_address[1]
            base = f"http://127.0.0.1:{port}"
            try:
                eng.control_sim({"action": "start", "target": "sitl", "scenario": "clean", "speed": 50})
                time.sleep(0.25)
                eng.control_sim({"action": "stop"})
                eng._record_mqtt_log("2026-11-20T12:00:00.000 [test] PUBLISH karhutla/cmd AUTO")

                with urllib.request.urlopen(f"{base}/api/export-csv?source=sitl", timeout=3) as resp:
                    self.assertEqual(resp.status, 200)
                    self.assertIn("text/csv", resp.headers.get("Content-Type", ""))
                    self.assertIn("attachment;", resp.headers.get("Content-Disposition", ""))
                    csv_body = resp.read().decode("utf-8")
                lines = [ln for ln in csv_body.strip().splitlines() if ln.strip()]
                self.assertGreaterEqual(len(lines), 2)
                self.assertIn("t_s,seq,source,scenario,pm_true", lines[0])
                self.assertIn(",sitl,clean,", lines[1])

                with urllib.request.urlopen(f"{base}/api/export-log", timeout=3) as lresp:
                    self.assertEqual(lresp.status, 200)
                    self.assertIn("text/plain", lresp.headers.get("Content-Type", ""))
                    self.assertIn("attachment;", lresp.headers.get("Content-Disposition", ""))
                    log_body = lresp.read().decode("utf-8")
                self.assertIn("PUBLISH karhutla/cmd AUTO", log_body)
            finally:
                eng.shutdown()
                srv.shutdown()
                srv.server_close()

    def test_dashboard_html_round5_ergonomics_and_a11y_invariants(self):
        html = dashboard.RENDERED_HTML
        self.assertIn('id="sim-advanced-drawer"', html)
        self.assertIn('id="sim-adv-summary"', html)
        self.assertIn('id="kpi-t-dur"', html)
        self.assertIn('id="mob-btn-more"', html)
        self.assertIn('id="mob-dock-tray"', html)
        self.assertNotIn('id="mobile-reset-row"', html)
        self.assertIn('id="sw-shortcuts"', html)
        self.assertIn('class="kpi-abbr-btn"', html)
        self.assertNotIn('<span class="kpi-abbr"', html)
        self.assertIn('id="subchart-sr-summary"', html)
        self.assertIn('id="sel-cmd-target"', html)
        self.assertIn('id="btn-clear-hover"', html)
        self.assertIn('id="btn-export-csv"', html)
        self.assertIn('id="rs-export-csv"', html)
        self.assertIn('id="btn-export-log"', html)
        self.assertIn("Ambang 55/150 di luar skala ketat", html)
        self.assertIn("Pita State FSM", html)
        self.assertIn("#475569", html)
        self.assertIn("#854d0e", html)

    def test_dashboard_round6_routing_failsafe_mobile_and_harden_invariants(self):
        html = dashboard.RENDERED_HTML
        self.assertNotIn("siaga", html.lower())
        self.assertIn('href="#control-panel" class="skip-link"', html)
        self.assertIn('id="control-panel"', html)
        self.assertIn("Proyektor", html)

        # Sumber grafik demoted into #layers-drawer
        drawer_idx = html.index('id="layers-drawer"')
        legend_idx = html.index('id="chart-legend"')
        src_btn_idx = html.index('class="chip-btn src-btn"')
        self.assertGreater(src_btn_idx, drawer_idx)
        self.assertLess(src_btn_idx, legend_idx)
        self.assertIn("Ikuti Target Eksekusi", html)

        # Unified command target options & mobile visibility
        self.assertIn("Semua jalur aktif", html)
        self.assertIn("Hanya SITL (Python)", html)
        self.assertIn("Hanya ESP32 (:6669)", html)
        self.assertIn('id="mob-dock-cmd-status"', html)
        self.assertIn('id="mob-sel-cmd-target"', html)
        self.assertIn('aria-describedby="mob-dock-cmd-status"', html)

        # Dedicated FAILSAFE identity & projector scale & summary placement below sub-chart
        self.assertIn("--fault-light", html)
        self.assertIn("--fault-dark", html)
        self.assertIn('data-tone="fault"', html)
        self.assertIn("FI tidak valid", html)
        self.assertIn("faultHatch", html)
        self.assertIn("chartScale", html)
        self.assertGreater(html.index('id="run-summary-card"'), html.index('id="sub-chart"'))

        # Toolbar focusgroup parity
        for tid in ("preset-toolbar", "cmd-toolbar", "target-toolbar", "speed-toolbar"):
            pos = html.index(f'id="{tid}"')
            snippet = html[max(0, pos - 120):pos + 80]
            self.assertIn('focusgroup="toolbar wrap"', snippet)

        # Backend /api/cmd paths & sanitized MQTT error handling
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            srv = dashboard.start_dashboard_server(eng, host="127.0.0.1", port=0)
            port = srv.server_address[1]
            base = f"http://127.0.0.1:{port}"
            try:
                eng.control_sim({"action": "start", "target": "sitl", "scenario": "clean", "speed": 20})
                time.sleep(0.15)
                req = urllib.request.Request(
                    f"{base}/api/cmd",
                    data=json.dumps({"cmd": "FORCE_MAX", "target": "sitl"}).encode("utf-8"),
                    headers={"Content-Type": "application/json"},
                    method="POST",
                )
                res = json.loads(urllib.request.urlopen(req, timeout=3).read().decode("utf-8"))
                self.assertTrue(res["ok"])
                self.assertIsNone(res["reason"])
                self.assertEqual(res["paths"], {"sitl": True, "esp32": False})

                class BrokenClient:
                    def publish(self, *args, **kwargs):
                        raise RuntimeError("secret-internal-trace")

                with eng.lock:
                    eng.client = BrokenClient()
                    eng.broker_connected = True
                ok_fail = eng.send_cmd("AUTO", target="esp32")
                self.assertFalse(ok_fail)
                err_msg = eng._last_cmd_error or ""
                self.assertNotIn("secret-internal-trace", err_msg)
                self.assertIn("Broker MQTT", err_msg)
            finally:
                with eng.lock:
                    eng.client = None
                    eng.broker_connected = False
                eng.shutdown()
                srv.shutdown()
                srv.server_close()

    def test_dashboard_round7_visual_failsafe_and_projector_invariants(self):
        html = dashboard.RENDERED_HTML
        self.assertNotIn("siaga", html.lower())
        self.assertIn('id="sim-config-zone"', html)
        self.assertIn('id="card-pm"', html)
        self.assertIn('id="card-fc"', html)
        self.assertIn('id="kpi-pm-frozen"', html)
        self.assertIn('id="kpi-fc-frozen"', html)
        self.assertIn("DITANGGUHKAN", html)
        self.assertIn("FAILSAFE (Vd=0V)", html)
        self.assertIn("@container (max-width: 440px)", html)
        self.assertIn('.cmd-btn[aria-pressed="true"]:not(:disabled)', html)
        self.assertNotIn("Math.min(padL + 10, 24)", html)

        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            try:
                eng.control_sim({"action": "start", "target": "sitl", "scenario": "flaming", "speed": 50, "fault_inject": True})
                time.sleep(0.35)
                snap = eng.get_state_snapshot()
                fault_rows = [r for r in snap["history"] if r.get("state") == "FAILSAFE"]
                self.assertGreaterEqual(len(fault_rows), 1)
                self.assertEqual(fault_rows[-1].get("slope"), 0.0)
                self.assertIsNone(fault_rows[-1].get("eta_bahaya_s"))
            finally:
                eng.shutdown()

    def test_dashboard_round8_hils_failsafe_parity_and_polish_invariants(self):
        html = dashboard.RENDERED_HTML
        self.assertIn("PRAKIRAAN DITANGGUHKAN", html)
        self.assertIn("Ekstraksi Penuh 100%", html)
        self.assertIn("-- (beku)", html)
        self.assertIn("FI tidak valid (sensor debu putus)", html)
        self.assertIn("#badge-fsm { display: none; }", html)

        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            try:
                eng._ingest_external({"seq": 0, "pm": 65.0, "fi": 65.0, "state": "WASPADA"}, "esp32")
                eng._ingest_external({"seq": 1, "pm": 92.0, "fi": 92.0, "state": "WASPADA"}, "esp32")
                eng._ingest_external({"seq": 2, "pm": 92.0, "fi": 92.0, "state": "FAILSAFE", "v_dust": 0.0, "duty": 70.0}, "esp32")
                snap = eng.get_state_snapshot()
                esp_hist = snap["sources"]["esp32"]["history"]
                self.assertEqual(len(esp_hist), 3)
                last = esp_hist[-1]
                self.assertEqual(last["state"], "FAILSAFE")
                self.assertEqual(last["slope"], 0.0)
                self.assertIsNone(last.get("eta_bahaya_s"))
                self.assertEqual(last["forecast"], esp_hist[1]["forecast"])
            finally:
                eng.shutdown()

    def test_dashboard_round9_mobile_hils_canvas_and_clarity_invariants(self):
        html = dashboard.RENDERED_HTML
        self.assertIn("display: contents;", html)
        self.assertIn("di bilah bawah", html)
        self.assertIn("Injeksi Fault", html)
        self.assertIn("maks ΔFI", html)
        self.assertIn("Kurva Telemetri", html)
        self.assertIn("Metrik DSP &amp; Prakiraan AI", html)
        self.assertIn("const lbl = segW >= 124 ? 'FAILSAFE (Vd=0V)' : 'FAILSAFE';", html)
        self.assertNotIn("segW >= 64 ? 'FAILSAFE' : 'FAIL'", html)

        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            try:
                st0 = eng.get_sim_status()
                self.assertIn("enable_mqtt", st0)
                self.assertFalse(st0["enable_mqtt"])
                with eng.lock:
                    eng.hils_consec_timeouts = 3
                eng.stop_sim()
                self.assertEqual(eng.get_sim_status()["hils_consec_timeouts"], 0)
            finally:
                eng.shutdown()

    def test_dashboard_round10_forecast_a11y_mobile_and_polish_invariants(self):
        html = dashboard.RENDERED_HTML
        self.assertIn("Tercapai (FI ≥ 150)", html)
        self.assertIn("Prakiraan PM AI", html)
        self.assertIn('id="btn-theme-cycle"', html)
        self.assertIn('id="btn-quick-src"', html)
        self.assertIn('id="mob-sel-sim-target"', html)
        self.assertIn('id="mob-sel-scenario"', html)
        self.assertIn('id="mob-sw-fault"', html)
        self.assertIn('.cmd-btn[data-variant="danger"]:not(:disabled)', html)
        self.assertIn('.topbar kbd.key-hint', html)
        self.assertIn('role="switch"', html)
        self.assertIn('function formatClock(', html)
        self.assertIn('earlyFaultThr', html)

    def test_dashboard_round11_mobile_failsafe_ia_popover_and_minor_invariants(self):
        html = dashboard.RENDERED_HTML
        self.assertIn('<link rel="icon" type="image/svg+xml"', html)
        self.assertIn("#cbb2e8", html)
        self.assertLess(html.index('id="sim-panel"'), html.index('id="override-panel"'))
        self.assertLess(html.index('id="speed-toolbar"'), html.index('id="sim-advanced-drawer"'))
        self.assertLess(html.index('id="sw-preempt"'), html.index('id="sim-advanced-drawer"'))
        self.assertIn('id="override-scope-row"', html)
        self.assertIn('id="mob-cmd-scope-row"', html)
        self.assertIn('id="mob-speed-toolbar"', html)
        self.assertIn('id="mob-sw-preempt"', html)
        self.assertIn('id="lbl-theme-cycle"', html)
        self.assertIn('id="abbr-popover" popover="auto"', html)
        self.assertIn('<details class="help-sec" name="help-sec"', html)
        self.assertIn("suspendedFill", html)
        self.assertIn("FI:TIDAK VALID", html)
        self.assertIn("Fc:DITANGGUHKAN", html)
        self.assertIn("log-empty-note", html)
        self.assertIn("const expFontPx = 12;", html)

        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            srv = dashboard.start_dashboard_server(eng, host="127.0.0.1", port=0)
            port = srv.server_address[1]
            base = f"http://127.0.0.1:{port}"
            try:
                with urllib.request.urlopen(f"{base}/favicon.ico", timeout=3) as fav:
                    self.assertEqual(fav.status, 204)

                eng.control_sim({"action": "start", "target": "sitl", "scenario": "flaming", "speed": 50})
                time.sleep(0.25)
                eng.control_sim({"action": "stop"})
                ls = eng.get_sim_status()["last_summary"]
                self.assertIsNotNone(ls)
                self.assertTrue(ls.get("stopped_early"))
                self.assertGreater(ls.get("samples", 0), 0)
            finally:
                eng.shutdown()
                srv.shutdown()
                srv.server_close()


import base64
import csv as _csv
import datetime as _dt
import glob
import struct
import zlib
import urllib.error

_REC = struct.Struct("<4sBBBBIII10fI")


def _mk_rec(epoch_s, uptime_ms, seq, rtc=True, pm=10.0):
    flags = 1 if rtc else 0
    head = struct.pack("<4sBBBBIII10f", b"KLOG", 1, 1, 0, flags, epoch_s, uptime_ms, seq,
                       0.1, 0.2, pm, pm, 0.0, 400.0, 0.0, 0.0, 0.6, 50.0)
    return head + struct.pack("<I", zlib.crc32(head) & 0xFFFFFFFF)


class FakeEsp:
    """Meniru firmware: menjawab karhutla/log/req lewat engine.ingest_mqtt."""

    def __init__(self, records):
        self.records = list(records)
        self.eng = None
        self.published = []
        self.corrupt = False
        self.clear_reject_reason = None

    def raw(self):
        return b"".join(self.records)

    def hb(self, rtc_ok=True, epoch_s=None):
        epoch_s = int(time.time()) if epoch_s is None else epoch_s
        self.eng.ingest_mqtt("karhutla/hb", json.dumps({
            "mode": "RIIL", "uptime_ms": 1000, "rtc_ok": rtc_ok, "epoch_s": epoch_s,
            "log_count": len(self.records), "log_cap": 43008, "log_full": False,
            "log_bytes": len(self.records) * 64, "log_cap_bytes": 2752512}))

    def publish(self, topic, payload, *a, **k):
        self.published.append((topic, payload))
        if topic != "karhutla/log/req":
            return
        e = self.eng
        if payload == "META":
            e.ingest_mqtt("karhutla/log/meta", json.dumps({
                "count": len(self.records), "cap": 43008, "full": False,
                "used_bytes": len(self.records) * 64, "cap_bytes": 2752512,
                "crc32": zlib.crc32(self.raw()) & 0xFFFFFFFF, "rtc_ok": True, "epoch_s": 0}))
        elif payload.startswith("GET:"):
            _, st, cnt = payload.split(":")
            st, cnt = int(st), int(cnt)
            data = bytearray(b"".join(self.records[st:st + cnt]))
            if self.corrupt and data:
                data[5] ^= 0xFF
            e.ingest_mqtt("karhutla/log/chunk", json.dumps({
                "start": st, "count": len(self.records[st:st + cnt]), "total": len(self.records),
                "data": base64.b64encode(bytes(data)).decode()}))
        elif payload.startswith("CLEAR:"):
            if self.clear_reject_reason is not None:
                e.ingest_mqtt("karhutla/log/ack", json.dumps({
                    "action": "CLEAR", "status": "done", "ok": False, "reason": self.clear_reject_reason
                }))
                return
            e.ingest_mqtt("karhutla/log/ack", json.dumps({"action": "CLEAR", "status": "started", "sectors": 4}))
            self.records = []
            e.ingest_mqtt("karhutla/log/ack", json.dumps({"action": "CLEAR", "status": "done", "ok": True}))


class TestEspBackupSyncAndClear(unittest.TestCase):
    def _setup(self, tmp, records):
        eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
        esp = FakeEsp(records)
        esp.eng = eng
        eng.client = esp
        srv = dashboard.start_dashboard_server(eng, host="127.0.0.1", port=0)
        base = f"http://127.0.0.1:{srv.server_address[1]}"
        esp.hb()
        return eng, esp, srv, base

    def _req(self, base, path, method="GET"):
        rq = urllib.request.Request(base + path, method=method, data=b"{}" if method == "POST" else None)
        try:
            with urllib.request.urlopen(rq, timeout=10) as r:
                return r.status, r.read()
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    def _stop(self, eng, srv):
        eng.shutdown()
        srv.shutdown()
        srv.server_close()

    def _recs(self, n):
        t = 1764000000
        return [_mk_rec(t + i, 1000 * i, i + 1) for i in range(n)]

    def test_clear_blocked_with_409_until_synced_and_downloaded(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng, esp, srv, base = self._setup(tmp, self._recs(5))
            try:
                self.assertEqual(self._req(base, "/api/esp-log/clear", "POST")[0], 409)
                st, body = self._req(base, "/api/esp-log/sync", "POST")
                self.assertEqual(st, 200, body)
                self.assertEqual(self._req(base, "/api/esp-log/clear", "POST")[0], 409)
                self.assertFalse([p for p in esp.published if str(p[1]).startswith("CLEAR")])
                st, body = self._req(base, "/api/esp-log/download")
                self.assertEqual(st, 200)
                crc = zlib.crc32(esp.raw()) & 0xFFFFFFFF
                st, body = self._req(base, "/api/esp-log/clear", "POST")
                self.assertEqual(st, 200, body)
                self.assertIn(("karhutla/log/req", f"CLEAR:5:{crc}"), esp.published)
            finally:
                self._stop(eng, srv)

    def test_clear_blocked_if_new_offline_record_appended_after_sync(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng, esp, srv, base = self._setup(tmp, self._recs(5))
            try:
                self.assertEqual(self._req(base, "/api/esp-log/sync", "POST")[0], 200)
                self.assertEqual(self._req(base, "/api/esp-log/download")[0], 200)
                esp.records.append(_mk_rec(1764000100, 99000, 6))
                esp.hb()
                self.assertEqual(self._req(base, "/api/esp-log/clear", "POST")[0], 409)
                self.assertFalse([p for p in esp.published if str(p[1]).startswith("CLEAR")])
            finally:
                self._stop(eng, srv)

    def test_sync_rejects_corrupt_crc_or_torn_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng, esp, srv, base = self._setup(tmp, self._recs(3))
            try:
                esp.corrupt = True
                st, body = self._req(base, "/api/esp-log/sync", "POST")
                self.assertNotEqual(st, 200)
                status = json.loads(self._req(base, "/api/esp-log/status")[1])
                self.assertFalse(status["synced"])
                self.assertEqual(glob.glob(os.path.join(tmp, "karhutla_esp32_*")), [])
            finally:
                self._stop(eng, srv)

    def test_sync_multiday_and_unsynced_clock_dedup(self):
        wib = _dt.timezone(_dt.timedelta(hours=7))
        t1 = int(_dt.datetime(2026, 11, 25, 23, 59, 30, tzinfo=wib).timestamp())
        t2 = int(_dt.datetime(2026, 11, 26, 0, 0, 30, tzinfo=wib).timestamp())
        recs = [_mk_rec(t1, 1000, 1), _mk_rec(t2, 61000, 2),
                _mk_rec(0, 5000, 1, rtc=False), _mk_rec(0, 6000, 1, rtc=False)]
        with tempfile.TemporaryDirectory() as tmp:
            eng, esp, srv, base = self._setup(tmp, recs)
            try:
                self.assertEqual(self._req(base, "/api/esp-log/sync", "POST")[0], 200)
                self.assertEqual(self._req(base, "/api/esp-log/sync", "POST")[0], 200)
                self.assertFalse(glob.glob(os.path.join(tmp, "*1970-01-01*")))
                today_str = _dt.datetime.now(dashboard.DashboardEngine.ESP_WIB).date().isoformat()
                for d in ("2026-11-25", "2026-11-26"):
                    p = os.path.join(tmp, f"karhutla_esp32_backup_{d}.csv")
                    self.assertTrue(os.path.exists(p), p)
                    with open(p, newline="", encoding="utf-8") as f:
                        expected_n = 3 if d == today_str else 1
                        self.assertEqual(len(list(_csv.DictReader(f))), expected_n)
                self.assertFalse(glob.glob(os.path.join(tmp, "*unsynced*")))
                today = os.path.join(tmp, f"karhutla_esp32_backup_{today_str}.csv")
                with open(today, newline="", encoding="utf-8") as f:
                    rows = list(_csv.DictReader(f))
                unsynced_ts = [r["timestamp"] for r in rows if r["timestamp"].startswith("UNSYNCED+")]
                self.assertEqual(unsynced_ts, ["UNSYNCED+5000ms", "UNSYNCED+6000ms"])
                st, body = self._req(base, "/api/esp-log/download")
                self.assertEqual(st, 200)
                rows = list(_csv.DictReader(io.StringIO(body.decode("utf-8"))))
                self.assertEqual(len(rows), 4)
            finally:
                self._stop(eng, srv)

    def test_unsynced_and_implausible_epoch_route_to_today_csv(self):
        recs = [_mk_rec(0, 5000, 1, rtc=False), _mk_rec(100, 6000, 2, rtc=True)]
        with tempfile.TemporaryDirectory() as tmp:
            eng, esp, srv, base = self._setup(tmp, recs)
            try:
                self.assertEqual(self._req(base, "/api/esp-log/sync", "POST")[0], 200)
                self.assertFalse(glob.glob(os.path.join(tmp, "*1970-01-01*")))
                self.assertFalse(glob.glob(os.path.join(tmp, "*unsynced*")))
                today_wib = _dt.datetime.now(dashboard.DashboardEngine.ESP_WIB).date().isoformat()
                today = os.path.join(tmp, f"karhutla_esp32_backup_{today_wib}.csv")
                with open(today, newline="", encoding="utf-8") as f:
                    rows = list(_csv.DictReader(f))
                self.assertEqual([r["timestamp"] for r in rows], ["UNSYNCED+5000ms", "UNSYNCED+6000ms"])
            finally:
                self._stop(eng, srv)

    def test_auto_time_sync_published_on_drift_or_unset_rtc(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng, esp, srv, base = self._setup(tmp, [])
            try:
                esp.published.clear()
                esp.hb(rtc_ok=False, epoch_s=0)
                syncs = [p for p in esp.published if p[0] == K.T_TIME_SYNC]
                self.assertEqual(len(syncs), 1)
                self.assertAlmostEqual(int(syncs[0][1]), time.time(), delta=5)
                esp.published.clear()
                esp.hb(rtc_ok=True, epoch_s=int(time.time()) + 60)
                self.assertEqual(len([p for p in esp.published if p[0] == K.T_TIME_SYNC]), 1)
                esp.published.clear()
                esp.hb(rtc_ok=True)
                self.assertEqual([p for p in esp.published if p[0] == K.T_TIME_SYNC], [])
            finally:
                self._stop(eng, srv)

    def test_esp_log_ui_card_and_mobile_config(self):
        html = dashboard.RENDERED_HTML
        self.assertIn('id="esp-log-card"', html)
        self.assertIn("Memori Backup ESP32 &amp; RTC", html)
        self.assertIn('id="esp-rtc-badge"', html)
        self.assertIn('id="esp-rtc-clock"', html)
        self.assertIn('id="esp-log-bar"', html)
        self.assertIn('id="esp-log-usage"', html)
        self.assertIn("43.008 sampel", html)
        self.assertIn('id="esp-log-full-badge"', html)
        self.assertIn("MEMORI PENUH — BUZZER AKTIF", html)
        self.assertIn('id="btn-esp-log-sync"', html)
        self.assertIn("Sinkronkan &amp; Unduh Log ESP32", html)
        self.assertIn('id="btn-esp-log-clear"', html)
        self.assertIn("Kosongkan Memori ESP32", html)
        # Clear button must be disabled by default in rendered HTML
        clear_idx = html.index('id="btn-esp-log-clear"')
        clear_tag = html[html.rfind("<button", 0, clear_idx):html.index(">", clear_idx) + 1]
        self.assertIn("disabled", clear_tag)
        # Zero external CDN <script src="http or <link href="http
        self.assertNotIn('<script src="http', html)
        self.assertNotIn('<link href="http', html)
        self.assertNotIn("<link rel=\"stylesheet\" href=\"http", html)
        # SSE and API hooks present
        self.assertIn("esp_log_status", html)
        self.assertIn("/api/esp-log/sync", html)
        self.assertIn("/api/esp-log/download", html)
        self.assertIn("/api/esp-log/clear", html)

        # Mobile IoT MQTT Panel config must include log_pct and log_full panels
        cfg = dashboard.build_mobile_dashboard_config(broker_host="192.168.1.10")
        panels = cfg["connections"][0]["dashboards"][0]["panels"]
        topics = {p.get("topic"): p for p in panels}
        self.assertIn("karhutla/t/log_pct", topics)
        self.assertIn("karhutla/t/log_full", topics)
        self.assertEqual(topics["karhutla/t/log_pct"]["type"], "progress")
        self.assertEqual(topics["karhutla/t/log_full"]["type"], "led_status")

    def test_esp_log_status_broadcast_on_online_offline_transitions(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng = dashboard.DashboardEngine(out_dir=tmp, enable_mqtt=False)
            eng.esp_stale_s = 0.25
            try:
                q = eng.register_sse()

                def drain_esp_log():
                    out = []
                    while q:
                        raw = q.popleft().decode("utf-8")
                        if raw.startswith("data: "):
                            ev = json.loads(raw[6:].strip())
                            if ev.get("type") == "esp_log_status":
                                out.append(ev)
                    return out

                eng._esp_alive("RIIL")
                ev_on = drain_esp_log()
                self.assertTrue(any(e.get("online") is True for e in ev_on), ev_on)

                # Wait for watchdog to mark ESP32 offline
                deadline = time.time() + 2.0
                ev_off = []
                while time.time() < deadline and not any(e.get("online") is False for e in ev_off):
                    time.sleep(0.1)
                    ev_off.extend(drain_esp_log())
                self.assertTrue(any(e.get("online") is False for e in ev_off), ev_off)

                # Reconnect with identical heartbeat fields -> must still broadcast online=True
                eng._esp_alive("RIIL")
                ev_re = drain_esp_log()
                self.assertTrue(any(e.get("online") is True for e in ev_re), ev_re)
            finally:
                eng.shutdown()

    def test_clear_surfaces_immediate_mismatch_done_without_504(self):
        with tempfile.TemporaryDirectory() as tmp:
            eng, esp, srv, base = self._setup(tmp, self._recs(3))
            try:
                self.assertEqual(self._req(base, "/api/esp-log/sync", "POST")[0], 200)
                self.assertEqual(self._req(base, "/api/esp-log/download")[0], 200)
                esp.clear_reject_reason = "mismatch"
                t0 = time.monotonic()
                st, body = self._req(base, "/api/esp-log/clear", "POST")
                elapsed = time.monotonic() - t0
                self.assertEqual(st, 409, body)
                self.assertLess(elapsed, 2.0, f"expected fast 409 without 5s wait, took {elapsed:.2f}s")
                data = json.loads(body)
                self.assertFalse(data["ok"])
                self.assertEqual(data["ack"]["reason"], "mismatch")
                self.assertFalse(data["status"]["synced"])
                self.assertFalse(data["status"]["downloaded"])
                self.assertFalse(data["status"]["can_clear"])
            finally:
                self._stop(eng, srv)

    def test_sync_quarantines_flash_resident_torn_slot_when_aggregate_crc_matches(self):
        with tempfile.TemporaryDirectory() as tmp:
            valid = self._recs(2)
            torn_slot = b"TORN" + bytes(range(60))  # 64 B with bad magic & CRC, stored on flash
            eng, esp, srv, base = self._setup(tmp, [valid[0], torn_slot, valid[1]])
            try:
                st, body = self._req(base, "/api/esp-log/sync", "POST")
                self.assertEqual(st, 200, body)
                data = json.loads(body)
                self.assertTrue(data["ok"])
                self.assertEqual(data["corrupt_skipped"], 1)
                self.assertEqual(data["count"], 2)
                self.assertEqual(data["status"]["corrupt_skipped"], 1)
                self.assertTrue(data["status"]["synced"])

                st_dl, csv_body = self._req(base, "/api/esp-log/download")
                self.assertEqual(st_dl, 200)
                batch_rows = list(_csv.DictReader(io.StringIO(csv_body.decode("utf-8"))))
                self.assertEqual(len(batch_rows), 2)

                arch_files = glob.glob(os.path.join(tmp, "karhutla_esp32_backup_*.csv"))
                self.assertEqual(len(arch_files), 1)
                with open(arch_files[0], newline="", encoding="utf-8") as f:
                    self.assertEqual(len(list(_csv.DictReader(f))), 2)

                # CLEAR must unlock after download even though 1 torn slot was quarantined
                st_clr, clr_body = self._req(base, "/api/esp-log/clear", "POST")
                self.assertEqual(st_clr, 200, clr_body)
            finally:
                self._stop(eng, srv)


if __name__ == "__main__":
    unittest.main()
