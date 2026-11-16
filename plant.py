"""Plant: ruang asap (smoke chamber) + model tegangan sensor Sharp & MQ-135.

Plant menghasilkan TEGANGAN SENSOR (bukan nilai PM), sehingga firmware
(virtual maupun ESP32 asli) menjalankan seluruh rantai pemrosesannya.
"""
import math
import random
import config as K

SCENARIOS = ("clean", "smolder", "flaming", "fault")

LAMBDA_LEAK = 0.002        # [1/s] peluruhan alami
LAMBDA_FAN = 0.03          # [1/s] tambahan pada duty 100%
FAN_MIN_DUTY = 0.20        # kipas tidak berputar di bawah ini (dead-zone)
KGAS = 3.0                 # ppm-ekuivalen per ug/m3
TAU_GAS = 25.0             # [s] respons lambat MQ-135
V0_DRIFT = 2e-5            # [V/s] drift baseline Sharp
SIM_RS_CLEAN = 30.0        # [kOhm] Rs MQ-135 di udara bersih (model)
NOISE_V_DUST = 0.02
NOISE_V_MQ = 0.005
SPIKE_P = 0.02             # peluang spike impuls pada ADC Sharp


class SmokeChamber:
    def __init__(self, scenario="smolder", seed=1, dt=K.TS):
        if scenario not in SCENARIOS:
            raise ValueError(f"skenario harus salah satu dari {SCENARIOS}")
        self.scenario = scenario
        self.rng = random.Random(seed)
        self.dt = dt
        self.C = 0.0                      # PM sebenarnya, relatif terhadap udara bersih
        self.gas_ppm = K.MQ_ATM_PPM
        self.r0_true = SIM_RS_CLEAN * (K.MQ_ATM_PPM / K.MQ_PARA) ** (1.0 / K.MQ_PARB)

    def source(self, t):
        """Laju produksi asap [ug/m3/s]."""
        s = self.scenario
        if s in ("smolder", "fault"):
            if 60 <= t < 300:
                return 1.5 * (t - 60) / 240.0
            if 300 <= t < 600:
                return 1.5
        elif s == "flaming":
            if 100 <= t < 160:
                return 12.0
            if 160 <= t < 400:
                return 3.0
        return 0.0

    def dust_disconnected(self, t):
        return self.scenario == "fault" and 200 <= t < 260

    def advance(self, duty01, t):
        flow = duty01 if duty01 >= FAN_MIN_DUTY else 0.0
        lam = LAMBDA_LEAK + LAMBDA_FAN * flow
        self.C = max(0.0, self.C + (self.source(t) - lam * self.C) * self.dt)
        target = K.MQ_ATM_PPM + KGAS * self.C
        self.gas_ppm += (target - self.gas_ppm) * (1.0 - math.exp(-self.dt / TAU_GAS))

    def sensors(self, t):
        """Tegangan di pin OUTPUT sensor (sebelum divider)."""
        if self.dust_disconnected(t):
            vd = abs(self.rng.gauss(0, 0.005))
        else:
            vd = K.DUST_V0 + V0_DRIFT * t + self.C / K.DUST_SLOPE + self.rng.gauss(0, NOISE_V_DUST)
            if self.rng.random() < SPIKE_P:
                vd += self.rng.choice((-1, 1)) * self.rng.uniform(0.4, 1.0)
        rs = self.r0_true * (self.gas_ppm / K.MQ_PARA) ** (-1.0 / K.MQ_PARB)
        vm = K.MQ_VCC * K.MQ_RL_KOHM / (rs + K.MQ_RL_KOHM) + self.rng.gauss(0, NOISE_V_MQ)
        return vd, vm
