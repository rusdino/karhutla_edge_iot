"""Parameter bersama SITL / HILS / RIIL.

PENTING: nilai-nilai di sini harus identik dengan
firmware/karhutla_esp32/config.h agar hasil SITL = hasil ESP32.
"""

# ---------- Waktu ----------
TS = 1.0                       # periode sampel [s]

# ---------- MQTT ----------
BROKER_HOST = "localhost"      # HILS/riil: isi IP LAN laptop, mis. "192.168.1.10"
BROKER_PORT = 6669
MQTT_USER = None
MQTT_PASS = None
T_TELEM = "karhutla/telemetry"         # ESP32 -> laptop/HP (JSON)
T_TELEM_SCALAR = "karhutla/t"          # karhutla/t/pm, /state, /duty ... (untuk dashboard HP)
T_CMD = "karhutla/cmd"                 # AUTO | FORCE_MAX | FORCE_OFF | RESET
T_HILS_IN = "karhutla/hils/sensor"     # laptop -> ESP32: "seq,v_dust,v_mq"
T_FORECAST = "karhutla/forecast"       # laptop -> HP (JSON)
T_FORECAST_SCALAR = "karhutla/f"       # karhutla/f/pm_forecast, /eta ...
T_STATUS = "karhutla/status"           # online | offline (retained)
T_HEARTBEAT = "karhutla/hb"            # ESP32 -> laptop: JSON heartbeat 1 Hz (deteksi dicabut)

# ---------- Sharp GP2Y1010AU0F ----------
DUST_V0 = 0.60                 # [V] tegangan sensor saat udara bersih (UKUR SENDIRI!)
DUST_SLOPE = 200.0             # [ug/m3 per V]  (datasheet: 0,5 V per 0,1 mg/m3)
V_VALID_MIN = 0.10             # [V] di bawah ini = kabel putus
V_VALID_MAX = 4.20             # [V] di atas ini = korslet / di luar rentang
V0_ADAPT_ALPHA = 0.002         # pelacak baseline lambat
V0_ADAPT_MAXDEV = 0.15         # [V] batas simpangan baseline dari DUST_V0
V0_ADAPT_PM_MAX = 15.0         # hanya adaptasi jika PM < ini
V0_ADAPT_RATE_MAX = 0.3        # dan |dC/dt| < ini
V0_ADAPT_G_MAX = 0.10          # dan skor gas < ini

# ---------- MQ-135 ----------
MQ_VCC = 5.0
MQ_RL_KOHM = 10.0              # cek beban pada modul Anda (sering 1k atau 10k)
MQ_RS_CLEAN_KOHM = 0.0         # 0 = auto-kalibrasi di MQ_CALIB_N sampel pertama
MQ_CALIB_N = 30                # riil: pakai >= 120 setelah preheat
MQ_PARA = 116.6020682
MQ_PARB = 2.769034857
MQ_ATM_PPM = 400.0
GAS_FULL_PPM = 800.0           # kelebihan ppm yang dianggap skor gas = 1
GAS_EMA_ALPHA = 0.15

# ---------- Kalman 2-state ----------
KF_Q = 0.25                    # PSD percepatan proses
KF_R = 36.0                    # varians ukur [ (ug/m3)^2 ]

# ---------- Fusi & State Machine ----------
W_GAS = 0.30
GAS_FULL_PM = 100.0
LEAD_S = 15.0                  # suku prediktif: + max(0,dC/dt) * LEAD_S
T_W_IN, T_W_OUT = 55.0, 42.0   # NORMAL <-> WASPADA
T_B_IN, T_B_OUT = 150.0, 125.0 # WASPADA <-> BAHAYA
N_ENTER = 3                    # debounce naik [sampel]
N_EXIT = 10                    # debounce turun [sampel]
IMMEDIATE_MARGIN = 25.0        # lewat ambang + margin ini = langsung pindah

# ---------- Aktuator ----------
DUTY_WASPADA_MIN = 40.0
DUTY_WASPADA_MAX = 90.0
DUTY_FAILSAFE = 70.0
DUTY_SLEW = 10.0               # [%/sampel] pembatas laju (hanya mode AUTO, non-BAHAYA)

# ---------- Failsafe ----------
FS_INVALID_N = 5
FS_RECOVER_N = 5
HILS_WDT_S = 5.0

# ---------- AI ----------
FORECAST_WINDOW_S = 60
FORECAST_HORIZON_S = 60
PREEMPT_ON = 150.0
PREEMPT_OFF = 125.0
PREEMPT_HOLD_S = 30.0
