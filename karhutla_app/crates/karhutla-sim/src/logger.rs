//! Daily rotating CSV & MQTT loggers (ported from `logger.py`).

use chrono::{Local, SecondsFormat};
use karhutla_protocol::TelemetryRow;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const CSV_FIELDS: [&str; 22] = [
    "timestamp",
    "t_s",
    "source",
    "scenario",
    "pm_true",
    "v_dust",
    "v_mq",
    "pm_raw",
    "pm",
    "dcdt",
    "gas_ppm",
    "g",
    "fi",
    "v0",
    "state",
    "mode",
    "duty",
    "forecast",
    "slope",
    "eta_bahaya_s",
    "mae",
    "sensor_fault",
];

fn opt_f64(v: Option<f64>) -> String {
    match v {
        Some(x) if x.is_finite() => format!("{x}"),
        _ => String::new(),
    }
}

pub struct DailyCsvLogger {
    folder: PathBuf,
    prefix: String,
    source: String,
    day: Option<String>,
    writer: Option<csv::Writer<File>>,
    pub path: Option<PathBuf>,
}

impl DailyCsvLogger {
    pub fn new(folder: impl AsRef<Path>, prefix: &str, source: &str) -> io::Result<Self> {
        fs::create_dir_all(folder.as_ref())?;
        Ok(Self {
            folder: folder.as_ref().to_path_buf(),
            prefix: prefix.to_string(),
            source: source.to_string(),
            day: None,
            writer: None,
            path: None,
        })
    }

    fn ensure_open(&mut self, day: &str) -> io::Result<()> {
        if self.day.as_deref() == Some(day) && self.writer.is_some() {
            return Ok(());
        }
        let path = self
            .folder
            .join(format!("{}_{}_{}.csv", self.prefix, self.source, day));
        let is_new = !path.exists() || fs::metadata(&path).map(|m| m.len() == 0).unwrap_or(true);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut wtr = csv::Writer::from_writer(file);
        if is_new {
            wtr.write_record(CSV_FIELDS)?;
            wtr.flush()?;
        }
        self.day = Some(day.to_string());
        self.path = Some(path);
        self.writer = Some(wtr);
        Ok(())
    }

    pub fn write(&mut self, row: &TelemetryRow) -> io::Result<()> {
        let now = Local::now();
        let day = now.format("%Y-%m-%d").to_string();
        self.ensure_open(&day)?;
        let ts = now.to_rfc3339_opts(SecondsFormat::Millis, false);
        let rec = [
            ts,
            format!("{}", row.t_s),
            self.source.clone(),
            row.scenario.clone(),
            opt_f64(row.pm_true),
            format!("{}", row.v_dust),
            format!("{}", row.v_mq),
            format!("{}", row.pm_raw),
            format!("{}", row.pm),
            format!("{}", row.dcdt),
            format!("{}", row.gas_ppm),
            format!("{}", row.g),
            format!("{}", row.fi),
            format!("{}", row.v0),
            row.state.clone(),
            row.mode.clone(),
            format!("{}", row.duty),
            opt_f64(row.forecast),
            opt_f64(row.slope),
            opt_f64(row.eta_bahaya_s),
            opt_f64(row.mae),
            if row.sensor_fault {
                "True".to_string()
            } else {
                "False".to_string()
            },
        ];
        if let Some(wtr) = self.writer.as_mut() {
            wtr.write_record(&rec)?;
            wtr.flush()?;
        }
        Ok(())
    }
}

pub struct DailyMqttLogger {
    folder: PathBuf,
    prefix: String,
    day: Option<String>,
    file: Option<File>,
    pub path: Option<PathBuf>,
}

impl DailyMqttLogger {
    pub fn new(folder: impl AsRef<Path>, prefix: &str) -> io::Result<Self> {
        fs::create_dir_all(folder.as_ref())?;
        Ok(Self {
            folder: folder.as_ref().to_path_buf(),
            prefix: prefix.to_string(),
            day: None,
            file: None,
            path: None,
        })
    }

    fn ensure_open(&mut self, day: &str) -> io::Result<()> {
        if self.day.as_deref() == Some(day) && self.file.is_some() {
            return Ok(());
        }
        let path = self.folder.join(format!("{}_{}.log", self.prefix, day));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.day = Some(day.to_string());
        self.path = Some(path);
        self.file = Some(file);
        Ok(())
    }

    pub fn log_event(
        &mut self,
        client_id: &str,
        event: &str,
        topic: &str,
        payload: &str,
    ) -> io::Result<String> {
        let now = Local::now();
        let day = now.format("%Y-%m-%d").to_string();
        let ts = now.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
        let clean_topic = topic.replace(['\r', '\n'], " ");
        let clean_topic = clean_topic.trim();
        let clean_payload = payload.replace(['\r', '\n'], " ");
        let clean_payload = clean_payload.trim();
        let mut parts = vec![format!("{ts} [{client_id}] {event}")];
        if !clean_topic.is_empty() {
            parts.push(clean_topic.to_string());
        }
        if !clean_payload.is_empty() {
            parts.push(clean_payload.to_string());
        }
        let line = parts.join(" ");
        self.ensure_open(&day)?;
        if let Some(f) = self.file.as_mut() {
            writeln!(f, "{line}")?;
            f.flush()?;
        }
        Ok(line)
    }
}
