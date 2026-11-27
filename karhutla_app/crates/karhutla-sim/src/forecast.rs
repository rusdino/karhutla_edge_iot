//! Linear trend forecaster (t + 60s) & safe proactive preempt controller (ported from `forecast.py`).

use karhutla_core::cfg;
use karhutla_protocol::CmdAction;
use std::collections::VecDeque;

pub const FORECAST_WINDOW_S: f64 = 60.0;
pub const FORECAST_HORIZON_S: f64 = 60.0;
pub const PREEMPT_ON: f64 = 150.0;
pub const PREEMPT_OFF: f64 = 125.0;
pub const PREEMPT_HOLD_S: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForecastOutput {
    pub pm_forecast: f64,
    pub slope: f64,
    pub eta_bahaya_s: Option<f64>,
    pub mae: Option<f64>,
}

pub struct LinearTrendForecaster {
    window_s: f64,
    horizon_s: f64,
    min_pts: usize,
    danger: f64,
    buf: VecDeque<(f64, f64)>,
    pending: VecDeque<(f64, f64)>,
    err: VecDeque<f64>,
}

impl Default for LinearTrendForecaster {
    fn default() -> Self {
        Self::new(FORECAST_WINDOW_S, FORECAST_HORIZON_S, 10, cfg::T_B_IN as f64)
    }
}

impl LinearTrendForecaster {
    pub fn new(window_s: f64, horizon_s: f64, min_pts: usize, danger: f64) -> Self {
        Self {
            window_s,
            horizon_s,
            min_pts,
            danger,
            buf: VecDeque::new(),
            pending: VecDeque::new(),
            err: VecDeque::with_capacity(301),
        }
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.pending.clear();
        self.err.clear();
    }

    pub fn update(&mut self, t: f64, c: f64) -> ForecastOutput {
        self.buf.push_back((t, c));
        while let Some(&(t0, _)) = self.buf.front() {
            if t - t0 > self.window_s {
                self.buf.pop_front();
            } else {
                break;
            }
        }
        let n = self.buf.len();
        let mut slope = 0.0;
        let mut fc = c;
        if n >= self.min_pts {
            let nf = n as f64;
            let tm = self.buf.iter().map(|(x, _)| *x).sum::<f64>() / nf;
            let cm = self.buf.iter().map(|(_, y)| *y).sum::<f64>() / nf;
            let sxx = self.buf.iter().map(|(x, _)| (x - tm).powi(2)).sum::<f64>();
            let sxy = self
                .buf
                .iter()
                .map(|(x, y)| (x - tm) * (y - cm))
                .sum::<f64>();
            slope = if sxx > 1e-9 { sxy / sxx } else { 0.0 };
            fc = cm + slope * ((t + self.horizon_s) - tm);
        }
        fc = fc.max(0.0);
        if n >= self.min_pts {
            self.pending.push_back((t + self.horizon_s, fc));
        }
        while let Some(&(t_due, _)) = self.pending.front() {
            if t_due <= t + 1e-9 {
                let (_, f_old) = self.pending.pop_front().unwrap();
                if self.err.len() >= 300 {
                    self.err.pop_front();
                }
                self.err.push_back(c - f_old);
            } else {
                break;
            }
        }
        let mae = if self.err.is_empty() {
            None
        } else {
            Some(self.err.iter().map(|e| e.abs()).sum::<f64>() / self.err.len() as f64)
        };
        let eta_bahaya_s = if c >= self.danger {
            Some(0.0)
        } else if slope > 1e-3 {
            Some((self.danger - c) / slope)
        } else {
            None
        };
        ForecastOutput {
            pm_forecast: fc,
            slope,
            eta_bahaya_s,
            mae,
        }
    }
}

pub struct PreemptController {
    pub on_th: f64,
    pub off_th: f64,
    pub hold_s: f64,
    pub grace_s: f64,
    pub active: bool,
    t_on: f64,
}

impl Default for PreemptController {
    fn default() -> Self {
        Self::new(PREEMPT_ON, PREEMPT_OFF, PREEMPT_HOLD_S, 5.0)
    }
}

impl PreemptController {
    pub fn new(on_th: f64, off_th: f64, hold_s: f64, grace_s: f64) -> Self {
        Self {
            on_th,
            off_th,
            hold_s,
            grace_s,
            active: false,
            t_on: 0.0,
        }
    }

    pub fn decide(&mut self, t: f64, mode_now: &str, forecast: f64) -> Option<CmdAction> {
        if self.active {
            if t - self.t_on > self.grace_s && mode_now != "FORCE_MAX" {
                self.active = false;
                return None;
            }
            if t - self.t_on >= self.hold_s && forecast <= self.off_th {
                self.active = false;
                return Some(CmdAction::Auto);
            }
            return None;
        }
        if mode_now == "AUTO" && forecast >= self.on_th {
            self.active = true;
            self.t_on = t;
            return Some(CmdAction::ForceMax);
        }
        None
    }
}
