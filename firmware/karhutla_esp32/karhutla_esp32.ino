/*
  Karhutla Edge-IoT - ESP32 (satu sketsa, dua mode: HILS / RIIL; lihat config.h)

  Library : PubSubClient (Nick O'Leary). Board: ESP32 Dev Module (core Arduino-ESP32 2.x atau 3.x)
  Arsitektur FreeRTOS:
    - controlTask (core 1, prio 2, stack 8192) : akuisisi/HILS -> median3 -> PM -> Kalman -> fusi -> FSM -> PWM/LED
    - buzzerTask  (core 1, prio 1, stack 2048) : pola kedip buzzer non-blocking (membaca activeBuzzerState)
    - netTask     (core 0, prio 1, stack 8192) : Wi-Fi + MQTT. Kontrol lokal TIDAK bergantung jaringan.
    - loop()                                   : dihapus dengan vTaskDelete(NULL)
  Algoritma identik dengan core.py (SITL).
*/
#include <Arduino.h>
#include <WiFi.h>
#include <PubSubClient.h>
#include <math.h>
#include "config.h"

// ===================== Tipe =====================
enum : uint8_t { ST_NORMAL = 0, ST_WASPADA = 1, ST_BAHAYA = 2, ST_FAILSAFE = 3 };
enum : uint8_t { MD_AUTO = 0, MD_FORCE_MAX = 1, MD_FORCE_OFF = 2 };
enum CmdMsg : uint8_t { CMD_AUTO = 0, CMD_FORCE_MAX = 1, CMD_FORCE_OFF = 2, CMD_RESET = 3 };

static const char* STATE_NAMES[] = {"NORMAL", "WASPADA", "BAHAYA", "FAILSAFE"};
static const char* MODE_NAMES[]  = {"AUTO", "FORCE_MAX", "FORCE_OFF"};

static inline float clampf(float x, float lo, float hi) { return x < lo ? lo : (x > hi ? hi : x); }

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

// ===================== Global =====================
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

static Tel tel;
static SemaphoreHandle_t mtx;
static QueueHandle_t cmdQueue;
#if MODE_HILS
static QueueHandle_t hilsQueue;
#endif

static WiFiClient net;
static PubSubClient mqtt(net);

// ===================== PWM =====================
#if defined(ESP_ARDUINO_VERSION_MAJOR) && ESP_ARDUINO_VERSION_MAJOR >= 3
  #define PWM_INIT()  ledcAttach(PIN_FAN, PWM_FREQ, 8)
  #define PWM_WRITE(v) ledcWrite(PIN_FAN, (v))
#else
  #define PWM_INIT()  do { ledcSetup(0, PWM_FREQ, 8); ledcAttachPin(PIN_FAN, 0); } while (0)
  #define PWM_WRITE(v) ledcWrite(0, (v))
#endif

// ===================== Gas =====================
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

// ===================== Reset & failsafe =====================
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
  PWM_WRITE(0);
  digitalWrite(PIN_LED_G, HIGH);
  digitalWrite(PIN_LED_Y, LOW);
  digitalWrite(PIN_LED_R, LOW);
  xSemaphoreTake(mtx, portMAX_DELAY);
  tel = Tel(); tel.state = ST_NORMAL; tel.fresh = true;
  xSemaphoreGive(mtx);
}

// ===================== Keluaran =====================
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
  PWM_WRITE((uint32_t)(duty * 2.55f + 0.5f));
  digitalWrite(PIN_LED_G, st == ST_NORMAL);
  digitalWrite(PIN_LED_Y, st == ST_WASPADA || st == ST_FAILSAFE);
  digitalWrite(PIN_LED_R, st == ST_BAHAYA || st == ST_FAILSAFE);
}

static void updateBuzzer(uint32_t now) {
  uint8_t st = activeBuzzerState;
  bool on = false;
  if (st == ST_WASPADA)      on = (now % 2000) < 100;
  else if (st == ST_BAHAYA)  on = (now % 400) < 200;
  else if (st == ST_FAILSAFE) { uint32_t p = now % 2000; on = p < 100 || (p >= 200 && p < 300); }
  digitalWrite(PIN_BUZZER, on);
}

static void publishState(uint32_t seq, float vd, float vm, float pm, float rate, float fi, uint8_t st) {
  xSemaphoreTake(mtx, portMAX_DELAY);
  tel.seq = seq; tel.v_dust = vd; tel.v_mq = vm; tel.pm_raw = pmRaw; tel.pm = pm; tel.dcdt = rate;
  tel.gas_ppm = gasPpm; tel.g = gScore; tel.fi = fi; tel.v0 = v0; tel.duty = duty;
  tel.state = st; tel.mode = cmdMode; tel.alarm = st; tel.fresh = true;
  xSemaphoreGive(mtx);
}

// ===================== Satu langkah pemrosesan =====================
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
  Serial.printf("[%lu] %s pm=%.1f fi=%.1f duty=%.0f%% mode=%s\n", (unsigned long)seq,
                STATE_NAMES[st], pm, fi, duty, MODE_NAMES[cmdMode]);
}

#if MODE_HILS
// Watchdog HILS: data sensor berhenti -> failsafe
static void watchdogTrip() {
  if (!failsafe) { badCnt = FS_INVALID_N; goodCnt = 0; enterFailsafe(); }
  activeBuzzerState = ST_FAILSAFE;
  applyOutputs(ST_FAILSAFE);
  publishState(tel.seq, tel.v_dust, tel.v_mq, kf.x0, kf.x1, lastFi, ST_FAILSAFE);
}
#endif

// Kuras perintah masuk dari antrean cmdQueue (dipanggil di controlTask)
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

// ===================== Akuisisi RIIL =====================
#if !MODE_HILS
static float readDustVolts() {
  float s[DUST_PULSES];
  for (int i = 0; i < DUST_PULSES; i++) {
    digitalWrite(PIN_DUST_LED, LOW);          // LED IR ON
    delayMicroseconds(280);
    s[i] = (float)analogReadMilliVolts(PIN_DUST_ADC);
    delayMicroseconds(40);
    digitalWrite(PIN_DUST_LED, HIGH);         // LED IR OFF
    delayMicroseconds(9680);
  }
  for (int i = 1; i < DUST_PULSES; i++) {     // insertion sort, lalu trimmed mean (buang min & max)
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
    sum += (float)analogReadMilliVolts(PIN_MQ_ADC);
    vTaskDelay(pdMS_TO_TICKS(1));             // lepas CPU agar buzzerTask tidak tertahan
  }
  return (sum / 16.0f) / 1000.0f * DIV_RATIO;
}
#endif

// ===================== Tasks Core 1 (Kontrol & Buzzer) =====================
static void controlTask(void*) {
#if MODE_HILS
  for (;;) {
    drainCmds();
    HilsIn in;
    if (xQueueReceive(hilsQueue, &in, pdMS_TO_TICKS(50)) == pdTRUE) {
      drainCmds();
      lastSampleMs = millis();
      hilsActive = true;
      step(in.vd, in.vm, in.seq);
    } else {
      uint32_t now = millis();
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
    updateBuzzer(millis());
    vTaskDelay(pdMS_TO_TICKS(20));
  }
}

// ===================== Jaringan (task core 0) =====================
static void onMsg(char* topic, byte* pl, unsigned int len) {
  char buf[64];
  if (len >= sizeof(buf)) len = sizeof(buf) - 1;
  memcpy(buf, pl, len); buf[len] = 0;
  if (!strcmp(topic, T_CMD)) {
    uint8_t c = 0xFF;
    if (!strcmp(buf, "AUTO")) c = CMD_AUTO;
    else if (!strcmp(buf, "FORCE_MAX")) c = CMD_FORCE_MAX;
    else if (!strcmp(buf, "FORCE_OFF")) c = CMD_FORCE_OFF;
    else if (!strcmp(buf, "RESET")) c = CMD_RESET;
    if (c != 0xFF) xQueueSend(cmdQueue, &c, 0);
  }
#if MODE_HILS
  else if (!strcmp(topic, T_HILS_IN)) {
    unsigned long seq; float vd, vm;
    if (sscanf(buf, "%lu,%f,%f", &seq, &vd, &vm) == 3) {
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

static void publishTel(const Tel& t) {
  char js[440];
  snprintf(js, sizeof(js),
    "{\"seq\":%lu,\"v_dust\":%.3f,\"v_mq\":%.3f,\"pm_raw\":%.2f,\"pm\":%.2f,\"dcdt\":%.3f,"
    "\"gas_ppm\":%.1f,\"g\":%.3f,\"fi\":%.2f,\"v0\":%.3f,\"state\":\"%s\",\"mode\":\"%s\",\"duty\":%.1f,\"alarm\":%d}",
    (unsigned long)t.seq, t.v_dust, t.v_mq, t.pm_raw, t.pm, t.dcdt, t.gas_ppm, t.g, t.fi, t.v0,
    STATE_NAMES[t.state], MODE_NAMES[t.mode], t.duty, t.alarm);
  mqtt.publish(T_TELEM, js);
  char v[24];
  snprintf(v, sizeof(v), "%.2f", t.pm);      mqtt.publish(T_TELEM_SCALAR "/pm", v);
  snprintf(v, sizeof(v), "%.2f", t.pm_raw);  mqtt.publish(T_TELEM_SCALAR "/pm_raw", v);
  snprintf(v, sizeof(v), "%.2f", t.fi);      mqtt.publish(T_TELEM_SCALAR "/fi", v);
  snprintf(v, sizeof(v), "%.3f", t.dcdt);    mqtt.publish(T_TELEM_SCALAR "/dcdt", v);
  snprintf(v, sizeof(v), "%.1f", t.gas_ppm); mqtt.publish(T_TELEM_SCALAR "/gas_ppm", v);
  snprintf(v, sizeof(v), "%.2f", t.g);       mqtt.publish(T_TELEM_SCALAR "/g", v);
  snprintf(v, sizeof(v), "%.0f", t.duty);    mqtt.publish(T_TELEM_SCALAR "/duty", v);
  mqtt.publish(T_TELEM_SCALAR "/state", STATE_NAMES[t.state]);
  mqtt.publish(T_TELEM_SCALAR "/mode", MODE_NAMES[t.mode]);
}

static void netTask(void*) {
  uint32_t lastTry = 0;
  for (;;) {
    if (WiFi.status() == WL_CONNECTED) {
      if (!mqtt.connected()) {
        if (millis() - lastTry > 5000) {
          lastTry = millis();
          String id = "karhutla-esp32-" + String((uint32_t)ESP.getEfuseMac(), HEX);
          bool ok = (MQTT_USER[0])
              ? mqtt.connect(id.c_str(), MQTT_USER, MQTT_PASS, T_STATUS, 0, true, "offline")
              : mqtt.connect(id.c_str(), nullptr, nullptr, T_STATUS, 0, true, "offline");
          if (ok) {
            mqtt.publish(T_STATUS, "online", true);
            mqtt.subscribe(T_CMD);
#if MODE_HILS
            mqtt.subscribe(T_HILS_IN);
#endif
          }
        }
      } else {
        mqtt.loop();
        Tel copy; bool fresh = false;
        xSemaphoreTake(mtx, portMAX_DELAY);
        if (tel.fresh) { copy = tel; tel.fresh = false; fresh = true; }
        xSemaphoreGive(mtx);
        if (fresh) publishTel(copy);
      }
    }
    vTaskDelay(pdMS_TO_TICKS(5));
  }
}

// ===================== Arduino =====================
void setup() {
  pinMode(PIN_FAN, OUTPUT); digitalWrite(PIN_FAN, LOW);       // kipas mati sedini mungkin
  Serial.begin(115200);
  pinMode(PIN_DUST_LED, OUTPUT); digitalWrite(PIN_DUST_LED, HIGH);
  pinMode(PIN_BUZZER, OUTPUT);   digitalWrite(PIN_BUZZER, LOW);
  pinMode(PIN_LED_G, OUTPUT); pinMode(PIN_LED_Y, OUTPUT); pinMode(PIN_LED_R, OUTPUT);
  analogReadResolution(12);
  analogSetPinAttenuation(PIN_DUST_ADC, ADC_11db);
  analogSetPinAttenuation(PIN_MQ_ADC, ADC_11db);
  PWM_INIT();
  PWM_WRITE(0);

  mtx = xSemaphoreCreateMutex();
  cmdQueue = xQueueCreate(8, sizeof(uint8_t));
#if MODE_HILS
  hilsQueue = xQueueCreate(4, sizeof(HilsIn));
#endif
  resetAll();

  WiFi.mode(WIFI_STA);
  WiFi.setAutoReconnect(true);
  WiFi.begin(WIFI_SSID, WIFI_PASS);
  mqtt.setServer(MQTT_HOST, MQTT_PORT);
  mqtt.setCallback(onMsg);
  mqtt.setBufferSize(512);

  Serial.println(MODE_HILS ? "Mode: HILS" : "Mode: RIIL");

  xTaskCreatePinnedToCore(netTask,     "net",  8192, nullptr, 1, nullptr, 0);
  xTaskCreatePinnedToCore(controlTask, "ctrl", 8192, nullptr, 2, nullptr, 1);
  xTaskCreatePinnedToCore(buzzerTask,  "buzz", 2048, nullptr, 1, nullptr, 1);
}

void loop() {
  vTaskDelete(NULL);  // logika utama berjalan di controlTask & buzzerTask; hapus loopTask
}
