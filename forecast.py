"""AI: prakiraan tren linear (t + horizon) + pengendali preempt yang aman."""
from collections import deque
import config as K


class LinearTrendForecaster:
    """Regresi linear (least squares) atas jendela PM terfilter-Kalman.

    Juga mengevaluasi dirinya sendiri: setiap prakiraan dibandingkan dengan
    nilai aktual saat t+horizon tiba -> MAE berjalan (untuk laporan akademik).
    """

    def __init__(self, window_s=K.FORECAST_WINDOW_S, horizon_s=K.FORECAST_HORIZON_S,
                 min_pts=10, danger=K.T_B_IN):
        self.window_s, self.h = window_s, horizon_s
        self.min_pts, self.danger = min_pts, danger
        self.buf, self.pending = deque(), deque()
        self.err = deque(maxlen=300)

    def update(self, t, c):
        self.buf.append((t, c))
        while self.buf and t - self.buf[0][0] > self.window_s:
            self.buf.popleft()
        n = len(self.buf)
        slope, fc = 0.0, c
        if n >= self.min_pts:
            tm = sum(x for x, _ in self.buf) / n
            cm = sum(y for _, y in self.buf) / n
            sxx = sum((x - tm) ** 2 for x, _ in self.buf)
            sxy = sum((x - tm) * (y - cm) for x, y in self.buf)
            slope = sxy / sxx if sxx > 1e-9 else 0.0
            fc = cm + slope * ((t + self.h) - tm)
        fc = max(0.0, fc)
        if n >= self.min_pts:
            self.pending.append((t + self.h, fc))
        while self.pending and self.pending[0][0] <= t + 1e-9:
            _, f_old = self.pending.popleft()
            self.err.append(c - f_old)
        mae = sum(abs(e) for e in self.err) / len(self.err) if self.err else float("nan")
        if c >= self.danger:
            eta = 0.0
        elif slope > 1e-3:
            eta = (self.danger - c) / slope
        else:
            eta = None
        return dict(pm_forecast=fc, slope=slope, eta_bahaya_s=eta, mae=mae)


class PreemptController:
    """Mengirim FORCE_MAX lebih awal jika prakiraan >= ambang bahaya.

    Aturan keselamatan:
      * hanya bertindak bila mode saat ini AUTO (perintah manusia diprioritaskan);
      * bila mode berubah dari FORCE_MAX oleh orang lain, AI mengalah;
      * FORCE_MAX baru dilepas (kembali AUTO) setelah hold & prakiraan <= ambang off.
    """

    def __init__(self, on_th=K.PREEMPT_ON, off_th=K.PREEMPT_OFF,
                 hold_s=K.PREEMPT_HOLD_S, grace_s=5.0):
        self.on_th, self.off_th, self.hold_s, self.grace_s = on_th, off_th, hold_s, grace_s
        self.active = False
        self.t_on = 0.0

    def decide(self, t, mode_now, forecast):
        if self.active:
            if t - self.t_on > self.grace_s and mode_now != "FORCE_MAX":
                self.active = False          # manusia mengambil alih
                return None
            if t - self.t_on >= self.hold_s and forecast <= self.off_th:
                self.active = False
                return "AUTO"
            return None
        if mode_now == "AUTO" and forecast >= self.on_th:
            self.active, self.t_on = True, t
            return "FORCE_MAX"
        return None
