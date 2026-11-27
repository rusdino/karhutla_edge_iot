# Product

<!-- impeccable:product-schema 1 -->

## Platform

web, desktop (Embedded Host Linux/macOS/Windows), android (Dioxus 0.7 Full Rust UI)

## Users

Mahasiswa/peneliti serta dosen penguji saat sidang tugas besar, eksperimen laboratorium kotak asap (`SITL`/`HILS`), maupun pemantauan lapangan (`Riil`) secara langsung.

## Product Purpose

Karhutla Edge-IoT adalah sistem prediksi dini dan mitigasi asap kebakaran hutan/lahan (karhutla) berbasis Edge-IoT dan AI. Aplikasi terpadu Rust/Dioxus (`karhutla_app/`) beserta skrip referensi Python (`dashboard.py`) berfungsi sebagai pusat kendali dan telemetri *real-time* untuk menjalankan simulasi ruang asap, menguji paritas ESP32 fisik secara *lock-step*, memverifikasi transisi FSM serta kondisi `FAILSAFE`, dan mendemonstrasikan mitigasi kipas proaktif berbasis prediksi tren.

## Positioning

Satu basis algoritma DSP dan kontrol yang 100% identik antara pustaka Rust `karhutla-core` (`firmware/karhutla_esp32_rs/crates/karhutla-core/`, dipakai oleh firmware ESP32 Rust `no_std` + Embassy maupun simulator `karhutla-sim` / `karhutla-server`), implementasi referensi Python (`core.py`), serta firmware fallback Native ESP-IDF v5.5.5 (`firmware/karhutla_esp32/src/main.cpp`, tanpa Arduino Core) — mencakup filter `Median3`, adaptasi baseline `v0`, Kalman Filter 2-state (`pm`, `dcdt`), Sensor Fusion Index (`FI`), FSM 4-state (`NORMAL`, `WASPADA`, `BAHAYA`, `FAILSAFE` 70%), 8 pola buzzer komposit (4 state FSM × status memori penuh), RTC eksternal I2C (`DS3231/DS1307`), pencatatan cadangan (*backup logger*) flash lokal 2,625 MiB (43.008 sampel × 64 byte), serta AI Linear Trend Forecast 60 detik dan `PreemptController` — yang dapat diakses dari **Web WASM**, **Desktop Embedded Host**, maupun **Aplikasi Android (`Direct MQTT :6669` / `Server LAN :8080`)**.

## Operating Context

- Digunakan dalam sesi demo sidang tugas besar serta uji laboratorium kotak asap pada jaringan LAN lokal / hotspot 2.4 GHz (tanpa ketergantungan internet).
- **Mode Host Lokal / Server LAN (`:8080`):** Aplikasi Desktop (`karhutla-ui --features desktop`) atau `karhutla-server` menjalankan `KarhutlaEngine`, Mosquitto `:6669`, dan server HTTP/WebSocket `:8080` (sekaligus menyajikan bundel Web WASM untuk browser laptop & HP di Wi-Fi yang sama).
- **Mode Direct MQTT (`:6669`):** Aplikasi Desktop atau Android APK (`karhutla-ui --features mobile`) terhubung langsung ke broker Mosquitto lapangan via TCP `:6669` (`rumqttc`) tanpa host HTTP laptop untuk pemantauan telemetri *real-time*, ekspor CSV/log lokal, dan override cepat `karhutla/cmd`.
- Seluruh sesi di host dicatat otomatis secara harian ke `logs/karhutla_<sumber>_YYYY-MM-DD.csv` dan `logs/mqtt_YYYY-MM-DD.log`; klien opsional `IoT MQTT Panel` juga tetap didukung lewat ekspor `karhutla_iot_mqtt_panel.json`.

## Capabilities and Constraints

- **Offline-first & Tanpa CDN Eksternal:** Seluruh aset UI (`assets/karhutla.css`), WebAssembly, dan biner Desktop/Mobile berjalan 100% lokal di LAN/perangkat tanpa memanggil skrip/font/CDN eksternal.
- **Bahasa & Terminologi:** Seluruh label dan pesan UI menggunakan Bahasa Indonesia dengan istilah teknis yang akurat (`FI`, `PM-proxy`, `Kalman`, `FSM`, `NORMAL`, `WASPADA`, `BAHAYA`, `FAILSAFE`, `HILS`, `SITL`, `AI Preempt`, `AUTO`, `FORCE_MAX`, `FORCE_OFF`, `RESET`).
- **Kendali Cepat Multi-Perangkat:** Tata letak desktop `1280×768` tanpa *scroll* vertikal untuk proyektor dan *bottom dock* mobile `390×844`, lengkap dengan saklar **Mode Koneksi (`Host Lokal` / `Server LAN :8080` / `Direct MQTT :6669`)**.
- **Fitur Terkonfirmasi:** Grafik SVG multi-overlay *real-time*, 6 kartu KPI, kontrol override aktuator (`karhutla/cmd`), operasi simulasi terpadu untuk target `SITL (Rust)` dan `HILS (ESP32)` (`1x`–`20x`, skenario `flaming`/`smolder`/`clean`/`fault`, saklar `AI Preempt` & injeksi putus kabel sensor `FAILSAFE`), panel **Memori Backup ESP32 & RTC** (status RTC & kapasitas 43.008 sampel, sinkronisasi & unduh CSV `/api/esp-log/sync` → `/api/esp-log/download`, penghapusan dua fase aman dengan gerbang `HTTP 409`), ekspor SVG/PNG/CSV/Log, serta live MQTT log `:6669`.
- **Batasan Operasional:** Saat menjalankan target `HILS (ESP32)` dari UI host/server, jangan menjalankan runner HILS terpisah (`hils.py`) di terminal secara bersamaan. Di browser Web WASM, `Direct MQTT (:6669)` dinonaktifkan secara aman karena sandbox browser tidak membuka raw TCP socket — gunakan mode `Server LAN (:8080)` di browser atau aplikasi Desktop/Android APK untuk Direct MQTT. Pembundelan lokal APK Android membutuhkan `ANDROID_HOME` & Android NDK (`ANDROID_NDK_HOME`), dan tersedia otomatis di workflow CI `.github/workflows/build-apps.yml`.
- **Belum Diputuskan / Di Luar Cakupan Saat Ini:** Deployment cloud publik internet, broker MQTT berenkripsi TLS mTLS publik, serta autentikasi multi-pengguna.

## Brand Commitments

- **Nama Produk:** Karhutla Edge-IoT.
- **Suara & Copy:** Teknis-akademik berbahasa Indonesia yang lugas, akurat, dan berbasis data terukur tanpa klaim pemasaran berlebihan.

## Evidence on Hand

- Pengujian Rust & UI: `karhutla-core` test lulus, `karhutla_app` workspace test (`karhutla-protocol`, `karhutla-sim`, `karhutla-server` API integration, serta `karhutla-ui` termasuk uji live roundtrip Mosquitto Direct MQTT) lulus, Web WASM (`dx build --platform web`) & Desktop Linux (`dx build --platform desktop`) terkompilasi, plus audit CDP Chromium & `impeccable detect --json` (`0` temuan) pada `1280×768` dan `390×844`.
- Referensi Python & visual: `tests/test_mqtt_and_dashboard.py`, `sitl_*.png`, `firmware/karhutla_esp32_rs/`, `firmware/karhutla_esp32/src/main.cpp`, `karhutla_iot_mqtt_panel.json`.
- **Ketiadaan Bukti (jangan difabrikasi):** Build biner `.apk` siap pasang dilakukan di GitHub Actions CI (`ubuntu-24.04` dengan Android SDK/NDK) karena mesin pengembangan ini belum memasang Android NDK; belum ada data pengujian kebakaran hutan skala penuh atau sertifikasi industri.

## Product Principles

1. **Akurasi & Transparansi Data:** Setiap angka KPI, status FSM, dan kurva grafik merefleksikan data aktual `karhutla-core`/`core.py`/ESP32 tanpa menyamarkan gangguan koneksi atau kondisi `FAILSAFE`.
2. **Kendali Cepat saat Demo & Eksperimen:** Kontrol kritis (`AUTO`, `FORCE_MAX`, `FORCE_OFF`, `RESET`, mulai/jeda/kecepatan `1x`–`20x`, perpindahan target `SITL`/`HILS`) dapat diakses cepat di laptop maupun HP.
3. **Mandiri & Offline di LAN maupun Lapangan:** Tanpa CDN internet; mendukung Host Lokal/LAN (`:8080`) sekaligus koneksi Direct MQTT (`:6669`) langsung di lapangan.
4. **Kejelasan Status Hardware & Keselamatan:** Pemisahan sumber data (`SITL` vs `ESP32`), deteksi `ESP32` offline, dan status `FAILSAFE` selalu tegas dan tidak ambigu.
