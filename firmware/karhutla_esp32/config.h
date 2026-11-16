#pragma once
// ======================================================================
//  Konfigurasi firmware.  Nilai algoritma HARUS sama dengan config.py
// ======================================================================

// ---- MODE: 1 = HILS (sensor dari laptop via MQTT), 0 = RIIL (sensor fisik) ----
#define MODE_HILS 1

// ---- Jaringan ----
#define WIFI_SSID   "NAMA_WIFI"
#define WIFI_PASS   "PASSWORD_WIFI"
#define MQTT_HOST   "192.168.1.10"     // IP LAN laptop (broker Mosquitto)
#define MQTT_PORT   6669
#define MQTT_USER   ""                 // kosong = tanpa autentikasi
#define MQTT_PASS   ""

#define T_TELEM         "karhutla/telemetry"
#define T_TELEM_SCALAR  "karhutla/t"
#define T_CMD           "karhutla/cmd"
#define T_HILS_IN       "karhutla/hils/sensor"
#define T_STATUS        "karhutla/status"

// ---- Pin (gunakan ADC1; ADC2 tidak bisa saat Wi-Fi aktif) ----
#define PIN_DUST_ADC  34      // Sharp Vo  -> divider 10k/20k -> GPIO34
#define PIN_MQ_ADC    35      // MQ-135 AO -> divider 10k/20k -> GPIO35
#define PIN_DUST_LED  4       // Sharp LED (aktif LOW) lewat R150 + C220uF sesuai datasheet
#define PIN_FAN       18      // gate MOSFET (pull-down 10k + seri 100R)
#define PIN_BUZZER    23
#define PIN_LED_G     25
#define PIN_LED_Y     26
#define PIN_LED_R     27

constexpr float DIV_RATIO   = 1.5f;      // (10k+20k)/20k  -> V_sensor = V_adc * 1.5
constexpr uint32_t PWM_FREQ = 25000;     // turunkan ke ~1000 jika kipas berdengung / stall
constexpr int DUST_PULSES   = 8;         // jumlah denyut per sampel (trimmed mean)

// ---- Waktu ----
constexpr float TS_S = 1.0f;
constexpr uint32_t HILS_WDT_MS = 5000;

// ---- Sharp ----
constexpr float DUST_V0 = 0.60f;
constexpr float DUST_SLOPE = 200.0f;
constexpr float V_VALID_MIN = 0.10f;
constexpr float V_VALID_MAX = 4.20f;
constexpr float V0_ADAPT_ALPHA = 0.002f;
constexpr float V0_ADAPT_MAXDEV = 0.15f;
constexpr float V0_ADAPT_PM_MAX = 15.0f;
constexpr float V0_ADAPT_RATE_MAX = 0.3f;
constexpr float V0_ADAPT_G_MAX = 0.10f;

// ---- MQ-135 ----
constexpr float MQ_VCC = 5.0f;
constexpr float MQ_RL_KOHM = 10.0f;
constexpr float MQ_RS_CLEAN_KOHM = 0.0f;   // 0 = auto-kalibrasi; riil: isi setelah preheat & udara bersih
constexpr int   MQ_CALIB_N = 30;           // riil: >= 120
constexpr float MQ_PARA = 116.6020682f;
constexpr float MQ_PARB = 2.769034857f;
constexpr float MQ_ATM_PPM = 400.0f;
constexpr float GAS_FULL_PPM = 800.0f;
constexpr float GAS_EMA_ALPHA = 0.15f;

// ---- Kalman ----
constexpr float KF_Q = 0.25f;
constexpr float KF_R = 36.0f;

// ---- Fusi & FSM ----
constexpr float W_GAS = 0.30f;
constexpr float GAS_FULL_PM = 100.0f;
constexpr float LEAD_S = 15.0f;
constexpr float T_W_IN = 55.0f,  T_W_OUT = 42.0f;
constexpr float T_B_IN = 150.0f, T_B_OUT = 125.0f;
constexpr int   N_ENTER = 3;
constexpr int   N_EXIT = 10;
constexpr float IMMEDIATE_MARGIN = 25.0f;

// ---- Aktuator ----
constexpr float DUTY_WASPADA_MIN = 40.0f;
constexpr float DUTY_WASPADA_MAX = 90.0f;
constexpr float DUTY_FAILSAFE = 70.0f;
constexpr float DUTY_SLEW = 10.0f;

// ---- Failsafe ----
constexpr int FS_INVALID_N = 5;
constexpr int FS_RECOVER_N = 5;
