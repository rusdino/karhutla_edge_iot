"""Firmware virtual ESP32 (Python). Algoritma identik dengan karhutla_esp32.ino.

Rantai: validasi -> median-3 -> konversi PM (+baseline adaptif) -> Kalman 2-state
        -> skor gas (MQ-135, linearisasi log) -> fusion index -> FSM berhisteresis
        -> duty PWM (slew limit) + override HP + failsafe.
"""
import config as K

NORMAL, WASPADA, BAHAYA, FAILSAFE = range(4)
STATE_NAMES = ("NORMAL", "WASPADA", "BAHAYA", "FAILSAFE")
MODES = ("AUTO", "FORCE_MAX", "FORCE_OFF")


def clamp(x, lo, hi):
    return lo if x < lo else hi if x > hi else x


class Median3:
    def __init__(self):
        self.buf = None

    def reset(self):
        self.buf = None

    def push(self, x):
        if self.buf is None:
            self.buf = [x, x, x]
        self.buf = [self.buf[1], self.buf[2], x]
        a, b, c = self.buf
        return a + b + c - min(a, b, c) - max(a, b, c)


class Kalman2:
    """x = [konsentrasi, dC/dt]; model kecepatan konstan, pengukuran H=[1,0]."""

    def __init__(self, q, r):
        self.q, self.r = q, r
        self.x0 = self.x1 = 0.0
        self.p00 = self.p01 = self.p10 = self.p11 = 0.0
        self.init = False

    def reset(self):
        self.init = False

    def update(self, z, dt):
        q, r = self.q, self.r
        if not self.init:
            self.x0, self.x1 = z, 0.0
            self.p00, self.p01, self.p10, self.p11 = r, 0.0, 0.0, 10.0
            self.init = True
            return
        dt2 = dt * dt
        dt3 = dt2 * dt
        px0 = self.x0 + dt * self.x1
        px1 = self.x1
        P00 = self.p00 + dt * (self.p10 + self.p01) + dt2 * self.p11 + q * dt3 / 3.0
        P01 = self.p01 + dt * self.p11 + q * dt2 / 2.0
        P10 = self.p10 + dt * self.p11 + q * dt2 / 2.0
        P11 = self.p11 + q * dt
        S = P00 + r
        k0, k1 = P00 / S, P10 / S
        y = z - px0
        self.x0 = px0 + k0 * y
        self.x1 = px1 + k1 * y
        n00 = (1 - k0) * P00
        n01 = (1 - k0) * P01
        n10 = P10 - k1 * P00
        n11 = P11 - k1 * P01
        self.p00, self.p11 = n00, n11
        self.p01 = self.p10 = 0.5 * (n01 + n10)
        if self.x0 < 0:
            self.x0 = 0.0


class HysteresisFSM:
    def __init__(self):
        self.reset()

    def reset(self):
        self.state = NORMAL
        self.cnt_up = 0
        self.cnt_dn = 0

    def update(self, fi):
        s = self.state
        up_th = K.T_W_IN if s == NORMAL else K.T_B_IN
        dn_th = K.T_W_OUT if s == WASPADA else K.T_B_OUT
        up = s < BAHAYA and fi >= up_th
        dn = s > NORMAL and fi <= dn_th
        self.cnt_up = self.cnt_up + 1 if up else 0
        self.cnt_dn = self.cnt_dn + 1 if dn else 0
        new = s
        if s == NORMAL and fi >= K.T_B_IN:
            new = BAHAYA
        elif up and (self.cnt_up >= K.N_ENTER or fi >= up_th + K.IMMEDIATE_MARGIN):
            new = s + 1
        elif dn and self.cnt_dn >= K.N_EXIT:
            new = s - 1
        if new != s:
            self.state = new
            self.cnt_up = 0
            self.cnt_dn = 0
        return self.state


def _r0_from_clean(rs_clean):
    return rs_clean * (K.MQ_ATM_PPM / K.MQ_PARA) ** (1.0 / K.MQ_PARB)


class Firmware:
    def __init__(self):
        self.mode = "AUTO"
        self.seq = 0
        self.med_d, self.med_m = Median3(), Median3()
        self.kf = Kalman2(K.KF_Q, K.KF_R)
        self.fsm = HysteresisFSM()
        self.v0 = K.DUST_V0
        self.last_vd = K.DUST_V0
        self.last_vm = 1.0
        self.pm_raw = 0.0
        self.ppm = K.MQ_ATM_PPM
        self.g = 0.0
        self.fi = 0.0
        self.duty = 0.0
        self.failsafe = False
        self.bad = 0
        self.good = 0
        self.calib = []
        self.r0 = _r0_from_clean(K.MQ_RS_CLEAN_KOHM) if K.MQ_RS_CLEAN_KOHM > 0 else None

    # ---- API ----
    def current_telemetry(self, v_dust=None, v_mq=None):
        state = FAILSAFE if self.failsafe else self.fsm.state
        vd = self.last_vd if v_dust is None else v_dust
        vm = self.last_vm if v_mq is None else v_mq
        return dict(seq=self.seq, v_dust=vd, v_mq=vm, pm_raw=self.pm_raw,
                    pm=self.kf.x0, dcdt=self.kf.x1, gas_ppm=self.ppm, g=self.g,
                    fi=self.fi, v0=self.v0, state=STATE_NAMES[state],
                    mode=self.mode, duty=self.duty, alarm=state)

    def reset(self):
        self.med_d.reset()
        self.med_m.reset()
        self.kf.reset()
        self.kf.x0 = self.kf.x1 = 0.0
        self.fsm.reset()
        self.v0 = K.DUST_V0
        self.pm_raw = 0.0
        self.ppm = K.MQ_ATM_PPM
        self.g = 0.0
        self.fi = 0.0
        self.duty = 0.0
        self.failsafe = False
        self.bad = 0
        self.good = 0
        self.calib = []
        self.r0 = _r0_from_clean(K.MQ_RS_CLEAN_KOHM) if K.MQ_RS_CLEAN_KOHM > 0 else None
        self.mode = "AUTO"
        return self.current_telemetry()

    def set_mode(self, m):
        if m in MODES:
            self.mode = m

    def apply_command(self, cmd):
        c = (cmd or "").strip().upper()
        if c == "RESET":
            return self.reset()
        if c in MODES:
            self.mode = c
            state = FAILSAFE if self.failsafe else self.fsm.state
            self.duty = self._target_duty(state)
            return self.current_telemetry()
        return None

    def force_failsafe(self):
        """Dipakai bridge HILS untuk meniru watchdog timeout di ESP32."""
        self.bad, self.good = K.FS_INVALID_N, 0
        self._enter_failsafe()

    # ---- internal ----
    def _enter_failsafe(self):
        self.failsafe = True
        self.kf.reset()
        self.med_d.reset()
        self.med_m.reset()
        self.fsm.reset()

    def _gas_update(self, vm):
        rs = K.MQ_RL_KOHM * (K.MQ_VCC - vm) / vm
        g_t = 0.0
        if self.r0 is None:
            self.ppm = K.MQ_ATM_PPM
            self.calib.append(rs)
            if len(self.calib) >= K.MQ_CALIB_N:
                self.r0 = _r0_from_clean(sum(self.calib) / len(self.calib))
        else:
            self.ppm = K.MQ_PARA * (rs / self.r0) ** (-K.MQ_PARB)
            g_t = clamp((self.ppm - K.MQ_ATM_PPM) / K.GAS_FULL_PPM, 0.0, 1.0)
        self.g += K.GAS_EMA_ALPHA * (g_t - self.g)

    def _target_duty(self, state):
        if self.mode == "FORCE_MAX":
            return 100.0
        if self.mode == "FORCE_OFF":
            return 0.0
        if state == WASPADA:
            x = clamp((self.fi - K.T_W_IN) / (K.T_B_IN - K.T_W_IN), 0.0, 1.0)
            return K.DUTY_WASPADA_MIN + (K.DUTY_WASPADA_MAX - K.DUTY_WASPADA_MIN) * x
        if state == BAHAYA:
            return 100.0
        if state == FAILSAFE:
            return K.DUTY_FAILSAFE
        return 0.0

    def step(self, v_dust, v_mq, seq=None, dt=K.TS):
        self.seq = self.seq + 1 if seq is None else seq
        self.last_vd, self.last_vm = v_dust, v_mq
        dust_ok = K.V_VALID_MIN <= v_dust <= K.V_VALID_MAX
        mq_ok = K.V_VALID_MIN <= v_mq <= K.V_VALID_MAX

        if dust_ok:
            self.good += 1
            self.bad = 0
        else:
            self.bad += 1
            self.good = 0
            if self.bad >= K.FS_INVALID_N and not self.failsafe:
                self._enter_failsafe()
        if self.failsafe and self.good >= K.FS_RECOVER_N:
            self.failsafe = False

        vd = 0.0
        if dust_ok:
            vd = self.med_d.push(v_dust)
            self.pm_raw = max(0.0, (vd - self.v0) * K.DUST_SLOPE)
            self.kf.update(self.pm_raw, dt)
        pm, rate = self.kf.x0, self.kf.x1

        if mq_ok:
            self._gas_update(self.med_m.push(v_mq))

        lead = max(0.0, rate) * K.LEAD_S
        self.fi = pm + K.W_GAS * K.GAS_FULL_PM * self.g + lead

        if (dust_ok and not self.failsafe and self.fsm.state == NORMAL
                and pm < K.V0_ADAPT_PM_MAX and abs(rate) < K.V0_ADAPT_RATE_MAX
                and self.g < K.V0_ADAPT_G_MAX):
            self.v0 += K.V0_ADAPT_ALPHA * (vd - self.v0)
            self.v0 = clamp(self.v0, K.DUST_V0 - K.V0_ADAPT_MAXDEV,
                            K.DUST_V0 + K.V0_ADAPT_MAXDEV)

        if not self.failsafe and dust_ok:
            self.fsm.update(self.fi)
        state = FAILSAFE if self.failsafe else self.fsm.state

        target = self._target_duty(state)
        if self.mode != "AUTO" or state in (BAHAYA, FAILSAFE):
            self.duty = target
        else:
            self.duty += clamp(target - self.duty, -K.DUTY_SLEW, K.DUTY_SLEW)

        return dict(seq=self.seq, v_dust=v_dust, v_mq=v_mq, pm_raw=self.pm_raw,
                    pm=pm, dcdt=rate, gas_ppm=self.ppm, g=self.g, fi=self.fi, v0=self.v0,
                    state=STATE_NAMES[state], mode=self.mode, duty=self.duty,
                    alarm=state)
