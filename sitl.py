#!/usr/bin/env python3
"""SITL: firmware virtual (core.Firmware) + plant ruang asap, tanpa hardware.

Contoh:
  python sitl.py --scenario all                       # uji cepat 4 skenario
  python sitl.py --scenario flaming --ai-preempt --plot out.png
  python sitl.py --scenario smolder --mqtt --speed 1  # tampilkan di dashboard HP
"""
import argparse
import time
import config as K
from core import Firmware
from plant import SmokeChamber, SCENARIOS
from forecast import LinearTrendForecaster, PreemptController
from logger import DailyCsvLogger
from report import summarize


def make_plot(rows, path):
    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:
        print("matplotlib belum terpasang, plot dilewati")
        return
    t = [r["t"] for r in rows]
    fig, (a1, a2, a3) = plt.subplots(3, 1, sharex=True, figsize=(10, 8.5))

    # Panel 1: PM-proxy & Fusion Index (FI)
    a1.plot(t, [r["pm_true"] for r in rows], label="PM sebenarnya (plant)")
    a1.plot(t, [r["pm_raw"] for r in rows], alpha=.4, label="PM-proxy mentah (median)")
    a1.plot(t, [r["pm"] for r in rows], label="PM-proxy Kalman")
    a1.plot(t, [r["fi"] for r in rows], color="tab:red", lw=1.2, alpha=.85, label="Fusion Index (FI)")
    a1.plot([x + K.FORECAST_HORIZON_S for x in t], [r["forecast"] for r in rows], "--", label="prakiraan t+h")
    for th, lbl in ((K.T_W_IN, "ambang WASPADA (55)"), (K.T_B_IN, "ambang BAHAYA (150)")):
        a1.axhline(th, color="gray", ls=":", label=lbl)
    a1.set_ylabel("PM-proxy / FI [ug/m3]")
    a1.legend(loc="upper right", fontsize=8, ncol=2)
    a1.grid(alpha=.3)

    # Panel 2: Sensor Gas MQ-135 (ppm sebenarnya vs estimasi ppm + skor gas g)
    l1 = a2.plot(t, [r["gas_true"] for r in rows], color="tab:green", label="Gas sebenarnya (plant) [ppm]")
    l2 = a2.plot(t, [r["gas_ppm"] for r in rows], color="tab:olive", alpha=.75, ls="--", label="Gas estimasi MQ-135 [ppm]")
    a2.axvline(K.MQ_CALIB_N * K.TS, color="gray", ls=":", alpha=.6)
    a2.set_ylabel("Gas MQ-135 [ppm]")
    a2.grid(alpha=.3)

    a2_g = a2.twinx()
    l3 = a2_g.plot(t, [r["g"] for r in rows], color="tab:purple", lw=1.5, label="Skor gas g (EMA 0-1)")
    a2_g.set_ylabel("Skor gas g [0-1]", color="tab:purple")
    a2_g.set_ylim(-0.05, 1.05)
    lines = l1 + l2 + l3
    a2.legend(lines, [l.get_label() for l in lines], loc="upper right", fontsize=8)

    # Panel 3: Aktuator (Duty Kipas) & State FSM
    a3.plot(t, [r["duty"] for r in rows], label="duty kipas %")
    code = {"NORMAL": 0, "WASPADA": 1, "BAHAYA": 2, "FAILSAFE": 3}
    a3.step(t, [code[r["state"]] * 25 for r in rows], where="post", label="state (x25)")
    a3.set_ylabel("Duty [%] / State")
    a3.set_xlabel("t [s]")
    a3.legend(loc="upper right", fontsize=8)
    a3.grid(alpha=.3)

    fig.tight_layout()
    fig.savefig(path, dpi=120)
    print(f"plot disimpan: {path}")


def run(scenario, args):
    plant = SmokeChamber(scenario, seed=args.seed)
    fw = Firmware()
    fc = LinearTrendForecaster()
    pre = PreemptController() if args.ai_preempt else None
    log = DailyCsvLogger(args.out, source="sitl")
    client = None
    if args.mqtt:
        import mqtt_io
        def on_msg(c, u, msg):
            tel_now = fw.apply_command(msg.payload.decode(errors="ignore").strip())
            if tel_now is not None:
                mqtt_io.publish_telemetry(c, tel_now)
        def on_conn(c, u, flags, rc, props=None):
            c.subscribe(K.T_CMD)
        client = mqtt_io.make_client(f"karhutla-sitl-{scenario}", on_conn, on_msg, publish_status=True)
        mqtt_io.connect(client)

    rows = []
    for k in range(int(args.duration / K.TS)):
        t = k * K.TS
        vd, vm = plant.sensors(t)
        tel = fw.step(vd, vm, seq=k)
        f = fc.update(t, tel["pm"])
        if pre:
            cmd = pre.decide(t, fw.mode, f["pm_forecast"])
            if cmd:
                fw.set_mode(cmd)
        row = dict(t=t, t_s=t, scenario=scenario, pm_true=plant.C, gas_true=plant.gas_ppm,
                   v_dust=vd, v_mq=vm, pm_raw=tel["pm_raw"], pm=tel["pm"], dcdt=tel["dcdt"],
                   gas_ppm=tel["gas_ppm"], g=tel["g"], fi=tel["fi"], v0=tel["v0"],
                   state=tel["state"], mode=tel["mode"], duty=tel["duty"],
                   forecast=f["pm_forecast"], slope=f["slope"],
                   eta_bahaya_s=f["eta_bahaya_s"], mae=f["mae"],
                   sensor_fault=plant.dust_disconnected(t))
        rows.append(row)
        log.write(row)
        plant.advance(tel["duty"] / 100.0, t)
        if client:
            import mqtt_io
            mqtt_io.publish_telemetry(client, tel)
            mqtt_io.publish_forecast(client, f)
        if args.speed > 0:
            time.sleep(K.TS / args.speed)
    if client:
        import mqtt_io
        mqtt_io.disconnect(client)
    log.close()
    ok = summarize(rows, scenario, "SITL")
    if args.plot is not None:
        make_plot(rows, args.plot if len(args.plot) else f"sitl_{scenario}.png")
    return ok


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scenario", default="all", choices=SCENARIOS + ("all",))
    ap.add_argument("--duration", type=float, default=900, help="detik simulasi")
    ap.add_argument("--speed", type=float, default=0, help="0 = secepat mungkin, 1 = real-time")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--mqtt", action="store_true", help="publikasi ke broker + terima perintah HP")
    ap.add_argument("--dashboard", action="store_true", help="buka Modern Web Dashboard di laptop selama simulasi")
    ap.add_argument("--dashboard-port", type=int, default=8080, help="port HTTP untuk --dashboard (default: 8080)")
    ap.add_argument("--ai-preempt", action="store_true", help="AI memicu FORCE_MAX lebih awal")
    ap.add_argument("--out", default="logs")
    ap.add_argument("--plot", nargs="?", const="", default=None, help="simpan grafik PNG")
    args = ap.parse_args()
    dash_srv = None
    dash_eng = None
    if args.dashboard:
        args.mqtt = True
        import dashboard
        dash_eng = dashboard.DashboardEngine(out_dir=args.out, enable_mqtt=True)
        dash_srv = dashboard.start_dashboard_server(dash_eng, port=args.dashboard_port)
        url = f"http://localhost:{dash_srv.server_address[1]}/"
        print(f"Dashboard Web aktif di: {url}")
        if args.scenario != "all":
            dash_eng.control_sim({
                "action": "start",
                "scenario": args.scenario,
                "speed": args.speed if args.speed > 0 else 1.0,
                "ai_preempt": args.ai_preempt,
            })
            try:
                while dash_eng.sim_running or True:
                    time.sleep(1.0)
            except KeyboardInterrupt:
                print("\nMenutup simulasi & dashboard ...")
            finally:
                dash_eng.shutdown()
                dash_srv.shutdown()
                dash_srv.server_close()
            raise SystemExit(0)
    try:
        names = SCENARIOS if args.scenario == "all" else (args.scenario,)
        results = [run(s, args) for s in names]
    finally:
        if dash_eng:
            dash_eng.shutdown()
        if dash_srv:
            dash_srv.shutdown()
            dash_srv.server_close()
    print("\nHASIL AKHIR:", "SEMUA LULUS" if all(results) else "ADA YANG GAGAL")
    raise SystemExit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
