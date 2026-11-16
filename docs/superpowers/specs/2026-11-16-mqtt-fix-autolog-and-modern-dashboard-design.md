# Desain: Perbaikan MQTT SITL (Port 6669 & Paritas Topik), Auto-Logger MQTT, dan Modern Web Dashboard

## 1. Tujuan & Alasan (Intent & Why)
- **Memperbaiki MQTT di SITL:** Menyelesaikan masalah ketidaksinkronan port antara `mosquitto.conf` (`6669`) dan konfigurasi kode (`1883`), serta melengkapi topik dan perilaku yang absen di SITL (`karhutla/status` *retained*, penanganan perintah `RESET`, publikasi skalar `gas_ppm`/`pm_raw`/`v_dust`/`v_mq`/`v0`, dan respons instan saat tombol HP/dashboard ditekan).
- **Otomatisasi Log MQTT (`logs/mqtt_YYYY-MM-DD.log`):** Menggantikan pencatatan manual (*copy-paste* terminal) dengan sistem rotasi log harian otomatis setiap kali `sitl.py`, `hils.py`, `field_node.py`, atau `dashboard.py` dijalankan, serta menyediakan auto-start broker Mosquitto lokal bila belum berjalan.
- **Modern Web Dashboard di Laptop (`dashboard.py`):** Menyediakan antarmuka pemantau dan kendali *real-time* di browser laptop yang intuitif, mendukung penumpukan (*overlay*) kurva secara bebas, kontrol aktuator (`karhutla/cmd`), dan operasi simulasi langsung dari panel sesuai standar **Modern Web Guidance**.

## 2. Cakupan & Batasan (Goals, Non-Goals, Anti-Goals, Constraints)
- **Goals:**
  1. **Penyeragaman Port 6669:** Mengubah `BROKER_PORT = 6669` di `config.py`, `#define MQTT_PORT 6669` di `firmware/karhutla_esp32/config.h`, `listener 6669 0.0.0.0` di `mosquitto.conf`, dan memperbarui panduan di `README.md`.
  2. **Paritas & Kelengkapan Topik MQTT:**
     - Menambahkan `T_STATUS = "karhutla/status"` di `config.py`.
     - Mengatur *Last Will* (`T_STATUS -> "offline"`, `retain=True`) dan mem-publish `"online"` (`retain=True`) saat klien sumber (`sitl.py` / `dashboard.py` SITL runner) terhubung, serta `"offline"` (`retain=True`) saat berhenti.
     - Menambahkan metode `Firmware.reset()` di `core.py` (identik dengan `resetAll()` pada `karhutla_esp32.ino`) dan memproses perintah `RESET`, `AUTO`, `FORCE_MAX`, `FORCE_OFF` di `sitl.py` dan `dashboard.py` dengan publikasi ulang telemetri seketika.
     - Menambahkan `gas_ppm` pada payload JSON `karhutla/telemetry` dan topik skalar `karhutla/t/gas_ppm` di `karhutla_esp32.ino` agar paritas 100% dengan `core.py`.
     - Memperluas `mqtt_io.publish_telemetry()` untuk mem-publish sub-topik skalar `karhutla/t/{pm,pm_raw,dcdt,fi,g,gas_ppm,v_dust,v_mq,v0,state,mode,duty,seq}` dan `mqtt_io.publish_forecast()` untuk `karhutla/f/{pm_forecast,eta_bahaya_s,slope,mae}`.
  3. **Pencatatan Log MQTT Otomatis (`DailyMqttLogger`):**
     - Menambahkan kelas `DailyMqttLogger` di `logger.py` yang menulis ke `logs/mqtt_YYYY-MM-DD.log` dengan rotasi tanggal otomatis.
     - Mengintegrasikan `DailyMqttLogger` ke `mqtt_io.py` sehingga seluruh aktivitas (`CONNECT`, `DISCONNECT`, `SUBSCRIBE`, `PUBLISH`, `RECV`) tercatat otomatis beserta *timestamp* milidetik, serta menyediakan fungsi `ensure_broker()` untuk menyalakan `mosquitto -c mosquitto.conf` secara otomatis apabila port `6669` di `localhost` belum aktif.
  4. **Pembuatan `dashboard.py` (Modern Web Guidance Compliant):**
     - Server HTTP + SSE/JSON API berbasis Python standard library (`http.server`, `threading`) di `http://localhost:8080` (dapat dikustomisasi dengan `--port`).
     - Mendukung dua mode kerja sekaligus: **Mode Pasif/Monitor** (mendengarkan telemetri dari `sitl.py`, `hils.py`, atau `field_node.py` via MQTT `:6669`) dan **Mode Aktif/Embedded SITL Runner** (menjalankan/mengontrol simulasi SITL langsung dari tombol panel UI tanpa perlu membuka terminal terpisah).
     - Mengimplementasikan UI sesuai pedoman **Modern Web Guidance** (`dark-mode`, `color`, `css-layout`, `size-aware-styling`, `card`, `toolbar`, `switch`, `customize-scrollbar-color-and-thickness`).
- **Non-Goals:**
  - Tidak mengubah protokol komunikasi dari MQTT ke micro-ROS/DDS.
  - Tidak mengubah parameter algoritma fusi Kalman, model ruang asap `SmokeChamber`, atau ambang FSM.
- **Anti-Goals:**
  - Tidak menambahkan dependensi *framework* web/GUI eksternal di luar `requirements.txt` (`numpy`, `matplotlib`, `paho-mqtt`).
  - Tidak membuat elemen UI yang hanya mengandalkan warna tanpa ikon/teks penjelas status (WCAG 1.4.1).
- **Constraints:**
  - Beberapa file di repositori (`config.py`, `mqtt_io.py`, `logger.py`, `mosquitto.conf`, `field_node.py`, `forecast.py`, `plant.py`) saat ini memiliki permission *read-only* (`0444`); sebelum diedit wajib dibuka izin tulisnya (`chmod u+w`).
  - Komentar kode, teks UI, dan dokumentasi menggunakan Bahasa Indonesia.

## 3. Pendekatan (Approaches)
1. **Pendekatan 1 (Dipilih):** Web Dashboard Mandiri + Kontrol Simulasi Terintegrasi (`dashboard.py`) menggunakan Python `http.server` + Server-Sent Events (SSE) + `paho-mqtt` dengan *embedded SITL runner*, dilengkapi `DailyMqttLogger` otomatis di `logger.py`/`mqtt_io.py` dan penyeragaman port `6669`.
   - *Alasan dipilih:* Memberikan pengalaman paling intuitif dan lengkap (grafik overlay mulus, kontrol aktuator dan simulasi berfungsi langsung dari satu halaman web modern, serta tetap kompatibel penuh dengan aplikasi HP dan mode HILS/Riil).
2. **Pendekatan 2:** Web Dashboard Murni Pasif + Kontrol via Topik MQTT khusus simulasi (`karhutla/sim/*`) (ditolak karena mengharuskan banyak terminal dibuka manual dan menambah topik non-firmware).
3. **Pendekatan 3:** Koneksi Browser Langsung ke Mosquitto WebSockets (ditolak karena membutuhkan listener port tambahan di broker, tidak dapat menulis log `logs/mqtt_YYYY-MM-DD.log` dari sisi browser, dan tidak bisa mengendalikan proses Python secara langsung).

## 4. Spesifikasi Teknis (Keputusan yang Telah Ditetapkan)

### 4.1 Konfigurasi Broker & Topik MQTT (Port 6669)
- **`mosquitto.conf`:**
  - Mengatur `listener 6669 0.0.0.0` dan `allow_anonymous true` agar dapat diakses baik dari `localhost` maupun perangkat LAN (HP dan ESP32).
- **`config.py` & `firmware/karhutla_esp32/config.h`:**
  - `BROKER_PORT = 6669` dan `#define MQTT_PORT 6669`.
  - Menambahkan `T_STATUS = "karhutla/status"` di `config.py`.
- **Tabel Topik Lengkap:**
  - `karhutla/status`: `"online"` / `"offline"` (*retained*, QoS 0, dilengkapi *Last Will*).
  - `karhutla/telemetry`: JSON lengkap (`seq`, `v_dust`, `v_mq`, `pm_raw`, `pm`, `dcdt`, `gas_ppm`, `g`, `fi`, `v0`, `state`, `mode`, `duty`, `alarm`).
  - `karhutla/t/<key>`: Skalar untuk `pm`, `pm_raw`, `dcdt`, `fi`, `g`, `gas_ppm`, `v_dust`, `v_mq`, `v0`, `state`, `mode`, `duty`, `seq`.
  - `karhutla/forecast`: JSON prakiraan AI (`pm_forecast`, `slope`, `eta_bahaya_s`, `mae`).
  - `karhutla/f/<key>`: Skalar untuk `pm_forecast`, `eta_bahaya_s`, `slope`, `mae`.
  - `karhutla/cmd`: Perintah `AUTO`, `FORCE_MAX`, `FORCE_OFF`, `RESET`.
  - `karhutla/hils/sensor`: Input tegangan HILS `seq,v_dust,v_mq`.

### 4.2 Perbaikan `core.py`, `mqtt_io.py`, dan `sitl.py`
- **`core.py`:**
  - Menambahkan metode `Firmware.reset()` yang mereset `med_d`, `med_m`, `kf`, `fsm`, mengembalikan `v0 = K.DUST_V0`, `pm_raw = 0.0`, `ppm = K.MQ_ATM_PPM`, `g = 0.0`, `fi = 0.0`, `duty = 0.0`, `failsafe = False`, `bad = 0`, `good = 0`, `calib = []`, `r0`, serta `mode = "AUTO"`.
  - Menambahkan metode `Firmware.apply_command(cmd)` yang menangani `"RESET"` (memanggil `reset()`) maupun `"AUTO"`, `"FORCE_MAX"`, `"FORCE_OFF"` (memperbarui `mode` dan langsung menghitung ulang `duty` saat ini tanpa menunggu sampel berikutnya).
- **`mqtt_io.py`:**
  - Mengintegrasikan `DailyMqttLogger` dari `logger.py` secara otomatis pada klien MQTT.
  - Menambahkan parameter `publish_status=False` pada `make_client()` / `connect()`: bila `True` (dipakai oleh `sitl.py` dan *runner* SITL di `dashboard.py`), otomatis memasang `will_set(K.T_STATUS, "offline", qos=0, retain=True)`, mem-publish `karhutla/status = "online"` (`retain=True`) saat `on_connect`, dan menyediakan `disconnect(c)` yang mem-publish `"offline"` (`retain=True`) sebelum `loop_stop()` + `disconnect()`.
  - Menambahkan fungsi `ensure_broker(host, port)` yang mengecek apakah broker lokal di `localhost:6669` sudah aktif; jika belum, otomatis menjalankan proses `mosquitto -c mosquitto.conf` di latar belakang.
- **`sitl.py`:**
  - Memastikan `ensure_broker()` dipanggil saat `--mqtt` aktif, menangani semua perintah `karhutla/cmd` (termasuk `RESET`) melalui `fw.apply_command()`, langsung mem-publish ulang telemetri saat perintah masuk, dan menutup koneksi MQTT dengan bersih (`mqtt_io.disconnect(client)`) di akhir `run()`.
  - Menambahkan argumen opsional `--dashboard` pada `sitl.py` untuk ikut menyalakan server web dashboard secara otomatis saat simulasi berjalan.

### 4.3 Auto-Logger MQTT (`logger.py`)
- Menambahkan kelas `DailyMqttLogger(folder="logs", prefix="mqtt")`:
  - Menyimpan log ke `logs/mqtt_YYYY-MM-DD.log` dengan rotasi harian otomatis.
  - Format baris log: `<ISO-timestamp> [<client_id>] <EVENT> <topic/detail> <payload>` sehingga mudah dibaca manusia dan tetap kompatibel untuk audit/debugging seperti file `logs/mqtt_2026-11-16.log`.

### 4.4 Spesifikasi `dashboard.py` & Kepatuhan Modern Web Guidance
- **Arsitektur Backend `dashboard.py`:**
  - Menjalankan HTTP server lokal (`default port 8080`) dengan endpoint:
    - `GET /`: menyajikan halaman HTML5 modern.
    - `GET /api/stream`: Server-Sent Events (SSE) yang mengalirkan update telemetri, forecast, status koneksi, status simulasi, dan entri log MQTT secara *real-time*.
    - `GET /api/state`: mengambil snapshot riwayat data (buffer hingga 900 sampel terakhir) dan konfigurasi saat halaman pertama kali dibuka/di-refresh.
    - `POST /api/cmd`: mem-publish perintah `AUTO`, `FORCE_MAX`, `FORCE_OFF`, atau `RESET` ke topik `karhutla/cmd`.
    - `POST /api/sim`: mengontrol *embedded SITL runner* (`start`, `stop`, `pause`, `resume`, ganti `scenario` [`clean`/`smolder`/`flaming`/`fault`], ubah `speed` [`1`, `2`, `5`, `10`, `20`], toggle `ai_preempt`, toggle injeksi `fault_inject`, dan `clear_history`).
- **Arsitektur Frontend & Standar Modern Web Guidance:**
  - **`dark-mode` & `color`:** `<meta name="color-scheme" content="light dark">`, `:root { color-scheme: light dark; }` dengan fallback `@media (prefers-color-scheme: dark)` dan `@supports (color: light-dark(white, black))`, dukungan `@media (forced-colors: active)`, serta indikator non-warna (ikon + bentuk + teks) di setiap status.
  - **`css-layout`, `size-aware-styling`, & `card`:** Tata letak utama menggunakan `grid-template-areas`, kartu KPI menggunakan `<article class="kpi-card">` dengan `grid-template-rows: subgrid` (dan fallback `grid-template-rows: auto 1fr auto`), serta `container-type: inline-size` untuk tipografi responsif berbasis `cqi`.
  - **`toolbar` & Grafik Multi-Overlay:** Kanvas grafik interaktif HTML5 `<canvas>` (`display: block`, mendukung *hover crosshair tooltip* nilai per-detik, zoom rentang waktu, dan ekspor PNG) yang dikontrol oleh `<div role="toolbar" focusgroup="toolbar wrap">` dengan tombol `<button type="button" aria-pressed="true|false">` untuk menyalakan/mematikan kurva `Fusion Index (FI)`, `PM Kalman`, `PM Plant (True)`, `PM Raw (Median)`, `AI Forecast (t+60s)`, `Gas MQ-135 (ppm)`, `Skor Gas (g x100)`, `Tegangan Sensor (v_dust & v_mq)`, serta sub-grafik sinkron `Duty Kipas (%)` & `State FSM`.
  - **`switch` & `customize-scrollbar-color-and-thickness`:** Menggunakan `<input type="checkbox" switch>` untuk saklar `AI Preempt`, `Injeksi Sensor Fault`, dan `Auto-Scroll Log`, serta `scrollbar-color` + `scrollbar-width: thin` + `scrollbar-gutter: stable` + `overscroll-behavior: contain` (dengan fallback `@supports not (scrollbar-color: auto)`) pada panel *Live MQTT Stream*.

## 5. Hal yang Diserahkan ke Builder (What's Left to the Builder)
- Detail struktur internal *thread-safe ring buffer* (`collections.deque(maxlen=1800)`) di `dashboard.py` untuk mendistribusikan event ke klien SSE.
- Detail penggambaran kurva dan *hover tooltip* pada elemen `<canvas>` 2D di frontend `dashboard.py`.
- Pengujian unit/integrasi otomatis untuk memastikan `sitl.py`, `mqtt_io.py`, `logger.py`, dan endpoint `dashboard.py` berjalan mulus di port `6669`.
