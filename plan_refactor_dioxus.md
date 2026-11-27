# Rencana Migrasi Karhutla Edge-IoT ke Full Rust + Dioxus 0.7 (Web, Desktop, Android + MQTT `:6669`)

Dokumen ini merangkum seluruh keputusan arsitektur yang disepakati melalui sesi `/grill-me` untuk memigrasikan antarmuka web, aplikasi desktop, aplikasi Android, serta engine simulasi/backend Karhutla Edge-IoT ke **Full Rust + Dioxus 0.7** dengan tetap mempertahankan **MQTT (`:6669`)** dan **`karhutla-core`** sebagai satu sumber kebenaran (*single source of truth*) lintas ESP32, SITL, dan HILS.

---

## 1. Keputusan Arsitektur Terkunci

1. **Arsitektur Inti & Protokol MQTT:**
   - **Full Rust + Dioxus 0.7 Fullstack** (`Axum` HTTP/SSE/WebSocket + `rumqttc` ke Mosquitto TCP `:6669`).
   - Runner **SITL** dan firmware referensi **HILS** langsung menggunakan crate [`firmware/karhutla_esp32_rs/crates/karhutla-core`](firmware/karhutla_esp32_rs/crates/karhutla-core) (`no_std`: `Median3`, adaptasi `v0`, `Kalman2`, Sensor Fusion Index `FI`, `HysteresisFSM`, `FAILSAFE` 70%, dan `BackupRecord` 64-byte CRC-32) — nol duplikasi logika DSP.
   - Seluruh topik MQTT yang ada di `config.py` & firmware ESP32 dipertahankan 100% kompatibel (`karhutla/telemetry`, `karhutla/t/*`, `karhutla/cmd`, `karhutla/hils/sensor`, `karhutla/forecast`, `karhutla/f/*`, `karhutla/status`, `karhutla/hb`, `karhutla/time/sync`, `karhutla/log/req`, `karhutla/log/meta`, `karhutla/log/chunk`, `karhutla/log/ack`).

2. **Isolasi Workspace & Transisi Aman dari Python:**
   - Workspace Rust host/app dibuat terpisah di **`karhutla_app/`** dan mereferensikan `karhutla-core` via *path dependency* relatif (`../firmware/karhutla_esp32_rs/crates/karhutla-core`).
   - [`firmware/karhutla_esp32_rs/`](firmware/karhutla_esp32_rs/) tetap terisolasi agar `.cargo/config.toml` (`target = "xtensa-esp32-none-elf"`, `build-std`) tidak berbenturan dengan target `x86_64`, `wasm32-unknown-unknown`, maupun `aarch64-linux-android`.
   - `dashboard.py`, `core.py`, `sitl.py`, `hils.py`, dan `tests/test_mqtt_and_dashboard.py` dipertahankan utuh sebagai referensi uji paritas & *fallback* sampai seluruh fitur Dioxus terverifikasi 100%.

3. **Topologi Koneksi Klien (`TransportClient` Trait):**
   - **Desktop (Linux/Windows):** Berjalan sebagai **Embedded Host + UI sekaligus** — otomatis menjalankan `KarhutlaEngine`, menghubungkan `rumqttc` ke `:6669` (auto-start Mosquitto lokal bila belum aktif), serta membuka server HTTP/WS LAN (`:8080`) agar browser laptop/HP lain bisa langsung tersambung. Tetap menyediakan opsi sambung ke Host Server LAN lain.
   - **Web (WASM):** Klien tipis yang terhubung ke Host Server LAN (`http://<IP_LAN>:8080`) melalui HTTP + WebSocket/SSE (karena browser WASM tidak dapat membuka soket TCP `:6669` mentah).
   - **Android (APK):** Default terhubung ke **Host Server LAN (`http://<IP_LAN>:8080`)** untuk akses penuh (SITL, HILS, Backup Flash 43.008 slot, ekspor CSV/Log), ditambah **toggle Mode Direct MQTT (`<IP_BROKER>:6669`)** via `rumqttc` untuk pantauan & override ESP32 langsung di lapangan meski laptop dimatikan.

4. **Engine Grafik & Bahasa Visual:**
   - **SVG Reaktif Murni (RSX Dioxus):** 100% Rust tanpa blok JS charting — tajam di semua DPI (proyektor & HP), dual-lane (`#main-chart` + `#sub-chart`), 9 kurva overlay, zona `+60s` (`SEKARANG → +60s` / slate netral saat `FAILSAFE`), arsiran diagonal `<pattern>` untuk `FAILSAFE`, Duty PWM + garis `70% FAILSAFE` + pita 4-state FSM, crosshair tersinkron (pointer & keyboard), serta ekspor SVG/PNG.
   - **Paritas Fitur Penuh + Peningkatan UX Run 12 Sejak Awal:**
     - Seluruh fitur `dashboard.py`: 6 kartu KPI, kontrol SITL/HILS (`1x–20x`, 4 skenario, `AI Preempt`, `Injeksi Fault`), Override (`AUTO`/`FORCE_MAX`/`FORCE_OFF`/`RESET` + pengunci sasaran progresif), panel **Memori Backup ESP32 & RTC** (43.008 slot, `sync` → `download` CSV → `clear` 2-fase `HTTP 409`), live MQTT log `:6669`, ekspor `IoT MQTT Panel (.json)`, Glosarium 3 akordeon, dan tema **Proyektor / Gelap / Sistem** pas `1280×768` tanpa *scroll* (CSS token lokal via `asset!()`, 100% offline tanpa CDN).
     - Peningkatan UX Run 12 langsung terintegrasi:
       - Mobile `390×844`: grafik utama lega (`>=170–180px` + sub-chart `~92px`), toolbar grafik 1 baris ringkas, legenda 1 baris dengan petunjuk gulir tepi, status dock ringkas anti-potong, tanpa duplikasi `#sim-panel`.
       - Mode `FAILSAFE` lebih tenang: aksen magenta penuh hanya di banner & kartu dominan `STATE FSM`, 3 kartu terdampak (`FI`, `PM`, `Prakiraan`) berborder putus-putus netral + status muted, kartu `Aktuator Kipas PWM (70%)` memakai warna pengaman aktif (`--accent`/`--ok`).
       - Alur sidang cepat: `Berhenti` langsung tanpa modal pemblokir (menampilkan `Ringkasan Simulasi`), tombol koreksi cepat 1-klik pada peringatan mismatch sasaran perintah, `Paritas HILS` tunggal tanpa memecah header, verifikasi `r.ok` sebelum toast ekspor, serta copy bebas referensi hantu `--no-mqtt`.

---

## 2. Struktur Workspace `karhutla_app/`

```text
karhutla_app/
├── Cargo.toml
├── crates/
│   ├── karhutla-protocol/    # Tipe wire JSON, enum MQTT/Sim/Cmd/BackupLog, ServerEvent, IoT MQTT Panel JSON (WASM-safe)
│   ├── karhutla-sim/         # SmokeChamber (plant), LinearTrendForecaster, PreemptController, DailyCsv/MqttLogger, ESP32 BackupLog decoder (pakai karhutla-core)
│   └── karhutla-server/      # Library + CLI binary: KarhutlaEngine, rumqttc (:6669) + auto-Mosquitto, Axum HTTP/SSE/WS router
└── apps/
    └── karhutla-ui/          # (Fase 2–3) Dioxus 0.7: Web WASM + Desktop (Embedded Host) + Android (Remote + Direct MQTT)
```

---

## 3. Roadmap Bertahap & Gerbang Verifikasi

### Fase 1 — Engine Simulasi & Backend Rust (`karhutla-protocol`, `karhutla-sim`, `karhutla-server`)
- [x] Desain kontrak protokol (`karhutla-protocol`) kompatibel 100% dengan `/api/*` & `wasm32-unknown-unknown`.
- [x] Porting `plant.py`, `forecast.py`, `logger.py`, serta dekoder 64-byte `BackupRecord` & arsip CSV (`karhutla-sim`) menggunakan `karhutla-core`.
- [x] Implementasi `KarhutlaEngine` (SITL, HILS lock-step + retry + paritas, watchdog ESP32, RTC time-sync, backup log 2-fase `409`), jembatan `rumqttc` (`:6669`), dan server Axum HTTP/SSE/WebSocket (`karhutla-server`).
- **Gerbang Verifikasi Fase 1:**
  - `cargo test --workspace` di `karhutla_app/` lulus 100% (unit test `plant`/`forecast`/`logger`/`esp_log` + integrasi HTTP/SSE/sync-clear `409`→`200`).
  - `cargo check -p karhutla-protocol --target wasm32-unknown-unknown` lulus tanpa error.
  - `cargo test --target x86_64-unknown-linux-gnu -p karhutla-core` di `firmware/karhutla_esp32_rs/` tetap lulus `11/11`.

### Fase 2 — UI Dioxus 0.7 (Web WASM + Desktop Linux Embedded Host)
- [x] Buat `karhutla_app/apps/karhutla-ui` dengan CSS token offline (`Proyektor` / `Gelap` / `Sistem`).
- [x] Bangun abstraksi `TransportClient` (`EmbeddedHostTransport` untuk Desktop + `RemoteServerTransport` untuk Web/Mobile).
- [x] Implementasikan 6 KPI + `#abbr-popover`, grafik dual-lane SVG reaktif murni + crosshair + ekspor SVG/PNG, panel Simulasi/Override, panel Memori Backup ESP32 & RTC, live MQTT log, Glosarium 3 akordeon, dan tata letak responsif `1280×768` & `390×844` (termasuk seluruh perbaikan Run 12).
- **Gerbang Verifikasi Fase 2:**
  - `dx build --platform web` & `dx build --platform desktop` lulus bersih.
  - Verifikasi visual + CDP `impeccable detect` pada `1280×768` & `390×844` (Light & Dark, Idle/BAHAYA/FAILSAFE) dengan `0` temuan (`[]`).

### Fase 3 — Mode Direct MQTT Lapangan & Build Android APK
- [x] Tambahkan `DirectMqttTransport` (`rumqttc` TCP `:6669`) + saklar mode koneksi di pengaturan aplikasi Mobile/Desktop (`src/direct_mqtt.rs`, `BackendTarget::DirectMqtt`, `ConnectionPrefs`, modal `#conn-dialog`).
- [x] Siapkan target `aarch64-linux-android`, skrip/CI Android SDK+NDK (`.github/workflows/build-apps.yml` & `karhutla_app/scripts/build_android_apk.sh`), ikon & `[bundle]` `Dioxus.toml`; uji lokal `cargo check -p karhutla-ui --no-default-features --features mobile`, target `aarch64-linux-android` untuk `karhutla-protocol`/`karhutla-sim`, serta uji live roundtrip Mosquitto lokal (`dx bundle --platform android` dijalankan di CI atau lokal setelah `ANDROID_HOME`/`ANDROID_NDK_HOME` terpasang).
- [x] Perbarui `PRODUCT.md` dan `README.md` setelah paritas penuh tercapai.
