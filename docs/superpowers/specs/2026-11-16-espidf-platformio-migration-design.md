# Desain: Migrasi Penuh Firmware ESP32 (HILS & Riil) ke Native ESP-IDF dengan PlatformIO

## 1. Tujuan & Alasan (Intent & Why)
- Menggantikan seluruh implementasi firmware berbasis Arduino (`karhutla_esp32.ino`, `Arduino.h`, `WiFi.h`, `PubSubClient.h`) menjadi **100% Native ESP-IDF (`framework = espidf`)** yang dikelola dengan **PlatformIO** untuk board `esp32dev`.
- Memanfaatkan driver dan layanan resmi bawaan ESP-IDF (`driver/gpio.h`, `driver/ledc.h`, `esp_adc/adc_oneshot.h`, `esp_adc/adc_cali.h`, `esp_wifi.h`, `esp_event.h`, `nvs_flash.h`, `mqtt_client.h`, `esp_timer.h`, `esp_rom_sys.h`, `esp_log.h`) sehingga eksekusi lebih deterministik, bebas ketergantungan library pihak ketiga (`PubSubClient`), dan memiliki manajemen koneksi MQTT/Wi-Fi *event-driven* bawaan.
- Menyediakan konfigurasi proyek PlatformIO multi-environment (`[env:hils]` dan `[env:riil]`) serta menyelaraskan `hils.py` dan `field_node.py` dengan skema telemetri terbaru (`gas_ppm`, port `6669`, dan `mqtt_io.disconnect()`).

## 2. Cakupan & Batasan (Goals, Non-Goals, Anti-Goals, Constraints)
- **Goals:**
  1. **Penghapusan `.ino` & Struktur Standar PlatformIO ESP-IDF:**
     - Menghapus `firmware/karhutla_esp32/karhutla_esp32.ino` dan memindahkan `config.h` ke `firmware/karhutla_esp32/include/config.h`.
     - Membuat `firmware/karhutla_esp32/platformio.ini` (dengan environment `hils` dan `riil` pada `board = esp32dev` dan `framework = espidf`), `firmware/karhutla_esp32/CMakeLists.txt`, `firmware/karhutla_esp32/src/CMakeLists.txt`, dan `firmware/karhutla_esp32/src/main.cpp`.
  2. **Implementasi Driver Perangkat Keras Native ESP-IDF:**
     - **GPIO (`driver/gpio.h`):** Menggunakan `gpio_config()` dan `gpio_set_level()` untuk `PIN_FAN` (`GPIO_NUM_18`), `PIN_DUST_LED` (`GPIO_NUM_4`, aktif LOW), `PIN_BUZZER` (`GPIO_NUM_23`), `PIN_LED_G` (`GPIO_NUM_25`), `PIN_LED_Y` (`GPIO_NUM_26`), dan `PIN_LED_R` (`GPIO_NUM_27`).
     - **PWM Kipas (`driver/ledc.h`):** Menggunakan `ledc_timer_config()` (`LEDC_LOW_SPEED_MODE`, `LEDC_TIMER_8_BIT`, `LEDC_TIMER_0`, frekuensi `PWM_FREQ = 25000` Hz) dan `ledc_channel_config()` (`LEDC_CHANNEL_0`), serta memperbarui *duty cycle* dengan `ledc_set_duty()` + `ledc_update_duty()`.
     - **ADC One-Shot & Kalibrasi (`esp_adc/adc_oneshot.h`, `esp_adc/adc_cali.h`, `esp_adc/adc_cali_scheme.h`):** Pada mode Riil (`MODE_HILS == 0`), membaca `ADC_UNIT_1` kanal `ADC_CHANNEL_6` (GPIO34 — Sharp Vo) dan `ADC_CHANNEL_7` (GPIO35 — MQ-135 AO) dengan `ADC_BITWIDTH_12` dan `ADC_ATTEN_DB_12`, dikonversi ke milivolt melalui `adc_cali_raw_to_voltage()` (dengan fallback `raw * 3300.0f / 4095.0f` jika skema kalibrasi eFuse belum tersedia).
     - **Pewaktu Presisi (`esp_rom_sys.h` & `esp_timer.h`):** Menggunakan `esp_rom_delay_us()` untuk pulsa LED IR Sharp (`280 µs`, `40 µs`, `9680 µs`) dan `(uint32_t)(esp_timer_get_time() / 1000ULL)` sebagai pengganti `millis()`.
  3. **Implementasi Jaringan & MQTT Native ESP-IDF:**
     - **Wi-Fi Event-Driven (`esp_wifi.h`, `esp_event.h`, `esp_netif.h`, `nvs_flash.h`):** Menginisialisasi NVS dan event loop default, menghubungkan Wi-Fi STA, serta melakukan auto-reconnect saat `WIFI_EVENT_STA_DISCONNECTED`.
     - **MQTT Client Native (`mqtt_client.h` / `esp-mqtt`):** Mengonfigurasi `esp_mqtt_client_config_t` ke `MQTT_HOST` dan `MQTT_PORT` (`6669`) dengan *Last Will* pada `T_STATUS` (`"offline"`, `retain = 1`). Event `MQTT_EVENT_CONNECTED` mem-publish `"online"` (`retain = 1`) dan men-subscribe `T_CMD` serta `T_HILS_IN` (bila `MODE_HILS == 1`). Event `MQTT_EVENT_DATA` mem-parsing topik dan mendorong pesan ke `cmdQueue` atau `hilsQueue`.
     - **`txTask` (Core 0, prioritas 2, stack 6144):** Memeriksa `tel.fresh` di bawah mutex `mtx` saat MQTT terhubung dan mem-publish JSON `karhutla/telemetry` beserta sub-topik skalar `karhutla/t/{pm,pm_raw,fi,dcdt,gas_ppm,g,duty,state,mode}` via `esp_mqtt_client_publish()`.
  4. **Sinkronisasi `hils.py`, `field_node.py`, dan `README.md`:**
     - Menambahkan pencatatan `gas_true` dan `gas_ppm` pada `row` di `hils.py` serta `gas_ppm` di `field_node.py`, dan menggunakan `mqtt_io.disconnect(client)` saat keluar.
     - Memperbarui dokumentasi di `README.md` dan pengujian unit di `tests/test_mqtt_and_dashboard.py` agar memverifikasi struktur ESP-IDF PlatformIO baru tanpa Arduino.
- **Non-Goals:**
  - Tidak mengubah algoritma matematika di `Median3`, `Kalman2`, `Fsm`, `gasUpdate`, `targetDuty`, `applyOutputs`, atau `step`.
  - Tidak mengubah nilai konstanta kalibrasi dan ambang di `config.h` / `config.py`.
- **Anti-Goals:**
  - Tidak menyertakan `#include <Arduino.h>`, `WiFi.h`, atau `PubSubClient.h` di mana pun dalam kode firmware.

## 3. Pendekatan (Approaches)
1. **Pendekatan 1 (Dipilih):** Native ESP-IDF Event-Driven (`esp_mqtt_client` + `esp_event`) + `controlTask` & `buzzerTask` di Core 1 + `txTask` di Core 0, dengan konfigurasi multi-environment `[env:hils]` dan `[env:riil]` di `platformio.ini`.
   - *Alasan dipilih:* Sepenuhnya native ESP-IDF tanpa library eksternal, bebas *polling loop* jaringan manual, dan memudahkan kompilasi/upload mode HILS maupun Riil langsung lewat CLI PlatformIO.
2. **Pendekatan 2:** Porting 1-ke-1 dengan `netTask` *polling* manual di atas `esp_mqtt_client` (ditolak karena menduplikasi task internal `esp-mqtt`).
3. **Pendekatan 3:** Dua folder proyek PlatformIO terpisah untuk HILS dan Riil (ditolak karena menyulitkan sinkronisasi algoritma tunggal).

## 4. Spesifikasi Teknis

### 4.1 Struktur File `firmware/karhutla_esp32/`
```text
firmware/karhutla_esp32/
├── CMakeLists.txt
├── platformio.ini
├── include/
│   └── config.h
└── src/
    ├── CMakeLists.txt
    └── main.cpp
```
- **`platformio.ini`:**
  - `[env]` bersama: `platform = espressif32`, `board = esp32dev`, `framework = espidf`, `monitor_speed = 115200`.
  - `[env:hils]`: `build_flags = -DMODE_HILS=1`.
  - `[env:riil]`: `build_flags = -DMODE_HILS=0`.
- **`include/config.h`:**
  - Menggunakan `#ifndef MODE_HILS` / `#define MODE_HILS 1` / `#endif` sehingga nilai `MODE_HILS` dapat di-*override* oleh `build_flags` di `platformio.ini`.
  - Menyimpan seluruh konstanta pin (`PIN_DUST_ADC = 34`, `PIN_MQ_ADC = 35`, `PIN_DUST_LED = 4`, `PIN_FAN = 18`, `PIN_BUZZER = 23`, `PIN_LED_G = 25`, `PIN_LED_Y = 26`, `PIN_LED_R = 27`), topik MQTT, port `6669`, dan parameter algoritma yang 100% identik dengan `config.py`.

### 4.2 Detail Implementasi `src/main.cpp`
- **Header ESP-IDF yang digunakan:**
  - `<stdio.h>`, `<string.h>`, `<math.h>`, `"freertos/FreeRTOS.h"`, `"freertos/task.h"`, `"freertos/queue.h"`, `"freertos/semphr.h"`, `"driver/gpio.h"`, `"driver/ledc.h"`, `"esp_adc/adc_oneshot.h"`, `"esp_adc/adc_cali.h"`, `"esp_adc/adc_cali_scheme.h"`, `"esp_rom_sys.h"`, `"esp_timer.h"`, `"esp_wifi.h"`, `"esp_event.h"`, `"esp_netif.h"`, `"esp_mac.h"`, `"nvs_flash.h"`, `"mqtt_client.h"`, `"esp_log.h"`, `"config.h"`.
- **Arsitektur Task & Sinkronisasi:**
  - `controlTask` (Core 1, prioritas 5, stack 8192): Menjalankan `drainCmds()`, membaca `hilsQueue` (mode HILS + watchdog `HILS_WDT_MS`) atau membaca `readDustVolts()` & `readMqVolts()` dengan `vTaskDelayUntil` 1 Hz (mode Riil), mengeksekusi `step()`, memperbarui `activeBuzzerState`, PWM kipas (`ledc_set_duty` + `ledc_update_duty`), LED indikator (`gpio_set_level`), mencetak log via `ESP_LOGI`, dan menyimpan state terbaru ke `tel` (`tel.fresh = true`) di bawah `mtx`.
  - `buzzerTask` (Core 1, prioritas 3, stack 2048): Membaca `activeBuzzerState` setiap 20 ms dan mengatur `gpio_set_level((gpio_num_t)PIN_BUZZER, on)`.
  - `txTask` (Core 0, prioritas 2, stack 6144): Ketika flag `mqttConnected` aktif dan `tel.fresh` bernilai `true`, menyalin `tel` di bawah `mtx` dan memanggil `publishTel(copy)` menggunakan `esp_mqtt_client_publish()`.

## 5. Hal yang Diserahkan ke Builder (What's Left to the Builder)
- Detail inisialisasi kurva kalibrasi ADC (`adc_cali_create_scheme_line_fitting` pada ESP32 klasik) dan fallback konversi linear bila kalibrasi eFuse tidak tersedia.
- Penanganan pemotongan string topik dan payload yang tidak *null-terminated* (`event->topic_len`, `event->data_len`) pada callback `MQTT_EVENT_DATA` dari `esp_mqtt_client`.
