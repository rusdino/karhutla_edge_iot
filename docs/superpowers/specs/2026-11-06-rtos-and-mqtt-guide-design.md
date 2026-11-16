# Desain: Optimisasi FreeRTOS ESP32 & Panduan Setup Dashboard MQTT HP

## 1. Tujuan & Alasan (Intent & Why)
- Mengoptimalkan arsitektur *real-time* di ESP32 menggunakan FreeRTOS bawaan Arduino-ESP32 agar pembacaan sensor fisik (~96 ms) tidak menahan kedipan buzzer darurat, pewaktuan sampling 1 Hz bebas *drift*, serta komunikasi antar-core bebas *race condition*.
- Menyediakan panduan praktis langkah demi langkah di `README.md` agar pengguna dapat langsung menyiapkan dashboard dan tombol *override* di HP Android/iOS (untuk mode SITL, HILS, maupun Riil).

## 2. Cakupan & Batasan (Goals, Non-Goals, Constraints)
- **Goals:**
  1. Refactor penjadwalan di `firmware/karhutla_esp32/karhutla_esp32.ino` menjadi 3 *task* FreeRTOS eksplisit (`controlTask`, `buzzerTask`, `netTask`) dengan *Queue*, *Mutex*, dan variabel tunggal `volatile uint8_t activeBuzzerState`.
  2. Menghapus *task* `loopTask` bawaan Arduino dengan memanggil `vTaskDelete(NULL)` di dalam `loop()`, setelah logika utama dipindah penuh ke `controlTask`.
  3. Menambahkan panduan lengkap setup MQTT di HP pada `README.md` (persiapan jaringan/firewall, koneksi broker, daftar *widget* lengkap dengan label akurat **"PM-proxy"** bukan PM2,5 tersertifikasi, dan cara uji coba langsung lewat SITL).
  4. Mengompilasi `karhutla_esp32.ino` menggunakan **toolchain ESP32 asli** (`~/.platformio/penv/bin/pio` dengan `framework-arduinoespressif32` & `toolchain-xtensa-esp-elf`) untuk kedua mode (`MODE_HILS=1` dan `MODE_HILS=0`), serta mencatat secara jujur bahwa paritas runtime dan HILS tetap wajib diuji pada perangkat ESP32 fisik.
- **Non-Goals:**
  - Tidak mengubah konstanta di `config.py` dan `config.h`.
  - Tidak mengubah algoritma pemrosesan sinyal di `step()`, `Median3`, `Kalman2`, atau `Fsm`.
- **Constraints:**
  - Komentar dan dokumentasi wajib dalam Bahasa Indonesia.
  - Paritas algoritma antara `core.py` dan `karhutla_esp32.ino` harus terjaga 100%.

## 3. Pendekatan (Approaches)
1. **Pendekatan A (Dipilih):** Panduan Setup MQTT HP Lengkap di `README.md` + Optimisasi FreeRTOS Ringan 3-Task (`controlTask`, `buzzerTask`, `netTask`) dengan `QueueHandle_t`, `vTaskDelayUntil`, stack `controlTask` 8192 bytes, variabel tunggal `volatile uint8_t activeBuzzerState`, dan `vTaskDelete(NULL)` di `loop()`.
2. **Pendekatan B:** Hanya menambahkan panduan MQTT HP tanpa mengubah firmware (ditolak karena jitter ~96 ms pada buzzer dan variabel global masih tersisa).
3. **Pendekatan C:** Refactor 4-task penuh dengan `EventGroup` (ditolak karena *over-engineered* untuk `TS_S = 1.0 s` dan menyulitkan *lock-step* HILS).

## 4. Spesifikasi Teknis
- **Antrean FreeRTOS (`QueueHandle_t`) & State Buzzer:**
  - `cmdQueue`: panjang 8, elemen `uint8_t` (`CMD_AUTO=0`, `CMD_FORCE_MAX=1`, `CMD_FORCE_OFF=2`, `CMD_RESET=3`).
  - `hilsQueue` (khusus `MODE_HILS=1`): panjang 4, elemen `struct HilsIn { uint32_t seq; float vd, vm; }`.
  - `static volatile uint8_t activeBuzzerState = ST_NORMAL;`: satu-satunya variabel state yang ditulis oleh `controlTask` (di `resetAll()`, `step()`, dan `watchdogTrip()`) dan dibaca oleh `buzzerTask`. `buzzerTask` **tidak boleh** membaca `fsm.state` atau `failsafe` secara langsung.
- **Tasks FreeRTOS:**
  - `controlTask` (Core 1, Prioritas 2, **Stack 8192**):
    - Stack diset 8192 bytes karena `step()` memanggil `Serial.printf` dengan format *floating-point*.
    - Menguras `cmdQueue` non-blocking (`xQueueReceive(..., 0)`). Jika menerima `CMD_RESET`, panggil `resetAll()` dan `xQueueReset(hilsQueue)` (bila HILS). Jika menerima mode (`AUTO`/`FORCE_MAX`/`FORCE_OFF`), perbarui `cmdMode` dan langsung panggil `applyOutputs(activeBuzzerState)` agar respons tombol HP instan.
    - Pada `MODE_HILS=1`: menunggu sampel dari `hilsQueue` hingga `pdMS_TO_TICKS(50)`. Jika ada sampel, jalankan `step(in.vd, in.vm, in.seq)`. Jika sudah `hilsActive` dan lewat `HILS_WDT_MS` (5000 ms) sejak sampel terakhir, jalankan `watchdogTrip()` tiap 1000 ms.
    - Pada `MODE_HILS=0`: menggunakan `vTaskDelayUntil(&lastWake, pdMS_TO_TICKS(TS_S * 1000.0f))` untuk menjaga frekuensi sampling 1 Hz tetap presisi, serta mengganti `delay(1)` di `readMqVolts()` dengan `vTaskDelay(pdMS_TO_TICKS(1))` agar `buzzerTask` dapat berjalan di sela pembacaan ADC.
  - `buzzerTask` (Core 1, Prioritas 1, **Stack 2048**):
    - Loop dengan `vTaskDelay(pdMS_TO_TICKS(20))` memanggil `updateBuzzer(millis())` yang hanya membaca `activeBuzzerState`.
  - `netTask` (Core 0, Prioritas 1, **Stack 8192**):
    - Mengelola Wi-Fi dan MQTT (`PubSubClient`), mengirim pesan masuk `karhutla/cmd` ke `cmdQueue` dan `karhutla/hils/sensor` ke `hilsQueue`, serta membaca salinan `tel` dengan perlindungan `mtx` untuk di-*publish*.
  - `loop()`: memanggil `vTaskDelete(NULL)` untuk membebaskan memori stack `loopTask` bawaan Arduino karena seluruh logika akuisisi, `step()`, dan output sudah berjalan di `controlTask`.
