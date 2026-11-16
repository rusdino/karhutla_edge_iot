# Desain: Plot 3-Panel (PM-proxy + Fusion Index, Sensor Gas MQ-135 PPM & Skor g, Aktuator & State)

## 1. Tujuan & Alasan (Intent & Why)
- Menampilkan dinamika sensor gas MQ-135 secara utuh pada grafik `--plot` di `sitl.py` (konsentrasi gas sebenarnya di ruang asap `gas_true`, estimasi konsentrasi `gas_ppm` dari sensor MQ-135, dan skor gas ternormalisasi `g`), serta menampilkan kurva `Fusion Index (FI)` di panel pertama.

## 2. Cakupan & Batasan (Goals, Non-Goals, Constraints)
- **Goals:**
  1. Menyimpan estimasi `ppm` (`self.ppm`, bernilai `MQ_ATM_PPM` selama kalibrasi `r0 is None`, lalu hasil `MQ_PARA * (rs / self.r0) ** (-MQ_PARB)`) di `core.py` (`tel["gas_ppm"]`) dan `karhutla_esp32.ino` (`gasPpm`) agar tetap sinkron.
  2. Menyimpan `gas_true=plant.gas_ppm` dan `gas_ppm=tel["gas_ppm"]` pada setiap baris simulasi di `sitl.py`.
  3. Mengubah `make_plot()` di `sitl.py` menjadi 3 panel (`figsize=(10, 8.5)`):
     - Panel 1: PM sebenarnya, PM mentah (median), PM Kalman, Fusion Index (FI), prakiraan t+h, dan garis ambang WASPADA/BAHAYA.
     - Panel 2: Sumbu Y kiri (`ppm`) untuk Gas sebenarnya (plant) vs Gas estimasi MQ-135; Sumbu Y kanan (`twinx`, `0–1.05`) untuk Skor gas `g` (EMA).
     - Panel 3: Duty kipas (%) dan State FSM (`x25`).
  4. Menampilkan metrik ringkasan Gas puncak & RMSE gas di `report.py` serta memperbarui keterangan di `README.md`.
- **Non-Goals:** Tidak mengubah parameter di `config.py` / `config.h` dan tidak mengubah keputusan kontrol/state machine.

## 3. Pendekatan (Approaches)
- Dipilih **Pendekatan A**: 3 panel dengan *dual-axis* (`twinx`) pada Panel 2 untuk memisahkan skala `ppm` dan skor `g` (`0–1`).
