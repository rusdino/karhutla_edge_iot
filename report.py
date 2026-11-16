"""Ringkasan & uji lulus/gagal per skenario (dipakai SITL dan HILS)."""
import math


def _rmse(a, b):
    n = len(a)
    return math.sqrt(sum((x - y) ** 2 for x, y in zip(a, b)) / n) if n else float("nan")


def summarize(rows, scenario, label=""):
    states = [r["state"] for r in rows]
    transitions = sum(1 for a, b in zip(states, states[1:]) if a != b)
    ok_rows = [r for r in rows if not r.get("sensor_fault") and r["state"] != "FAILSAFE"]
    truth = [r["pm_true"] for r in ok_rows]
    rmse_raw = _rmse([r["pm_raw"] for r in ok_rows], truth)
    rmse_kf = _rmse([r["pm"] for r in ok_rows], truth)
    t_true = next((r["t"] for r in rows if r["pm_true"] >= 55), None)
    t_det = next((r["t"] for r in rows if r["state"] in ("WASPADA", "BAHAYA")), None)
    maes = [r["mae"] for r in rows if r.get("mae") == r.get("mae") and r.get("mae") is not None]

    print(f"\n=== {label} skenario: {scenario} ({len(rows)} langkah) ===")
    print(f"PM puncak  : sebenarnya {max(truth, default=0):.0f}  | estimasi {max(r['pm'] for r in rows):.0f} ug/m3")
    print(f"RMSE       : raw {rmse_raw:.2f}  ->  Kalman {rmse_kf:.2f} ug/m3")
    if ok_rows and "gas_true" in ok_rows[0] and "gas_ppm" in ok_rows[0]:
        gas_ok = [r for r in ok_rows if r["t"] >= 30]
        rmse_gas = _rmse([r["gas_ppm"] for r in gas_ok], [r["gas_true"] for r in gas_ok]) if gas_ok else float("nan")
        print(f"Gas puncak : sebenarnya {max(r['gas_true'] for r in rows):.0f}  | estimasi {max(r['gas_ppm'] for r in rows):.0f} ppm"
              f"  | skor g maks {max(r['g'] for r in rows):.2f} (RMSE: {rmse_gas:.1f} ppm)")
    print(f"Transisi   : {transitions}   (indikator chattering)")
    print("Waktu state: " + ", ".join(f"{s}={states.count(s)}s" for s in ("NORMAL", "WASPADA", "BAHAYA", "FAILSAFE")))
    print(f"Duty rata2 : {sum(r['duty'] for r in rows) / len(rows):.1f}%")
    if t_true is not None and t_det is not None:
        print(f"Deteksi    : PM nyata >=55 pada t={t_true:.0f}s, alarm pada t={t_det:.0f}s (selisih {t_det - t_true:+.0f}s)")
    if maes:
        print(f"MAE forecast (akhir): {maes[-1]:.1f} ug/m3")

    checks = []
    if scenario == "clean":
        checks.append(("tidak ada alarm palsu", all(s == "NORMAL" for s in states)))
    elif scenario == "smolder":
        checks += [("WASPADA tercapai", "WASPADA" in states), ("tanpa chattering (transisi<=8)", transitions <= 8)]
    elif scenario == "flaming":
        checks += [("BAHAYA tercapai", "BAHAYA" in states), ("tanpa chattering (transisi<=8)", transitions <= 8),
                   ("kembali NORMAL di akhir", states[-1] == "NORMAL")]
    elif scenario == "fault":
        checks += [("FAILSAFE tercapai", "FAILSAFE" in states), ("FAILSAFE pulih", states[-1] != "FAILSAFE")]
    allok = True
    for name, ok in checks:
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}")
        allok &= ok
    return allok
