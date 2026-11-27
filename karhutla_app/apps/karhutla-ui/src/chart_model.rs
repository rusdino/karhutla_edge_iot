use crate::state::LayerVisibility;
use karhutla_protocol::TelemetryRow;

#[derive(Debug, Clone, PartialEq)]
pub struct ChartPalette {
    pub grid: &'static str,
    pub text: &'static str,
    pub muted: &'static str,
    pub fi: &'static str,
    pub pm: &'static str,
    pub pm_true: &'static str,
    pub pm_raw: &'static str,
    pub forecast: &'static str,
    pub forecast_fill: &'static str,
    pub now_line: &'static str,
    pub suspended_fill: &'static str,
    pub suspended_line: &'static str,
    pub gas_ppm: &'static str,
    pub g_scaled: &'static str,
    pub v_dust: &'static str,
    pub v_mq: &'static str,
    pub waspada: &'static str,
    pub bahaya: &'static str,
    pub fault: &'static str,
    pub fault_fill: &'static str,
    pub fault_hatch: &'static str,
    pub fsm: &'static str,
    pub duty: &'static str,
    pub tip_bg: &'static str,
    pub tip_stroke: &'static str,
    pub tip_fg: &'static str,
    pub export_bg: &'static str,
}

impl ChartPalette {
    pub fn for_theme(dark: bool) -> Self {
        if dark {
            Self {
                grid: "rgba(156,167,180,0.18)",
                text: "#cbd5e1",
                muted: "#9ca7b4",
                fi: "#fb923c",
                pm: "#38bdf8",
                pm_true: "#4ade80",
                pm_raw: "#9ca7b4",
                forecast: "#cbb2e8",
                forecast_fill: "rgba(203,178,232,0.08)",
                now_line: "rgba(203,178,232,0.62)",
                suspended_fill: "rgba(148,163,184,0.12)",
                suspended_line: "rgba(148,163,184,0.62)",
                gas_ppm: "#68c89c",
                g_scaled: "#facc15",
                v_dust: "#60a5fa",
                v_mq: "#cbb2e8",
                waspada: "#fbbf24",
                bahaya: "rgba(248,113,113,0.85)",
                fault: "#f472b6",
                fault_fill: "rgba(244,114,182,0.22)",
                fault_hatch: "rgba(244,114,182,0.48)",
                fsm: "#fbbf24",
                duty: "#4ade80",
                tip_bg: "rgba(22,26,31,0.96)",
                tip_stroke: "#333c47",
                tip_fg: "#f3f5f7",
                export_bg: "#161a1f",
            }
        } else {
            Self {
                grid: "rgba(75,85,99,0.16)",
                text: "#374151",
                muted: "#4b5563",
                fi: "#c2410c",
                pm: "#0369a1",
                pm_true: "#14532d",
                pm_raw: "#475569",
                forecast: "#6d28d9",
                forecast_fill: "rgba(109,40,217,0.06)",
                now_line: "rgba(109,40,217,0.55)",
                suspended_fill: "rgba(100,116,139,0.10)",
                suspended_line: "rgba(71,85,105,0.52)",
                gas_ppm: "#0f766e",
                g_scaled: "#854d0e",
                v_dust: "#2563eb",
                v_mq: "#7c3aed",
                waspada: "#92400e",
                bahaya: "rgba(185,28,28,0.82)",
                fault: "#9d174d",
                fault_fill: "rgba(157,23,77,0.14)",
                fault_hatch: "rgba(157,23,77,0.34)",
                fsm: "#92400e",
                duty: "#14532d",
                tip_bg: "rgba(255,255,255,0.97)",
                tip_stroke: "#c9d2dc",
                tip_fg: "#111827",
                export_bg: "#ffffff",
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegendItem {
    pub label: &'static str,
    pub color: &'static str,
    pub style: &'static str,
    pub dimmed: bool,
}

pub fn legend_specs(
    layers: &LayerVisibility,
    allow_pm_true: bool,
    latest_is_fault: bool,
    pal: &ChartPalette,
) -> Vec<LegendItem> {
    let mut out = Vec::new();
    if layers.fi {
        out.push(LegendItem {
            label: "Fusion Index (FI)",
            color: pal.fi,
            style: "solid",
            dimmed: latest_is_fault,
        });
    }
    if layers.pm {
        out.push(LegendItem {
            label: "PM-proxy Kalman",
            color: pal.pm,
            style: "solid",
            dimmed: false,
        });
    }
    if layers.pm_true && allow_pm_true {
        out.push(LegendItem {
            label: "PM Ruang Asap (True)",
            color: pal.pm_true,
            style: "solid",
            dimmed: false,
        });
    }
    if layers.pm_raw {
        out.push(LegendItem {
            label: "PM Mentah (Median)",
            color: pal.pm_raw,
            style: "solid",
            dimmed: false,
        });
    }
    if layers.forecast {
        out.push(LegendItem {
            label: "Prakiraan PM AI (+60s)",
            color: pal.forecast,
            style: "dotted",
            dimmed: latest_is_fault,
        });
    }
    if layers.gas_ppm {
        out.push(LegendItem {
            label: "Gas MQ-135 (×0.2)",
            color: pal.gas_ppm,
            style: "dashed",
            dimmed: false,
        });
    }
    if layers.g_scaled {
        out.push(LegendItem {
            label: "Skor g (×100)",
            color: pal.g_scaled,
            style: "dotted",
            dimmed: false,
        });
    }
    if layers.voltages {
        out.push(LegendItem {
            label: "Vd Sharp (×50)",
            color: pal.v_dust,
            style: "dashed",
            dimmed: false,
        });
        out.push(LegendItem {
            label: "Vm MQ-135 (×50)",
            color: pal.v_mq,
            style: "dashed",
            dimmed: false,
        });
    }
    if layers.thresholds {
        out.push(LegendItem {
            label: "Ambang Waspada (55)",
            color: pal.waspada,
            style: "dashed",
            dimmed: false,
        });
        out.push(LegendItem {
            label: "Ambang Bahaya (150)",
            color: pal.bahaya,
            style: "dashed",
            dimmed: false,
        });
    }
    out
}

pub fn nice_step(raw_step: f64) -> f64 {
    if raw_step <= 0.0 || !raw_step.is_finite() {
        return 50.0;
    }
    let pow = 10f64.powf(raw_step.log10().floor());
    let frac = raw_step / pow;
    let nice = if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 2.5 {
        2.5
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * pow
}

#[derive(Debug, Clone, Copy)]
pub struct ChartPads {
    pub compact: bool,
    pub fsm_compact: bool,
    pub pad_l: f64,
    pub pad_r: f64,
}

pub fn chart_pads(w: f64, has_right_axis: bool) -> ChartPads {
    let compact = w < 520.0;
    let fsm_compact = w < 420.0;
    ChartPads {
        compact,
        fsm_compact,
        pad_l: if compact { 46.0 } else { 54.0 },
        pad_r: if has_right_axis {
            if fsm_compact {
                54.0
            } else {
                84.0
            }
        } else {
            16.0
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GridYLine {
    pub y: f64,
    pub left_label: String,
    pub right_spans: Vec<(String, &'static str)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GridXLine {
    pub x: f64,
    pub label: String,
    pub label_x: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdLine {
    pub y: f64,
    pub label: &'static str,
    pub label_x: f64,
    pub anchor: &'static str,
    pub color: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HorizonBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub fill: &'static str,
    pub stroke: &'static str,
    pub label: &'static str,
    pub label_color: &'static str,
    pub label_x: f64,
    pub label_anchor: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaultBand {
    pub x: f64,
    pub w: f64,
    pub draw_right_line: bool,
    pub inline_label: Option<&'static str>,
    pub inline_label_x: f64,
    pub inline_label_y: f64,
    pub inline_pill_w: f64,
    pub top_tag_label: &'static str,
    pub top_tag_x: f64,
    pub top_tag_w: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolySeries {
    pub points_list: Vec<String>,
    pub color: &'static str,
    pub stroke_width: f64,
    pub dash: &'static str,
    pub opacity: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveForecastTip {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub color: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CrosshairTooltip {
    pub x: f64,
    pub box_x: f64,
    pub box_y: f64,
    pub box_w: f64,
    pub box_h: f64,
    pub lines: Vec<String>,
    pub live_readout: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RibbonSegment {
    pub state: String,
    pub label: String,
    pub x: f64,
    pub w: f64,
    pub fill: &'static str,
    pub stroke: &'static str,
    pub hatch: bool,
    pub draw_inner_label: bool,
    pub text_x: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RightAxisHeaderSpan {
    pub text: &'static str,
    pub color: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MainChartModel {
    pub w: f64,
    pub h: f64,
    pub pad_l: f64,
    pub pad_r: f64,
    pub pad_t: f64,
    pub pad_b: f64,
    pub plot_w: f64,
    pub plot_h: f64,
    pub min_t: f64,
    pub span_t: f64,
    pub empty_message: Option<String>,
    pub grid_y: Vec<GridYLine>,
    pub grid_x: Vec<GridXLine>,
    pub right_header_spans: Vec<RightAxisHeaderSpan>,
    pub thresholds: Vec<ThresholdLine>,
    pub zoom_note: Option<(String, f64, &'static str)>,
    pub horizon: Option<HorizonBox>,
    pub fault_bands: Vec<FaultBand>,
    pub series: Vec<PolySeries>,
    pub forecast_tip: Option<ActiveForecastTip>,
    pub crosshair: Option<CrosshairTooltip>,
    pub sr_summary: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubChartModel {
    pub w: f64,
    pub h: f64,
    pub pad_l: f64,
    pub pad_r: f64,
    pub duty_top: f64,
    pub duty_h: f64,
    pub duty_bot: f64,
    pub sep_y: f64,
    pub ribbon_top: f64,
    pub ribbon_h: f64,
    pub compact: bool,
    pub empty_message: Option<&'static str>,
    pub duty_y_ticks: Vec<(f64, &'static str)>,
    pub y70: f64,
    pub duty_points: String,
    pub ribbon_segments: Vec<RibbonSegment>,
    pub tail_label: Option<(String, f64, &'static str)>,
    pub horizon: Option<(f64, f64, f64, f64, &'static str, &'static str)>,
    pub crosshair_x: Option<f64>,
    pub sr_summary: String,
}

pub fn filter_monotonic(rows: &[TelemetryRow]) -> Vec<&TelemetryRow> {
    let mut mono: Vec<&TelemetryRow> = Vec::with_capacity(rows.len());
    for r in rows {
        if let Some(last) = mono.last_mut() {
            if r.t < last.t {
                mono.clear();
                mono.push(r);
            } else if (r.t - last.t).abs() < 1e-9 {
                *last = r;
            } else {
                mono.push(r);
            }
        } else {
            mono.push(r);
        }
    }
    mono
}

pub fn time_window_for(
    rows: &[TelemetryRow],
    window_sec: u32,
    show_forecast: bool,
) -> Option<(f64, f64, f64)> {
    let mono = filter_monotonic(rows);
    let last = mono.last()?;
    let first = mono.first()?;
    let max_t = last.t;
    let min_t = if window_sec > 0 {
        (max_t - window_sec as f64).max(0.0)
    } else {
        first.t
    };
    let span_t = ((max_t + if show_forecast { 60.0 } else { 5.0 }) - min_t).max(60.0);
    Some((min_t, max_t, span_t))
}

pub fn x_to_timestamp(
    px: f64,
    w: f64,
    has_right_axis: bool,
    rows: &[TelemetryRow],
    window_sec: u32,
    show_forecast: bool,
) -> Option<f64> {
    let (min_t, _max_t, span_t) = time_window_for(rows, window_sec, show_forecast)?;
    let pads = chart_pads(w, has_right_axis);
    let plot_w = (w - pads.pad_l - pads.pad_r).max(40.0);
    if px < pads.pad_l || px > w - pads.pad_r {
        return None;
    }
    Some(min_t + ((px - pads.pad_l) / plot_w) * span_t)
}

pub fn step_hover_t(
    rows: &[TelemetryRow],
    window_sec: u32,
    current_hover_t: Option<f64>,
    delta: isize,
) -> Option<f64> {
    let mono = filter_monotonic(rows);
    let max_t = mono.last()?.t;
    let min_t = if window_sec > 0 {
        (max_t - window_sec as f64).max(0.0)
    } else {
        mono.first()?.t
    };
    let visible: Vec<&TelemetryRow> = mono.into_iter().filter(|r| r.t >= min_t).collect();
    if visible.is_empty() {
        return None;
    }
    let idx = if let Some(ht) = current_hover_t {
        let mut best = 0usize;
        for (i, r) in visible.iter().enumerate().skip(1) {
            if (r.t - ht).abs() < (visible[best].t - ht).abs() {
                best = i;
            }
        }
        (best as isize + delta).clamp(0, (visible.len() - 1) as isize) as usize
    } else {
        visible.len() - 1
    };
    Some(visible[idx].t)
}

pub fn build_chart_models(
    rows: &[TelemetryRow],
    layers: &LayerVisibility,
    allow_pm_true: bool,
    window_sec: u32,
    zoom_y: bool,
    dark: bool,
    main_w: f64,
    main_h: f64,
    sub_w: f64,
    sub_h: f64,
    hover_t: Option<f64>,
    active_source: &str,
    esp_online: bool,
) -> (MainChartModel, SubChartModel) {
    let pal = ChartPalette::for_theme(dark);
    let w = main_w.max(260.0);
    let h = main_h.max(150.0);
    let sw = sub_w.max(260.0);
    let sh = sub_h.max(84.0);
    let has_right_axis = layers.has_right_axis();
    let pads = chart_pads(w, has_right_axis);
    let pad_t = 24.0;
    let pad_b = 26.0;
    let plot_w = (w - pads.pad_l - pads.pad_r).max(40.0);
    let plot_h = (h - pad_t - pad_b).max(40.0);

    let s_pad_t = 22.0;
    let s_pad_b = 8.0;
    let s_inner_h = (sh - s_pad_t - s_pad_b).max(44.0);
    let duty_top = s_pad_t;
    let duty_h = (s_inner_h * 0.56).round().max(22.0);
    let duty_bot = duty_top + duty_h;
    let sep_y = duty_bot + (s_inner_h * 0.06).round().max(2.0);
    let ribbon_top = duty_bot + (s_inner_h * 0.12).round().max(5.0);
    let ribbon_h = (sh - s_pad_b - ribbon_top).max(16.0);
    let sy_of = |v: f64| duty_top + duty_h - (v.clamp(0.0, 100.0) / 100.0) * duty_h;

    let mono = filter_monotonic(rows);
    if mono.is_empty() {
        let is_esp_waiting = active_source == "esp32" && !esp_online;
        let msg = if is_esp_waiting {
            "Menunggu telemetri ESP32 dari broker MQTT (:6669)...".to_string()
        } else if w < 768.0 {
            "Belum ada data SITL — tekan \"Mulai\" di bilah bawah.".to_string()
        } else {
            "Belum ada data SITL — tekan \"Mulai\" di panel kontrol.".to_string()
        };
        let main = MainChartModel {
            w,
            h,
            pad_l: pads.pad_l,
            pad_r: pads.pad_r,
            pad_t,
            pad_b,
            plot_w,
            plot_h,
            min_t: 0.0,
            span_t: 300.0,
            empty_message: Some(msg.clone()),
            grid_y: Vec::new(),
            grid_x: Vec::new(),
            right_header_spans: Vec::new(),
            thresholds: Vec::new(),
            zoom_note: None,
            horizon: None,
            fault_bands: Vec::new(),
            series: Vec::new(),
            forecast_tip: None,
            crosshair: None,
            sr_summary: msg,
        };
        let sub = SubChartModel {
            w: sw,
            h: sh,
            pad_l: pads.pad_l,
            pad_r: pads.pad_r,
            duty_top,
            duty_h,
            duty_bot,
            sep_y,
            ribbon_top,
            ribbon_h,
            compact: pads.compact,
            empty_message: Some("Belum ada histori duty kipas & pita state FSM."),
            duty_y_ticks: Vec::new(),
            y70: sy_of(70.0),
            duty_points: String::new(),
            ribbon_segments: Vec::new(),
            tail_label: None,
            horizon: None,
            crosshair_x: None,
            sr_summary: "Belum ada histori duty kipas dan state FSM.".to_string(),
        };
        return (main, sub);
    }

    let max_t = mono.last().unwrap().t;
    let min_t = if window_sec > 0 {
        (max_t - window_sec as f64).max(0.0)
    } else {
        mono.first().unwrap().t
    };
    let span_t = ((max_t + if layers.forecast { 60.0 } else { 5.0 }) - min_t).max(60.0);
    let visible: Vec<&TelemetryRow> = mono.into_iter().filter(|r| r.t >= min_t).collect();

    let mut data_max = 0.0f64;
    for r in &visible {
        if layers.fi {
            data_max = data_max.max(r.fi * 1.12);
        }
        if layers.pm {
            data_max = data_max.max(r.pm * 1.12);
        }
        if layers.pm_true && allow_pm_true {
            if let Some(pt) = r.pm_true {
                data_max = data_max.max(pt * 1.12);
            }
        }
        if layers.pm_raw {
            data_max = data_max.max(r.pm_raw * 1.12);
        }
        if layers.forecast {
            if let Some(fc) = r.forecast {
                data_max = data_max.max(fc * 1.12);
            }
        }
        if layers.gas_ppm {
            data_max = data_max.max(r.gas_ppm * 0.22);
        }
        if layers.g_scaled {
            data_max = data_max.max(r.g * 105.0);
        }
        if layers.voltages {
            data_max = data_max.max(r.v_dust * 55.0).max(r.v_mq * 55.0);
        }
    }

    let base_floor = if zoom_y {
        data_max.max(20.0)
    } else if layers.thresholds {
        data_max.max(170.0)
    } else {
        data_max.max(40.0)
    };
    let raw_max_y = data_max.max(base_floor);
    let y_step = nice_step(raw_max_y / 4.0);
    let max_y = (raw_max_y / y_step).ceil() * y_step;

    let x_of = |t: f64| pads.pad_l + ((t - min_t) / span_t) * plot_w;
    let y_of = |v: f64| pad_t + plot_h - (v.clamp(0.0, max_y) / max_y) * plot_h;

    let latest_vis = visible.last().copied();
    let latest_is_fault = latest_vis
        .map(|r| r.state == "FAILSAFE" || r.sensor_fault)
        .unwrap_or(false);

    // Horizon (+60s)
    let x_now = x_of(max_t);
    let x_right = w - pads.pad_r;
    let horizon = if layers.forecast && x_now < x_right {
        let now_lbl = if latest_is_fault {
            if pads.compact {
                "JEDA AI"
            } else {
                "PRAKIRAAN DITANGGUHKAN"
            }
        } else if pads.compact {
            "KINI"
        } else {
            "SEKARANG → +60s"
        };
        let hw = x_right - x_now;
        let (label_x, label_anchor) = if hw >= 135.0 {
            (x_now + 6.0, "start")
        } else {
            (x_right - 4.0, "end")
        };
        Some(HorizonBox {
            x: x_now,
            y: pad_t,
            w: hw,
            h: plot_h,
            fill: if latest_is_fault {
                pal.suspended_fill
            } else {
                pal.forecast_fill
            },
            stroke: if latest_is_fault {
                pal.suspended_line
            } else {
                pal.now_line
            },
            label: now_lbl,
            label_color: if latest_is_fault {
                pal.muted
            } else {
                pal.forecast
            },
            label_x,
            label_anchor,
        })
    } else {
        None
    };

    // Right axes definitions (preserving individual unit colors!)
    let mut right_header_spans = Vec::new();
    let mut right_unit_fns: Vec<(&'static str, &'static str, fn(f64) -> String)> = Vec::new();
    if layers.gas_ppm {
        right_unit_fns.push(("ppm", pal.gas_ppm, |v| format!("{}", (v / 0.2).round() as i64)));
    }
    if layers.g_scaled {
        right_unit_fns.push(("g", pal.g_scaled, |v| format!("{:.1}", v / 100.0)));
    }
    if layers.voltages {
        right_unit_fns.push(("V", pal.v_dust, |v| format!("{:.1}", v / 50.0)));
    }
    for (idx, (unit, color, _)) in right_unit_fns.iter().enumerate() {
        if pads.fsm_compact && idx > 0 {
            break;
        }
        if idx > 0 {
            right_header_spans.push(RightAxisHeaderSpan {
                text: "/",
                color: pal.muted,
            });
        }
        right_header_spans.push(RightAxisHeaderSpan { text: unit, color });
    }

    // Y Grid
    let mut grid_y = Vec::new();
    let mut val = 0.0f64;
    while val <= max_y + 0.001 {
        let mut right_spans = Vec::new();
        if right_unit_fns.len() == 1 {
            let (unit, color, fmt_fn) = right_unit_fns[0];
            let txt = if pads.fsm_compact {
                fmt_fn(val)
            } else {
                format!("{} {unit}", fmt_fn(val))
            };
            right_spans.push((txt, color));
        } else if right_unit_fns.len() > 1 {
            for (idx, (_unit, color, fmt_fn)) in right_unit_fns.iter().enumerate() {
                if pads.fsm_compact && idx > 0 {
                    break;
                }
                if idx > 0 {
                    right_spans.push(("/".to_string(), pal.muted));
                }
                right_spans.push((fmt_fn(val), color));
            }
        }
        grid_y.push(GridYLine {
            y: y_of(val),
            left_label: format!("{}", val.round() as i64),
            right_spans,
        });
        val += y_step;
    }

    // X Grid
    let x_step = nice_step(span_t / if pads.compact { 4.0 } else { 6.0 });
    let first_tick = (min_t / x_step).ceil() * x_step;
    let mut grid_x = Vec::new();
    let mut last_tick_x = f64::NEG_INFINITY;
    let mut tv = first_tick;
    while tv <= min_t + span_t + 0.001 {
        let x = x_of(tv);
        let tx = (x - 10.0).clamp(pads.pad_l, w - pads.pad_r - 28.0);
        let label = if tx - last_tick_x >= 44.0 {
            last_tick_x = tx;
            format!("{}s", tv.round() as i64)
        } else {
            String::new()
        };
        grid_x.push(GridXLine {
            x,
            label,
            label_x: tx,
        });
        tv += x_step;
    }

    // Thresholds
    let early_fault_thr = latest_is_fault && x_of(max_t) < pads.pad_l + 170.0;
    let mut thresholds = Vec::new();
    let mut zoom_note = None;
    if layers.thresholds {
        if 55.0 <= max_y {
            thresholds.push(ThresholdLine {
                y: y_of(55.0),
                label: "WASPADA (55)",
                label_x: if early_fault_thr {
                    w - pads.pad_r - 6.0
                } else {
                    pads.pad_l + 6.0
                },
                anchor: if early_fault_thr { "end" } else { "start" },
                color: pal.waspada,
            });
        }
        if 150.0 <= max_y {
            thresholds.push(ThresholdLine {
                y: y_of(150.0),
                label: "BAHAYA (150)",
                label_x: if early_fault_thr {
                    w - pads.pad_r - 6.0
                } else {
                    pads.pad_l + 6.0
                },
                anchor: if early_fault_thr { "end" } else { "start" },
                color: pal.bahaya,
            });
        }
        if zoom_y && max_y < 55.0 {
            let nx = if layers.forecast {
                pads.pad_l + 8.0
            } else {
                w - pads.pad_r - 210.0
            };
            zoom_note = Some((
                "Ambang 55/150 di luar skala ketat".to_string(),
                nx.max(pads.pad_l + 8.0),
                pal.muted,
            ));
        }
    }

    // Historical FAILSAFE bands
    let mut raw_faults: Vec<(f64, f64)> = Vec::new();
    for (i, r) in visible.iter().enumerate() {
        let is_f = r.state == "FAILSAFE" || r.sensor_fault;
        if !is_f {
            continue;
        }
        let next_t = visible.get(i + 1).map(|nr| nr.t).unwrap_or(r.t);
        if let Some(last) = raw_faults.last_mut() {
            if (last.1 - r.t).abs() <= 2.0 {
                last.1 = last.1.max(next_t);
                continue;
            }
        }
        raw_faults.push((r.t, next_t));
    }
    let x_now_edge = if layers.forecast {
        x_of(max_t).min(w - pads.pad_r)
    } else {
        w - pads.pad_r
    };
    let fault_bands: Vec<FaultBand> = raw_faults
        .iter()
        .map(|&(start_t, end_t)| {
            let x1 = x_of(start_t).max(pads.pad_l);
            let clamp_right = if end_t >= max_t {
                x_now_edge
            } else {
                w - pads.pad_r
            };
            let x2 = x_of(end_t).min(clamp_right).max(x1 + 3.0).min(w - pads.pad_r);
            let seg_w = (x2 - x1).max(2.0);
            let draw_right = !(layers.forecast && (x2 - x_of(max_t)).abs() < 2.5);
            let (inline_label, inline_pill_w) = if seg_w >= 124.0 {
                (Some("FAILSAFE (Vd=0V)"), 118.0)
            } else if seg_w >= 64.0 {
                (Some("FAILSAFE"), 66.0)
            } else {
                (None, 0.0)
            };
            let inline_label_x = (x1 + 3.0)
                .max(pads.pad_l + 3.0)
                .min((x_now_edge - inline_pill_w - 4.0).max(pads.pad_l + 3.0));
            let top_tag_w = 46.0;
            let right_limit = if layers.forecast && end_t >= max_t {
                (x_now_edge - top_tag_w - 4.0).max(pads.pad_l + 2.0)
            } else {
                (x_right - top_tag_w - 2.0).max(pads.pad_l + 2.0)
            };
            let top_tag_x = (x1 + seg_w * 0.5 - top_tag_w * 0.5)
                .max(pads.pad_l + 2.0)
                .min(right_limit);
            FaultBand {
                x: x1,
                w: seg_w,
                draw_right_line: draw_right,
                inline_label,
                inline_label_x,
                inline_label_y: (pad_t + plot_h * 0.44).round(),
                inline_pill_w,
                top_tag_label: "FAULT",
                top_tag_x,
                top_tag_w,
            }
        })
        .collect();

    // Build curve polylines
    let make_series = |extract: &dyn Fn(&TelemetryRow) -> Option<f64>,
                       x_offset: f64,
                       color: &'static str,
                       stroke_width: f64,
                       dash: &'static str,
                       opacity: f64|
     -> PolySeries {
        let mut points_list = Vec::new();
        let mut cur = Vec::new();
        let mut prev_t = f64::NEG_INFINITY;
        for r in &visible {
            let val_opt = extract(r).filter(|v| v.is_finite());
            let Some(v) = val_opt else {
                if cur.len() >= 2 {
                    points_list.push(cur.join(" "));
                }
                cur.clear();
                prev_t = f64::NEG_INFINITY;
                continue;
            };
            let rt = r.t + x_offset;
            let x = x_of(rt);
            let y = y_of(v);
            if !cur.is_empty() && rt <= prev_t {
                if cur.len() >= 2 {
                    points_list.push(cur.join(" "));
                }
                cur.clear();
            }
            cur.push(format!("{x:.1},{y:.1}"));
            prev_t = rt;
        }
        if cur.len() >= 2 {
            points_list.push(cur.join(" "));
        }
        PolySeries {
            points_list,
            color,
            stroke_width,
            dash,
            opacity,
        }
    };

    let mut series = Vec::new();
    if layers.pm_raw {
        series.push(make_series(&|r| Some(r.pm_raw), 0.0, pal.pm_raw, 1.4, "", 1.0));
    }
    if layers.pm_true && allow_pm_true {
        series.push(make_series(&|r| r.pm_true, 0.0, pal.pm_true, 2.0, "", 1.0));
    }
    if layers.voltages {
        series.push(make_series(
            &|r| Some(r.v_dust * 50.0),
            0.0,
            pal.v_dust,
            1.6,
            "4 3",
            1.0,
        ));
        series.push(make_series(
            &|r| Some(r.v_mq * 50.0),
            0.0,
            pal.v_mq,
            1.6,
            "6 3",
            1.0,
        ));
    }
    if layers.gas_ppm {
        series.push(make_series(
            &|r| Some(r.gas_ppm * 0.2),
            0.0,
            pal.gas_ppm,
            1.8,
            "4 3",
            1.0,
        ));
    }
    if layers.g_scaled {
        series.push(make_series(
            &|r| Some(r.g * 100.0),
            0.0,
            pal.g_scaled,
            1.8,
            "2 3",
            1.0,
        ));
    }
    if layers.pm {
        series.push(make_series(&|r| Some(r.pm), 0.0, pal.pm, 2.4, "", 1.0));
    }
    if layers.fi {
        series.push(make_series(
            &|r| {
                if r.state == "FAILSAFE" || r.sensor_fault {
                    None
                } else {
                    Some(r.fi)
                }
            },
            0.0,
            pal.fi,
            2.8,
            "",
            1.0,
        ));
        if !fault_bands.is_empty() {
            series.push(make_series(
                &|r| {
                    if r.state == "FAILSAFE" || r.sensor_fault {
                        Some(r.fi)
                    } else {
                        None
                    }
                },
                0.0,
                pal.fault,
                1.8,
                "5 4",
                1.0,
            ));
        }
    }
    let mut forecast_tip = None;
    if layers.forecast && !latest_is_fault {
        series.push(make_series(
            &|r| {
                if r.t + 60.0 <= max_t {
                    r.forecast
                } else {
                    None
                }
            },
            60.0,
            pal.forecast,
            1.6,
            "2 3",
            0.52,
        ));
        if let Some(lat) = latest_vis {
            if let Some(fc) = lat.forecast {
                forecast_tip = Some(ActiveForecastTip {
                    x0: x_of(max_t),
                    y0: y_of(lat.pm),
                    x1: x_of(max_t + 60.0),
                    y1: y_of(fc),
                    color: pal.forecast,
                });
            }
        }
    }

    // Sub-chart Duty step function + 4-state FSM ribbon
    let mut duty_pts = Vec::with_capacity(visible.len() * 2);
    let mut prev_t = f64::NEG_INFINITY;
    let mut prev_y: Option<f64> = None;
    for (idx, r) in visible.iter().enumerate() {
        let x = x_of(r.t);
        let y = sy_of(r.duty);
        if idx == 0 || r.t <= prev_t || prev_y.is_none() {
            duty_pts.push(format!("{x:.1},{y:.1}"));
        } else {
            duty_pts.push(format!("{x:.1},{:.1}", prev_y.unwrap()));
            duty_pts.push(format!("{x:.1},{y:.1}"));
        }
        prev_t = r.t;
        prev_y = Some(y);
    }

    let mut raw_segs: Vec<(String, f64, f64)> = Vec::new();
    for (i, r) in visible.iter().enumerate() {
        let next_t = visible.get(i + 1).map(|nr| nr.t).unwrap_or(r.t);
        if let Some(last) = raw_segs.last_mut() {
            if last.0 == r.state {
                last.2 = last.2.max(next_t);
                continue;
            }
        }
        raw_segs.push((r.state.clone(), r.t, next_t));
    }

    let sx_now_bound = if layers.forecast {
        x_of(max_t).min(sw - pads.pad_r)
    } else {
        sw - pads.pad_r
    };
    let mut ribbon_segments = Vec::with_capacity(raw_segs.len());
    let mut last_labeled = false;
    let mut last_seg_x2 = pads.pad_l;
    let seg_count = raw_segs.len();
    for (idx, (st_name, start_t, end_t)) in raw_segs.into_iter().enumerate() {
        let (fill, stroke, short, hatch) = match st_name.as_str() {
            "WASPADA" => (
                if dark {
                    "rgba(251,191,36,0.26)"
                } else {
                    "rgba(146,64,14,0.18)"
                },
                pal.waspada,
                "WASP",
                false,
            ),
            "BAHAYA" => (
                if dark {
                    "rgba(248,113,113,0.30)"
                } else {
                    "rgba(185,28,28,0.20)"
                },
                pal.bahaya,
                "BHY",
                false,
            ),
            "FAILSAFE" => (pal.fault_fill, pal.fault, "FAIL", true),
            _ => (
                if dark {
                    "rgba(74,222,128,0.22)"
                } else {
                    "rgba(20,83,45,0.15)"
                },
                pal.pm_true,
                "NORM",
                false,
            ),
        };
        let x1 = x_of(start_t).max(pads.pad_l);
        let raw_x2 = x_of(end_t);
        let clamp_r = if end_t >= max_t {
            sx_now_bound.max(x1 + 3.0)
        } else {
            sw - pads.pad_r
        };
        let x2 = raw_x2.min(clamp_r).max(x1 + 3.0).min(sw - pads.pad_r);
        let seg_w = (x2 - x1).max(2.0);
        if idx + 1 == seg_count {
            last_seg_x2 = x2;
        }
        let est_full_w = st_name.len() as f64 * 7.4;
        let est_short_w = short.len() as f64 * 7.4;
        let (label, lbl_w) = if est_full_w + 10.0 <= seg_w {
            (st_name.clone(), est_full_w)
        } else {
            (short.to_string(), est_short_w)
        };
        let draw_inner_label = lbl_w + 8.0 <= seg_w;
        if draw_inner_label && idx + 1 == seg_count {
            last_labeled = true;
        }
        ribbon_segments.push(RibbonSegment {
            state: st_name,
            label,
            x: x1,
            w: seg_w,
            fill,
            stroke,
            hatch,
            draw_inner_label,
            text_x: x1 + seg_w * 0.5,
        });
    }

    let tail_label = if !last_labeled {
        ribbon_segments.last().map(|last_seg| {
            let lbl = if pads.fsm_compact {
                match last_seg.state.as_str() {
                    "WASPADA" => "WASP".to_string(),
                    "BAHAYA" => "BHY".to_string(),
                    "FAILSAFE" => "FAIL".to_string(),
                    _ => "NORM".to_string(),
                }
            } else {
                last_seg.state.clone()
            };
            let tail_w = lbl.len() as f64 * 7.4;
            let fit_past = (sx_now_bound - pads.pad_l) >= (tail_w + 6.0);
            let tail_x = if fit_past {
                (last_seg_x2 - tail_w - 3.0)
                    .min(sx_now_bound - tail_w - 3.0)
                    .max(pads.pad_l + 2.0)
            } else {
                (last_seg_x2 + 6.0).min(sw - tail_w - 6.0)
            };
            (lbl, tail_x, last_seg.stroke)
        })
    } else {
        None
    };

    let sub_horizon = if layers.forecast && x_of(max_t) < (sw - pads.pad_r) {
        let sx = x_of(max_t);
        Some((
            sx,
            duty_top,
            (sw - pads.pad_r) - sx,
            (ribbon_top + ribbon_h) - duty_top,
            if latest_is_fault {
                pal.suspended_fill
            } else {
                pal.forecast_fill
            },
            if latest_is_fault {
                pal.suspended_line
            } else {
                pal.now_line
            },
        ))
    } else {
        None
    };

    // Synchronized Crosshair
    let crosshair = hover_t.and_then(|ht| {
        if ht < min_t || ht > min_t + span_t || visible.is_empty() {
            return None;
        }
        let mut nearest = visible[0];
        for &r in visible.iter().skip(1) {
            if (r.t - ht).abs() < (nearest.t - ht).abs() {
                nearest = r;
            }
        }
        let row_is_fault = nearest.state == "FAILSAFE" || nearest.sensor_fault;
        let cx = x_of(nearest.t);
        let mut lines = vec![
            format!("t={:.0}s | FSM:{}", nearest.t, nearest.state),
            if row_is_fault {
                format!("FI:TIDAK VALID | PM:beku ({:.1})", nearest.pm)
            } else {
                format!("FI:{:.1} | PM:{:.1}", nearest.fi, nearest.pm)
            },
            format!("Gas:{:.0}ppm (g={:.2})", nearest.gas_ppm, nearest.g),
            if row_is_fault {
                format!("Duty:{:.0}% | Fc:DITANGGUHKAN", nearest.duty)
            } else {
                format!(
                    "Duty:{:.0}% | Fc:{}",
                    nearest.duty,
                    nearest
                        .forecast
                        .map(|f| format!("{f:.1}"))
                        .unwrap_or_else(|| "-".to_string())
                )
            },
        ];
        if (layers.pm_true && allow_pm_true) || layers.pm_raw {
            let pt = nearest
                .pm_true
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "-".to_string());
            lines.push(if row_is_fault {
                format!("True:{pt} | Mentah:-- | dC:beku")
            } else {
                format!(
                    "True:{pt} | Mentah:{:.1} | dC:{:.2}",
                    nearest.pm_raw, nearest.dcdt
                )
            });
        }
        if layers.voltages {
            lines.push(format!(
                "Vd:{:.2}V | Vm:{:.2}V",
                nearest.v_dust, nearest.v_mq
            ));
        }
        if let (Some(dfi), Some(rfi)) = (nearest.parity_delta_fi, nearest.ref_fi) {
            lines.push(format!("RefFI:{rfi:.1} | dFI:{dfi:.2}"));
        }

        let live_readout = if row_is_fault {
            format!(
                "Inspeksi t={:.0} detik, FAILSAFE, FI tidak valid (sensor debu putus), PM beku {:.1}, Prakiraan ditangguhkan, Kipas {:.0} persen.",
                nearest.t, nearest.pm, nearest.duty
            )
        } else {
            format!(
                "Inspeksi t={:.0} detik, {}, FI {:.1}, PM {:.1}, Kipas {:.0} persen.",
                nearest.t, nearest.state, nearest.fi, nearest.pm, nearest.duty
            )
        };

        let max_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(18) as f64;
        let box_w = (max_chars * 7.2 + 18.0).clamp(184.0, (w - 16.0).min(272.0));
        let box_h = 14.0 + (lines.len() as f64) * 17.0;
        let mut box_x = cx + 10.0;
        if box_x + box_w > w - pads.pad_r - 4.0 {
            box_x = cx - box_w - 10.0;
        }
        box_x = box_x.max(pads.pad_l + 4.0).min(w - box_w - 6.0);

        Some(CrosshairTooltip {
            x: cx,
            box_x,
            box_y: pad_t + 4.0,
            box_w,
            box_h,
            lines,
            live_readout,
        })
    });

    let last_row = visible.last().unwrap();
    let last_is_fault = last_row.state == "FAILSAFE" || last_row.sensor_fault;
    let sr_summary = if last_is_fault {
        format!(
            "Sampel terbaru t={:.0}s, State FAILSAFE, FI tidak valid (sensor debu putus), PM beku {:.1}, Kipas {:.0}%.",
            last_row.t, last_row.pm, last_row.duty
        )
    } else {
        format!(
            "Sampel terbaru t={:.0}s, State {}, FI {:.1}, PM {:.1}, Kipas {:.0}%.",
            last_row.t, last_row.state, last_row.fi, last_row.pm, last_row.duty
        )
    };
    let sub_sr = format!(
        "Sub-grafik t={:.0}s: Duty Kipas {:.0}%, Pita State FSM {} ({}).",
        last_row.t, last_row.duty, last_row.state, last_row.mode
    );

    let main = MainChartModel {
        w,
        h,
        pad_l: pads.pad_l,
        pad_r: pads.pad_r,
        pad_t,
        pad_b,
        plot_w,
        plot_h,
        min_t,
        span_t,
        empty_message: None,
        grid_y,
        grid_x,
        right_header_spans,
        thresholds,
        zoom_note,
        horizon,
        fault_bands,
        series,
        forecast_tip,
        crosshair: crosshair.clone(),
        sr_summary,
    };

    let sub = SubChartModel {
        w: sw,
        h: sh,
        pad_l: pads.pad_l,
        pad_r: pads.pad_r,
        duty_top,
        duty_h,
        duty_bot,
        sep_y,
        ribbon_top,
        ribbon_h,
        compact: pads.compact,
        empty_message: None,
        duty_y_ticks: vec![(sy_of(0.0), "0%"), (sy_of(50.0), "50%"), (sy_of(100.0), "100%")],
        y70: sy_of(70.0),
        duty_points: duty_pts.join(" "),
        ribbon_segments,
        tail_label,
        horizon: sub_horizon,
        crosshair_x: crosshair.as_ref().map(|c| c.x),
        sr_summary: sub_sr,
    };

    (main, sub)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Renders a standalone composite SVG string containing opaque background, title, wrapped legend,
/// main overlay chart, and dual-lane sub-chart — used for both `.svg` export and `.png` rasterization.
pub fn render_standalone_export_svg(
    main: &MainChartModel,
    sub: &SubChartModel,
    legend: &[LegendItem],
    pal: &ChartPalette,
    title: &str,
    subtitle: &str,
) -> (String, u32, u32) {
    let cw = main.w.round() as u32;
    let mh = main.h.round() as u32;
    let sh = sub.h.round() as u32;

    let mut rows: Vec<Vec<&LegendItem>> = Vec::new();
    let mut cur: Vec<&LegendItem> = Vec::new();
    let mut cur_w = 0.0f64;
    let max_row_w = (main.w - 16.0).max(220.0);
    for item in legend {
        let iw = 34.0 + (item.label.len() as f64) * 6.8;
        if !cur.is_empty() && cur_w + iw > max_row_w {
            rows.push(cur);
            cur = vec![item];
            cur_w = iw;
        } else {
            cur.push(item);
            cur_w += iw;
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }

    let header_h: u32 = if rows.is_empty() {
        48
    } else {
        48 + (rows.len() as u32) * 20
    };
    let gap: u32 = 10;
    let total_w = cw + 24;
    let total_h = header_h + mh + gap + sh + 16;
    let mut s = String::with_capacity(8192);

    s.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{total_w}" height="{total_h}" viewBox="0 0 {total_w} {total_h}" font-family="system-ui, -apple-system, sans-serif">"#
    ));
    s.push_str(&format!(
        r#"<defs>
<pattern id="exp-hatch" width="7" height="7" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
  <line x1="0" y1="0" x2="0" y2="7" stroke="{}" stroke-width="2" />
</pattern>
</defs>"#,
        pal.fault_hatch
    ));
    s.push_str(&format!(
        r#"<rect x="0" y="0" width="{total_w}" height="{total_h}" fill="{}"/>"#,
        pal.export_bg
    ));
    s.push_str(&format!(
        r#"<text x="14" y="22" font-size="14" font-weight="700" fill="{}">{}</text>"#,
        pal.tip_fg,
        xml_escape(title)
    ));
    s.push_str(&format!(
        r#"<text x="14" y="38" font-size="12" fill="{}">{}</text>"#,
        pal.muted,
        xml_escape(subtitle)
    ));

    for (r_idx, r_items) in rows.iter().enumerate() {
        let mut lx = 14.0f64;
        let ly = 58.0 + (r_idx as f64) * 19.0;
        for it in r_items {
            let dash = match it.style {
                "dashed" => r#" stroke-dasharray="5 3""#,
                "dotted" => r#" stroke-dasharray="2 3""#,
                _ => "",
            };
            let op = if it.dimmed { "0.55" } else { "1" };
            s.push_str(&format!(
                r#"<line x1="{lx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="2.5"{dash} opacity="{op}"/>"#,
                ly - 4.0,
                lx + 16.0,
                ly - 4.0,
                it.color
            ));
            s.push_str(&format!(
                r#"<text x="{:.1}" y="{ly:.1}" font-size="12" font-weight="600" fill="{}" opacity="{op}">{}</text>"#,
                lx + 20.0,
                pal.tip_fg,
                xml_escape(it.label)
            ));
            lx += 34.0 + (it.label.len() as f64) * 6.8;
        }
    }

    // Main chart group
    s.push_str(&format!(r#"<g transform="translate(12,{header_h})">"#));
    s.push_str(&format!(
        r#"<rect x="0" y="0" width="{:.1}" height="{:.1}" rx="10" fill="{}" stroke="{}" />"#,
        main.w, main.h, pal.export_bg, pal.grid
    ));
    if let Some(ref hz) = main.horizon {
        s.push_str(&format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{}"/>"#,
            hz.x, hz.y, hz.w, hz.h, hz.fill
        ));
        s.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="1.25" stroke-dasharray="4 3"/>"#,
            hz.x, hz.y, hz.x, main.h - main.pad_b, hz.stroke
        ));
        s.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-size="11" font-weight="700" text-anchor="{}" fill="{}">{}</text>"#,
            hz.label_x,
            hz.y + 14.0,
            hz.label_anchor,
            hz.label_color,
            xml_escape(hz.label)
        ));
    }
    for gy in &main.grid_y {
        s.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" />"#,
            main.pad_l, gy.y, main.w - main.pad_r, gy.y, pal.grid
        ));
        s.push_str(&format!(
            r#"<text x="6" y="{:.1}" font-size="11" font-family="monospace" fill="{}">{}</text>"#,
            gy.y + 4.0,
            pal.text,
            xml_escape(&gy.left_label)
        ));
    }
    for gx in &main.grid_x {
        s.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" />"#,
            gx.x, main.pad_t, gx.x, main.h - main.pad_b, pal.grid
        ));
        if !gx.label.is_empty() {
            s.push_str(&format!(
                r#"<text x="{:.1}" y="{:.1}" font-size="11" font-family="monospace" fill="{}">{}</text>"#,
                gx.label_x,
                main.h - 9.0,
                pal.text,
                xml_escape(&gx.label)
            ));
        }
    }
    for thr in &main.thresholds {
        s.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="1.5" stroke-dasharray="7 5"/>"#,
            main.pad_l, thr.y, main.w - main.pad_r, thr.y, thr.color
        ));
        s.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-size="11" font-weight="700" text-anchor="{}" fill="{}">{}</text>"#,
            thr.label_x, thr.y - 4.0, thr.anchor, thr.color, xml_escape(thr.label)
        ));
    }
    for fb in &main.fault_bands {
        s.push_str(&format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{}"/>"#,
            fb.x, main.pad_t, fb.w, main.plot_h, pal.fault_fill
        ));
        s.push_str(&format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="url(#exp-hatch)"/>"#,
            fb.x, main.pad_t, fb.w, main.plot_h
        ));
        s.push_str(&format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="15" rx="4" fill="{}" stroke="{}" stroke-width="1"/>"#,
            fb.top_tag_x, main.pad_t + 2.0, fb.top_tag_w, pal.tip_bg, pal.fault
        ));
        s.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-size="10" font-weight="700" text-anchor="middle" fill="{}">FAULT</text>"#,
            fb.top_tag_x + fb.top_tag_w * 0.5,
            main.pad_t + 13.0,
            pal.fault
        ));
    }
    for ser in &main.series {
        for pts in &ser.points_list {
            let dash = if ser.dash.is_empty() {
                String::new()
            } else {
                format!(r#" stroke-dasharray="{}""#, ser.dash)
            };
            s.push_str(&format!(
                r#"<polyline fill="none" stroke="{}" stroke-width="{:.2}" opacity="{:.2}"{dash} points="{pts}" stroke-linejoin="round" stroke-linecap="round"/>"#,
                ser.color, ser.stroke_width, ser.opacity
            ));
        }
    }
    if let Some(ref tip) = main.forecast_tip {
        s.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="2.5" stroke-dasharray="5 4"/>"#,
            tip.x0, tip.y0, tip.x1, tip.y1, tip.color
        ));
        s.push_str(&format!(
            r#"<circle cx="{:.1}" cy="{:.1}" r="4" fill="{}"/>"#,
            tip.x1, tip.y1, tip.color
        ));
    }
    s.push_str("</g>");

    // Sub chart group
    let sub_y = header_h + mh + gap;
    s.push_str(&format!(r#"<g transform="translate(12,{sub_y})">"#));
    s.push_str(&format!(
        r#"<rect x="0" y="0" width="{:.1}" height="{:.1}" rx="10" fill="{}" stroke="{}" />"#,
        sub.w, sub.h, pal.export_bg, pal.grid
    ));
    for &(y, lbl) in &sub.duty_y_ticks {
        s.push_str(&format!(
            r#"<line x1="{:.1}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="{}"/>"#,
            sub.pad_l, sub.w - sub.pad_r, pal.grid
        ));
        s.push_str(&format!(
            r#"<text x="6" y="{:.1}" font-size="11" font-family="monospace" fill="{}">{lbl}</text>"#,
            y + 4.0, pal.text
        ));
    }
    s.push_str(&format!(
        r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-dasharray="4 4" opacity="0.7"/>"#,
        sub.pad_l, sub.y70, sub.w - sub.pad_r, sub.y70, pal.fault
    ));
    if !sub.duty_points.is_empty() {
        s.push_str(&format!(
            r#"<polyline fill="none" stroke="{}" stroke-width="2.4" points="{}" />"#,
            pal.duty, sub.duty_points
        ));
    }
    for seg in &sub.ribbon_segments {
        s.push_str(&format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{}" stroke="{}" stroke-width="1.3"/>"#,
            seg.x, sub.ribbon_top, seg.w, sub.ribbon_h, seg.fill, seg.stroke
        ));
        if seg.hatch {
            s.push_str(&format!(
                r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="url(#exp-hatch)"/>"#,
                seg.x, sub.ribbon_top, seg.w, sub.ribbon_h
            ));
        }
        if seg.draw_inner_label {
            s.push_str(&format!(
                r#"<text x="{:.1}" y="{:.1}" font-size="11" font-weight="700" text-anchor="middle" fill="{}">{}</text>"#,
                seg.text_x,
                sub.ribbon_top + sub.ribbon_h * 0.5 + 4.0,
                pal.tip_fg,
                xml_escape(&seg.label)
            ));
        }
    }
    s.push_str("</g></svg>");

    (s, total_w, total_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(t: f64, fi: f64, pm: f64, state: &str, sensor_fault: bool) -> TelemetryRow {
        let mut r = TelemetryRow::idle();
        r.t = t;
        r.t_s = t;
        r.seq = (t as u32) + 1;
        r.scenario = "flaming".to_string();
        r.fi = fi;
        r.pm = pm;
        r.pm_raw = pm + 1.0;
        r.pm_true = Some(pm - 0.5);
        r.gas_ppm = 480.0;
        r.g = 0.25;
        r.state = state.to_string();
        r.duty = if state == "FAILSAFE" { 70.0 } else { 100.0 };
        r.forecast = if sensor_fault { None } else { Some(pm + 25.0) };
        r.sensor_fault = sensor_fault;
        r
    }

    #[test]
    fn test_nice_step_and_empty_model() {
        assert!((nice_step(38.0) - 50.0).abs() < 1e-6);
        assert!((nice_step(110.0) - 200.0).abs() < 1e-6);
        let layers = LayerVisibility::default();
        let (main, sub) = build_chart_models(
            &[],
            &layers,
            true,
            300,
            false,
            false,
            960.0,
            310.0,
            960.0,
            148.0,
            None,
            "sitl",
            false,
        );
        assert!(main.empty_message.is_some());
        assert!(sub.empty_message.is_some());
        assert!(main.series.is_empty());
    }

    #[test]
    fn test_failsafe_tooltip_hides_invalid_fi_and_suspends_forecast() {
        let rows = vec![
            sample_row(0.0, 20.0, 12.0, "NORMAL", false),
            sample_row(2.0, 0.0, 12.0, "FAILSAFE", true),
            sample_row(4.0, 0.0, 12.0, "FAILSAFE", true),
        ];
        let layers = LayerVisibility::default();
        let (main, _sub) = build_chart_models(
            &rows,
            &layers,
            true,
            300,
            false,
            true,
            960.0,
            310.0,
            960.0,
            148.0,
            Some(4.0),
            "sitl",
            false,
        );
        let ch = main.crosshair.expect("crosshair active");
        let joined = ch.lines.join("\n");
        assert!(joined.contains("FI:TIDAK VALID"));
        assert!(joined.contains("Fc:DITANGGUHKAN"));
        assert!(!joined.contains("FI:0.0"));
        let hz = main.horizon.expect("horizon shown");
        assert_eq!(hz.label, "PRAKIRAAN DITANGGUHKAN");
    }

    #[test]
    fn test_narrow_fault_band_always_emits_tag_and_splits_fi_segments() {
        let rows = vec![
            sample_row(0.0, 25.0, 18.0, "NORMAL", false),
            sample_row(2.0, 60.0, 42.0, "WASPADA", false),
            sample_row(4.0, 0.0, 42.0, "FAILSAFE", true),
        ];
        let layers = LayerVisibility::default();
        let (main, sub) = build_chart_models(
            &rows,
            &layers,
            true,
            300,
            false,
            false,
            374.0,
            178.0,
            374.0,
            92.0,
            None,
            "sitl",
            false,
        );
        assert_eq!(main.fault_bands.len(), 1);
        assert_eq!(main.fault_bands[0].top_tag_label, "FAULT");
        assert!(sub.ribbon_segments.iter().any(|s| s.hatch && s.state == "FAILSAFE"));
        // Normal/Waspada FI segment + FAILSAFE dashed FI segment
        let fi_polylines: Vec<_> = main.series.iter().collect();
        assert!(fi_polylines.len() >= 3);
    }

    #[test]
    fn test_standalone_export_svg_well_formed() {
        let rows = vec![
            sample_row(0.0, 30.0, 20.0, "NORMAL", false),
            sample_row(10.0, 165.0, 110.0, "BAHAYA", false),
        ];
        let layers = LayerVisibility::default();
        let pal = ChartPalette::for_theme(true);
        let (main, sub) = build_chart_models(
            &rows,
            &layers,
            true,
            300,
            false,
            true,
            960.0,
            310.0,
            960.0,
            148.0,
            None,
            "sitl",
            false,
        );
        let legend = legend_specs(&layers, true, false, &pal);
        let (svg, w, h) = render_standalone_export_svg(
            &main,
            &sub,
            &legend,
            &pal,
            "Karhutla Edge-IoT — SITL",
            "Preset: Demo Sidang",
        );
        assert!(w > 960 && h > 450);
        assert!(svg.starts_with("<svg "));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("<polyline"));
    }
}
