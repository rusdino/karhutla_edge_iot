# Migrasi Penuh Firmware ESP32 (HILS & Riil) ke Native ESP-IDF + PlatformIO Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Menghapus `karhutla_esp32.ino` (Arduino) dan menggantinya dengan proyek **Native ESP-IDF (`framework = espidf`) di PlatformIO** (`firmware/karhutla_esp32/`) untuk board `esp32dev` dengan environment `[env:hils]` dan `[env:riil]`, menyinkronkan `hils.py` & `field_node.py`, serta memverifikasi kompilasi pada kedua mode.

**Architecture:** Firmware di `firmware/karhutla_esp32/src/main.cpp` menggunakan `app_main()` dengan driver resmi ESP-IDF (`driver/gpio.h`, `driver/ledc.h`, `esp_adc/adc_oneshot.h`, `esp_adc/adc_cali.h`, `esp_rom_sys.h`, `esp_timer.h`, `esp_wifi.h`, `esp_event.h`, `nvs_flash.h`, `mqtt_client.h`, `esp_log.h`). Kontrol lokal berjalan di Core 1 pada `controlTask` (prioritas 5, stack 8192) dan `buzzerTask` (prioritas 3, stack 2048), sementara komunikasi Wi-Fi/MQTT ditangani secara *event-driven* oleh task internal `esp_mqtt_client` dan `txTask` (Core 0, prioritas 2, stack 6144).

**Tech Stack:** C++ / C (ESP-IDF v5.x native via PlatformIO `espressif32`), CMake, Python 3.x (`hils.py`, `field_node.py`, `unittest`).

**Spec:** `docs/superpowers/specs/2026-11-16-espidf-platformio-migration-design.md`

## Global Constraints

- Hapus `firmware/karhutla_esp32/karhutla_esp32.ino` sepenuhnya; tidak boleh ada `#include <Arduino.h>`, `WiFi.h`, atau `PubSubClient.h` di mana pun pada proyek.
- Gunakan `board = esp32dev` dan `framework = espidf` di `firmware/karhutla_esp32/platformio.ini` dengan dua environment: `[env:hils]` (`-DMODE_HILS=1`) dan `[env:riil]` (`-DMODE_HILS=0`).
- Pertahankan paritas 100% algoritma `Median3`, `Kalman2`, `Fsm`, `gasUpdate`, `targetDuty`, `applyOutputs`, dan `step` antara `firmware/karhutla_esp32/src/main.cpp` dan `core.py`.
- Semua komentar kode dan dokumentasi menggunakan **Bahasa Indonesia**.

## Review Focus

- Di `MQTT_EVENT_DATA` (`mqtt_client.h`), `event->topic` dan `event->data` **tidak** *null-terminated*; wajib disalin menggunakan `event->topic_len` dan `event->data_len` dengan batas ukuran buffer dan diberi terminator `\0` sebelum diproses dengan `strcmp` atau `sscanf`.
- Di `nvs_flash_init()`, jika kembalian adalah `ESP_ERR_NVS_NO_FREE_PAGES` atau `ESP_ERR_NVS_NEW_VERSION_FOUND`, wajib panggil `nvs_flash_erase()` lalu `nvs_flash_init()` kembali agar inisialisasi Wi-Fi tidak gagal (`ESP_ERROR_CHECK`).
- Pada mode Riil (`MODE_HILS == 0`), konversi ADC `adc_oneshot_read()` ke milivolt harus mencoba kalibrasi eFuse (`adc_cali_create_scheme_line_fitting` + `adc_cali_raw_to_voltage`), dan memiliki fallback aman `(raw * 3300.0f / 4095.0f)` bila kalibrasi belum tersedia.
- Saat `hils.py` atau `field_node.py` menerima telemetri JSON dari firmware ESP-IDF (`karhutla/telemetry`), field `gas_ppm` harus tercatat ke `DailyCsvLogger` dan koneksi MQTT ditutup dengan `mqtt_io.disconnect(client)`.
- Di `include/config.h`, `#ifndef MODE_HILS` / `#define MODE_HILS 1` / `#endif` harus digunakan agar `-DMODE_HILS=0` dari `[env:riil]` di `platformio.ini` tidak menghasilkan *redefinition warning/error*.

---

### Task 1: Migrasi `firmware/karhutla_esp32/` ke Struktur Proyek PlatformIO Native ESP-IDF (`platformio.ini`, `CMakeLists.txt`, `include/config.h`, `src/main.cpp`)

**Files:**
- Delete: `firmware/karhutla_esp32/karhutla_esp32.ino`
- Delete: `firmware/karhutla_esp32/config.h`
- Create: `firmware/karhutla_esp32/platformio.ini`
- Create: `firmware/karhutla_esp32/CMakeLists.txt`
- Create: `firmware/karhutla_esp32/src/CMakeLists.txt`
- Create: `firmware/karhutla_esp32/include/config.h`
- Create: `firmware/karhutla_esp32/src/main.cpp`
- Modify: `tests/test_mqtt_and_dashboard.py`

**Interfaces:**
- Consumes: Parameter dan konstanta dari `config.py`
- Produces:
  - `firmware/karhutla_esp32/platformio.ini` (`[env:hils]` dan `[env:riil]`, `framework = espidf`, `board = esp32dev`)
  - `firmware/karhutla_esp32/include/config.h` (`MQTT_PORT 6669`, `#ifndef MODE_HILS`)
  - `firmware/karhutla_esp32/src/main.cpp` (`extern "C" void app_main(void)` dengan native ESP-IDF APIs)

- [ ] **Step 1: Perbarui pengujian di `tests/test_mqtt_and_dashboard.py` untuk memverifikasi struktur PlatformIO ESP-IDF tanpa Arduino (RED)**

Perbarui `TestConfigAndCoreCommands` dan tambahkan `test_espidf_firmware_structure_no_arduino` di `tests/test_mqtt_and_dashboard.py` yang memastikan:
1. `firmware/karhutla_esp32/karhutla_esp32.ino` sudah tidak ada.
2. `firmware/karhutla_esp32/platformio.ini` memuat `framework = espidf`, `board = esp32dev`, `[env:hils]`, dan `[env:riil]`.
3. `firmware/karhutla_esp32/include/config.h` memuat `#define MQTT_PORT   6669` dan `#ifndef MODE_HILS`.
4. `firmware/karhutla_esp32/src/main.cpp` memuat `app_main`, `mqtt_client.h`, `driver/gpio.h`, `driver/ledc.h`, `esp_wifi.h`, `gas_ppm`, dan **tidak** memuat `Arduino.h` maupun `PubSubClient`.

- [ ] **Step 2: Jalankan pengujian untuk memverifikasi kegagalan (RED)**

Run: `./sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: FAIL (`karhutla_esp32.ino` masih ada dan `platformio.ini` / `src/main.cpp` belum ada)

- [ ] **Step 3: Hapus `.ino` lama dan buat seluruh file proyek PlatformIO Native ESP-IDF (`platformio.ini`, `CMakeLists.txt`, `src/CMakeLists.txt`, `include/config.h`, `src/main.cpp`)**

1. Hapus `firmware/karhutla_esp32/karhutla_esp32.ino` dan `firmware/karhutla_esp32/config.h`.
2. Buat `firmware/karhutla_esp32/platformio.ini` dengan `[env:hils]` (`build_flags = -DMODE_HILS=1`) dan `[env:riil]` (`build_flags = -DMODE_HILS=0`) menggunakan `platform = espressif32`, `board = esp32dev`, `framework = espidf`, `monitor_speed = 115200`.
3. Buat `firmware/karhutla_esp32/CMakeLists.txt` dan `firmware/karhutla_esp32/src/CMakeLists.txt` (`idf_component_register(SRCS "main.cpp" INCLUDE_DIRS "../include")`).
4. Buat `firmware/karhutla_esp32/include/config.h` dengan `#ifndef MODE_HILS` / `#define MODE_HILS 1` / `#endif` dan `#define MQTT_PORT   6669`.
5. Buat `firmware/karhutla_esp32/src/main.cpp` menggunakan murni ESP-IDF (`driver/gpio.h`, `driver/ledc.h`, `esp_adc/adc_oneshot.h`, `esp_adc/adc_cali.h`, `esp_adc/adc_cali_scheme.h`, `esp_rom_sys.h`, `esp_timer.h`, `esp_wifi.h`, `esp_event.h`, `esp_netif.h`, `esp_mac.h`, `nvs_flash.h`, `mqtt_client.h`, `esp_log.h`).

- [ ] **Step 4: Jalankan pengujian unit untuk memverifikasi keberhasilan (GREEN)**

Run: `./sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: PASS

---

### Task 2: Sinkronisasi `hils.py`, `field_node.py`, Kompilasi PlatformIO (`hils` & `riil`), dan Pembaruan `README.md`

**Files:**
- Modify: `hils.py`
- Modify: `field_node.py`
- Modify: `README.md`
- Modify: `tests/test_mqtt_and_dashboard.py`

**Interfaces:**
- Consumes: `mqtt_io.disconnect`, `logger.FIELDS` (`gas_ppm`)
- Produces:
  - `hils.py` & `field_node.py` mencatat `gas_ppm` (dan `gas_true` di HILS) serta menutup klien dengan `mqtt_io.disconnect(client)`.
  - Dokumentasi `README.md` diperbarui untuk **Native ESP-IDF + PlatformIO** (`pio run -e hils` & `pio run -e riil`).

- [ ] **Step 1: Tambahkan pengujian untuk paritas `gas_ppm` dan `mqtt_io.disconnect` pada `hils.py` & `field_node.py` (RED)**

Tambahkan `test_hils_and_field_node_sync_with_espidf_telemetry` di `tests/test_mqtt_and_dashboard.py` yang memastikan `hils.py` dan `field_node.py` menyertakan `gas_ppm` pada `row` logger dan memanggil `mqtt_io.disconnect(client)`.

- [ ] **Step 2: Jalankan pengujian untuk memverifikasi kegagalan (RED)**

Run: `./sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: FAIL (`gas_ppm` dan `mqtt_io.disconnect` belum ada di `hils.py` / `field_node.py`)

- [ ] **Step 3: Perbarui `hils.py`, `field_node.py`, dan `README.md`**

1. Di `hils.py`, tambahkan `gas_true=plant.gas_ppm` dan `gas_ppm=tel.get("gas_ppm", K.MQ_ATM_PPM)` pada `row`, serta ganti `client.loop_stop()` dengan `mqtt_io.disconnect(client)`.
2. Di `field_node.py`, tambahkan `gas_ppm=tel.get("gas_ppm", K.MQ_ATM_PPM)` pada `row`, dan ganti `client.loop_stop()` di blok `finally` dengan `mqtt_io.disconnect(client)`.
3. Di `README.md`, perbarui bagian Struktur, Arsitektur Firmware ESP32 (kini **Native ESP-IDF + FreeRTOS di PlatformIO** tanpa Arduino/PubSubClient), Instalasi (`pio run -e hils` / `pio run -e riil`), HILS, dan Riil.

- [ ] **Step 4: Jalankan seluruh unit test, simulasi `sitl.py`, dan kompilasi PlatformIO (`pio run -e hils` & `pio run -e riil`)**

Run: `./sitl_venv/bin/python -m unittest discover -s tests -v && ./sitl_venv/bin/python sitl.py --scenario all`
Expected: Semua unit test `OK` dan `sitl.py` mencetak `HASIL AKHIR: SEMUA LULUS`.
