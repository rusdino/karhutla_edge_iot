# Desain: Migrasi Firmware ESP32 ke Rust Embassy (`no_std`), RTC I2C, Backup Logger Flash `2,625 MiB`, Buzzer Komposit, dan Sinkronisasi Web

## 1. Tujuan & Alasan (Intent & Why)

Proyek **Karhutla Edge-IoT** memiliki tiga syarat arsitektur utama:
1. Node lapangan dan uji HILS berbasis **ESP32** (`esp32dev`, 4 MB flash).
2. **Laptop** sebagai pusat logger harian (`logs/`), pengolahan/prediksi AI (`LinearTrendForecaster` & `PreemptController`), broker MQTT lokal (`:6669`), dan antarmuka web (`dashboard.py` di `:8080`).
3. **HP** sebagai tampilan sederhana dan kendali jarak dekat di jaringan LAN/hotspot lokal (melalui web responsif maupun aplikasi `IoT MQTT Panel`).

Untuk menjadikan sistem tangguh ketika koneksi Wi-Fi/broker terputus, diperlukan dua kapabilitas baru di ESP32 beserta kendalinya di website:
- **RTC fisik I2C (`DS3231` / `DS1307`)** agar stempel waktu log pada ESP32 tetap akurat secara kalender/jam nyata tanpa bergantung pada koneksi laptop.
- **Backup logger harian di flash ESP32** yang **hanya menulis saat ESP32 tidak terkoneksi ke server MQTT**, menampung data hingga kapasitas maksimal, membunyikan **pola buzzer komposit** saat penuh, serta memaksa alur **sinkronisasi dan pengunduhan log di website sebelum memori ESP32 boleh dikosongkan**.

Bersamaan dengan penambahan fitur I/O tersebut, firmware ditulis ulang ke **Rust `no_std` + Embassy** pada branch baru (`feat/embassy-rust-firmware`) sehingga:
- Manipulasi buffer paket MQTT, parser topik/payload, sinkronisasi antar-task, serta baca-tulis record flash & register RTC dijamin *memory-safe* dan bebas *data race* sejak *compile-time*.
- Menghapus lapisan FreeRTOS/LwIP/ESP-IDF C++ pada firmware lapangan sehingga ukuran binary app jauh lebih kecil dan ruang partisi flash backup dapat diperbesar signifikan.

### Bukti Pengukuran Hardware (*Spike* `spike/embassy-nostd`, commit `9ef8a68`)
Sebelum desain ini dikunci, *throwaway spike* Rust `no_std` + Embassy telah dikompilasi, di-flash ke ESP32 fisik (`/dev/ttyUSB0`, `esp32 rev v3.1`), dan diuji langsung terhadap Mosquitto `:6669`:

| Metrik Terukur | Firmware ESP-IDF C++ (`main`) | *Spike* Embassy `no_std` |
|---|---|---|
| Ukuran App `.bin` | `935 KB` (`hils`) / `925 KB` (`riil`) | **`573 KB`** (`586.112` byte, turun ~38%) |
| Partisi Flash Backup (`logs`) | `1,375 MiB` (`1.408 KiB`) | **`2,625 MiB`** (`2.688 KiB` di `0x150000`) |
| Boot → Wi-Fi IP → MQTT `:6669` | — | **`~2,05 detik`** |
| Uji 50 Paket Lock-Step MQTT | — | **`50/50` sukses (`0` drop)**, RTT rata-rata **`10,72 ms`** (min `7,66 ms`, p95 `17,67 ms`, maks `20,74 ms`) |
| Tulis Flash saat Boot & Live MQTT | — | **`PASS`** keduanya (tulis live `60,51 ms` tanpa memutus MQTT) |

---

## 2. Cakupan: Goals, Non-Goals, Anti-Goals, & Constraints

### Goals
1. **Workspace Rust `no_std` di `firmware/karhutla_esp32_rs/`** terdiri atas dua crate:
   - `karhutla-core` (`#![no_std]`, dapat dikompilasi dan diuji di host `x86_64` via `cargo test`): berisi seluruh algoritma DSP & kontrol (`Median3`, `Kalman2`, `HysteresisFSM`, pemrosesan gas MQ-135, adaptasi `v0`, Fusion Index, `target_duty`, *slew-limit*, failsafe), *state machine* pola buzzer komposit, codec biner `BackupRecord` (64 byte + CRC-32), serta konversi BCD RTC ↔ Unix epoch.
   - `karhutla-esp32` (`#![no_std]`, target `xtensa-esp32-none-elf`, feature `hils` atau `riil`): runtime `esp-hal 1.2.2` + `esp-rtos 0.4.0` + `esp-radio 1.0.0-beta.1` + `embassy-net 0.9.1` + `esp-storage 0.10.0`, driver ADC/GPIO/LEDC/I2C RTC, klien MQTT 3.1.1 QoS0 kustom dengan *Last Will & Testament* (`karhutla/status` `"offline"` retain) dan *auto-reconnect*.
   - *(Keputusan Perancang)*: Direktori firmware C++ lama `firmware/karhutla_esp32/` tetap dipertahankan utuh di branch ini agar 26 unit test eksisting tetap lulus dan pengguna tetap memiliki pembanding/fallback.
2. **Paritas 100% dengan `core.py` & `config.py`:** Seluruh konstanta kalibrasi, ambang FSM, periode `TS_S = 1.0`, watchdog `HILS_WDT_MS = 5000`, skema JSON `karhutla/telemetry`, topik skalar `karhutla/t/*`, dan handler `karhutla/cmd` (`AUTO`, `FORCE_MAX`, `FORCE_OFF`, `RESET`) identik dengan implementasi saat ini.
3. **Driver RTC I2C (`DS3231` / `DS1307`) dengan Fallback Aman:**
   - Terhubung ke `GPIO21` (SDA) dan `GPIO22` (SCL), alamat `0x68`, kecepatan bus `100 kHz`.
   - *(Keputusan Perancang)*: RTC menyimpan dan melaporkan waktu sebagai **UTC Unix epoch (`u32`)**; konversi ke waktu lokal laptop (`WIB` / `Asia/Jakarta` sesuai sistem host) dilakukan saat penggabungan/ekspor CSV di laptop.
   - Sinkronisasi waktu otomatis dari laptop melalui topik MQTT `karhutla/time/sync` (payload string integer detik UTC) setiap kali ESP32 terhubung dan selisih waktu > `5 detik` atau flag RTC belum tersetel.
   - Bila modul RTC tidak terpasang/tidak merespons I2C (misalnya saat uji meja HILS), firmware **tidak panic**: menandai `rtc_ok = false`, tetap menjalankan seluruh kendali, dan memakai jam software berbasis `embassy_time::Instant` yang disinkronkan dari `karhutla/time/sync` (atau `epoch_s = 0` dengan `rtc_valid = 0` bila belum pernah tersinkron).
4. **Backup Logger Flash Offline-Only (`2,625 MiB` / `43.008` Sampel):**
   - Partisi kustom `partitions.csv` pada flash 4 MB:
     - `nvs` (`0x9000`, `0x6000` / `24 KiB`)
     - `factory` (`0x10000`, `0x140000` / `1.280 KiB` / `1,25 MiB`)
     - `logs` (`0x150000`, `0x2A0000` / `2.688 KiB` / `2,625 MiB`, `672` sektor @ `4.096` byte)
   - **Kapan menulis:** Firmware **hanya menulis ke flash ketika `!mqtt_connected`**.
     - Pada mode `RIIL`: menulis 1 record setiap langkah kontrol (`1 Hz`) selama MQTT terputus.
     - Pada mode `HILS` *(Keputusan Perancang)*: jika sesi HILS sempat aktif lalu MQTT/broker terputus dan memicu `watchdogTrip()`, firmware hanya mencatat **1 record transisi `FAILSAFE` tunggal** per kejadian putus koneksi (tidak mengulang `1 Hz` terus-menerus agar broker yang sedang mati tidak memenuhi flash dengan baris duplikat); selama MQTT terhubung normal tidak ada penulisan flash.
   - **Format record biner tetap `64` byte (4-byte aligned, `64` record per sektor 4 KiB, kapasitas total `43.008` record = `~11,95` jam nonstop pada `1 Hz`):**
     - Offset `0..4`: `magic: [u8; 4]` = `*b"KLOG"`
     - Offset `4`: `version: u8` = `1`
     - Offset `5`: `state: u8` (`0=NORMAL, 1=WASPADA, 2=BAHAYA, 3=FAILSAFE`)
     - Offset `6`: `mode: u8` (`0=AUTO, 1=FORCE_MAX, 2=FORCE_OFF`)
     - Offset `7`: `flags: u8` (`bit0 = rtc_valid`, `bit1 = mode_hils`)
     - Offset `8..12`: `epoch_s: u32` (little-endian UTC Unix timestamp)
     - Offset `12..16`: `uptime_ms: u32` (little-endian)
     - Offset `16..20`: `seq: u32` (little-endian)
     - Offset `20..60`: 10× `f32` little-endian (`v_dust`, `v_mq`, `pm_raw`, `pm`, `dcdt`, `gas_ppm`, `g`, `fi`, `v0`, `duty`)
     - Offset `60..64`: `crc32: u32` (little-endian CRC-32/IEEE atas byte `0..60`)
   - **Append sekuensial & pemindaian boot cepat *(Keputusan Perancang)*:** Slot kosong di NOR flash bernilai `0xFF`. Saat boot, *binary search* atas `43.008` slot (`≤ 16` baca flash, `< 2 ms`) hanya mencari indeks tulis berikutnya (`log_count` & `log_full`) tanpa memindai seluruh partisi. **CRC-32 agregat** (CRC-32/IEEE standar atas gabungan mentah seluruh byte slot `0..log_count`, yakni `bytes[0..log_count*64]`) dihitung secara *lazy* saat permintaan `META` pertama (membaca per sektor 4 KiB sambil `.await` berkala), di-cache sebagai `(cached_count, cached_crc32)`, lalu diperbarui secara inkremental (`O(1)`) setiap kali ada penambahan record offline baru. Setiap tulis memakai `FlashStorage::write_nor` langsung ke slot `0xFF` (`< 1 ms`, tanpa erase sektor per record sehingga keausan flash minimal). Penghapusan sektor hanya dilakukan saat perintah `CLEAR` diterima (menghapus sektor 4 KiB yang benar-benar terpakai saja).
   - **Saat penuh (`log_count >= 43.008`):** Set `log_full = true`, hentikan penulisan record baru (data lama dipertahankan, tidak menimpa), **seluruh loop kontrol asap, Kalman, FSM, kipas PWM, dan MQTT reconnect tetap berjalan normal**, dan aktifkan pola buzzer komposit.
5. **Pola Buzzer Komposit (Memori Belum Penuh vs Memori Penuh):**
   - Buzzer dijalankan non-blocking pada resolusi `20 ms` berdasarkan kombinasi `(storage_full, active_state)`:
     - **Memori belum penuh (`storage_full == false`):**
       - `NORMAL`: diam (`0 ms`)
       - `WASPADA`: `100 ms` ON / `1900 ms` OFF (siklus `2000 ms`)
       - `BAHAYA`: `200 ms` ON / `200 ms` OFF (siklus `400 ms`)
       - `FAILSAFE`: `100 ms` ON, `100 ms` OFF, `100 ms` ON, `1700 ms` OFF (siklus `2000 ms`)
     - **Memori penuh (`storage_full == true`, selalu diawali beep panjang `1500 ms`):**
       - `Penuh + NORMAL`: `1500 ms` ON / `1500 ms` OFF (siklus `3000 ms`)
       - `Penuh + WASPADA`: `1500 ms` ON, `250 ms` OFF, `100 ms` ON, `1150 ms` OFF (beep lama + 1 beep pendek, siklus `3000 ms`)
       - `Penuh + BAHAYA`: `1500 ms` ON, `200 ms` OFF, `4×(150 ms ON, 150 ms OFF)`, `300 ms` OFF (beep lama + rentetan beep cepat, siklus `3200 ms`)
       - `Penuh + FAILSAFE`: `1500 ms` ON, `250 ms` OFF, `100 ms` ON, `100 ms` OFF, `100 ms` ON, `950 ms` OFF (beep lama + 2 beep pendek, siklus `3000 ms`)
6. **Protokol Sinkronisasi, Unduh, & Kosongkan Memori di Website (`dashboard.py`):**
   - Seluruh alur memakai broker MQTT `:6669` yang sudah ada agar tetap bekerja di jaringan hotspot/LAN tanpa perlu IP statis ESP32 maupun server HTTP di mikrokontroler.
   - **Perluasan Heartbeat (`karhutla/hb`) yang *backwards-compatible*:**
     ```json
     {"mode":"RIIL","uptime_ms":123456,"rtc_ok":true,"epoch_s":1795680000,"log_count":420,"log_cap":43008,"log_full":false,"log_bytes":26880,"log_cap_bytes":2752512}
     ```
     Ditambah topik skalar untuk HP: `karhutla/t/log_pct` (`"1.0"`–`"100.0"`) dan `karhutla/t/log_full` (`"0"`/`"1"`).
   - **Topik Sinkronisasi & Hapus Log:**
     - `karhutla/time/sync` (Laptop → ESP32): string angka UTC epoch detik.
     - `karhutla/log/req` (Laptop → ESP32):
       - `META` — minta ringkasan isi flash & `crc32` agregat.
       - `GET:<start>:<count>` — minta chunk record (`count` maks `16` record per pesan agar aman dalam frame MQTT/TCP).
       - `CLEAR:<expected_count>:<expected_crc32>` — hapus memori flash hanya bila jumlah dan CRC-32 cocok dengan data yang baru disinkronkan.
     - `karhutla/log/meta` (ESP32 → Laptop):
       `{"count":420,"cap":43008,"full":false,"used_bytes":26880,"cap_bytes":2752512,"crc32":3141592653,"rtc_ok":true,"epoch_s":1795680000}`
       (`crc32` adalah CRC-32/IEEE standar atas gabungan mentah seluruh byte slot `0..count`).
     - `karhutla/log/chunk` (ESP32 → Laptop) *(Keputusan Perancang)*:
       Mengirimkan slice biner mentah `BackupRecord` (masing-masing `64` byte) dalam **Base64** agar *bit-exact* dengan flash (`16` record = `1.024` byte mentah = `1.368` byte Base64, total JSON `~1,42 KiB` sehingga aman dalam buffer TX `2 KiB` tanpa kehilangan presisi `f32`):
       `{"start":0,"count":16,"total":420,"data":"<base64_raw_records>"}`
     - `karhutla/log/ack` (ESP32 → Laptop) *(Keputusan Perancang, Two-Phase Ack)*:
       Ketika `CLEAR:<expected_count>:<expected_crc32>` valid, ESP32 langsung mengirim fase-1 `{"action":"CLEAR","status":"started","sectors":7}`, menghapus hanya sektor 4 KiB yang terpakai (sambil `Timer::after_millis(10).await` tiap 4 sektor agar `control_task`, `buzzer_task`, dan keepalive MQTT tidak terputus), lalu mengirim fase-2 `{"action":"CLEAR","status":"done","ok":true,"erased_sectors":7,"count":0}`. Bila tidak cocok, langsung membalas `{"action":"CLEAR","status":"done","ok":false,"reason":"mismatch"}`.
   - **Endpoint HTTP Baru di `dashboard.py`:**
     - `GET /api/esp-log/status`: Mengembalikan status terkini memori flash ESP32, status RTC, progres sync/clear aktif, serta flag gerbang `synced` & `downloaded` untuk `(log_count, crc32)` aktif.
     - `POST /api/esp-log/sync` *(Keputusan Perancang)*: Mengirim `META`, menarik seluruh chunk `GET:<start>:16` berurutan (dengan retry otomatis hingga `3×` per chunk bila timeout), mendekode Base64, memvalidasi `magic == b"KLOG"` + `crc32` tiap record (`slot[60..64]`), serta memverifikasi `zlib.crc32` seluruh gabungan byte mentah terhadap `meta.crc32`. Record hasil sinkronisasi disimpan ke dua keluaran:
       1. Arsip harian `logs/karhutla_esp32_backup_YYYY-MM-DD.csv` (dikelompokkan per tanggal lokal dari `epoch_s`).
       2. Satu file batch lengkap `logs/karhutla_esp32_sync_<YYYYMMDD_HHMMSS>.csv` berisi 100% record hasil tarik (termasuk jika rentang offline melewati tengah malam / multi-hari).
       - Penanganan record tanpa jam RTC valid (`rtc_valid == 0` atau `epoch_s < 1_700_000_000`): dimasukkan ke file arsip tanggal sinkronisasi hari ini (bukan `1970-01-01.csv`), kolom `timestamp` diisi `"UNSYNCED+<uptime_ms>ms"`, dan penggabungan CSV harian memakai kunci deduplikasi 4-tuple **`(epoch_s, uptime_ms, seq, record_crc32)`** agar sesi beda *power cycle* tidak saling menimpa.
     - `GET /api/esp-log/download`: Mengirimkan satu file CSV batch lengkap (`karhutla_esp32_sync_<YYYYMMDD_HHMMSS>.csv`) hasil sinkronisasi terakhir sebagai attachment unduhan browser dan menandai `downloaded = true` untuk pasangan `(count, crc32)` tersebut.
     - `POST /api/esp-log/clear`: **Gerbang keras server** — menolak dengan `HTTP 409 Conflict` bila `log_count > 0` dan pasangan `(log_count, crc32)` belum berstatus **`synced && downloaded`**, atau bila ESP32 offline. Jika valid, mengirim `CLEAR:<count>:<crc32>` ke ESP32 dan menunggu fase-1 (`started`) lalu fase-2 (`done`) pada `karhutla/log/ack` dengan **timeout dinamis** `max(20 dtk, ceil(sectors * 0.12) + 15 dtk)` (maks `90 dtk` untuk kapasitas penuh 672 sektor), sembari menyiarkan progres penghapusan lewat SSE, lalu mereset status cache memori ke `0`.
   - **Antarmuka Website (`dashboard.py`):**
     - Kartu **"Memori Backup ESP32 & RTC"** pada tampilan desktop dan mobile berbahasa Indonesia (tanpa build step/CDN):
       - Menampilkan status RTC (`Tersinkron` / `Tanpa Modul RTC`), jam ESP32 terkini, bar kapasitas flash (`X / 43.008 sampel`, persentase, dan badge merah `MEMORI PENUH` saat `log_full`).
       - **Langkah 1 — Tombol `Sinkronkan & Unduh Log ESP32`:** Menarik data via `/api/esp-log/sync`, menampilkan progres chunk (`N / M`), lalu otomatis memicu unduhan `/api/esp-log/download`.
       - **Langkah 2 — Tombol `Kosongkan Memori ESP32`:** **Terkunci (`disabled`)** selama `log_count > 0` dan Langkah 1 belum selesai untuk isi memori saat itu. Setelah terbuka, meminta dialog konfirmasi tegas sebelum memanggil `/api/esp-log/clear`.

### Non-Goals
- Tidak mengubah parameter DSP, Kalman, FSM, maupun AI Linear Trend Forecaster di `config.py` / `core.py`.
- Tidak menambahkan pembaruan OTA (`app0`/`app1`), karena penghapusan slot OTA adalah cara utama membebaskan ruang partisi `logs` menjadi `2,625 MiB`.
- Tidak menambahkan autentikasi pengguna, database eksternal, ataupun dependensi cloud/CDN internet.

### Anti-Goals
- **Jangan pernah** mengizinkan pengosongan memori flash ESP32 sebelum log berhasil disinkronkan ke `logs/` dan diunduh dari website.
- **Jangan pernah** menghentikan pembacaan sensor, Kalman, FSM, atau PWM kipas hanya karena flash backup penuh, RTC I2C tidak terpasang, atau Wi-Fi/MQTT putus.
- **Jangan pernah** memakai crate `rust-mqtt 0.6.0` dengan asumsi dukungan MQTT 3.1.1 (hasil verifikasi *spike* membuktikan fitur `v3` pada crate tersebut belum aktif dan hanya mendukung MQTT 5.0).

### Constraints
- Target perangkat keras: **ESP32 Dev Module (`esp32dev`, Xtensa LX6, flash 4 MB)**.
- Toolchain build: Rust channel `esp` (`espup` + `~/export-esp.sh`) dan `espflash 4.7.0`.
- Seluruh sistem wajib berjalan **100% offline-first di LAN lokal** (Mosquitto `:6669`, Dashboard `:8080`).
- Seluruh label UI website wajib berbahasa Indonesia yang konsisten dengan `PRODUCT.md`.

---

## 3. Pendekatan yang Dipertimbangkan (Approaches)

| Kriteria | 1. Bertahan di C++ Native ESP-IDF (PlatformIO) | 2. Rust `std` + FreeRTOS (`esp-idf-svc`) | 3. **Rust `no_std` + Embassy (`esp-hal`) — DIPILIH** |
|---|---|---|---|
| *Memory & Concurrency Safety* | Rendah (raw pointer, `snprintf`/`sscanf`, mutex manual) | Sedang-Tinggi (kode aplikasi aman, namun masih membungkus C ESP-IDF) | **Tertinggi** (100% Rust dari aplikasi hingga HAL, *state machine* task statis) |
| Ukuran Binary `.bin` | `935 KB` | `~1,0–1,1 MB` | **`573 KB`** (terukur pada *spike*) |
| Ruang Partisi Backup (`logs`) | `1,375 MiB` (default OTA) | `~1,375–2,0 MiB` | **`2,625 MiB`** (`43.008` sampel, terukur) |
| Kompleksitas Build & Risiko | Paling rendah (sudah ada) | Tinggi (kompilasi silang CMake ESP-IDF + `embuild`) | Rendah di build (`47 dtk`, hanya `cargo +esp`), risiko jaringan telah dibuktikan lolos di *spike* |

**Alasan Pemilihan:** *Spike* hardware membuktikan **Pendekatan 3 (`no_std` + Embassy)** memberikan ukuran binary paling ringkas, ruang backup `2,625 MiB`, serta performa MQTT lock-step (`10,72 ms` rata-rata, `0` drop) yang jauh melampaui kebutuhan HILS `20×`.

---

## 4. Arsitektur Teknis Lengkap

### 4.1 Pemetaan Pin Perangkat Keras (`esp32dev`)
| Fungsi | Pin ESP32 | Antarmuka `esp-hal` | Keterangan |
|---|---|---|---|
| Sharp GP2Y1010AU0F `Vo` | `GPIO34` (`ADC1_CH6`) | `Adc` (`Attenuation::_11dB` / rentang penuh) | Divider `10k/20k`, `DIV_RATIO = 1.5` |
| MQ-135 `AO` | `GPIO35` (`ADC1_CH7`) | `Adc` (`Attenuation::_11dB` / rentang penuh) | Divider `10k/20k`, `DIV_RATIO = 1.5` |
| Sharp LED IR | `GPIO4` | `Output` (aktif LOW) | Default `HIGH` (mati) sedini mungkin |
| MOSFET Kipas PWM | `GPIO18` | `Ledc` (`25 kHz`, resolusi `8-bit`) | Default `0%` sedini mungkin |
| Buzzer Aktif | `GPIO23` | `Output` | Dikendalikan `buzzer_task` tiap `20 ms` |
| LED Hijau / Kuning / Merah | `GPIO25` / `GPIO26` / `GPIO27` | `Output` | Hijau ON saat boot (`NORMAL`) |
| **RTC I2C (`DS3231` / `DS1307`)** | **`GPIO21` (SDA), `GPIO22` (SCL)** | `I2c` Master (`100 kHz`, addr `0x68`) | Pull-up internal/eksternal; fallback aman jika absen |

### 4.2 Arsitektur Task Embassy (Single-Core Core 0) & Sinkronisasi
Seluruh task berjalan di **Core 0** di bawah `esp_rtos::embassy::Executor`. Menjalankan satu core scheduler menghindarkan *core parking* CPU1 saat menulis flash `esp-storage`, sekaligus menghilangkan perlombaan akses memori mentah:

- **`control_task`**:
  - Mode `RIIL`: membaca ADC Sharp (8 pulsa: *busy-wait* singkat hanya `280 µs` + `40 µs` saat LED IR menyala, lalu **`Timer::after_micros(9680).await`** saat LED mati agar task jaringan dan buzzer tetap berjalan mulus) dan MQ-135 (16 sampel rata-rata dengan `Timer::after_millis(1).await`).
  - Mode `HILS`: menunggu sampel dari `HILS_CH` (`Channel<NoopRawMutex, HilsIn, 4>`) dengan timeout/watchdog `5000 ms`.
  - Menjalankan `karhutla-core` (`step`), memperbarui PWM LEDC & LED indikator, memperbarui `BUZZER_STATE: AtomicU8` (`bit0..1 = state`, `bit7 = storage_full`), mengirim snapshot ke `TEL_SIG: Signal` jika `MQTT_CONNECTED == true`, atau mengirim `BackupRecord` ke `BACKUP_CH: Channel<_, BackupRecord, 8>` jika `MQTT_CONNECTED == false`.
- **`buzzer_task`**: bangun tiap `20 ms` (`Ticker::every(Duration::from_millis(20))`), membaca `BUZZER_STATE: AtomicU8`, dan menggerakkan `GPIO23` sesuai tabel pola komposit deterministik berbasis fase milidetik.
- **`storage_rtc_task`**: satu-satunya pemilik eksklusif `FlashStorage` dan bus `I2c` RTC (`0x68`)—sehingga tidak perlu mutex I2C maupun mutex flash.
  - Saat boot: memindai partisi `logs` (`0x150000..0x3F0000`) dengan *binary search* (`≤ 16` baca flash, `< 2 ms`) untuk menetapkan `log_count` dan `log_full`, lalu membaca jam RTC awal. CRC-32 agregat dihitung secara *lazy* saat permintaan `META` pertama dan diperbarui inkremental setiap append baru.
  - Melayani penulisan record dari `BACKUP_CH` ketika offline (`write_nor` 64 byte), permintaan sinkronisasi waktu (`TIME_SYNC_SIG`), serta perintah `META` / `GET` / `CLEAR` dari `LOG_REQ_CH` dan mengembalikan hasilnya lewat `LOG_RESP_CH`.
- **`wifi_task` & `net_task`**: mengelola koneksi STA (`Hotspot LSKK`), `PowerSaveMode::None`, DHCPv4 `embassy-net`, dan reconnect otomatis.
- **`mqtt_task`**: klien MQTT 3.1.1 QoS0 di atas `embassy_net::tcp::TcpSocket` (`CONNECT` dengan LWT `karhutla/status="offline"`, `SUBSCRIBE` ke `karhutla/cmd`, `karhutla/time/sync`, `karhutla/log/req`, serta `karhutla/hils/sensor` pada mode `HILS`). Mem-publish `karhutla/telemetry` + `karhutla/hb` (`1 Hz`) + chunk log bila diminta, serta memelihara `MQTT_CONNECTED: AtomicBool`.

### 4.3 Struktur Direktori Rencana di Branch `feat/embassy-rust-firmware`
```text
firmware/karhutla_esp32_rs/
├── .cargo/config.toml
├── Cargo.toml                  # workspace: karhutla-core + karhutla-esp32
├── rust-toolchain.toml         # channel = "esp"
├── partitions.csv              # nvs 24K | factory 1.25M | logs 2.625M
├── crates/
│   └── karhutla-core/          # #![no_std], host-testable (cargo test)
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs          # config constants, Median3, Kalman2, FSM, Firmware
│           ├── buzzer.rs       # pola buzzer normal & penuh-komposit
│           ├── record.rs       # BackupRecord 64-byte encode/decode + CRC-32
│           └── rtc.rs          # DS3231/DS1307 BCD <-> Unix epoch helper
└── firmware/                   # crate binari esp32 (karhutla-esp32)
    ├── Cargo.toml              # features: default=["hils"], hils=[], riil=[]
    ├── build.rs                # -Tlinkall.x
    └── src/
        ├── main.rs
        ├── mqtt.rs             # MQTT 3.1.1 QoS0 + LWT + topic router
        ├── storage.rs          # binary search + append 64B + chunk reader + erase
        └── hardware.rs         # GPIO, LEDC PWM, ADC+cali, I2C RTC driver
```

### 4.4 Strategi Verifikasi
1. **Uji Paritas Host (`cargo test -p karhutla-core`):** Menguji keluaran numerik `karhutla-core` langkah-demi-langkah terhadap `core.py` (skenario `flaming`, `smolder`, `clean`, `fault`), pola buzzer 8 kombinasi `(full, state)`, codec `BackupRecord` + CRC-32, serta konversi BCD RTC.
2. **Build Target ESP32 (`cargo +esp build --release`):** Memverifikasi kedua varian fitur (`--features hils` dan `--features riil`) terkompilasi tanpa error dan ukuran `.bin` muat nyaman di partisi `factory` (`1.280 KiB`).
3. **Uji Website & Protokol (`tests/test_mqtt_and_dashboard.py`):** Menambah pengujian otomatis untuk `POST /api/esp-log/sync`, verifikasi CRC-32 + deduplikasi CSV harian, gerbang `HTTP 409 Conflict` pada `POST /api/esp-log/clear` sebelum sync+unduh, dan terbukanya izin hapus setelah `/api/esp-log/download`.
4. **Uji Hardware Langsung (`/dev/ttyUSB0`):** Menguji HILS lock-step + paritas `core.py`, pemutusan broker untuk memicu tulis offline ke partisi `logs`, pengecekan status penuh & pola buzzer, sinkronisasi+unduh via browser, dan penghapusan memori ESP32.

---

## 5. Hal yang Diserahkan ke Builder (What's Left to the Builder)

- Detail boilerplate inisialisasi driver `esp-hal 1.2.2` untuk LEDC PWM 25 kHz, ADC1 one-shot (`GPIO34`/`GPIO35`) + pembacaan kalibrasi eFuse ESP32, serta transaksi I2C Master `embedded-hal` untuk `DS3231`/`DS1307`.
- Penyusunan CSS/HTML kartu **"Memori Backup ESP32 & RTC"** di `dashboard.py` agar mengikuti palet, tipografi, dan tata letak responsif yang sudah ada.
- Penyetelan nilai konstanta kecil non-algoritmik seperti jeda *backoff* reconnect Wi-Fi/MQTT dan jumlah sektor per jeda `.await` saat perintah `CLEAR` menghapus flash agar tidak memicu *watchdog*.
