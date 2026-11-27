//! Karhutla simulation, forecasting, daily logging, and ESP32 backup log decoding
//! built directly on top of `karhutla-core` (same crate as ESP32 firmware).

pub mod esp_log;
pub mod forecast;
pub mod logger;
pub mod plant;

use karhutla_core::{CmdMsg, Telemetry};
use karhutla_protocol::{CmdAction, TelemetryRow};

pub fn cmd_to_core(cmd: CmdAction) -> CmdMsg {
    match cmd {
        CmdAction::Auto => CmdMsg::Auto,
        CmdAction::ForceMax => CmdMsg::ForceMax,
        CmdAction::ForceOff => CmdMsg::ForceOff,
        CmdAction::Reset => CmdMsg::Reset,
    }
}

pub fn telemetry_to_row(
    tel: &Telemetry,
    t: f64,
    scenario: &str,
    source: &str,
    pm_true: Option<f64>,
    gas_true: Option<f64>,
    fc: Option<forecast::ForecastOutput>,
    sensor_fault: bool,
) -> TelemetryRow {
    TelemetryRow {
        t,
        t_s: t,
        seq: tel.seq,
        scenario: scenario.to_string(),
        source: Some(source.to_string()),
        pm_true,
        gas_true,
        v_dust: tel.v_dust as f64,
        v_mq: tel.v_mq as f64,
        pm_raw: tel.pm_raw as f64,
        pm: tel.pm as f64,
        dcdt: tel.dcdt as f64,
        gas_ppm: tel.gas_ppm as f64,
        g: tel.g as f64,
        fi: tel.fi as f64,
        v0: tel.v0 as f64,
        state: tel.state.as_str().to_string(),
        mode: tel.mode.as_str().to_string(),
        duty: tel.duty as f64,
        forecast: fc.map(|f| f.pm_forecast),
        slope: fc.map(|f| f.slope),
        eta_bahaya_s: fc.and_then(|f| f.eta_bahaya_s),
        mae: fc.and_then(|f| f.mae),
        sensor_fault,
        ref_fi: None,
        ref_pm: None,
        ref_state: None,
        parity_delta_fi: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esp_log::{
        decode_and_verify_batch, decode_base64_chunk, write_esp_archives_and_batch,
    };
    use crate::forecast::{LinearTrendForecaster, PreemptController};
    use crate::logger::{DailyCsvLogger, DailyMqttLogger};
    use crate::plant::SmokeChamber;
    use base64::engine::general_purpose::STANDARD as BASE64_STD;
    use base64::Engine;
    use karhutla_core::record::{crc32_ieee, BackupRecord, RECORD_SIZE};
    use karhutla_core::{AlarmState, FirmwareCore};
    use karhutla_protocol::Scenario;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("karhutla_sim_{tag}_{ns}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn test_flaming_reaches_bahaya_and_fault_reaches_failsafe_70() {
        let mut plant = SmokeChamber::new(Scenario::Flaming, 1, 1.0);
        let mut fw = FirmwareCore::new();
        let mut saw_bahaya = false;
        for k in 0..250u32 {
            let t = k as f64;
            let (vd, vm) = plant.sensors(t);
            let tel = fw.step(vd as f32, vm as f32, k, 1.0);
            if tel.state == AlarmState::Bahaya {
                saw_bahaya = true;
                assert!((tel.duty - 100.0).abs() < 1e-3);
                break;
            }
            plant.advance(tel.duty as f64 / 100.0, t);
        }
        assert!(saw_bahaya, "flaming scenario must enter BAHAYA");

        let mut f_plant = SmokeChamber::new(Scenario::Fault, 1, 1.0);
        let mut f_fw = FirmwareCore::new();
        let mut saw_failsafe = false;
        for k in 0..240u32 {
            let t = k as f64;
            let (vd, vm) = f_plant.sensors(t);
            let tel = f_fw.step(vd as f32, vm as f32, k, 1.0);
            if tel.state == AlarmState::Failsafe {
                saw_failsafe = true;
                assert!((tel.duty - 70.0).abs() < 1e-3);
                break;
            }
            f_plant.advance(tel.duty as f64 / 100.0, t);
        }
        assert!(saw_failsafe, "fault scenario must enter FAILSAFE at 70%");
    }

    #[test]
    fn test_forecaster_and_preempt_controller() {
        let mut fc = LinearTrendForecaster::default();
        let mut pre = PreemptController::default();
        let mut triggered = false;
        for k in 0..80 {
            let t = k as f64;
            let c = t * 2.0; // slope = 2.0 ug/m3/s => t+60s reaches 150 early
            let out = fc.update(t, c);
            if k >= 10 {
                assert!((out.slope - 2.0).abs() < 1e-3);
                assert!((out.pm_forecast - (c + 120.0)).abs() < 1e-2);
            }
            if pre.decide(t, "AUTO", out.pm_forecast) == Some(CmdAction::ForceMax) {
                triggered = true;
            }
        }
        assert!(triggered, "PreemptController must trigger FORCE_MAX on rising ramp");
        // Hold then release when forecast drops below PREEMPT_OFF
        assert_eq!(pre.decide(120.0, "FORCE_MAX", 80.0), Some(CmdAction::Auto));
        assert!(!pre.active);
    }

    #[test]
    fn test_daily_loggers_and_esp_backup_record_dedup() {
        let dir = tmp_dir("log_esp");
        let mut csv_log = DailyCsvLogger::new(&dir, "karhutla", "sitl").unwrap();
        let mut mqtt_log = DailyMqttLogger::new(&dir, "mqtt").unwrap();
        let row = TelemetryRow::idle();
        csv_log.write(&row).unwrap();
        let line = mqtt_log
            .log_event("karhutla-rust", "PUBLISH", "karhutla/cmd", "AUTO")
            .unwrap();
        assert!(line.contains("PUBLISH karhutla/cmd AUTO"));
        assert!(csv_log.path.as_ref().unwrap().exists());

        // Encode 2 BackupRecords + verify CRC + archive dedup
        let mut fw = FirmwareCore::new();
        let t0 = fw.step(0.62, 1.2, 1, 1.0);
        let t1 = fw.step(0.68, 1.2, 2, 1.0);
        let r0 = BackupRecord::from_telemetry(&t0, 1_795_688_100, 1000, true, false);
        let r1 = BackupRecord::from_telemetry(&t1, 1_795_688_101, 2000, true, true);
        let mut raw = vec![0u8; 2 * RECORD_SIZE];
        let mut s0 = [0u8; RECORD_SIZE];
        let mut s1 = [0u8; RECORD_SIZE];
        r0.encode(&mut s0);
        r1.encode(&mut s1);
        raw[..RECORD_SIZE].copy_from_slice(&s0);
        raw[RECORD_SIZE..].copy_from_slice(&s1);
        let b64 = BASE64_STD.encode(&raw);
        let decoded_bytes = decode_base64_chunk(&b64, 2).unwrap();
        let crc = crc32_ieee(&decoded_bytes);
        let batch = decode_and_verify_batch(&decoded_bytes, 2, crc).unwrap();
        assert_eq!(batch.rows.len(), 2);
        assert_eq!(batch.corrupt_skipped, 0);

        let batch_csv = write_esp_archives_and_batch(&dir, &batch).unwrap();
        assert!(batch_csv.exists());
        // Writing the same batch again must deduplicate the daily archive
        let _ = write_esp_archives_and_batch(&dir, &batch).unwrap();
        let day = &batch.rows[0].day;
        let arch = std::fs::read_to_string(dir.join(format!("karhutla_esp32_backup_{day}.csv")))
            .unwrap();
        assert_eq!(arch.lines().count(), 3); // 1 header + 2 unique rows
        let _ = std::fs::remove_dir_all(&dir);
    }
}
