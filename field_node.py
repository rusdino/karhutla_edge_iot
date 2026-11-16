#!/usr/bin/env python3
"""Node lapangan (RIIL): laptop mendengarkan telemetri ESP32 asli (sensor fisik),
menjalankan AI forecast, (opsional) preempt FORCE_MAX, dan mencatat CSV harian.

  python field_node.py --host 192.168.1.10
  python field_node.py --ai-preempt        # AI boleh memicu FORCE_MAX lebih awal
"""
import argparse
import json
import threading
import time
import config as K
import mqtt_io
from forecast import LinearTrendForecaster, PreemptController
from logger import DailyCsvLogger


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--host", default=None)
    ap.add_argument("--ai-preempt", action="store_true")
    ap.add_argument("--out", default="logs")
    args = ap.parse_args()

    fc = LinearTrendForecaster()
    pre = PreemptController() if args.ai_preempt else None
    log = DailyCsvLogger(args.out, source="field")
    lock = threading.Lock()
    client = None

    def on_conn(c, u, flags, rc, props=None):
        print("terhubung ke broker, menunggu telemetri ...")
        c.subscribe(K.T_TELEM)

    def on_msg(c, u, msg):
        try:
            tel = json.loads(msg.payload.decode())
        except ValueError:
            return
        with lock:
            t = time.time()
            f = fc.update(t, tel["pm"])
            if pre:
                cmd = pre.decide(t, tel["mode"], f["pm_forecast"])
                if cmd:
                    print(f"[AI] kirim {cmd} (prakiraan {f['pm_forecast']:.0f} ug/m3)")
                    c.publish(K.T_CMD, cmd)
            mqtt_io.publish_forecast(c, f)
            row = dict(t_s=round(t, 3), scenario="field", v_dust=tel.get("v_dust"), v_mq=tel.get("v_mq"),
                       pm_raw=tel["pm_raw"], pm=tel["pm"], dcdt=tel["dcdt"], g=tel["g"],
                       fi=tel["fi"], v0=tel.get("v0"), state=tel["state"], mode=tel["mode"],
                       duty=tel["duty"], forecast=f["pm_forecast"], slope=f["slope"],
                       eta_bahaya_s=f["eta_bahaya_s"], mae=f["mae"])
            log.write(row)
            print(f"{tel['state']:8s} PM={tel['pm']:6.1f}  FI={tel['fi']:6.1f}  duty={tel['duty']:4.0f}%  "
                  f"prakiraan(t+{K.FORECAST_HORIZON_S}s)={f['pm_forecast']:6.1f}")

    client = mqtt_io.make_client("karhutla-field", on_conn, on_msg)
    mqtt_io.connect(client, args.host)
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    finally:
        client.loop_stop()
        log.close()


if __name__ == "__main__":
    main()
