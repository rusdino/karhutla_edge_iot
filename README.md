# Karhutla Edge-IoT — SITL, HILS, dan Riil

Satu basis algoritma, tiga cara menjalankan:

| Mode | Firmware | Sensor | Plant | Perintah |
|---|---|---|---|---|
| **SITL** | `core.py` (virtual ESP32) | model Python | `plant.py` | `python sitl.py` |
| **Web Dashboard** | `core.py` / monitor MQTT `:6669` | model Python / live MQTT | `plant.py` / live | `python dashboard.py` |
| **HILS** | ESP32 asli (Rust Embassy `--features hils`, fallback `pio run -e hils`) | tegangan dikirim laptop via MQTT | `plant.py` | `python hils.py` |
| **Riil** | ESP32 asli (Rust Embassy `--features riil`, fallback `pio run -e riil`) | Sharp + MQ-135 fisik (+ RTC DS3231/DS1307) | dunia nyata | `python field_node.py` |

## Struktur
```
config.py            parameter bersama (harus sama dgn crates/karhutla-core/src/lib.rs & include/config.h)
core.py              firmware virtual: median3 -> Kalman 2-state -> fusi -> FSM -> PWM -> failsafe
plant.py             ruang asap + model tegangan Sharp & MQ-135 (skenario clean/smolder/flaming/fault)
forecast.py          AI tren linear (t+horizon) + PreemptController (FORCE_MAX proaktif yang aman)
logger.py            CSV harian logs/karhutla_<sumber>_YYYY-MM-DD.csv & log MQTT logs/mqtt_YYYY-MM-DD.log
report.py            ringkasan + uji lulus/gagal
dashboard.py         Modern Web Dashboard di laptop (grafik multi-overlay, kontrol cmd, runner SITL & HILS, Memori Backup ESP32 & RTC, live MQTT log)
sitl.py  hils.py  field_node.py
mosquitto.conf       broker lokal (port 6669)
firmware/karhutla_esp32_rs/
  partitions.csv               tabel partisi 4 MiB (nvs, factory 1,25 MiB, logs 2,625 MiB)
  crates/karhutla-core/        pustaka no_std DSP, FSM, RTC BCD, 8 pola buzzer, codec MQTT & record flash 64 B
  firmware/                    firmware utama ESP32 Rust + Embassy (esp-hal 1.2.2, esp-rtos, embassy-net, esp-storage)
firmware/karhutla_esp32/       firmware fallback Native ESP-IDF (PlatformIO: platformio.ini, include/config.h, src/main.cpp)
```

## Arsitektur Firmware ESP32 Utama: Rust `no_std` + Embassy (`firmware/karhutla_esp32_rs/`)
Firmware utama ditulis dalam **Rust `no_std` berbasis `esp-hal 1.2.2`, `esp-rtos`, `embassy-executor`, `embassy-net`, dan `esp-storage`** dengan logika inti di `crates/karhutla-core/` yang dapat diuji langsung di host Linux (`cargo test`):
- **`control_task`:** Menjalankan rantai DSP & kontrol (`Median3` → PM-proxy → `Kalman2` → skor gas → fusi `FI` → `Fsm` → PWM kipas `LEDC` 25 kHz 8-bit & LED indikator) pada 1 Hz (mode `riil` membaca ADC + pulsa IR Sharp) atau *lock-step* dari `HILS_CH` (mode `hils`, dengan watchdog 5 detik). Saat MQTT terputus, setiap sampel dikirim ke `BACKUP_CH` untuk disimpan ke flash lokal.
- **`buzzer_task`:** Berjalan independen setiap 20 ms mengevaluasi 8 pola buzzer komposit (`karhutla_core::buzzer::buzzer_on`) berdasarkan kombinasi `AlarmState` dan status `log_full` dari memori backup flash.
- **`storage_rtc_task`:** Mengelola partisi flash `logs` (`0x150000`, ukuran `0x2A0000` = 2,625 MiB, kapasitas **43.008 sampel × 64 byte**, *append-only* tanpa menimpa data lama saat penuh), membaca/menyinkronkan RTC eksternal **DS3231/DS1307** pada bus I2C0 (`GPIO21`/`GPIO22`, `0x68`), serta melayani protokol sinkronisasi & penghapusan dua fase (`META`, `GET`, `CLEAR`).
- **`wifi_task`, `net_task`, dan `mqtt_task`:** Mengelola koneksi Wi-Fi STA, stack TCP/IP `embassy-net`, dan klien MQTT 3.1.1 QoS 0 kustom dengan *Last Will* (`karhutla/status`), heartbeat 1 Hz (`karhutla/hb` memuat `log_count`, `log_cap`, `log_full`, `rtc_ok`, `epoch_s`), sinkronisasi waktu (`karhutla/time/sync`), serta transfer chunk log backup (`karhutla/log/{req,meta,chunk,ack}`). **Kontrol lokal dan penyimpanan backup tetap berjalan meskipun Wi-Fi/MQTT terputus.**

### Arsitektur Fallback Native ESP-IDF & FreeRTOS (`firmware/karhutla_esp32/src/main.cpp`)
Firmware fallback tetap dipertahankan **100% menggunakan Native ESP-IDF (`framework = espidf`) di PlatformIO** tanpa ketergantungan pada framework/library Arduino sama sekali (`app_main()`, `driver/gpio.h`, `driver/ledc.h`, `esp_adc/adc_oneshot.h`, `esp_adc/adc_cali.h`, `esp_wifi.h`, `esp_event.h`, `nvs_flash.h`, dan `mqtt_client.h`):
- **`controlTask` (Core 1, prioritas 5, stack 8192 bytes):** Menjalankan akuisisi ADC (`adc_oneshot` + `adc_cali` pada mode Riil dengan pewaktu deterministik `vTaskDelayUntil` 1 Hz dan pulsa IR `esp_rom_delay_us`) atau menerima sampel dari `hilsQueue` (mode HILS *lock-step*), mengeksekusi rantai `step()` (`Median3` → PM-proxy → `Kalman2` → skor gas → fusi `FI` → `Fsm` → `ledc` PWM kipas & `gpio_set_level` LED), serta memproses perintah masuk dari `cmdQueue`.
- **`buzzerTask` (Core 1, prioritas 3, stack 2048 bytes):** Berjalan independen setiap 20 ms membaca variabel tunggal `volatile uint8_t activeBuzzerState` yang diperbarui oleh `controlTask` menggunakan waktu milidetik dari `esp_timer_get_time()`, sehingga pola kedip buzzer tetap presisi tanpa tertahan saat pembacaan sensor fisik (~96 ms).
- **Wi-Fi & `esp_mqtt_client` Event-Driven + `txTask` (Core 0, prioritas 2, stack 6144 bytes):** Menangani koneksi ulang Wi-Fi (`WIFI_EVENT_STA_DISCONNECTED`) dan broker MQTT `:6669` secara *event-driven*, mendorong perintah `karhutla/cmd` ke `cmdQueue` dan sampel `karhutla/hils/sensor` ke `hilsQueue` pada event `MQTT_EVENT_DATA`, serta mempublikasikan telemetri yang dilindungi mutex (`mtx`). **Kontrol lokal di Core 1 tidak bergantung pada koneksi Wi-Fi/MQTT.**

## Instalasi & Kompilasi Firmware
```bash
pip install -r requirements.txt
mosquitto -c mosquitto.conf -v        # untuk HILS / riil / dashboard HP
```

### Firmware Utama: Rust + Embassy (`firmware/karhutla_esp32_rs/`)
Edit kredensial jaringan & parameter di `firmware/karhutla_esp32_rs/crates/karhutla-core/src/lib.rs` (`mod cfg`: `WIFI_SSID`, `WIFI_PASS`, `MQTT_HOST` = IP LAN laptop) dan `BROKER_HOST` di `config.py`, lalu aktifkan toolchain Xtensa ESP (`espup`) dan kompilasi/flash:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
source "$HOME/export-esp.sh"

# 1. Uji unit & paritas golden-vector pada host Linux
(cd firmware/karhutla_esp32_rs && cargo test --target x86_64-unknown-linux-gnu -p karhutla-core)

# 2. Kompilasi firmware ESP32 (pilih mode hils atau riil)
(cd firmware/karhutla_esp32_rs/firmware && cargo +esp build --release --features hils)
(cd firmware/karhutla_esp32_rs/firmware && cargo +esp build --release --no-default-features --features riil)

# 3. (Opsional saat migrasi pertama dari PlatformIO) Bersihkan partisi logs lama ke 0xFF
espflash erase-region --port /dev/ttyUSB0 0x150000 0x2a0000

# 4. Flash ke ESP32 dengan tabel partisi kustom (wajib menyertakan partitions.csv)
espflash flash --port /dev/ttyUSB0 \
  --partition-table firmware/karhutla_esp32_rs/partitions.csv \
  firmware/karhutla_esp32_rs/target/xtensa-esp32-none-elf/release/karhutla-esp32
```

### Firmware Fallback: PlatformIO (Native ESP-IDF)
Edit `firmware/karhutla_esp32/include/config.h` (`WIFI_SSID`, `WIFI_PASS`, `MQTT_HOST` = IP LAN laptop) dan `BROKER_HOST` di `config.py`, lalu kompilasi/flash sesuai mode:
```bash
cd firmware/karhutla_esp32
pio run -e hils -t upload             # flash mode HILS (MODE_HILS=1) ke esp32dev
pio run -e riil -t upload             # flash mode Riil (MODE_HILS=0) ke esp32dev
```

## 1. SITL & Modern Web Dashboard di Laptop (tanpa hardware)
```bash
python sitl.py --scenario all                                       # 4 skenario, lulus/gagal otomatis
python sitl.py --scenario flaming --ai-preempt --plot               # simpan grafik ke sitl_flaming.png
python sitl.py --scenario smolder --mqtt --speed 1                  # real-time ke dashboard HP & auto-log MQTT
python sitl.py --scenario flaming --mqtt --speed 1 --dashboard      # jalankan SITL + buka Web Dashboard laptop
python dashboard.py                                                 # buka Command & Telemetry Center di http://localhost:8080
python dashboard.py --auto-sim flaming --speed 1                    # buka Web Dashboard & langsung mulai simulasi flaming
```
- **Auto-Start Broker & Auto-Log MQTT:** Saat `--mqtt` atau `dashboard.py` dijalankan, sistem otomatis menyalakan broker Mosquitto di port **`6669`** (jika belum aktif) dan mencatat seluruh trafik MQTT (`CONNECT`, `SUBSCRIBE`, `PUBLISH`, `RECV`, `DISCONNECT`) ke `logs/mqtt_YYYY-MM-DD.log`.
- **Fitur Modern Web Dashboard (`dashboard.py`):**
  - **Grafik Multi-Overlay Interaktif:** Toggle kurva `Fusion Index (FI)`, `PM-proxy Kalman`, `PM Plant (True)`, `PM Raw (Median)`, `AI Forecast (t+60s)`, `Gas MQ-135 (ppm)`, `Skor Gas (g)`, `Tegangan Sensor (V)`, serta sub-grafik sinkron `Duty Kipas (%)` dan `State FSM`.
  - **Kontrol Override & Operasi Simulasi:** Tombol `AUTO`, `FORCE MAX`, `FORCE OFF`, `RESET` (ke topik `karhutla/cmd`), pemilih **Target Eksekusi** (`SITL (Python)` atau `HILS (ESP32)`), kontrol eksekusi (`Mulai`, `Jeda`, `Berhenti`, `Bersihkan`, pilih skenario & kecepatan `1x`–`20x`), saklar `AI Preempt`, serta saklar injeksi uji putus kabel sensor (`FAILSAFE`).
    > **Catatan HILS via Web Dashboard:** Saat menjalankan `HILS (ESP32)` dari `dashboard.py`, pastikan tidak menjalankan `hils.py` di terminal secara bersamaan agar kedua runner tidak berebut mengirim sampel ke ESP32 yang sama.
- Flag `--plot` menghasilkan file gambar PNG 3-panel di folder kerja (default: `sitl_<skenario>.png`, misalnya `sitl_flaming.png`):
  1. **Panel 1 (PM-proxy & Fusion Index):** PM sebenarnya (plant), PM-proxy mentah (median), PM-proxy Kalman, Fusion Index (`FI`), prakiraan AI `t+60s`, serta garis ambang `WASPADA` (55) dan `BAHAYA` (150).
  2. **Panel 2 (Sensor Gas MQ-135):** Kurva konsentrasi gas sebenarnya (`plant.gas_ppm`) vs estimasi gas MQ-135 (`ppm`) pada sumbu kiri, serta kurva skor gas `g` (EMA `0–1`) pada sumbu kanan.
  3. **Panel 3 (Aktuator & State):** Persentase *duty cycle* PWM kipas dan status FSM (`NORMAL`, `WASPADA`, `BAHAYA`, `FAILSAFE`).
- Hasil uji (seed 1, 900 s): clean, smolder, flaming, dan fault **semua lulus** (tanpa alarm palsu,
tanpa chattering, FAILSAFE terdeteksi lalu pulih).

## 2. HILS (ESP32 asli, sensor virtual)
1. Flash firmware Rust mode `hils` (lihat bagian *Instalasi & Kompilasi Firmware* di atas), atau fallback PlatformIO: `cd firmware/karhutla_esp32 && pio run -e hils -t upload`.
2. `python hils.py --scenario flaming --speed 5 --shadow`
3. Uji watchdog: `python hils.py --scenario smolder --dropout-at 150 --dropout-s 8`

Lock-step: laptop kirim `seq,v_dust,v_mq` → ESP32 memproses → telemetri `seq` sama kembali dengan
`duty` PWM asli → plant dimajukan. `--shadow` menjalankan firmware virtual paralel dan melaporkan
persentase langkah yang berbeda (paritas SITL ↔ ESP32).
Validasi pin-level: ukur GPIO18 (gate MOSFET) dengan logic analyzer / LED / multimeter.

## 3. Riil (lapangan)
1. Flash firmware Rust mode `riil` (`cargo +esp build --release --no-default-features --features riil` lalu `espflash flash ...`), atau fallback PlatformIO: `cd firmware/karhutla_esp32 && pio run -e riil -t upload`.
2. `python field_node.py --host <IP_laptop>` → forecast, log CSV harian, (opsional) `--ai-preempt`.

### Wiring
| Fungsi | Pin ESP32 | Catatan |
|---|---|---|
| Sharp Vo | GPIO34 | divider 10k/20k (V_adc = Vo × 2/3) |
| MQ-135 AO | GPIO35 | divider 10k/20k |
| Sharp LED | GPIO4 | aktif LOW; R150 Ω + C220 µF ke 5 V sesuai datasheet |
| Kipas (gate MOSFET) | GPIO18 | seri 100 Ω, pull-down 10 k (PWM 25 kHz 8-bit) |
| Buzzer / LED G-Y-R | 23 / 25 / 26 / 27 | |
| RTC I2C SDA / SCL | GPIO21 / GPIO22 | DS3231 / DS1307 alamat I2C `0x68` (100 kHz), baterai cadangan CR2032 |

### Pola Buzzer Komposit (8 Kombinasi State FSM × Memori Penuh)
Pada firmware Rust Embassy, buzzer (`GPIO23`) membedakan status bahaya (`AlarmState`) sekaligus kondisi memori backup flash penuh (`storage_full = true`, yaitu mencapai 43.008 sampel) melalui 8 pola deterministik:

| State FSM | Memori Normal (`storage_full = false`) | Memori Penuh (`storage_full = true`) |
|---|---|---|
| **`NORMAL`** | Mati (`0 ms`) | `1500 ms ON`, `1500 ms OFF` (siklus `3000 ms`) |
| **`WASPADA`** | `100 ms ON`, `1900 ms OFF` (siklus `2000 ms`) | `1500 ms ON`, `250 ms OFF`, `100 ms ON`, `1150 ms OFF` (siklus `3000 ms`) |
| **`BAHAYA`** | `200 ms ON`, `200 ms OFF` (siklus `400 ms`) | `1500 ms ON`, `200 ms OFF`, `4×(150 ms ON, 150 ms OFF)`, `300 ms OFF` (siklus `3200 ms`) |
| **`FAILSAFE`** | `100 ms ON`, `100 ms OFF`, `100 ms ON`, `1700 ms OFF` (siklus `2000 ms`) | `1500 ms ON`, `250 ms OFF`, `100 ms ON`, `100 ms OFF`, `100 ms ON`, `950 ms OFF` (siklus `3000 ms`) |

### Kalibrasi sebelum data dipakai
1. **Sharp**: ukur tegangan sensor di udara bersih → `DUST_V0`; bandingkan dengan alat ukur PM referensi → `DUST_SLOPE`.
2. **MQ-135**: preheat 24–48 jam, ukur di udara bersih; setelah itu isi `MQ_RS_CLEAN_KOHM` (hindari auto-calibrasi di ruangan berasap). Cek nilai RL modul → `MQ_RL_KOHM`.
3. Ubah parameter di `config.py` dan `firmware/karhutla_esp32_rs/crates/karhutla-core/src/lib.rs` (serta `firmware/karhutla_esp32/include/config.h` jika memakai fallback PlatformIO).

---

## 4. Panduan Lengkap Setup Dashboard MQTT di HP (Bisa Langsung Tanpa Input 1 per 1!)

Anda punya **2 cara instan** untuk langsung memakai dashboard di HP tanpa perlu menambahkan *widget* satu per satu secara manual:

### Cara Instan 1 (Paling Direkomendasikan): Buka Web Dashboard Langsung di Browser HP
1. Jalankan `python dashboard.py` di laptop (pastikan HP dan laptop di Wi-Fi / Hotspot yang sama).
2. Buka browser **Chrome / Safari** di HP Anda, lalu ketik alamat IP LAN laptop pada port `8080`, misalnya:
   ```text
   http://10.8.43.78:8080
   ```
   *(Ganti dengan IP LAN laptop Anda; alamat ini juga dicetak otomatis saat `dashboard.py` dijalankan).*
3. Seluruh tampilan **Modern Web Dashboard** (6 kartu KPI, grafik *multi-overlay*, tombol `AUTO` / `FORCE MAX` / `FORCE OFF` / `RESET`, dan kontrol skenario) langsung muncul di layar HP secara *real-time* tanpa perlu instal aplikasi atau setting *widget* manual.

### Cara Instan 2: Import File JSON Otomatis di Aplikasi IoT MQTT Panel
Jika Anda tetap ingin memakai aplikasi Android **IoT MQTT Panel**:
1. Ambil file konfigurasi siap pakai [karhutla_iot_mqtt_panel.json](file:///home/rusdy/Downloads/karhutla_edge_iot/karhutla/karhutla_iot_mqtt_panel.json) di folder proyek ini (atau klik tombol **`📱 Import HP (.json)`** di pojok kanan atas grafik pada Web Dashboard laptop untuk mengunduhnya dengan IP LAN terbaru).
2. Kirim file `karhutla_iot_mqtt_panel.json` tersebut ke HP Anda.
3. Di aplikasi **IoT MQTT Panel**, ketuk ikon menu titik tiga (`⋮`) di kanan atas → **Backup and Restore** (atau **Import**) → pilih file `karhutla_iot_mqtt_panel.json`. Seluruh koneksi port `6669` dan ke-15 panel/tombol akan langsung terbuat otomatis!

---

*(Jika Anda ingin mengatur ulang widget satu per satu secara manual di aplikasi lain seperti MQTT Dash, ikuti langkah referensi di bawah ini).*

### Langkah 1: Jalankan Broker Mosquitto & Cek IP LAN Laptop
1. Jalankan broker Mosquitto di laptop menggunakan file konfigurasi repo ini (sudah diset `listener 6669 0.0.0.0` agar bisa diakses dari jaringan LAN, atau biarkan `dashboard.py` / `sitl.py --mqtt` menyalakannya otomatis):
   ```bash
   mosquitto -c mosquitto.conf -v
   ```
2. Cari alamat IP LAN laptop Anda:
   - **Linux:** `ip -4 addr` atau `hostname -I` (cari IP berawalan `192.168.x.x` atau `10.x.x.x`)
   - **Windows:** `ipconfig` (lihat bagian *Wireless LAN adapter Wi-Fi* → *IPv4 Address*)
3. **Penting (Firewall):** Jika HP gagal terhubung ke laptop, buka port `6669` TCP di firewall laptop:
   - **Linux (UFW):** `sudo ufw allow 6669/tcp`
   - **Windows:** Buat *Inbound Rule* untuk Port TCP `6669` di *Windows Defender Firewall*.

### Langkah 2: Buat Koneksi Broker di Aplikasi HP (IoT MQTT Panel / MQTT Dash)
1. Buka aplikasi **IoT MQTT Panel** (atau **MQTT Dash**) di HP, lalu ketuk **Add Connection (`+`)**:
   - **Connection Name:** `Karhutla Edge-IoT`
   - **Client ID:** `hp-dashboard-karhutla` (atau biarkan otomatis)
   - **Broker / Host / IP Address:** `<IP_LAN_Laptop>` (contoh: `192.168.1.10`)
   - **Port:** `6669`
   - **Network Protocol:** `TCP`
   - **Username / Password:** Kosongkan (atau isi sesuai `MQTT_USER` / `MQTT_PASS` jika autentikasi diaktifkan)
   - **Auto Reconnect:** Aktifkan (`ON`)
2. Buat sebuah Dashboard / Panel baru (misalnya `Monitor Karhutla`), lalu simpan.

### Langkah 3: Tambahkan Widget Pemantau & Tombol Kontrol
Tambahkan panel/*widget* berikut satu per satu di dalam dashboard HP Anda:

#### A. Widget Indikator & Pemantau (*Subscribe*)
| Nama Panel / Widget | Jenis Widget | Topic to Subscribe | Satuan / Rentang | Keterangan |
|---|---|---|---|---|
| **Gauge PM-proxy** | Gauge / Meter | `karhutla/t/pm` | `0` – `300` (`µg/m³`) | Estimasi PM-proxy hasil filter Kalman (bukan PM2,5 tersertifikasi) |
| **PM-proxy Mentah (Median)** | Text / Gauge | `karhutla/t/pm_raw` | `0` – `300` (`µg/m³`) | PM-proxy sebelum filter Kalman (keluaran Median-3) |
| **Gauge Fusion Index (FI)** | Gauge / Meter | `karhutla/t/fi` | `0` – `250` | Indeks gabungan (`WASPADA` ≥ 55, `BAHAYA` ≥ 150) |
| **Kecepatan Kipas (PWM)** | Gauge / Progress | `karhutla/t/duty` | `0` – `100` (`%`) | Persentase duty cycle PWM kipas saat ini |
| **Status Bahaya (FSM)** | Text / Log | `karhutla/t/state` | Teks | Menampilkan `NORMAL`, `WASPADA`, `BAHAYA`, atau `FAILSAFE` |
| **Mode Operasi** | Text / Log | `karhutla/t/mode` | Teks | Menampilkan `AUTO`, `FORCE_MAX`, atau `FORCE_OFF` |
| **Skor Gas (MQ-135)** | Text / Gauge | `karhutla/t/g` | `0.00` – `1.00` | Skor kontribusi gas ternormalisasi (0–1) |
| **Konsentrasi Gas MQ-135** | Text / Gauge | `karhutla/t/gas_ppm` | `ppm` | Estimasi konsentrasi gas MQ-135 dalam ppm |
| **Laju Kenaikan (dC/dt)** | Text | `karhutla/t/dcdt` | `µg/m³/s` | Turunan konsentrasi PM dari state ke-2 Kalman |
| **Tegangan Sensor (`v_dust` / `v_mq` / `v0`)** | Text | `karhutla/t/v_dust`<br>`karhutla/t/v_mq`<br>`karhutla/t/v0` | `Volt (V)` | Tegangan sensor debu Sharp, MQ-135, dan baseline adaptif `V0` |
| **Prakiraan AI (t+60s)** | Text / Gauge | `karhutla/f/pm_forecast` | `µg/m³` | Prediksi PM-proxy 60 detik ke depan (dari laptop) |
| **ETA Menuju BAHAYA** | Text | `karhutla/f/eta_bahaya_s` | `detik` (`-` jika aman) | Estimasi waktu tersisa sebelum mencapai ambang 150 |
| **Kemiringan Tren & MAE AI** | Text | `karhutla/f/slope`<br>`karhutla/f/mae` | `µg/m³/s` / `µg/m³` | Kemiringan regresi linear AI dan *Mean Absolute Error* |
| **Status Koneksi ESP32 / SITL** | Text / LED Indicator | `karhutla/status` | `online` / `offline` | *Retained message* (Last Will MQTT dari ESP32 maupun SITL) |
| **Grafik Tren PM-proxy** | Line Graph | Topic 1: `karhutla/t/pm`<br>Topic 2: `karhutla/f/pm_forecast` | `µg/m³` | Membandingkan kurva PM-proxy aktual vs prakiraan AI |

*(Opsional: jika aplikasi mendukung parsing JSON langsung, Anda juga dapat men-subscribe `karhutla/telemetry` dan `karhutla/forecast`).*

#### B. Widget Tombol Perintah Override (*Publish*)
Buat widget **Button** atau **Multi-State Button / Radio Buttons** dengan konfigurasi berikut:
| Nama Tombol | Jenis Widget | Topic to Publish | Payload (Pesan) | QoS | Fungsi |
|---|---|---|---|---|---|
| **Mode AUTO** | Button | `karhutla/cmd` | `AUTO` | `0` | Mengembalikan kendali kipas ke state machine otomatis |
| **Kipas 100% (FORCE MAX)** | Button | `karhutla/cmd` | `FORCE_MAX` | `0` | Memaksa kipas berputar 100% segera |
| **Matikan Kipas (FORCE OFF)** | Button | `karhutla/cmd` | `FORCE_OFF` | `0` | Memaksa kipas berhenti 0% (tetap dihormati saat `FAILSAFE`) |
| **Reset Sistem** | Button | `karhutla/cmd` | `RESET` | `0` | Mereset filter Kalman, Median-3, FSM, dan kembalikan ke `AUTO` |

### Langkah 4: Uji Coba Dashboard HP Tanpa Perangkat Keras (via SITL)
Anda bisa langsung mencoba tampilan dan tombol di HP bahkan sebelum menyalakan ESP32:
1. Pastikan `mosquitto -c mosquitto.conf -v` sudah berjalan.
2. Jalankan simulasi SITL dengan flag `--mqtt` dan kecepatan waktu nyata (`--speed 1`):
   ```bash
   python sitl.py --scenario flaming --mqtt --speed 1
   ```
3. Lihat angka di aplikasi HP bergerak secara *live*. Coba tekan tombol **FORCE MAX**, **FORCE OFF**, **AUTO**, dan **RESET** di HP — nilai pada panel **Mode Operasi** (`karhutla/t/mode`) dan **Kecepatan Kipas** (`karhutla/t/duty`) akan langsung berubah seketika.

---

## 5. Panduan Lengkap Modern Web Dashboard di Laptop (`dashboard.py`) & Auto-Logging

Selain memantau dari HP, Anda dapat membuka **Telemetry & Command Center** langsung di browser laptop (`http://localhost:8080`) yang terhubung ke broker MQTT `:6669` secara *real-time*.

### Cara Menjalankan Dashboard Laptop
```bash
# Mode 1: Buka dashboard dalam kondisi siap (bisa klik "▶ Mulai" dari web atau memantau HILS/ESP32)
python dashboard.py

# Mode 2: Buka dashboard dan otomatis mulai simulasi skenario tertentu
python dashboard.py --auto-sim flaming --speed 1 --ai-preempt

# Mode 3: Jalankan bersama sitl.py dari terminal
python sitl.py --scenario smolder --mqtt --speed 1 --dashboard
```

### Fitur Operasi pada Panel Dashboard Laptop
1. **6 Kartu KPI Real-Time (CSS Subgrid):** Menampilkan `State FSM & Mode`, `Fusion Index (FI)`, `PM-proxy Kalman (Raw & dC/dt)`, `Gas MQ-135 (ppm, skor g, & V0)`, `Prakiraan AI (t+60s, ETA Bahaya, & MAE)`, serta `Aktuator Kipas PWM (% & tegangan sensor)`.
2. **Grafik Multi-Overlay Interaktif:**
   - Klik tombol *chip* di atas grafik untuk menumpuk/menyembunyikan kurva secara bebas: `Fusion Index (FI)`, `PM-proxy Kalman`, `PM Plant (True)`, `PM Raw (Median)`, `AI Forecast (t+60s)`, `Gas MQ-135 (ppm×0.2)`, `Skor Gas (g×100)`, `Tegangan Sensor (V×50)`, dan `Garis Ambang (55 / 150)`.
   - Arahkan kursor (*hover*) ke titik mana pun pada grafik untuk melihat *crosshair* dan rincian nilai semua variabel pada detik tersebut.
   - Pilih jendela waktu (`120s`, `300s`, `900s`, `Semua`) atau klik **`⬇ PNG`** untuk mengunduh tangkapan layar grafik.
3. **Kontrol Override Aktuator (`karhutla/cmd`):** Tombol **`AUTO (FSM)`**, **`FORCE MAX (100%)`**, **`FORCE OFF (0%)`**, dan **`RESET SISTEM`** yang langsung mengirim perintah ke MQTT `:6669` dan memperbarui kipas seketika (sinkron antara tampilan laptop dan HP).
4. **Operasi Simulasi SITL Terpadu:**
   - Pilih skenario (`flaming`, `smolder`, `clean`, `fault`), klik **`▶ Mulai`**, **`⏸ Jeda`**, **`⏹ Berhenti`**, atau **`🗑 Bersihkan`**, serta atur kecepatan simulasi (`1x`, `2x`, `5x`, `10x`, `20x`) secara langsung saat simulasi sedang berjalan.
   - Saklar **`AI Preempt`** untuk mengaktifkan/mematikan pemicuan dini `FORCE_MAX` oleh AI.
   - Saklar **`Injeksi Putus Kabel Sensor Debu (FAILSAFE)`** untuk menyimulasikan kabel sensor Sharp putus (`v_dust = 0 V`) kapan saja dan menguji transisi masuk/keluar state `FAILSAFE` (`duty = 70%`).
5. **Panel Memori Backup ESP32 & RTC (`Sync → Download → Clear`):**
   - Menampilkan status RTC eksternal (`DS3231/DS1307`, sinkronisasi otomatis via `karhutla/time/sync`), jam WIB, serta bar kapasitas memori flash backup (`0 / 43.008 sampel`, partisi `logs` 2,625 MiB) yang diperbarui dari heartbeat `karhutla/hb` (`GET /api/esp-log/status`).
   - Saat koneksi Wi-Fi/MQTT terputus, ESP32 otomatis mencatat telemetri 64 byte/sampel ke flash lokal (tanpa menimpa data lama; jika penuh, badge `MEMORI PENUH — BUZZER AKTIF` menyala dan pola buzzer komposit aktif).
   - **Alur Sinkronisasi Aman:**
     1. Klik **`Sinkronkan & Unduh Log ESP32`** → memanggil `POST /api/esp-log/sync` (mengirim `META` dan `GET:<start>:<count>` ke `karhutla/log/req`, memverifikasi CRC32 dari `karhutla/log/meta` & `karhutla/log/chunk`, lalu menyimpan CSV lokal di `logs/`), dilanjutkan `GET /api/esp-log/download` untuk mengunduh file CSV ke browser.
     2. Klik **`Kosongkan Memori ESP32`** → memanggil `POST /api/esp-log/clear`. Endpoint ini melindungi data dengan gerbang **`HTTP 409 Conflict`** jika log belum selesai disinkronkan/terverifikasi. Setelah sinkronisasi valid, dashboard mengirim perintah dua fase `CLEAR:<count>:<crc32>` ke ESP32 dan menunggu konfirmasi `karhutla/log/ack` hingga `log_count == 0`.
6. **Auto-Logging CSV & MQTT (`logs/`):**
   - **Log Telemetri CSV:** `logs/karhutla_<sitl|hils|field>_YYYY-MM-DD.csv`
   - **Log Trafik MQTT Otomatis:** `logs/mqtt_YYYY-MM-DD.log` (mencatat seluruh aktivitas `BROKER_START`, `CONNECT`, `SUBSCRIBE`, `PUBLISH`, `RECV`, dan `DISCONNECT` dengan *timestamp* milidetik serta ditampilkan langsung pada panel *Live MQTT Stream* di kanan bawah dashboard).

---

## Catatan Perilaku
- Alarm bisa lebih awal dari saat PM nyata menembus 55 µg/m³: itu disengaja (suku *lead* dC/dt dan skor gas).
- FAILSAFE: sensor putus/di luar rentang ≥5 s, atau (HILS) data berhenti >5 s → kipas 70% + alarm. `FORCE_OFF` dari HP tetap dihormati.
- Hasil Sharp = **PM-proxy**, bukan PM2,5 tersertifikasi. Laporkan ini di bagian keterbatasan.
