/*
  Karhutla Edge-IoT — Firmware Native ESP-IDF (Tanpa Arduino)
  Board     : ESP32 Dev Module (esp32dev)
  Framework : ESP-IDF (PlatformIO: `pio run -e hils` atau `pio run -e riil`)

  Arsitektur FreeRTOS & Event-Driven ESP-IDF:
    - controlTask (Core 1, prio 5, stack 8192) : akuisisi ADC/HILS -> Median3 -> PM -> Kalman2 -> fusi -> FSM -> LEDC PWM/GPIO
    - buzzerTask  (Core 1, prio 3, stack 2048) : pola kedip buzzer non-blocking setiap 20 ms (membaca activeBuzzerState)
    - txTask      (Core 0, prio 2, stack 6144) : publikasi telemetri JSON & skalar melalui esp_mqtt_client
    - Wi-Fi & MQTT Event Handlers (Core 0)     : koneksi ulang otomatis & penerima perintah/sampel HILS non-blocking
  Algoritma identik 100% dengan core.py (SITL).
*/
#include <stdio.h>
#include <string.h>
#include <math.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "driver/gpio.h"
#include "driver/ledc.h"
#include "esp_adc/adc_oneshot.h"
#include "esp_adc/adc_cali.h"
#include "esp_adc/adc_cali_scheme.h"
#include "esp_rom_sys.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "esp_mac.h"
#include "nvs_flash.h"
#include "mqtt_client.h"
#include "esp_log.h"
#include "config.h"

static const char* TAG = "karhutla";

// ===================== Tipe & Struktur Algoritma =====================
enum : uint8_t { ST_NORMAL = 0, ST_WASPADA = 1, ST_BAHAYA = 2, ST_FAILSAFE = 3 };
enum : uint8_t { MD_AUTO = 0, MD_FORCE_MAX = 1, MD_FORCE_OFF = 2 };
enum CmdMsg : uint8_t { CMD_AUTO = 0, CMD_FORCE_MAX = 1, CMD_FORCE_OFF = 2, CMD_RESET = 3 };

static const char* STATE_NAMES[] = {"NORMAL", "WASPADA", "BAHAYA", "FAILSAFE"};
static const char* MODE_NAMES[]  = {"AUTO", "FORCE_MAX", "FORCE_OFF"};

static inline float clampf(float x, float lo, float hi) { return x < lo ? lo : (x > hi ? hi : x); }
static inline uint32_t nowMs() { return (uint32_t)(esp_timer_get_time() / 1000ULL); }

struct Median3 {
  float b[3]; bool init = false;
  void reset() { init = false; }
  float push(float x) {
    if (!init) { b[0] = b[1] = b[2] = x; init = true; }
    b[0] = b[1]; b[1] = b[2]; b[2] = x;
    float lo = fminf(b[0], fminf(b[1], b[2]));
    float hi = fmaxf(b[0], fmaxf(b[1], b[2]));
    return b[0] + b[1] + b[2] - lo - hi;
  }
};

struct Kalman2 {
  float x0 = 0, x1 = 0, p00 = 0, p01 = 0, p10 = 0, p11 = 0; bool init = false;
  void reset() { init = false; }
  void update(float z, float dt) {
    if (!init) { x0 = z; x1 = 0; p00 = KF_R; p01 = 0; p10 = 0; p11 = 10.0f; init = true; return; }
    float dt2 = dt * dt, dt3 = dt2 * dt;
    float px0 = x0 + dt * x1, px1 = x1;
    float P00 = p00 + dt * (p10 + p01) + dt2 * p11 + KF_Q * dt3 / 3.0f;
    float P01 = p01 + dt * p11 + KF_Q * dt2 / 2.0f;
    float P10 = p10 + dt * p11 + KF_Q * dt2 / 2.0f;
    float P11 = p11 + KF_Q * dt;
    float S = P00 + KF_R, k0 = P00 / S, k1 = P10 / S, y = z - px0;
    x0 = px0 + k0 * y;
    x1 = px1 + k1 * y;
    float n00 = (1 - k0) * P00, n01 = (1 - k0) * P01, n10 = P10 - k1 * P00, n11 = P11 - k1 * P01;
    p00 = n00; p11 = n11; p01 = p10 = 0.5f * (n01 + n10);
    if (x0 < 0) x0 = 0;
  }
};

struct Fsm {
  uint8_t state = ST_NORMAL; int cntUp = 0, cntDn = 0;
  void reset() { state = ST_NORMAL; cntUp = cntDn = 0; }
  void update(float fi) {
    uint8_t s = state;
    float upTh = (s == ST_NORMAL) ? T_W_IN : T_B_IN;
    float dnTh = (s == ST_WASPADA) ? T_W_OUT : T_B_OUT;
    bool up = (s < ST_BAHAYA) && fi >= upTh;
    bool dn = (s > ST_NORMAL) && fi <= dnTh;
    cntUp = up ? cntUp + 1 : 0;
    cntDn = dn ? cntDn + 1 : 0;
    uint8_t ns = s;
    if (s == ST_NORMAL && fi >= T_B_IN) ns = ST_BAHAYA;
    else if (up && (cntUp >= N_ENTER || fi >= upTh + IMMEDIATE_MARGIN)) ns = s + 1;
    else if (dn && cntDn >= N_EXIT) ns = s - 1;
    if (ns != s) { state = ns; cntUp = cntDn = 0; }
  }
};

struct Tel {
  uint32_t seq = 0;
  float v_dust = 0, v_mq = 0, pm_raw = 0, pm = 0, dcdt = 0, gas_ppm = MQ_ATM_PPM, g = 0, fi = 0, v0 = DUST_V0, duty = 0;
  uint8_t state = 0, mode = 0, alarm = 0;
  bool fresh = false;
};

struct HilsIn { uint32_t seq = 0; float vd = 0, vm = 0; };

// ===================== State Global =====================
static Median3 medD, medM;
static Kalman2 kf;
static Fsm fsm;
static float v0 = DUST_V0, pmRaw = 0, gasPpm = MQ_ATM_PPM, gScore = 0, lastFi = 0, duty = 0;
static bool failsafe = false;
static int badCnt = 0, goodCnt = 0;
static float calibSum = 0; static int calibN = 0;
static float r0 = 0; static bool r0Ok = false;
#if MODE_HILS
static uint32_t lastSampleMs = 0, lastWdtEmit = 0;
#else
static uint32_t seqCounter = 0;
#endif
static bool hilsActive = false;

static uint8_t cmdMode = MD_AUTO;
// Satu variabel state tunggal yang ditulis oleh controlTask dan dibaca oleh buzzerTask
static volatile uint8_t activeBuzzerState = ST_NORMAL;
static volatile bool mqttConnected = false;

static Tel tel;
static SemaphoreHandle_t mtx = nullptr;
static QueueHandle_t cmdQueue = nullptr;
#if MODE_HILS
static QueueHandle_t hilsQueue = nullptr;
#endif
static esp_mqtt_client_handle_t mqttClient = nullptr;

// ===================== Driver GPIO & LEDC PWM (Native ESP-IDF) =====================
static void initGpioAndPwm() {
  gpio_config_t ioConf = {};
  ioConf.intr_type = GPIO_INTR_DISABLE;
  ioConf.mode = GPIO_MODE_OUTPUT;
  ioConf.pin_bit_mask = (1ULL << PIN_FAN) | (1ULL << PIN_DUST_LED) | (1ULL << PIN_BUZZER) |
                        (1ULL << PIN_LED_G) | (1ULL << PIN_LED_Y) | (1ULL << PIN_LED_R);
  ioConf.pull_down_en = GPIO_PULLDOWN_DISABLE;
  ioConf.pull_up_en = GPIO_PULLUP_DISABLE;
  ESP_ERROR_CHECK(gpio_config(&ioConf));

  // Matikan kipas & buzzer sedini mungkin; LED IR Sharp aktif LOW (HIGH = mati)
  gpio_set_level((gpio_num_t)PIN_FAN, 0);
  gpio_set_level((gpio_num_t)PIN_DUST_LED, 1);
  gpio_set_level((gpio_num_t)PIN_BUZZER, 0);
  gpio_set_level((gpio_num_t)PIN_LED_G, 1);
  gpio_set_level((gpio_num_t)PIN_LED_Y, 0);
  gpio_set_level((gpio_num_t)PIN_LED_R, 0);

  ledc_timer_config_t timerConf = {};
  timerConf.speed_mode = LEDC_LOW_SPEED_MODE;
  timerConf.duty_resolution = LEDC_TIMER_8_BIT;
  timerConf.timer_num = LEDC_TIMER_0;
  timerConf.freq_hz = PWM_FREQ;
  timerConf.clk_cfg = LEDC_AUTO_CLK;
  ESP_ERROR_CHECK(ledc_timer_config(&timerConf));

  ledc_channel_config_t chanConf = {};
  chanConf.gpio_num = PIN_FAN;
  chanConf.speed_mode = LEDC_LOW_SPEED_MODE;
  chanConf.channel = LEDC_CHANNEL_0;
  chanConf.intr_type = LEDC_INTR_DISABLE;
  chanConf.timer_sel = LEDC_TIMER_0;
  chanConf.duty = 0;
  chanConf.hpoint = 0;
  ESP_ERROR_CHECK(ledc_channel_config(&chanConf));
}

static inline void pwmWrite(uint32_t val) {
  if (val > 255) val = 255;
  ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0, val);
  ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0);
}

// ===================== Driver ADC One-Shot (Mode RIIL) =====================
#if !MODE_HILS
static adc_oneshot_unit_handle_t adcUnit = nullptr;
static adc_cali_handle_t adcCali = nullptr;
static bool adcCaliOk = false;

static void initAdc() {
  adc_oneshot_unit_init_cfg_t initCfg = {};
  initCfg.unit_id = ADC_UNIT_1;
  initCfg.ulp_mode = ADC_ULP_MODE_DISABLE;
  ESP_ERROR_CHECK(adc_oneshot_new_unit(&initCfg, &adcUnit));

  adc_oneshot_chan_cfg_t chanCfg = {};
  chanCfg.bitwidth = ADC_BITWIDTH_12;
  chanCfg.atten = ADC_ATTEN_DB_12;
  // GPIO34 = ADC1_CHANNEL_6, GPIO35 = ADC1_CHANNEL_7 pada ESP32
  ESP_ERROR_CHECK(adc_oneshot_config_channel(adcUnit, ADC_CHANNEL_6, &chanCfg));
  ESP_ERROR_CHECK(adc_oneshot_config_channel(adcUnit, ADC_CHANNEL_7, &chanCfg));

#if ADC_CALI_SCHEME_LINE_FITTING_SUPPORTED
  adc_cali_line_fitting_config_t caliCfg = {};
  caliCfg.unit_id = ADC_UNIT_1;
  caliCfg.atten = ADC_ATTEN_DB_12;
  caliCfg.bitwidth = ADC_BITWIDTH_12;
  if (adc_cali_create_scheme_line_fitting(&caliCfg, &adcCali) == ESP_OK) {
    adcCaliOk = true;
  }
#endif
}

static float readAdcMilliVolts(adc_channel_t ch) {
  int raw = 0;
  if (adc_oneshot_read(adcUnit, ch, &raw) != ESP_OK) return 0.0f;
  int mv = 0;
  if (adcCaliOk && adc_cali_raw_to_voltage(adcCali, raw, &mv) == ESP_OK) {
    return (float)mv;
  }
  return ((float)raw * 3300.0f) / 4095.0f;
}

static float readDustVolts() {
  float s[DUST_PULSES];
  for (int i = 0; i < DUST_PULSES; i++) {
    gpio_set_level((gpio_num_t)PIN_DUST_LED, 0);  // LED IR ON (aktif LOW)
    esp_rom_delay_us(280);
    s[i] = readAdcMilliVolts(ADC_CHANNEL_6);
    esp_rom_delay_us(40);
    gpio_set_level((gpio_num_t)PIN_DUST_LED, 1);  // LED IR OFF
    esp_rom_delay_us(9680);
  }
  for (int i = 1; i < DUST_PULSES; i++) {         // insertion sort, lalu trimmed mean
    float k = s[i]; int j = i - 1;
    while (j >= 0 && s[j] > k) { s[j + 1] = s[j]; j--; }
    s[j + 1] = k;
  }
  float sum = 0;
  for (int i = 1; i < DUST_PULSES - 1; i++) sum += s[i];
  return (sum / (DUST_PULSES - 2)) / 1000.0f * DIV_RATIO;
}

static float readMqVolts() {
  float sum = 0;
  for (int i = 0; i < 16; i++) {
    sum += readAdcMilliVolts(ADC_CHANNEL_7);
    vTaskDelay(pdMS_TO_TICKS(1));                 // lepas CPU agar buzzerTask tidak tertahan
  }
  return (sum / 16.0f) / 1000.0f * DIV_RATIO;
}
#endif

// ===================== Pemrosesan Gas, Reset, & Aktuator =====================
static float r0FromClean(float rsClean) {
  return rsClean * powf(MQ_ATM_PPM / MQ_PARA, 1.0f / MQ_PARB);
}

static void gasUpdate(float vm) {
  float rs = MQ_RL_KOHM * (MQ_VCC - vm) / vm;
  float gt = 0;
  if (!r0Ok) {
    gasPpm = MQ_ATM_PPM;
    calibSum += rs; calibN++;
    if (calibN >= MQ_CALIB_N) { r0 = r0FromClean(calibSum / calibN); r0Ok = true; }
  } else {
    gasPpm = MQ_PARA * powf(rs / r0, -MQ_PARB);
    gt = clampf((gasPpm - MQ_ATM_PPM) / GAS_FULL_PPM, 0.0f, 1.0f);
  }
  gScore += GAS_EMA_ALPHA * (gt - gScore);
}

static void enterFailsafe() {
  failsafe = true;
  kf.reset(); medD.reset(); medM.reset(); fsm.reset();
}

static void resetAll() {
  medD.reset(); medM.reset(); kf.reset(); fsm.reset();
  kf.x0 = kf.x1 = 0;
  v0 = DUST_V0; pmRaw = 0; gasPpm = MQ_ATM_PPM; gScore = 0; lastFi = 0; duty = 0;
  failsafe = false; badCnt = goodCnt = 0;
  calibSum = 0; calibN = 0;
  if (MQ_RS_CLEAN_KOHM > 0) { r0 = r0FromClean(MQ_RS_CLEAN_KOHM); r0Ok = true; } else { r0Ok = false; }
  cmdMode = MD_AUTO;
  activeBuzzerState = ST_NORMAL;
  hilsActive = false;
  pwmWrite(0);
  gpio_set_level((gpio_num_t)PIN_LED_G, 1);
  gpio_set_level((gpio_num_t)PIN_LED_Y, 0);
  gpio_set_level((gpio_num_t)PIN_LED_R, 0);
  xSemaphoreTake(mtx, portMAX_DELAY);
  tel = Tel(); tel.state = ST_NORMAL; tel.fresh = true;
  xSemaphoreGive(mtx);
}

static float targetDuty(uint8_t st) {
  uint8_t m = cmdMode;
  if (m == MD_FORCE_MAX) return 100.0f;
  if (m == MD_FORCE_OFF) return 0.0f;
  if (st == ST_WASPADA) {
    float x = clampf((lastFi - T_W_IN) / (T_B_IN - T_W_IN), 0.0f, 1.0f);
    return DUTY_WASPADA_MIN + (DUTY_WASPADA_MAX - DUTY_WASPADA_MIN) * x;
  }
  if (st == ST_BAHAYA) return 100.0f;
  if (st == ST_FAILSAFE) return DUTY_FAILSAFE;
  return 0.0f;
}

static void applyOutputs(uint8_t st) {
  float target = targetDuty(st);
  if (cmdMode != MD_AUTO || st == ST_BAHAYA || st == ST_FAILSAFE) duty = target;
  else duty += clampf(target - duty, -DUTY_SLEW, DUTY_SLEW);
  pwmWrite((uint32_t)(duty * 2.55f + 0.5f));
  gpio_set_level((gpio_num_t)PIN_LED_G, st == ST_NORMAL ? 1 : 0);
  gpio_set_level((gpio_num_t)PIN_LED_Y, (st == ST_WASPADA || st == ST_FAILSAFE) ? 1 : 0);
  gpio_set_level((gpio_num_t)PIN_LED_R, (st == ST_BAHAYA || st == ST_FAILSAFE) ? 1 : 0);
}

static void updateBuzzer(uint32_t now) {
  uint8_t st = activeBuzzerState;
  bool on = false;
  if (st == ST_WASPADA)      on = (now % 2000) < 100;
  else if (st == ST_BAHAYA)  on = (now % 400) < 200;
  else if (st == ST_FAILSAFE) { uint32_t p = now % 2000; on = p < 100 || (p >= 200 && p < 300); }
  gpio_set_level((gpio_num_t)PIN_BUZZER, on ? 1 : 0);
}

static void publishState(uint32_t seq, float vd, float vm, float pm, float rate, float fi, uint8_t st) {
  xSemaphoreTake(mtx, portMAX_DELAY);
  tel.seq = seq; tel.v_dust = vd; tel.v_mq = vm; tel.pm_raw = pmRaw; tel.pm = pm; tel.dcdt = rate;
  tel.gas_ppm = gasPpm; tel.g = gScore; tel.fi = fi; tel.v0 = v0; tel.duty = duty;
  tel.state = st; tel.mode = cmdMode; tel.alarm = st; tel.fresh = true;
  xSemaphoreGive(mtx);
}

static void step(float vdIn, float vmIn, uint32_t seq) {
  bool dustOk = vdIn >= V_VALID_MIN && vdIn <= V_VALID_MAX;
  bool mqOk   = vmIn >= V_VALID_MIN && vmIn <= V_VALID_MAX;

  if (dustOk) { goodCnt++; badCnt = 0; }
  else {
    badCnt++; goodCnt = 0;
    if (badCnt >= FS_INVALID_N && !failsafe) enterFailsafe();
  }
  if (failsafe && goodCnt >= FS_RECOVER_N) failsafe = false;

  float vd = 0;
  if (dustOk) {
    vd = medD.push(vdIn);
    pmRaw = fmaxf(0.0f, (vd - v0) * DUST_SLOPE);
    kf.update(pmRaw, TS_S);
  }
  float pm = kf.x0, rate = kf.x1;

  if (mqOk) gasUpdate(medM.push(vmIn));

  float lead = fmaxf(0.0f, rate) * LEAD_S;
  float fi = pm + W_GAS * GAS_FULL_PM * gScore + lead;
  lastFi = fi;

  if (dustOk && !failsafe && fsm.state == ST_NORMAL && pm < V0_ADAPT_PM_MAX &&
      fabsf(rate) < V0_ADAPT_RATE_MAX && gScore < V0_ADAPT_G_MAX) {
    v0 += V0_ADAPT_ALPHA * (vd - v0);
    v0 = clampf(v0, DUST_V0 - V0_ADAPT_MAXDEV, DUST_V0 + V0_ADAPT_MAXDEV);
  }

  if (!failsafe && dustOk) fsm.update(fi);
  uint8_t st = failsafe ? (uint8_t)ST_FAILSAFE : fsm.state;
  activeBuzzerState = st;
  applyOutputs(st);
  publishState(seq, vdIn, vmIn, pm, rate, fi, st);
  ESP_LOGI(TAG, "[%lu] %s pm=%.1f fi=%.1f gas=%.0fppm duty=%.0f%% mode=%s",
           (unsigned long)seq, STATE_NAMES[st], pm, fi, gasPpm, duty, MODE_NAMES[cmdMode]);
}

#if MODE_HILS
static void watchdogTrip() {
  if (!failsafe) { badCnt = FS_INVALID_N; goodCnt = 0; enterFailsafe(); }
  activeBuzzerState = ST_FAILSAFE;
  applyOutputs(ST_FAILSAFE);
  publishState(tel.seq, tel.v_dust, tel.v_mq, kf.x0, kf.x1, lastFi, ST_FAILSAFE);
}
#endif

static bool drainCmds() {
  uint8_t c;
  bool didReset = false;
  while (xQueueReceive(cmdQueue, &c, 0) == pdTRUE) {
    if (c == CMD_RESET) {
      resetAll();
#if MODE_HILS
      xQueueReset(hilsQueue);
#endif
      didReset = true;
    } else if (c <= CMD_FORCE_OFF) {
      cmdMode = c;
      uint8_t st = activeBuzzerState;
      applyOutputs(st);
      xSemaphoreTake(mtx, portMAX_DELAY);
      tel.mode = cmdMode; tel.duty = duty; tel.fresh = true;
      xSemaphoreGive(mtx);
    }
  }
  return didReset;
}

// ===================== Tasks Core 1 (Kontrol & Buzzer) =====================
static void controlTask(void*) {
#if MODE_HILS
  for (;;) {
    drainCmds();
    HilsIn in;
    if (xQueueReceive(hilsQueue, &in, pdMS_TO_TICKS(50)) == pdTRUE) {
      drainCmds();
      lastSampleMs = nowMs();
      hilsActive = true;
      step(in.vd, in.vm, in.seq);
    } else {
      uint32_t now = nowMs();
      if (hilsActive && (now - lastSampleMs) > HILS_WDT_MS && (now - lastWdtEmit) >= 1000) {
        lastWdtEmit = now;
        watchdogTrip();
      }
    }
  }
#else
  const TickType_t periodTicks = pdMS_TO_TICKS((uint32_t)(TS_S * 1000.0f));
  TickType_t lastWake = xTaskGetTickCount();
  for (;;) {
    if (drainCmds()) lastWake = xTaskGetTickCount();
    float vd = readDustVolts();
    float vm = readMqVolts();
    drainCmds();
    step(vd, vm, ++seqCounter);
    vTaskDelayUntil(&lastWake, periodTicks);
  }
#endif
}

static void buzzerTask(void*) {
  for (;;) {
    updateBuzzer(nowMs());
    vTaskDelay(pdMS_TO_TICKS(20));
  }
}

// ===================== Jaringan Wi-Fi & MQTT Native ESP-IDF (Core 0) =====================
static void publishTel(const Tel& t) {
  if (!mqttConnected || mqttClient == nullptr) return;
  char js[440];
  snprintf(js, sizeof(js),
    "{\"seq\":%lu,\"v_dust\":%.3f,\"v_mq\":%.3f,\"pm_raw\":%.2f,\"pm\":%.2f,\"dcdt\":%.3f,"
    "\"gas_ppm\":%.1f,\"g\":%.3f,\"fi\":%.2f,\"v0\":%.3f,\"state\":\"%s\",\"mode\":\"%s\",\"duty\":%.1f,\"alarm\":%d}",
    (unsigned long)t.seq, t.v_dust, t.v_mq, t.pm_raw, t.pm, t.dcdt, t.gas_ppm, t.g, t.fi, t.v0,
    STATE_NAMES[t.state], MODE_NAMES[t.mode], t.duty, t.alarm);
  esp_mqtt_client_publish(mqttClient, T_TELEM, js, 0, 0, 0);

  // Batasi topik skalar HP maks 2 Hz (atau langsung saat state/mode berubah) agar Wi-Fi tidak jenuh saat HILS cepat
  static uint32_t lastScalarMs = 0;
  static uint8_t prevState = 0xFF, prevMode = 0xFF;
  uint32_t now = nowMs();
  if (t.state != prevState || t.mode != prevMode || (now - lastScalarMs) >= 500) {
    lastScalarMs = now;
    prevState = t.state;
    prevMode = t.mode;
    char v[24];
    snprintf(v, sizeof(v), "%.2f", t.pm);      esp_mqtt_client_publish(mqttClient, T_TELEM_SCALAR "/pm", v, 0, 0, 0);
    snprintf(v, sizeof(v), "%.2f", t.pm_raw);  esp_mqtt_client_publish(mqttClient, T_TELEM_SCALAR "/pm_raw", v, 0, 0, 0);
    snprintf(v, sizeof(v), "%.2f", t.fi);      esp_mqtt_client_publish(mqttClient, T_TELEM_SCALAR "/fi", v, 0, 0, 0);
    snprintf(v, sizeof(v), "%.3f", t.dcdt);    esp_mqtt_client_publish(mqttClient, T_TELEM_SCALAR "/dcdt", v, 0, 0, 0);
    snprintf(v, sizeof(v), "%.1f", t.gas_ppm); esp_mqtt_client_publish(mqttClient, T_TELEM_SCALAR "/gas_ppm", v, 0, 0, 0);
    snprintf(v, sizeof(v), "%.2f", t.g);       esp_mqtt_client_publish(mqttClient, T_TELEM_SCALAR "/g", v, 0, 0, 0);
    snprintf(v, sizeof(v), "%.0f", t.duty);    esp_mqtt_client_publish(mqttClient, T_TELEM_SCALAR "/duty", v, 0, 0, 0);
    esp_mqtt_client_publish(mqttClient, T_TELEM_SCALAR "/state", STATE_NAMES[t.state], 0, 0, 0);
    esp_mqtt_client_publish(mqttClient, T_TELEM_SCALAR "/mode", MODE_NAMES[t.mode], 0, 0, 0);
  }
}

static void txTask(void*) {
  uint32_t lastHb = 0;
  for (;;) {
    if (mqttConnected) {
      Tel copy; bool fresh = false;
      xSemaphoreTake(mtx, portMAX_DELAY);
      if (tel.fresh) { copy = tel; tel.fresh = false; fresh = true; }
      xSemaphoreGive(mtx);
      if (fresh) publishTel(copy);

      uint32_t now = nowMs();
      if (now - lastHb >= 1000) {
        lastHb = now;
        char hb[64];
        snprintf(hb, sizeof(hb), "{\"mode\":\"%s\",\"uptime_ms\":%lu}",
                 MODE_HILS ? "HILS" : "RIIL", (unsigned long)now);
        esp_mqtt_client_publish(mqttClient, T_HEARTBEAT, hb, 0, 0, 0);
      }
    }
    vTaskDelay(pdMS_TO_TICKS(10));
  }
}

static void handleMqttData(const char* topic, int topicLen, const char* data, int dataLen) {
  char tBuf[64];
  char dBuf[64];
  if (topicLen <= 0 || dataLen <= 0) return;
  int tCopy = topicLen < (int)sizeof(tBuf) - 1 ? topicLen : (int)sizeof(tBuf) - 1;
  int dCopy = dataLen < (int)sizeof(dBuf) - 1 ? dataLen : (int)sizeof(dBuf) - 1;
  memcpy(tBuf, topic, tCopy); tBuf[tCopy] = '\0';
  memcpy(dBuf, data, dCopy);  dBuf[dCopy] = '\0';

  if (!strcmp(tBuf, T_CMD)) {
    uint8_t c = 0xFF;
    if (!strcmp(dBuf, "AUTO")) c = CMD_AUTO;
    else if (!strcmp(dBuf, "FORCE_MAX")) c = CMD_FORCE_MAX;
    else if (!strcmp(dBuf, "FORCE_OFF")) c = CMD_FORCE_OFF;
    else if (!strcmp(dBuf, "RESET")) c = CMD_RESET;
    if (c != 0xFF) xQueueSend(cmdQueue, &c, 0);
  }
#if MODE_HILS
  else if (!strcmp(tBuf, T_HILS_IN)) {
    unsigned long seq; float vd, vm;
    if (sscanf(dBuf, "%lu,%f,%f", &seq, &vd, &vm) == 3) {
      HilsIn in; in.seq = (uint32_t)seq; in.vd = vd; in.vm = vm;
      if (xQueueSend(hilsQueue, &in, 0) != pdTRUE) {
        HilsIn drop;
        xQueueReceive(hilsQueue, &drop, 0);
        xQueueSend(hilsQueue, &in, 0);
      }
    }
  }
#endif
}

static void mqttEventHandler(void*, esp_event_base_t, int32_t eventId, void* eventData) {
  esp_mqtt_event_handle_t ev = (esp_mqtt_event_handle_t)eventData;
  switch ((esp_mqtt_event_id_t)eventId) {
    case MQTT_EVENT_CONNECTED:
      mqttConnected = true;
      esp_mqtt_client_publish(ev->client, T_STATUS, "online", 0, 0, 1);
      esp_mqtt_client_subscribe(ev->client, T_CMD, 0);
#if MODE_HILS
      esp_mqtt_client_subscribe(ev->client, T_HILS_IN, 0);
#endif
      ESP_LOGI(TAG, "MQTT terhubung ke %s:%d", MQTT_HOST, MQTT_PORT);
      break;
    case MQTT_EVENT_DISCONNECTED:
      mqttConnected = false;
      ESP_LOGW(TAG, "MQTT terputus");
      break;
    case MQTT_EVENT_DATA:
      handleMqttData(ev->topic, ev->topic_len, ev->data, ev->data_len);
      break;
    default:
      break;
  }
}

static void wifiEventHandler(void*, esp_event_base_t eventBase, int32_t eventId, void*) {
  if (eventBase == WIFI_EVENT && eventId == WIFI_EVENT_STA_START) {
    esp_wifi_connect();
  } else if (eventBase == WIFI_EVENT && eventId == WIFI_EVENT_STA_DISCONNECTED) {
    esp_wifi_connect();
  } else if (eventBase == IP_EVENT && eventId == IP_EVENT_STA_GOT_IP) {
    ESP_LOGI(TAG, "Wi-Fi terhubung, memulai MQTT client ...");
    if (mqttClient) esp_mqtt_client_start(mqttClient);
  }
}

static void initWifiAndMqtt() {
  esp_err_t ret = nvs_flash_init();
  if (ret == ESP_ERR_NVS_NO_FREE_PAGES || ret == ESP_ERR_NVS_NEW_VERSION_FOUND) {
    ESP_ERROR_CHECK(nvs_flash_erase());
    ret = nvs_flash_init();
  }
  ESP_ERROR_CHECK(ret);

  ESP_ERROR_CHECK(esp_netif_init());
  ESP_ERROR_CHECK(esp_event_loop_create_default());
  esp_netif_create_default_wifi_sta();

  wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
  ESP_ERROR_CHECK(esp_wifi_init(&cfg));
  ESP_ERROR_CHECK(esp_event_handler_instance_register(WIFI_EVENT, ESP_EVENT_ANY_ID, &wifiEventHandler, nullptr, nullptr));
  ESP_ERROR_CHECK(esp_event_handler_instance_register(IP_EVENT, IP_EVENT_STA_GOT_IP, &wifiEventHandler, nullptr, nullptr));

  wifi_config_t wifiCfg = {};
  strncpy((char*)wifiCfg.sta.ssid, WIFI_SSID, sizeof(wifiCfg.sta.ssid) - 1);
  strncpy((char*)wifiCfg.sta.password, WIFI_PASS, sizeof(wifiCfg.sta.password) - 1);
  ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
  ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &wifiCfg));

  uint8_t mac[6] = {0};
  esp_efuse_mac_get_default(mac);
  static char clientId[40];
  snprintf(clientId, sizeof(clientId), "karhutla-esp32-%02x%02x%02x", mac[3], mac[4], mac[5]);

  esp_mqtt_client_config_t mqttCfg = {};
  mqttCfg.broker.address.hostname = MQTT_HOST;
  mqttCfg.broker.address.port = MQTT_PORT;
  mqttCfg.broker.address.transport = MQTT_TRANSPORT_OVER_TCP;
  mqttCfg.credentials.client_id = clientId;
  if (MQTT_USER[0] != '\0') {
    mqttCfg.credentials.username = MQTT_USER;
    mqttCfg.credentials.authentication.password = MQTT_PASS;
  }
  mqttCfg.session.last_will.topic = T_STATUS;
  mqttCfg.session.last_will.msg = "offline";
  mqttCfg.session.last_will.msg_len = 7;
  mqttCfg.session.last_will.qos = 0;
  mqttCfg.session.last_will.retain = 1;
  mqttCfg.session.keepalive = 30;

  mqttClient = esp_mqtt_client_init(&mqttCfg);
  ESP_ERROR_CHECK(esp_mqtt_client_register_event(mqttClient, (esp_mqtt_event_id_t)ESP_EVENT_ANY_ID, mqttEventHandler, nullptr));
  ESP_ERROR_CHECK(esp_wifi_start());
  // Nonaktifkan Wi-Fi modem sleep agar latensi MQTT HILS tidak melonjak & burst
  ESP_ERROR_CHECK(esp_wifi_set_ps(WIFI_PS_NONE));
}

// ===================== Entry Point Native ESP-IDF =====================
extern "C" void app_main(void) {
  initGpioAndPwm();
#if !MODE_HILS
  initAdc();
#endif

  mtx = xSemaphoreCreateMutex();
  cmdQueue = xQueueCreate(8, sizeof(uint8_t));
#if MODE_HILS
  hilsQueue = xQueueCreate(4, sizeof(HilsIn));
#endif
  resetAll();

  ESP_LOGI(TAG, "Mode Firmware Native ESP-IDF: %s", MODE_HILS ? "HILS" : "RIIL");
  initWifiAndMqtt();

  xTaskCreatePinnedToCore(txTask,      "tx",   6144, nullptr, 2, nullptr, 0);
  xTaskCreatePinnedToCore(controlTask, "ctrl", 8192, nullptr, 5, nullptr, 1);
  xTaskCreatePinnedToCore(buzzerTask,  "buzz", 2048, nullptr, 3, nullptr, 1);
}
