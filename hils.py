#!/usr/bin/env python3
"""HILS (lock-step): plant di laptop -> tegangan sensor via MQTT -> ESP32 ASLI
menjalankan seluruh rantai -> telemetri (duty PWM) kembali -> plant dimajukan.

Persiapan: flash firmware dengan MODE_HILS=1, nyalakan broker (lihat mosquitto.conf),
isi BROKER_HOST di config.py dengan IP LAN laptop.

Contoh:
  python hils.py --scenario flaming --speed 5 --shadow
  python hils.py --scenario smolder --dropout-at 150 --dropout-s 8   # uji watchdog
"""
import argparse
import json
import threading
import time
import config as K
import mqtt_io
from core import Firmware
from plant import SmokeChamber, SCENARIOS
from forecast import LinearTrendForecaster, PreemptController
from logger import DailyCsvLogger
from report import summarize


class Link:
    def __init__(self):
        self.cond = threading.Condition()
        self.tel = None
        self.cmds = []

    def on_message(self, c, u, msg):
        payload = msg.payload.decode(errors="ignore").strip()
        with self.cond:
            if msg.topic == K.T_TELEM:
                try:
                    self.tel = json.loads(payload)
                except ValueError:
                    return
            elif msg.topic == K.T_CMD:
                self.cmds.append(payload)
            self.cond.notify_all()

    def wait_seq(self, seq, timeout):
        with self.cond:
            ok = self.cond.wait_for(lambda: self.tel is not None and self.tel.get("seq") == seq, timeout)
            return self.tel if ok else None

    def wait_any(self, timeout):
        with self.cond:
            self.cond.wait_for(lambda: self.tel is not None, timeout)
            return self.tel

    def drain_cmds(self):
        with self.cond:
            out, self.cmds = self.cmds, []
            return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scenario", default="flaming", choices=SCENARIOS)
    ap.add_argument("--duration", type=float, default=900)
    ap.add_argument("--speed", type=float, default=5, help="faktor kecepatan (1 = real-time)")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--host", default=None)
    ap.add_argument("--shadow", action="store_true", help="bandingkan dgn firmware virtual (cek paritas)")
    ap.add_argument("--ai-preempt", action="store_true")
    ap.add_argument("--dropout-at", type=float, default=None, help="t [s] mulai menghentikan data sensor")
    ap.add_argument("--dropout-s", type=float, default=8.0, help="lama berhenti [detik jam dinding]")
    ap.add_argument("--out", default="logs")
    args = ap.parse_args()

    link = Link()
    def on_conn(c, u, flags, rc, props=None):
        c.subscribe([(K.T_TELEM, 0), (K.T_CMD, 0)])
    client = mqtt_io.make_client("karhutla-hils", on_conn, link.on_message)
    mqtt_io.connect(client, args.host)

    print("Mereset ESP32 & menunggu telemetri ...")
    time.sleep(0.5)
    client.publish(K.T_CMD, "RESET")
    time.sleep(1.0)
    link.drain_cmds()
    link.tel = None
    # kirim sampel dummy tidak perlu: ESP32 menerbitkan telemetri tiap ada data / perintah RESET
    if link.wait_any(10) is None:
        raise SystemExit("ESP32 tidak terdeteksi (cek Wi-Fi, IP broker, dan MODE_HILS=1).")
    print("ESP32 terdeteksi. Mulai simulasi.")

    plant = SmokeChamber(args.scenario, seed=args.seed)
    shadow = Firmware() if args.shadow else None
    fc = LinearTrendForecaster()
    pre = PreemptController() if args.ai_preempt else None
    log = DailyCsvLogger(args.out, source="hils")
    rows, lost, mism, dropped = [], 0, 0, False
    last_duty = 0.0

    for k in range(int(args.duration / K.TS)):
        t0 = time.time()
        t = k * K.TS
        for cmd in link.drain_cmds():
            if shadow and cmd in ("AUTO", "FORCE_MAX", "FORCE_OFF"):
                shadow.set_mode(cmd)
        vd, vm = plant.sensors(t)

        if args.dropout_at is not None and not dropped and t >= args.dropout_at:
            dropped = True
            print(f"[t={t:.0f}s] DROPOUT {args.dropout_s}s: sensor berhenti dikirim (uji watchdog)")
            time.sleep(args.dropout_s)
            st = (link.tel or {}).get("state")
            print(f"  state ESP32 saat dropout = {st}  -> {'PASS' if st == 'FAILSAFE' else 'FAIL'}")
            if shadow:
                shadow.force_failsafe()

        client.publish(K.T_HILS_IN, f"{k},{vd:.4f},{vm:.4f}")
        tel = link.wait_seq(k, timeout=3.0)
        if tel is None:
            lost += 1
            print(f"[t={t:.0f}s] telemetri hilang (total {lost})")
            tel = dict(link.tel or {}, seq=k)
            if not tel.get("state"):
                continue
        last_duty = float(tel["duty"])

        if shadow:
            sh = shadow.step(vd, vm, seq=k)
            if abs(sh["pm"] - tel["pm"]) > 1.0 or sh["state"] != tel["state"]:
                mism += 1

        f = fc.update(t, tel["pm"])
        if pre:
            cmd = pre.decide(t, tel["mode"], f["pm_forecast"])
            if cmd:
                client.publish(K.T_CMD, cmd)
                if shadow:
                    shadow.set_mode(cmd)
        mqtt_io.publish_forecast(client, f)

        row = dict(t=t, t_s=t, scenario=args.scenario, pm_true=plant.C, v_dust=vd, v_mq=vm,
                   pm_raw=tel["pm_raw"], pm=tel["pm"], dcdt=tel["dcdt"], g=tel["g"],
                   fi=tel["fi"], v0=tel["v0"], state=tel["state"], mode=tel["mode"],
                   duty=last_duty, forecast=f["pm_forecast"], slope=f["slope"],
                   eta_bahaya_s=f["eta_bahaya_s"], mae=f["mae"],
                   sensor_fault=plant.dust_disconnected(t))
        rows.append(row)
        log.write(row)
        plant.advance(last_duty / 100.0, t)

        if args.speed > 0:
            time.sleep(max(0.0, K.TS / args.speed - (time.time() - t0)))

    log.close()
    client.loop_stop()
    ok = summarize(rows, args.scenario, "HILS")
    print(f"Telemetri hilang: {lost}")
    if shadow:
        print(f"Paritas shadow vs ESP32: {mism} langkah berbeda dari {len(rows)}"
              f" ({100.0 * mism / max(1, len(rows)):.1f}%)  [toleransi |PM|<1 ug/m3 & state sama]")
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
