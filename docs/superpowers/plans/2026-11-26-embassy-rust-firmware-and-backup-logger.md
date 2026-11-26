# Rust Embassy Firmware, RTC I2C, & Backup Logger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a `no_std` Rust + Embassy ESP32 firmware (`firmware/karhutla_esp32_rs/`) with 100% DSP/FSM parity to `core.py`, DS3231/DS1307 I2C RTC support, `2.625 MiB` offline-only flash backup logging (`43,008` × 64-byte records), composite full-memory buzzer patterns, and a mandatory sync+download gate in `dashboard.py` before clearing ESP32 flash.

**Architecture:** A two-crate Cargo workspace (`karhutla-core` host-testable `#![no_std]` logic + `karhutla-esp32` Embassy single-core runtime on Core 0) communicates over Mosquitto `:6669` using a custom MQTT 3.1.1 QoS0 client with LWT and Base64 chunked log transfer. `dashboard.py` enforces bit-exact CRC-32 verification, dual CSV persistence (daily archival + full batch download), and an HTTP `409 Conflict` gate on `POST /api/esp-log/clear` until `synced && downloaded`.

**Tech Stack:** Rust `esp` toolchain (`rustc 1.97.0-nightly`, `espflash 4.7.0`), `esp-hal 1.2.2`, `esp-rtos 0.4.0`, `esp-radio 1.0.0-beta.1`, `embassy-net 0.9.1`, `esp-storage 0.10.0`, Python 3.12 (`dashboard.py`, `paho-mqtt`, `unittest`).

**Spec:** [2026-11-26-embassy-rust-firmware-and-backup-logger-design.md](file:///home/rusdy/Downloads/karhutla_edge_iot/karhutla/docs/superpowers/specs/2026-11-26-embassy-rust-firmware-and-backup-logger-design.md)

## Global Constraints

- Target hardware: `esp32dev` (Xtensa LX6, 4 MB flash). Partition table `partitions.csv`: `nvs` (`0x9000`, `24 KiB`), `factory` (`0x10000`, `1,280 KiB`), `logs` (`0x150000`, `0x2A0000` / `2,688 KiB` / `2.625 MiB` = `43,008` × 64-byte records).
- Pin map: `GPIO34` (`ADC1_CH6` Sharp `Vo`), `GPIO35` (`ADC1_CH7` MQ-135 `AO`), `GPIO4` (Sharp IR LED active LOW), `GPIO18` (PWM Fan 25 kHz 8-bit), `GPIO23` (Buzzer), `GPIO25`/`26`/`27` (LED G/Y/R), `GPIO21`/`22` (I2C0 SDA/SCL `100 kHz`, `0x68`).
- Network & Offline-First: Wi-Fi `"Hotspot LSKK"` / `"stei.lskk"`, broker `10.8.43.78:6669`, dashboard `:8080`, zero external CDN/build steps, all UI copy in Bahasa Indonesia.
- Never use `rust-mqtt 0.6.0` (only supports MQTT v5; `v3` feature is unused).
- Keep `firmware/karhutla_esp32/` intact so all existing 26 tests continue passing.
- Never allow flash `CLEAR` before verified `sync + download` for the current `(count, crc32)`; never halt sensor/Kalman/FSM/fan control when RTC is absent, Wi-Fi/MQTT is disconnected, or flash is full.
- Note for host tests: `.cargo/config.toml` defaults to `xtensa-esp32-none-elf`, so host tests MUST pass `--target x86_64-unknown-linux-gnu`.

## Review Focus

1. **Torn/corrupt 64-byte flash slot at write boundary after power loss:** `find_write_cursor` and `FlashLogStore` must treat valid `KLOG` + valid CRC-32 slots as used, treat `0xFF` slots as empty, and halt safely without panicking or misreporting capacity on torn slots (pinned in Task 2 & Task 5).
2. **New offline record appended between `sync` and `clear`:** `POST /api/esp-log/clear` checks current `(log_count, crc32)` and rejects with `HTTP 409 Conflict` if stale; ESP32 `CLEAR:<count>:<crc32>` rejects mismatches with `{status:"done",ok:false,reason:"mismatch"}` (pinned in Task 2 & Task 5).
3. **Multi-day offline span & unsynced RTC (`rtc_valid == 0`, `epoch_s == 0`) across power cycles:** Never writes `1970-01-01.csv`; routes unsynced rows to today's archival CSV with `"UNSYNCED+<uptime_ms>ms"`, deduplicates via 4-tuple `(epoch_s, uptime_ms, seq, record_crc32)`, and `/api/esp-log/download` serves the single complete batch CSV (pinned in Task 5).
4. **Float/FSM parity drift between Rust `f32` and Python `core.py`:** `FirmwareCore` must match `core.py` across 900-step trajectories (`|pm_rs - pm_py| < 0.5`, identical states/modes/duties) (pinned in Task 1).
5. **Multi-byte MQTT remaining-length (`>= 128` bytes, e.g. `~1.42 KiB` Base64 chunks) & segmented TCP reads:** `mqtt_codec` and `mqtt.rs` must encode/decode 1–4-byte MQTT varint lengths and LWT `CONNECT` cleanly without truncation (pinned in Task 2 & Task 4).

---

### Task 1: Cargo Workspace & `karhutla-core` DSP/Kalman/FSM Engine

**Files:**
- Create: `firmware/karhutla_esp32_rs/Cargo.toml`
- Create: `firmware/karhutla_esp32_rs/rust-toolchain.toml`
- Create: `firmware/karhutla_esp32_rs/.cargo/config.toml`
- Create: `firmware/karhutla_esp32_rs/partitions.csv`
- Create: `firmware/karhutla_esp32_rs/.gitignore`
- Create: `firmware/karhutla_esp32_rs/crates/karhutla-core/Cargo.toml`
- Create: `firmware/karhutla_esp32_rs/crates/karhutla-core/src/lib.rs`

**Interfaces:**
- Consumes: Algorithm constants from `config.py` / `firmware/karhutla_esp32/include/config.h` and `core.py`.
- Produces in `karhutla_core`:
  - `pub enum AlarmState { Normal = 0, Waspada = 1, Bahaya = 2, Failsafe = 3 }` (`as_str(&self) -> &'static str`)
  - `pub enum OpMode { Auto = 0, ForceMax = 1, ForceOff = 2 }` (`as_str(&self) -> &'static str`)
  - `pub enum CmdMsg { Auto = 0, ForceMax = 1, ForceOff = 2, Reset = 3 }`
  - `pub struct Telemetry { pub seq: u32, pub v_dust: f32, pub v_mq: f32, pub pm_raw: f32, pub pm: f32, pub dcdt: f32, pub gas_ppm: f32, pub g: f32, pub fi: f32, pub v0: f32, pub duty: f32, pub state: AlarmState, pub mode: OpMode, pub alarm: u8 }`
  - `pub struct FirmwareCore` with `pub fn new() -> Self`, `pub fn reset(&mut self) -> Telemetry`, `pub fn apply_command(&mut self, cmd: CmdMsg) -> Telemetry`, `pub fn force_failsafe(&mut self) -> Telemetry`, `pub fn step(&mut self, v_dust: f32, v_mq: f32, seq: u32, dt: f32) -> Telemetry`, `pub fn state(&self) -> AlarmState`, `pub fn mode(&self) -> OpMode`
  - Constants module `pub mod cfg` matching `config.py` (`WIFI_SSID`, `WIFI_PASS`, `MQTT_HOST`, `MQTT_PORT`, topics, `TS_S`, `HILS_WDT_MS`, `DUST_V0`, `DUST_SLOPE`, `KF_Q`, `KF_R`, `T_W_IN`, `T_B_IN`, `MQ_*`, etc.)

- [ ] **Step 1: Scaffold workspace files (`Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`, `partitions.csv`, `.gitignore`) and `karhutla-core/Cargo.toml`**

Use workspace members `["crates/karhutla-core"]` initially (add `"firmware"` in Task 3), `libm = "0.2"` (`no_std` `powf`/`fabsf`/`fminf`/`fmaxf` via `libm::powf`, `f32::abs` is core? No — in `#![no_std]` `f32::abs` and `f32::powf` are NOT in `core`, use `libm::{fabsf, fmaxf, fminf, powf, roundf}`!). Set `partitions.csv` to:
```csv
# Name,   Type, SubType, Offset,   Size,     Flags
nvs,      data, nvs,     0x9000,   0x6000,
factory,  app,  factory, 0x10000,  0x140000,
logs,     data, spiffs,  0x150000, 0x2A0000,
```

- [ ] **Step 2: Write failing unit tests in `crates/karhutla-core/src/lib.rs` (including golden parity vectors generated from `core.py`)**

Run a one-off Python command against `core.py` and `plant.py` (`SmokeChamber("flaming", seed=1)` and `SmokeChamber("fault", seed=1)`) to capture reference checkpoints at steps `[0, 30, 120, 250, 400, 600]`, then write `#[cfg(test)] mod tests`:
- `test_median3_and_kalman2_parity` (assert `|kf.x0 - expected| < 1e-4`, `|kf.x1 - expected| < 1e-4`)
- `test_firmware_core_flaming_parity` (feed `SmokeChamber` or exact `(vd, vm)` sequence, assert state transitions `Normal -> Waspada -> Bahaya` and `|pm_rs - pm_py| < 0.5`, `|fi_rs - fi_py| < 0.5`, identical `duty`)
- `test_firmware_core_fault_and_commands` (5 invalid `0.05` samples -> `Failsafe` duty `70.0`, 5 valid `0.60` samples -> recover, `apply_command(CmdMsg::ForceMax)` -> duty `100.0`, `Reset` -> `Normal` & `Auto`)

- [ ] **Step 3: Run host test to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd firmware/karhutla_esp32_rs && cargo test --target x86_64-unknown-linux-gnu -p karhutla-core`
Expected: FAIL (unimplemented stubs / missing types)

- [ ] **Step 4: Implement `cfg`, `Median3`, `Kalman2`, `HysteresisFSM`, and `FirmwareCore` in `crates/karhutla-core/src/lib.rs`**

Port `core.py` / `firmware/karhutla_esp32/src/main.cpp` 1-to-1 using `libm::{fabsf, fmaxf, fminf, powf}` so `#![no_std]` compiles with zero `std` dependency (`#[cfg_attr(not(test), no_std)]` is NOT needed if tests also work under `#![no_std]` — keep `#![no_std]` unconditionally so we catch any accidental `std` usage!).

- [ ] **Step 5: Run host tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd firmware/karhutla_esp32_rs && cargo test --target x86_64-unknown-linux-gnu -p karhutla-core`
Expected: PASS (all tests green)

- [ ] **Step 6: Commit**

```bash
git add firmware/karhutla_esp32_rs/
git commit -m "feat(rust): scaffold workspace and karhutla-core DSP/FSM engine"
```

---

### Task 2: `karhutla-core` Buzzer, 64B `BackupRecord` + CRC-32 + Base64, RTC BCD, & MQTT 3.1.1 Codec

**Files:**
- Create: `firmware/karhutla_esp32_rs/crates/karhutla-core/src/buzzer.rs`
- Create: `firmware/karhutla_esp32_rs/crates/karhutla-core/src/record.rs`
- Create: `firmware/karhutla_esp32_rs/crates/karhutla-core/src/rtc.rs`
- Create: `firmware/karhutla_esp32_rs/crates/karhutla-core/src/mqtt_codec.rs`
- Modify: `firmware/karhutla_esp32_rs/crates/karhutla-core/src/lib.rs` (declare `pub mod buzzer; pub mod record; pub mod rtc; pub mod mqtt_codec;`)

**Interfaces:**
- Consumes: `AlarmState`, `OpMode`, `Telemetry` from `crate`.
- Produces:
  - `buzzer::buzzer_on(state: AlarmState, storage_full: bool, now_ms: u64) -> bool`
  - `record`:
    - `pub const LOG_PART_OFFSET: u32 = 0x15_0000;`
    - `pub const LOG_PART_SIZE: u32 = 0x2A_0000;`
    - `pub const SECTOR_SIZE: u32 = 4096;`
    - `pub const RECORD_SIZE: usize = 64;`
    - `pub const LOG_CAPACITY: u32 = 43_008;`
    - `pub const FLAG_RTC_VALID: u8 = 0x01; pub const FLAG_MODE_HILS: u8 = 0x02;`
    - `pub struct BackupRecord { pub state: AlarmState, pub mode: OpMode, pub flags: u8, pub epoch_s: u32, pub uptime_ms: u32, pub seq: u32, pub v_dust: f32, pub v_mq: f32, pub pm_raw: f32, pub pm: f32, pub dcdt: f32, pub gas_ppm: f32, pub g: f32, pub fi: f32, pub v0: f32, pub duty: f32 }`
    - `BackupRecord::from_telemetry(t: &Telemetry, epoch_s: u32, uptime_ms: u32, rtc_valid: bool, is_hils: bool) -> Self`
    - `BackupRecord::encode(&self, out: &mut [u8; 64])` & `BackupRecord::decode(raw: &[u8; 64]) -> Result<Self, RecordError>`
    - `pub fn crc32_ieee(data: &[u8]) -> u32` & `pub fn crc32_update(crc: u32, data: &[u8]) -> u32`
    - `pub fn encode_base64(input: &[u8], out: &mut [u8]) -> Result<usize, ()>`
    - `pub fn is_slot_erased(raw: &[u8; 64]) -> bool`
    - `pub fn find_write_cursor<E, F>(capacity: u32, read_slot: F) -> Result<u32, E> where F: FnMut(u32) -> Result<[u8; 64], E>`
  - `rtc`:
    - `pub struct RtcDateTime { pub year: u16, pub month: u8, pub day: u8, pub hour: u8, pub min: u8, pub sec: u8 }`
    - `decode_ds3231_regs(regs: &[u8; 7]) -> Option<RtcDateTime>` & `encode_ds3231_regs(dt: RtcDateTime) -> [u8; 7]`
    - `datetime_to_epoch_utc(dt: RtcDateTime) -> Option<u32>` & `epoch_utc_to_datetime(epoch_s: u32) -> RtcDateTime`
  - `mqtt_codec`:
    - `encode_connect_with_lwt(out: &mut [u8], client_id: &str, keepalive_s: u16, lwt_topic: &str, lwt_msg: &[u8]) -> Result<usize, ()>`
    - `encode_subscribe(out: &mut [u8], packet_id: u16, topic: &str) -> Result<usize, ()>`
    - `encode_publish(out: &mut [u8], topic: &str, payload: &[u8], retain: bool) -> Result<usize, ()>`
    - `parse_publish(hdr: u8, body: &[u8]) -> Option<(&str, &[u8])>`
    - `decode_remaining_length(bytes: &[u8]) -> Option<(usize, usize)>` (returns `(rem_len, bytes_consumed)`)

- [ ] **Step 1: Write failing tests across `buzzer.rs`, `record.rs`, `rtc.rs`, and `mqtt_codec.rs`**

Pin these exact spec & Review Focus cases:
- `test_buzzer_all_8_combinations`: sample timestamps across `[0, 50, 150, 300, 1450, 1600, 1780, 1920, 2200, 2600]` ms for all 4 states × `storage_full in [false, true]` matching exact ms timings from Spec §2.5.
- `test_record_encode_decode_crc_and_corruption`: round-trip 64B `BackupRecord`, verify `magic == *b"KLOG"`, flip 1 bit -> `Err(RecordError::CrcMismatch)`, check `crc32_ieee` against known `"123456789"` == `0xCBF43926` and incremental `crc32_update` equals full `crc32_ieee`.
- `test_find_write_cursor_with_torn_slot` *(Review Focus #1)*: mock flash array where slots `0..5` are valid, slot `5` has torn partial bytes (non-`0xFF` header + bad CRC), and `6..` are `0xFF` -> verify `find_write_cursor` returns `6` (skips torn slot without overwriting or halting at `0`) and handles empty (`0`) and full (`43_008`).
- `test_encode_base64_1024_bytes`: verify 1024 zero/pattern bytes produce 1368 Base64 bytes matching standard alphabet.
- `test_rtc_bcd_and_epoch_roundtrip`: verify `2026-11-26T10:15:30Z`, leap day `2024-02-29T23:59:59Z`, year `2099`, and invalid BCD (`0xFF` -> `None`).
- `test_mqtt_codec_lwt_and_multibyte_varint` *(Review Focus #5)*: encode/decode 1420-byte `PUBLISH` (2-byte varint remaining length), `encode_connect_with_lwt` sets Will flag `0x04` + Will Retain `0x20` + Clean Session `0x02`, and `parse_publish` handles QoS 0 and QoS 1 (skips 2-byte packet ID).

- [ ] **Step 2: Run host tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd firmware/karhutla_esp32_rs && cargo test --target x86_64-unknown-linux-gnu -p karhutla-core`
Expected: FAIL

- [ ] **Step 3: Implement `buzzer.rs`, `record.rs`, `rtc.rs`, and `mqtt_codec.rs`**

Keep zero heap allocation (`no_std` without `alloc`) in all 4 modules; expose `pub mod` in `lib.rs`. Note for `crc32_update(prev_crc: u32, chunk: &[u8])`: standard IEEE CRC-32 (`0xEDB88320` reflected polynomial, initial `prev_crc = 0`, internally invert `^ 0xFFFF_FFFF` before and after loop so `crc32_update(crc32_ieee(a), b) == crc32_ieee(a || b)`!).

- [ ] **Step 4: Run host tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd firmware/karhutla_esp32_rs && cargo test --target x86_64-unknown-linux-gnu -p karhutla-core`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add firmware/karhutla_esp32_rs/crates/karhutla-core/
git commit -m "feat(rust): add buzzer patterns, 64B flash record/CRC/base64, RTC BCD, and MQTT codec"
```

---

### Task 3: `karhutla-esp32` Hardware Drivers (`hardware.rs`) & Flash Log Store (`storage.rs`)

**Files:**
- Modify: `firmware/karhutla_esp32_rs/Cargo.toml` (add `"firmware"` to workspace members)
- Create: `firmware/karhutla_esp32_rs/firmware/Cargo.toml`
- Create: `firmware/karhutla_esp32_rs/firmware/build.rs`
- Create: `firmware/karhutla_esp32_rs/firmware/src/hardware.rs`
- Create: `firmware/karhutla_esp32_rs/firmware/src/storage.rs`
- Create: `firmware/karhutla_esp32_rs/firmware/src/main.rs` (minimal stub compiling `mod hardware; mod storage;` until Task 4)

**Interfaces:**
- Consumes: `karhutla-core` (`cfg`, `AlarmState`, `record::*`, `rtc::*`).
- Produces in `hardware.rs`:
  - `pub struct Actuators<'d>` (`new(...)`, `set_fan_duty_pct(&mut self, pct: f32)`, `set_leds(&mut self, st: AlarmState)`, `set_buzzer(&mut self, on: bool)`)
  - `pub struct Sensors<'d>` (`new(...)`, `pub async fn read_dust_volts(&mut self) -> f32`, `pub async fn read_mq_volts(&mut self) -> f32`) — compiled under `#[cfg(feature = "riil")]`
  - `pub struct RtcClock<'d>` (`new(i2c: I2c<'d, Blocking>) -> Self`, `pub fn is_hw_present(&self) -> bool`, `pub fn sync_epoch(&mut self, epoch_s: u32, now_ms: u64)`, `pub fn now_epoch(&mut self, now_ms: u64) -> (u32, bool)`)
- Produces in `storage.rs`:
  - `pub struct LogMeta { pub count: u32, pub cap: u32, pub full: bool, pub used_bytes: u32, pub cap_bytes: u32, pub crc32: u32 }`
  - `pub struct FlashLogStore<'d>` (`pub fn new(flash: Flash<'d>) -> Self`, `pub fn init_scan(&mut self)`, `pub fn status(&self) -> (u32, bool)`, `pub fn append(&mut self, rec: &BackupRecord) -> Result<bool, FlashStorageError>`, `pub async fn compute_meta(&mut self) -> Result<LogMeta, FlashStorageError>`, `pub fn read_chunk_b64(&mut self, start: u32, max_count: u32, b64_out: &mut [u8]) -> Result<(u32, u32, usize), FlashStorageError>`, `pub async fn clear_verified<F>(&mut self, expected_count: u32, expected_crc: u32, mut on_yield: F) -> Result<Option<u32>, FlashStorageError>`)

- [ ] **Step 1: Create `firmware/Cargo.toml`, `firmware/build.rs`, and update workspace `Cargo.toml`**

Use exact verified crate versions from spike: `esp-hal = "1.2.2"` (`esp32`, `unstable`, `rt`), `esp-rtos = "0.4.0"` (`esp32`, `embassy`, `esp-radio`, `esp-alloc`), `esp-radio = "1.0.0-beta.1"` (`esp32`, `wifi`, `esp-alloc`, `unstable`), `esp-alloc = "0.11.0"`, `esp-bootloader-esp-idf = "0.6.0"` (`esp32`), `esp-println = "0.18.0"` (`esp32`), `esp-backtrace = "0.20.0"` (`esp32`, `println`, `panic-handler`, `halt-cores`), `esp-storage = "0.10.0"` (`esp32`), `embassy-executor = "0.10.0"`, `embassy-futures = "0.1.2"`, `embassy-sync = "0.8.0"`, `embassy-time = "0.5.1"`, `embassy-net = "0.9.1"`, `static_cell = "2.1"`, `karhutla-core = { path = "../crates/karhutla-core" }`. Features: `default = ["hils"]`, `hils = []`, `riil = []`.

- [ ] **Step 2: Implement `firmware/src/hardware.rs`**

- `Actuators`: configure `GPIO18` LEDC `LowSpeed` Timer0 (`25 kHz`, `Duty8Bit` — use `channel.set_duty_hw(((duty.clamp(0.0, 100.0) * 2.55) + 0.5) as u32)` for 8-bit resolution parity with C++), `GPIO4` `Output` (`Level::High`), `GPIO23` (`Level::Low`), `GPIO25`/`26`/`27` (`High`/`Low`/`Low`).
- `Sensors` (`#[cfg(feature = "riil")]`): `ADC1` with `enable_pin_with_cal::<_, AdcCalLine<ADC1>>` on `GPIO34` and `GPIO35` at `Attenuation::_11dB`. `read_dust_volts`: 8 pulses (`set_low()`, `delay.delay_micros(280)`, `nb::block!(adc1.read_oneshot(&mut pin_dust))`, `delay.delay_micros(40)`, `set_high()`, `Timer::after_micros(9680).await`), insertion sort + trimmed mean over middle 6 samples × `DIV_RATIO / 1000.0`. `read_mq_volts`: 16 samples with `Timer::after_millis(1).await`.
- `RtcClock`: probe `0x68` registers `0x00..0x07` on `I2c::new(..., Config::default().with_frequency(Rate::from_khz(100)))`. Never panic if NACK/timeout; fall back to `soft_base_epoch + ((now_ms - soft_base_ms) / 1000) as u32`. On `sync_epoch`, update both software clock AND write DS3231 registers if `hw_present`, only writing if drift > `5 s` or invalid.

- [ ] **Step 3: Implement `firmware/src/storage.rs`**

- `FlashLogStore` owns `FlashStorage::new(flash)`.
- `init_scan`: calls `karhutla_core::record::find_write_cursor` over `0..LOG_CAPACITY` reading 64-byte aligned slots via `read_nor`, sets `count`, `full = count >= LOG_CAPACITY`, `cached_crc = None`.
- `append`: encodes 64-byte record, writes at `LOG_PART_OFFSET + count * 64` using `write_nor`, increments `count`, updates `full`, and if `cached_crc == Some((prev_cnt, prev_crc))` where `prev_cnt == count - 1`, updates `cached_crc = Some((count, crc32_update(prev_crc, &raw)))`.
- `compute_meta`: if `cached_crc` matches `self.count`, returns immediately; otherwise streams 4 KiB sector chunks across `0..self.count`, feeding `crc32_update` and calling `Timer::after_millis(1).await` every 8 sectors so network/buzzer tasks keep running.
- `clear_verified`: computes/checks `(self.count, crc)` against `(expected_count, expected_crc)` — returns `Ok(None)` on mismatch. On match, computes `sectors = (self.count * 64 + 4095) / 4096` (minimum `1` if `count > 0`, `0` if `count == 0`), calls `flash.erase(start, end)` in batches of 4 sectors (`16 KiB`) with `Timer::after_millis(10).await` between batches, resets `count = 0`, `full = false`, `cached_crc = Some((0, 0))`, and returns `Ok(Some(sectors))`.

- [ ] **Step 4: Compile-check both `hils` and `riil` firmware variants**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && source "$HOME/export-esp.sh" && cd firmware/karhutla_esp32_rs && cargo +esp check -p karhutla-esp32 --features hils && cargo +esp check -p karhutla-esp32 --no-default-features --features riil`
Expected: `Finished` with 0 errors on both variants

- [ ] **Step 5: Commit**

```bash
git add firmware/karhutla_esp32_rs/
git commit -m "feat(rust): implement hardware drivers (ADC/LEDC/RTC I2C) and 2.625MiB flash store"
```

---

### Task 4: `karhutla-esp32` MQTT Client (`mqtt.rs`) & Embassy Tasks (`main.rs`)

**Files:**
- Create: `firmware/karhutla_esp32_rs/firmware/src/mqtt.rs`
- Modify: `firmware/karhutla_esp32_rs/firmware/src/main.rs`

**Interfaces:**
- Consumes: `karhutla-core` (`cfg`, `FirmwareCore`, `Telemetry`, `buzzer::buzzer_on`, `record::BackupRecord`, `mqtt_codec::*`), `hardware`, `storage`.
- Produces: Complete Embassy firmware binary `karhutla-esp32` implementing:
  - `wifi_task` & `net_task`: `PowerSaveMode::None`, DHCPv4, reconnect loop.
  - `mqtt_task`: TCP connect to `10.8.43.78:6669`, MQTT `CONNECT` with LWT `karhutla/status` `"offline"` retain, publishes `"online"` retain on connect, subscribes to `karhutla/cmd`, `karhutla/time/sync`, `karhutla/log/req`, and (`hils`) `karhutla/hils/sensor`. Publishes `karhutla/telemetry` JSON (identical schema to `main.cpp`), (`riil`) 1 Hz scalar topics `karhutla/t/*` + `karhutla/t/log_pct` + `karhutla/t/log_full`, extended `karhutla/hb` JSON (`mode`, `uptime_ms`, `rtc_ok`, `epoch_s`, `log_count`, `log_cap`, `log_full`, `log_bytes`, `log_cap_bytes`), and `karhutla/log/{meta,chunk,ack}` responses.
  - `control_task`: runs `FirmwareCore::step` (`RIIL` 1 Hz via `Sensors`; `HILS` event-driven via `HILS_CH` + `5000 ms` watchdog), updates `Actuators` & `BUZZER_STATE: AtomicU8`, signals `TEL_SIG` if `MQTT_CONNECTED`, or sends `BackupRecord` to `BACKUP_CH` if `!MQTT_CONNECTED` (`RIIL`: every step; `HILS`: one-shot `FAILSAFE` transition record per disconnect outage).
  - `buzzer_task`: `Ticker::every(Duration::from_millis(20))` -> `buzzer_on(state, storage_full, Instant::now().as_millis())` -> `GPIO23`.
  - `storage_rtc_task`: handles `BACKUP_CH`, `TIME_SYNC_SIG`, `LOG_REQ_CH` (`Meta`, `Get{start,count}`, `Clear{expected_count,expected_crc}` with two-phase `started` -> `done` acks over `LOG_RESP_CH`), and keeps `SHARED_STATUS` (`AtomicU32` log_count / `AtomicBool` log_full / `rtc_ok` / `epoch_s`) updated for `karhutla/hb` & `BUZZER_STATE`.

- [ ] **Step 1: Implement `firmware/src/mqtt.rs` (custom MQTT 3.1.1 QoS0 + LWT + topic handlers)**

Use `static RX_BUF` / `TX_BUF` (`4096` bytes each so `16 × 64B` Base64 chunk frames + telemetry never block), `select` over socket read vs `TEL_SIG` vs `LOG_RESP_CH` vs 1 Hz `hb_ticker` vs 10 s `PINGREQ` keepalive.

- [ ] **Step 2: Implement `firmware/src/main.rs` (all 6 Embassy tasks & shared static channels/atomics)**

Initialize heap (`64 KiB` reclaimed + `36 KiB` internal), `CpuClock::_240MHz`, `esp_rtos::start`, `FlashLogStore::init_scan()` before spawning network tasks, and spawn `net_task`, `wifi_task`, `mqtt_task`, `storage_rtc_task`, `control_task`, `buzzer_task`.

- [ ] **Step 3: Build release binaries for both `hils` and `riil` and verify `.bin` size fits in `1,280 KiB` `factory` partition**

Run:
```bash
export PATH="$HOME/.cargo/bin:$PATH" && source "$HOME/export-esp.sh" && cd firmware/karhutla_esp32_rs
cargo +esp build --release --features hils
espflash save-image --chip esp32 target/xtensa-esp32-none-elf/release/karhutla-esp32 /tmp/karhutla_hils.bin
cargo +esp build --release --no-default-features --features riil
espflash save-image --chip esp32 target/xtensa-esp32-none-elf/release/karhutla-esp32 /tmp/karhutla_riil.bin
ls -lh /tmp/karhutla_hils.bin /tmp/karhutla_riil.bin
```
Expected: Both build cleanly (`0` errors) and both `.bin` files are `< 1,310,720` bytes (`1,280 KiB`).

- [ ] **Step 4: Commit**

```bash
git add firmware/karhutla_esp32_rs/firmware/
git commit -m "feat(rust): complete Embassy runtime, MQTT 3.1.1 LWT client, and composite buzzer"
```

---

### Task 5: Python Config & Dashboard Backup Sync / Gated Clear Backend

**Files:**
- Modify: `config.py`
- Modify: `dashboard.py:226-440` (engine state, MQTT handlers, auto time sync, log sync/clear methods) and `dashboard.py:6987-7131` (HTTP GET/POST routes)
- Modify: `tests/test_mqtt_and_dashboard.py`

**Interfaces:**
- Consumes: `karhutla/hb`, `karhutla/log/{meta,chunk,ack}` MQTT topics; `BackupRecord` 64-byte little-endian layout (`<4sBBBBIII10fI`, `struct.Struct("<4sBBBBIII10fI")`).
- Produces in `config.py`:
  - `T_TIME_SYNC = "karhutla/time/sync"`
  - `T_LOG_REQ = "karhutla/log/req"`
  - `T_LOG_META = "karhutla/log/meta"`
  - `T_LOG_CHUNK = "karhutla/log/chunk"`
  - `T_LOG_ACK = "karhutla/log/ack"`
- Produces in `dashboard.py` (`DashboardEngine`):
  - `get_esp_log_status(self) -> dict`
  - `sync_esp_logs(self) -> tuple[bool, dict]` (pulls `META`, fetches all `GET:<start>:16` chunks with 3× retry, Base64-decodes + checks per-record CRC-32 & `zlib.crc32(raw_all)` == `meta["crc32"]`, writes both daily archival `logs/karhutla_esp32_backup_YYYY-MM-DD.csv` with 4-tuple dedup `(epoch_s, uptime_ms, seq, record_crc32)` & unsynced handling, AND single complete batch CSV `logs/karhutla_esp32_sync_<YYYYMMDD_HHMMSS>.csv`)
  - `get_esp_log_download_path(self) -> tuple[str | None, str | None]` (marks `downloaded = True` for `(count, crc32)`)
  - `clear_esp_logs(self) -> tuple[int, dict]` (returns `(409, ...)` if `log_count > 0` and `(log_count, crc32)` not `synced && downloaded` or offline; otherwise sends `CLEAR:<count>:<crc32>` and waits for two-phase `started` -> `done` with dynamic timeout `min(90.0, max(20.0, math.ceil(sectors * 0.12) + 15.0))`)
- Produces HTTP endpoints: `GET /api/esp-log/status`, `POST /api/esp-log/sync`, `GET /api/esp-log/download`, `POST /api/esp-log/clear`.

- [ ] **Step 1: Write failing backend tests in `tests/test_mqtt_and_dashboard.py`**

Add `TestEspBackupSyncAndClear` covering all Review Focus backend cases:
1. `test_clear_blocked_with_409_until_synced_and_downloaded` (with `log_count=5`, `POST /api/esp-log/clear` returns `409`; after `POST /api/esp-log/sync` alone, still `409`; only after `GET /api/esp-log/download`, `POST /api/esp-log/clear` sends `CLEAR:<count>:<crc32>` and returns `200`).
2. `test_clear_blocked_if_new_offline_record_appended_after_sync` *(Review Focus #2)*: simulate heartbeat reporting `log_count` incremented from `5` to `6` after sync+download -> `POST /api/esp-log/clear` returns `409` again!
3. `test_sync_rejects_corrupt_crc_or_torn_payload` *(Review Focus #1)*: corrupt 1 byte in chunk Base64 -> `sync` returns error and leaves `synced == False`.
4. `test_sync_multiday_and_unsynced_clock_dedup` *(Review Focus #3)*: sync batch containing 2 records crossing midnight (`2026-11-25` & `2026-11-26` WIB) + 2 unsynced records (`rtc_valid=0, epoch_s=0, seq=1` with different `uptime_ms`) -> verify NO `1970-01-01.csv` created, both archival day CSVs written, `UNSYNCED+<uptime_ms>ms` preserved without collision, and `GET /api/esp-log/download` returns all 4 records in one CSV.
5. `test_auto_time_sync_published_on_drift_or_unset_rtc`: simulate `karhutla/hb` with `rtc_ok=False` or `abs(epoch_s - now) > 10` -> verify `karhutla/time/sync` published.

- [ ] **Step 2: Run new tests to verify they fail**

Run: `sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -k TestEspBackupSyncAndClear -v`
Expected: FAIL

- [ ] **Step 3: Add topics in `config.py` and implement sync/download/clear backend in `dashboard.py`**

Decode 64-byte records via `struct.Struct("<4sBBBBIII10fI")`, validate `magic == b"KLOG"` and `zlib.crc32(slot[:60]) == slot_crc`, verify `zlib.crc32(all_raw) == meta["crc32"]`, persist CSVs safely under `self.out_dir`, broadcast `{"type": "esp_log_status", ...}` over SSE on state/progress changes, and wire the 4 HTTP routes in `do_GET`/`do_POST`.

- [ ] **Step 4: Run full Python test suite to verify it passes**

Run: `sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: PASS (all existing 26 tests + 5 new tests green)

- [ ] **Step 5: Commit**

```bash
git add config.py dashboard.py tests/test_mqtt_and_dashboard.py
git commit -m "feat(dashboard): add ESP32 backup log sync, CRC verification, CSV export, and gated clear API"
```

---

### Task 6: Website UI Card "Memori Backup ESP32 & RTC" & Mobile MQTT Config

**Files:**
- Modify: `dashboard.py` (`build_mobile_dashboard_config`, HTML/CSS/JS template for **"Memori Backup ESP32 & RTC"**)
- Modify: `tests/test_mqtt_and_dashboard.py`

**Interfaces:**
- Consumes: `GET /api/esp-log/status`, `POST /api/esp-log/sync`, `GET /api/esp-log/download`, `POST /api/esp-log/clear`, SSE `"esp_log_status"`.
- Produces:
  - Card **"Memori Backup ESP32 & RTC"** visible in desktop & mobile responsive layouts with IDs `#esp-log-card`, `#esp-rtc-badge`, `#esp-rtc-clock`, `#esp-log-bar`, `#esp-log-usage`, `#esp-log-full-badge`, `#btn-esp-log-sync` (`Sinkronkan & Unduh Log ESP32`), `#btn-esp-log-clear` (`Kosongkan Memori ESP32`, disabled until synced+downloaded), and confirmation dialog before clearing.
  - `build_mobile_dashboard_config()` adds `karhutla/t/log_pct` and `karhutla/t/log_full` panels.

- [ ] **Step 1: Write failing UI & mobile config tests in `tests/test_mqtt_and_dashboard.py`**

Add tests checking `dashboard.RENDERED_HTML` contains `#esp-log-card`, `Memori Backup ESP32 & RTC`, `Sinkronkan & Unduh Log ESP32`, `Kosongkan Memori ESP32`, `#btn-esp-log-clear` disabled by default, zero external CDN `<script src="http`/`<link href="http`, and `build_mobile_dashboard_config()` contains `karhutla/t/log_pct` and `karhutla/t/log_full`.

- [ ] **Step 2: Run UI tests to verify they fail**

Run: `sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -k esp_log_ui -v`
Expected: FAIL

- [ ] **Step 3: Add UI card HTML/CSS/JS and mobile config panels in `dashboard.py`**

Keep UI strictly offline-first, Bahasa Indonesia, consistent with existing CSS tokens/cards, and ensure mobile (`390x844`) and desktop (`1280x768`) layouts remain clean without overflow.

- [ ] **Step 4: Run full Python test suite to verify it passes**

Run: `sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add dashboard.py tests/test_mqtt_and_dashboard.py
git commit -m "feat(ui): add Memori Backup ESP32 & RTC card and mobile log telemetry panels"
```

---

### Task 7: Documentation & Full End-to-End Hardware Verification

**Files:**
- Modify: `README.md`
- Modify: `PRODUCT.md`

**Interfaces:**
- Consumes: All deliverables from Tasks 1–6.
- Produces: Updated build/flash/RTC/buzzer/sync-clear documentation in `README.md` and `PRODUCT.md` + verified live test evidence on `/dev/ttyUSB0`.

- [ ] **Step 1: Update `README.md` and `PRODUCT.md`**

Document `firmware/karhutla_esp32_rs/` build (`source ~/export-esp.sh`, `cargo +esp build --release --features hils|riil`, `espflash flash --port /dev/ttyUSB0 --partition-table partitions.csv`), RTC I2C pins (`GPIO21`/`GPIO22`), composite buzzer patterns table, and website sync-download-clear workflow (while keeping PlatformIO ESP-IDF fallback instructions intact).

- [ ] **Step 2: Run all automated host & Python verification suites**

Run:
```bash
export PATH="$HOME/.cargo/bin:$PATH" && source "$HOME/export-esp.sh"
(cd firmware/karhutla_esp32_rs && cargo test --target x86_64-unknown-linux-gnu -p karhutla-core && cargo +esp build --release --features hils && cargo +esp build --release --no-default-features --features riil)
sitl_venv/bin/python -m unittest tests/test_mqtt_and_dashboard.py -v
```
Expected: 0 failures across Rust host tests, both ESP32 release builds, and all Python unit tests.

- [ ] **Step 3: Flash `hils` firmware to `/dev/ttyUSB0` and run end-to-end hardware verification**

1. Flash: `espflash flash --port /dev/ttyUSB0 --partition-table firmware/karhutla_esp32_rs/partitions.csv firmware/karhutla_esp32_rs/target/xtensa-esp32-none-elf/release/karhutla-esp32`
2. Run HILS parity check: `sitl_venv/bin/python hils.py --scenario flaming --duration 60 --speed 10 --shadow` -> verify `0` lost packets and `0.0%` parity mismatch.
3. Verify offline backup cycle: temporarily stop/block MQTT client connection or flash `riil` briefly / trigger HILS disconnect -> confirm `karhutla/hb` reports `log_count > 0`, test `POST /api/esp-log/clear` returns `409`, run `POST /api/esp-log/sync` + `GET /api/esp-log/download` + `POST /api/esp-log/clear` (`200`), and verify `log_count == 0`.
4. Commit documentation updates:
```bash
git add README.md PRODUCT.md
git commit -m "docs: document Rust Embassy firmware, RTC I2C, and backup logger workflow"
```
