import json
import os
import tempfile
import time
import unittest
import urllib.request
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
        eng = dashboard.DashboardEngine(broker_host="127.0.0.1", broker_port=K.BROKER_PORT, enable_mqtt=True)
        try:
            pub = mqtt_io.make_client("test-multi-scenario", publish_status=True)
            mqtt_io.connect(pub, "127.0.0.1", K.BROKER_PORT)
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


if __name__ == "__main__":
    unittest.main()
