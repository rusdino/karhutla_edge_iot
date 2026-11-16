# Optimisasi FreeRTOS ESP32 & Panduan Setup MQTT HP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mengoptimalkan penjadwalan FreeRTOS pada `firmware/karhutla_esp32/karhutla_esp32.ino` menjadi 3 *task* deterministik (`controlTask`, `buzzerTask`, `netTask`) yang dikompilasi dengan toolchain ESP32 asli (PlatformIO) serta menambahkan panduan lengkap setup aplikasi MQTT HP di `README.md`.

**Architecture:** Firmware ESP32 memindahkan seluruh logika akuisisi, `step()`, dan aktuator dari `loop()` ke `controlTask` (Core 1, prioritas 2, stack 8192), memisahkan kedipan buzzer ke `buzzerTask` (Core 1, prioritas 1, stack 2048) yang membaca variabel tunggal `volatile uint8_t activeBuzzerState`, menjaga `netTask` di Core 0 (stack 8192), serta menghapus `loopTask` bawaan dengan `vTaskDelete(NULL)`.

**Tech Stack:** C++ (Arduino-ESP32 + FreeRTOS API, dikompilasi dengan PlatformIO `espressif32`), Python 3.x, Markdown (`README.md`).

**Spec:** `docs/superpowers/specs/2026-11-06-rtos-and-mqtt-guide-design.md`

## Global Constraints

- Bahasa komentar kode dan dokumentasi wajib menggunakan **Bahasa Indonesia**.
- Algoritma inti (`Median3`, `Kalman2`, `Fsm`, `gasUpdate`, `step`, `targetDuty`, `applyOutputs`) tidak boleh berubah perilakunya agar 100% identik dengan `core.py`.
- Nilai konstanta di `config.py` dan `firmware/karhutla_esp32/config.h` tidak boleh diubah sepihak.
- Di `README.md`, gunakan istilah **"PM-proxy"** (bukan "PM2.5") untuk widget debu karena Sharp GP2Y1010AU0F bukan sensor PM2,5 tersertifikasi.
- Verifikasi firmware wajib menggunakan kompilasi toolchain ESP32 asli (`~/.platformio/penv/bin/pio`) pada kedua mode (`MODE_HILS=1` dan `MODE_HILS=0`), dan dokumentasi tetap jujur menyatakan bahwa pengujian runtime HILS/Riil masih membutuhkan perangkat ESP32 fisik.

## Review Focus

- `loop()` memanggil `vTaskDelete(NULL)` hanya setelah `controlTask`, `buzzerTask`, dan `netTask` berhasil dibuat di `setup()`.
- `controlTask` memiliki ukuran stack **8192 bytes** agar aman saat memanggil `Serial.printf` dengan argumen `float`.
- `buzzerTask` hanya membaca variabel tunggal `static volatile uint8_t activeBuzzerState` yang diperbarui oleh `controlTask` (tidak membaca `fsm.state` dan `failsafe` secara terpisah dari task lain).
- Saat perintah `RESET` diterima dari HP, `controlTask` memanggil `resetAll()`, mengosongkan `hilsQueue` (bila HILS), dan menyinkronkan ulang `lastWake = xTaskGetTickCount()` (bila RIIL).
- Di `README.md`, label widget debu tertulis **"Gauge PM-proxy"** (bukan "Gauge PM2.5") dan menjelaskan bahwa kompilasi menggunakan toolchain ESP32 asli sementara uji perangkat fisik tetap diperlukan.

---

### Task 1: Refactor 3-Task FreeRTOS di `karhutla_esp32.ino` & Kompilasi dengan Toolchain ESP32 Asli (PlatformIO)

**Files:**
- Modify: `firmware/karhutla_esp32/karhutla_esp32.ino`

**Interfaces:**
- Consumes: Konstanta dari `firmware/karhutla_esp32/config.h` (`MODE_HILS`, `TS_S`, `HILS_WDT_MS`, `T_CMD`, `T_HILS_IN`, dll.)
- Produces:
  - `enum CmdMsg : uint8_t { CMD_AUTO = 0, CMD_FORCE_MAX = 1, CMD_FORCE_OFF = 2, CMD_RESET = 3 };`
  - `static volatile uint8_t activeBuzzerState = ST_NORMAL;`
  - `static QueueHandle_t cmdQueue;`
  - `static QueueHandle_t hilsQueue;` (saat `MODE_HILS == 1`)
  - `static void controlTask(void*)` (Core 1, prioritas 2, stack 8192)
  - `static void buzzerTask(void*)` (Core 1, prioritas 1, stack 2048)
  - `static void netTask(void*)` (Core 0, prioritas 1, stack 8192)
  - `void loop() { vTaskDelete(NULL); }`

- [ ] **Step 1: Perbarui `firmware/karhutla_esp32/karhutla_esp32.ino` dengan `controlTask` (stack 8192), `buzzerTask` (`activeBuzzerState`), `cmdQueue`/`hilsQueue`, dan `vTaskDelete(NULL)` di `loop()`**

Terapkan perubahan pada `firmware/karhutla_esp32/karhutla_esp32.ino`:
1. Tambahkan `enum CmdMsg : uint8_t { CMD_AUTO = 0, CMD_FORCE_MAX = 1, CMD_FORCE_OFF = 2, CMD_RESET = 3 };` dan `static volatile uint8_t activeBuzzerState = ST_NORMAL;`.
2. Di `resetAll()`, set `activeBuzzerState = ST_NORMAL;`. Di `step()`, setelah menghitung `uint8_t st = failsafe ? ST_FAILSAFE : fsm.state;`, tulis `activeBuzzerState = st;`. Di `watchdogTrip()`, tulis `activeBuzzerState = ST_FAILSAFE;`.
3. Di `updateBuzzer(uint32_t now)`, baca `uint8_t st = activeBuzzerState;` (jangan baca `failsafe` atau `fsm.state`).
4. Di `readMqVolts()`, ganti `delay(1)` menjadi `vTaskDelay(pdMS_TO_TICKS(1))`.
5. Di `onMsg()`, kirim perintah topik `T_CMD` ke `cmdQueue` (`xQueueSend(cmdQueue, &c, 0)`) dan sampel `T_HILS_IN` ke `hilsQueue` (jika penuh, buang 1 item terlama lalu kirim item baru).
6. Buat fungsi `static bool drainCmds()` yang memproses semua item di `cmdQueue`. Jika menerima `CMD_RESET`, panggil `resetAll()`, kosongkan `hilsQueue` (pada `MODE_HILS`), dan kembalikan `true` agar pemanggil menyinkronkan ulang pewaktu (`lastWake` / `lastSampleMs`). Jika menerima perubahan mode (`CMD_AUTO`, `CMD_FORCE_MAX`, `CMD_FORCE_OFF`), perbarui `cmdMode`, panggil `applyOutputs(activeBuzzerState)`, dan perbarui `tel` di bawah `mtx` agar respons kipas langsung aktif.
7. Implementasikan `controlTask(void*)` (Core 1, prioritas 2, stack 8192) yang menjalankan seluruh logika akuisisi/HILS, `step()`, dan output, serta `buzzerTask(void*)` (Core 1, prioritas 1, stack 2048).
8. Di `setup()`, inisialisasi `cmdQueue = xQueueCreate(8, sizeof(uint8_t))` (dan `hilsQueue = xQueueCreate(4, sizeof(HilsIn))` saat `MODE_HILS`), lalu jalankan `netTask`, `controlTask`, dan `buzzerTask`.
9. Di `loop()`, panggil `vTaskDelete(NULL);`.

- [ ] **Step 2: Kompilasi `karhutla_esp32.ino` dengan toolchain ESP32 asli (`pio ci` / PlatformIO `espressif32`) untuk `MODE_HILS=1` dan `MODE_HILS=0`**

Jalankan kompilasi penuh menggunakan `/home/rusdy/.platformio/penv/bin/pio` dengan board `esp32dev` dan library `PubSubClient` untuk `MODE_HILS=1` dan `MODE_HILS=0`.
Expected: Kedua mode berhasil dikompilasi menghasilkan `firmware.bin` dengan status `SUCCESS` (exit code 0) tanpa error.

---

### Task 2: Tambahkan Panduan Lengkap Setup Dashboard MQTT HP (Label "PM-proxy") & Catatan Arsitektur FreeRTOS di `README.md`

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: Daftar topik MQTT dari `config.py` dan `firmware/karhutla_esp32/config.h`
- Produces: Bagian dokumentasi lengkap **"Panduan Lengkap Setup Dashboard MQTT di HP"** dan arsitektur FreeRTOS di `README.md`

- [ ] **Step 1: Perbarui `README.md` dengan penjelasan 3-task FreeRTOS dan panduan setup MQTT di HP (menggunakan istilah "PM-proxy")**

Perbarui `README.md` agar memuat:
1. Arsitektur 3-task FreeRTOS di ESP32 (`controlTask` di Core 1 dengan stack 8192, `buzzerTask` di Core 1 dengan stack 2048, `netTask` di Core 0 dengan stack 8192, serta penghapusan `loopTask` dengan `vTaskDelete(NULL)`).
2. Panduan langkah demi langkah setup aplikasi **IoT MQTT Panel** / **MQTT Dash** di HP:
   - Langkah 1: Persiapan Broker Mosquitto, cek IP LAN laptop, dan pembukaan firewall port `1883`.
   - Langkah 2: Pengaturan koneksi broker di aplikasi HP.
   - Langkah 3: Tabel konfigurasi *widget* lengkap (menggunakan nama **"Gauge PM-proxy"** — bukan PM2.5 — beserta Gauge Fusion Index, Gauge Kipas PWM, Text Status/State/Mode/Forecast/ETA Bahaya, Line Graph, dan tombol/switch `AUTO`, `FORCE_MAX`, `FORCE_OFF`, `RESET`).
   - Langkah 4: Cara uji coba dashboard HP langsung dari simulasi laptop (`python sitl.py --scenario flaming --mqtt --speed 1`).

- [ ] **Step 2: Verifikasi akhir `README.md` dan `sitl.py`**

Pastikan tidak ada label widget yang mengklaim "PM2.5" di `README.md`, dan jalankan `python sitl.py --scenario all` untuk memastikan tidak ada regresi di sisi Python.
Expected: `sitl.py` mencetak `Hasil akhir: SEMUA LULUS` (exit code 0).
