# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Mahasiswa/peneliti serta dosen penguji saat sidang tugas besar dan eksperimen laboratorium kotak asap menggunakan mode `SITL`, `HILS`, maupun `Riil`.

## Product Purpose

Karhutla Edge-IoT adalah sistem prediksi dini dan mitigasi asap kebakaran hutan/lahan (karhutla) berbasis Edge-IoT dan AI. Antarmuka web (`dashboard.py`) berfungsi sebagai pusat kendali dan telemetri *real-time* untuk menjalankan simulasi ruang asap, menguji paritas ESP32 fisik secara *lock-step*, memverifikasi transisi FSM serta kondisi `FAILSAFE`, dan mendemonstrasikan mitigasi kipas proaktif berbasis prediksi tren.

## Positioning

Satu basis algoritma DSP dan kontrol yang 100% identik antara Python (`core.py`) dan firmware Native ESP-IDF v5.5.5 (`firmware/karhutla_esp32/src/main.cpp`, tanpa Arduino Core) — mencakup filter `Median3`, adaptasi baseline `v0`, Kalman Filter 2-state (`pm`, `dcdt`), Sensor Fusion Index (`FI`), FSM 4-state (`NORMAL`, `WASPADA`, `BAHAYA`, `FAILSAFE` 70%), serta AI Linear Trend Forecast 60 detik dan `PreemptController` — yang dapat dijalankan lintas 3 mode (`SITL`, `HILS` via MQTT `:6669`, dan `Riil`) dari satu dashboard web terpadu.

## Operating Context

- Digunakan dalam sesi demo sidang tugas besar serta uji laboratorium kotak asap pada jaringan LAN lokal / hotspot 2.4 GHz (sering kali tanpa akses internet).
- Diakses melalui browser laptop (`http://localhost:8080`) dan browser HP pada jaringan LAN yang sama (`http://<IP_LAN>:8080`), terhubung ke broker Mosquitto lokal (`:6669`), dengan klien mobile opsional `IoT MQTT Panel` melalui ekspor konfigurasi `karhutla_iot_mqtt_panel.json`.
- Seluruh sesi dicatat otomatis secara harian ke `logs/karhutla_<sumber>_YYYY-MM-DD.csv` dan `logs/mqtt_YYYY-MM-DD.log`.

## Capabilities and Constraints

- **Offline-first & Zero-build:** Antarmuka web wajib tetap berjalan lokal/offline di LAN tanpa CDN internet dan tanpa toolchain build Node/React (tetap single-file Python/HTML di `dashboard.py`).
- **Bahasa & Terminologi:** Seluruh label dan pesan UI wajib menggunakan Bahasa Indonesia dengan istilah teknis yang akurat (`FI`, `PM-proxy`, `Kalman`, `FSM`, `NORMAL`, `WASPADA`, `BAHAYA`, `FAILSAFE`, `HILS`, `SITL`, `AI Preempt`, `AUTO`, `FORCE_MAX`, `FORCE_OFF`, `RESET`).
- **Kendali Cepat Multi-Perangkat:** Harus nyaman dipantau dan dikendalikan cepat saat demo langsung di layar laptop maupun browser HP.
- **Fitur Terkonfirmasi:** Grafik multi-overlay *real-time*, 6 kartu KPI, kontrol override aktuator (`karhutla/cmd`), operasi simulasi terpadu untuk target `SITL (Python)` dan `HILS (ESP32)` (`1x`–`20x`, skenario `flaming`/`smolder`/`clean`/`fault`, saklar `AI Preempt` & injeksi putus kabel sensor `FAILSAFE`), deteksi status `ESP32` online/offline otomatis, live MQTT log `:6669`, dan unduh konfigurasi HP `.json`.
- **Batasan Operasional:** Saat menjalankan target `HILS (ESP32)` di `dashboard.py`, jangan menjalankan `hils.py` di terminal secara bersamaan agar kedua runner tidak berebut mengirim sampel ke ESP32 yang sama.
- **Belum Diputuskan / Di Luar Cakupan Saat Ini:** Deployment cloud/publik internet, autentikasi multi-pengguna, dan operasional lapangan di luar uji laboratorium kotak asap.

## Brand Commitments

- **Nama Produk:** Karhutla Edge-IoT.
- **Suara & Copy:** Teknis-akademik berbahasa Indonesia yang lugas, akurat, dan berbasis data terukur tanpa klaim pemasaran berlebihan.

## Evidence on Hand

- Grafik validasi SITL 3-panel aktual: `sitl_flaming.png`, `sitl_fault.png`, `sitl_smolder.png`, `sitl_clean.png`.
- Screenshot dashboard & klien mobile aktual: `logs/dashboard_ss_terbaru.png`, `logs/another_dashboard_ss_terbaru.png`, `logs/mqtt_dashboard_ss.jpeg`.
- Pengujian otomatis: `tests/test_mqtt_and_dashboard.py` (26/26 unit test lulus).
- Firmware & konfigurasi aktual: `firmware/karhutla_esp32/src/main.cpp`, `README.md`, `karhutla_iot_mqtt_panel.json`.
- **Ketiadaan Bukti (jangan difabrikasi):** Belum ada data pengujian lapangan hutan karhutla skala penuh, testimoni pengguna eksternal, maupun sertifikasi industri.

## Product Principles

1. **Akurasi & Transparansi Data:** Setiap angka KPI, status FSM, dan kurva grafik harus merefleksikan data aktual `core.py`/ESP32 tanpa menyamarkan gangguan koneksi atau kondisi gagal.
2. **Kendali Cepat saat Demo & Eksperimen:** Kontrol kritis (`AUTO`, `FORCE_MAX`, `FORCE_OFF`, `RESET`, mulai/jeda/kecepatan `1x`–`20x`, perpindahan target `SITL`/`HILS`) harus dapat diakses cepat di laptop maupun HP.
3. **Mandiri & Offline di LAN:** Tanpa ketergantungan CDN internet atau proses build kompleks agar sistem selalu siap pakai di ruang sidang maupun lab.
4. **Kejelasan Status Hardware & Keselamatan:** Pemisahan sumber data (`SITL` vs `ESP32`), deteksi `ESP32` offline, dan status `FAILSAFE` harus selalu tegas dan tidak ambigu.
