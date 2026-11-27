//! ESP32 64-byte flash backup record decoder, CRC-32 verifier, and CSV archive writer.
//! Directly reuses `karhutla_core::record::BackupRecord` & `crc32_ieee` (zero format drift).

use base64::engine::general_purpose::STANDARD as BASE64_STD;
use base64::Engine;
use chrono::{DateTime, FixedOffset, Local, SecondsFormat};
use karhutla_core::record::{
    crc32_ieee, BackupRecord, FLAG_MODE_HILS, FLAG_RTC_VALID, RECORD_SIZE,
};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

pub const ESP_LOG_COLS: [&str; 19] = [
    "timestamp",
    "epoch_s",
    "uptime_ms",
    "seq",
    "record_crc32",
    "rtc_valid",
    "mode_hils",
    "state",
    "mode",
    "v_dust",
    "v_mq",
    "pm_raw",
    "pm",
    "dcdt",
    "gas_ppm",
    "g",
    "fi",
    "v0",
    "duty",
];

#[derive(Debug, Clone)]
pub struct DecodedBackupRow {
    pub day: String,
    pub key: (String, String, String, String),
    pub fields: [String; 19],
}

#[derive(Debug, Clone)]
pub struct DecodedBatch {
    pub rows: Vec<DecodedBackupRow>,
    pub corrupt_skipped: u32,
}

pub fn decode_base64_chunk(data_b64: &str, expected_records: usize) -> Result<Vec<u8>, String> {
    let raw = BASE64_STD
        .decode(data_b64.trim())
        .map_err(|e| format!("base64 rusak: {e}"))?;
    if raw.len() != expected_records * RECORD_SIZE {
        return Err(format!(
            "ukuran chunk tidak sesuai (dapat {} B, harap {} B)",
            raw.len(),
            expected_records * RECORD_SIZE
        ));
    }
    Ok(raw)
}

fn wib_offset() -> FixedOffset {
    FixedOffset::east_opt(7 * 3600).expect("valid WIB offset")
}

fn fmt_f32(v: f32) -> String {
    format!("{:.6}", v)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub fn decode_and_verify_batch(raw_all: &[u8], expected_count: usize, expected_crc32: u32) -> Result<DecodedBatch, String> {
    if raw_all.len() != expected_count * RECORD_SIZE {
        return Err("panjang byte total log tidak cocok dengan META".to_string());
    }
    if crc32_ieee(raw_all) != expected_crc32 {
        return Err("CRC-32 total log tidak cocok dengan META".to_string());
    }
    let wib = wib_offset();
    let fallback_day = Local::now().with_timezone(&wib).format("%Y-%m-%d").to_string();
    let mut rows = Vec::with_capacity(expected_count);
    let mut corrupt_skipped = 0u32;

    for chunk in raw_all.chunks_exact(RECORD_SIZE) {
        let slot: &[u8; RECORD_SIZE] = chunk.try_into().unwrap();
        let rec = match BackupRecord::decode(slot) {
            Ok(r) => r,
            Err(_) => {
                corrupt_skipped += 1;
                continue;
            }
        };
        let record_crc = u32::from_le_bytes([slot[60], slot[61], slot[62], slot[63]]);
        let rtc_valid = u8::from((rec.flags & FLAG_RTC_VALID) != 0);
        let mode_hils = u8::from((rec.flags & FLAG_MODE_HILS) != 0);
        let (timestamp, day) = if rtc_valid == 1 && rec.epoch_s >= 1_700_000_000 {
            if let Some(utc) = DateTime::from_timestamp(rec.epoch_s as i64, 0) {
                let dt = utc.with_timezone(&wib);
                (
                    dt.to_rfc3339_opts(SecondsFormat::Secs, false),
                    dt.format("%Y-%m-%d").to_string(),
                )
            } else {
                (format!("UNSYNCED+{}ms", rec.uptime_ms), fallback_day.clone())
            }
        } else {
            (format!("UNSYNCED+{}ms", rec.uptime_ms), fallback_day.clone())
        };

        let epoch_s = rec.epoch_s.to_string();
        let uptime_ms = rec.uptime_ms.to_string();
        let seq = rec.seq.to_string();
        let crc_s = record_crc.to_string();
        let key = (epoch_s.clone(), uptime_ms.clone(), seq.clone(), crc_s.clone());
        let fields = [
            timestamp,
            epoch_s,
            uptime_ms,
            seq,
            crc_s,
            rtc_valid.to_string(),
            mode_hils.to_string(),
            (rec.state as u8).to_string(),
            (rec.mode as u8).to_string(),
            fmt_f32(rec.v_dust),
            fmt_f32(rec.v_mq),
            fmt_f32(rec.pm_raw),
            fmt_f32(rec.pm),
            fmt_f32(rec.dcdt),
            fmt_f32(rec.gas_ppm),
            fmt_f32(rec.g),
            fmt_f32(rec.fi),
            fmt_f32(rec.v0),
            fmt_f32(rec.duty),
        ];
        rows.push(DecodedBackupRow { day, key, fields });
    }

    Ok(DecodedBatch {
        rows,
        corrupt_skipped,
    })
}

pub fn write_esp_archives_and_batch(
    out_dir: impl AsRef<Path>,
    batch: &DecodedBatch,
) -> io::Result<PathBuf> {
    let dir = out_dir.as_ref();
    fs::create_dir_all(dir)?;

    let mut by_day: BTreeMap<&str, Vec<&DecodedBackupRow>> = BTreeMap::new();
    for r in &batch.rows {
        by_day.entry(&r.day).or_default().push(r);
    }

    for (day, day_rows) in by_day {
        let arch_path = dir.join(format!("karhutla_esp32_backup_{day}.csv"));
        let mut seen: HashSet<(String, String, String, String)> = HashSet::new();
        let exists = arch_path.exists()
            && fs::metadata(&arch_path).map(|m| m.len() > 0).unwrap_or(false);
        if exists {
            let mut rdr = csv::Reader::from_path(&arch_path)?;
            for rec in rdr.records().flatten() {
                let k = (
                    rec.get(1).unwrap_or("").to_string(),
                    rec.get(2).unwrap_or("").to_string(),
                    rec.get(3).unwrap_or("").to_string(),
                    rec.get(4).unwrap_or("").to_string(),
                );
                seen.insert(k);
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&arch_path)?;
        let mut wtr = csv::Writer::from_writer(file);
        if !exists {
            wtr.write_record(ESP_LOG_COLS)?;
        }
        for r in day_rows {
            if seen.insert(r.key.clone()) {
                wtr.write_record(&r.fields)?;
            }
        }
        wtr.flush()?;
    }

    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let batch_path = dir.join(format!("karhutla_esp32_sync_{stamp}.csv"));
    let tmp_path = dir.join(format!("karhutla_esp32_sync_{stamp}.csv.tmp"));
    {
        let mut wtr = csv::Writer::from_path(&tmp_path)?;
        wtr.write_record(ESP_LOG_COLS)?;
        for r in &batch.rows {
            wtr.write_record(&r.fields)?;
        }
        wtr.flush()?;
    }
    fs::rename(&tmp_path, &batch_path)?;
    Ok(batch_path)
}
