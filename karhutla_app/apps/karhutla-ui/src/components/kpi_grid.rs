use crate::components::icons::{IcoClock, IcoCloud, IcoFan, IcoFlask, IcoFsm, IcoPulse};
use crate::state::{abbr_help_map, describe_fan_state, format_clock, format_time_s, AbbrPopoverState, AppState};
use dioxus::prelude::*;

fn open_abbr_help(mut state: Signal<AppState>, key: &'static str, evt: MouseEvent) {
    let map = abbr_help_map();
    let Some(body) = map.get(key) else {
        return;
    };
    let coords = evt.client_coordinates();
    let toggle_off = state
        .read()
        .abbr_popover
        .as_ref()
        .map(|p| p.title == key)
        .unwrap_or(false);
    if toggle_off {
        state.write().abbr_popover = None;
        return;
    }
    state.write().abbr_popover = Some(AbbrPopoverState {
        title: key.to_string(),
        body: (*body).to_string(),
        x: (coords.x - 20.0).max(12.0),
        y: (coords.y + 18.0).max(12.0),
    });
}

#[component]
pub fn KpiGrid() -> Element {
    let state = use_context::<Signal<AppState>>();
    let s = state.read();
    let src = s.active_source();
    let src_tag = if src == "esp32" { "ESP32" } else { "SITL" };
    let row = s.active_latest();

    let (
        state_tone,
        state_color_class,
        state_text,
        fsm_icon_key,
        mode_text,
        t_text,
        t_dur_text,
        seq_text,
    ) = if let Some(r) = row {
        let tone = match r.state.as_str() {
            "WASPADA" => "warn",
            "BAHAYA" => "danger",
            "FAILSAFE" => "fault",
            _ => "ok",
        };
        let col = match r.state.as_str() {
            "WASPADA" => "tone-warn",
            "BAHAYA" => "tone-danger",
            "FAILSAFE" => "tone-fault",
            _ => "tone-ok",
        };
        let show_dur = s.sim.duration > 0.0
            && (s.sim.running || src == "sitl" || s.sim.target == "hils");
        (
            tone,
            col,
            r.state.clone(),
            r.state.clone(),
            r.mode.clone(),
            if show_dur {
                format_clock(r.t)
            } else {
                format_time_s(r.t)
            },
            if show_dur {
                Some(format!(" / {}", format_clock(s.sim.duration)))
            } else {
                None
            },
            r.seq.to_string(),
        )
    } else {
        (
            "idle",
            "tone-muted",
            "BELUM ADA DATA".to_string(),
            "IDLE".to_string(),
            "--".to_string(),
            "0s".to_string(),
            None,
            "0".to_string(),
        )
    };

    // Single consolidated HILS Parity status inside State card
    let parity_ui = {
        let hp = &s.sim.hils_parity;
        if hp.samples > 0 && hp.last_delta_fi.is_some() && (src == "esp32" || s.sim.target == "hils") {
            let ok = hp.max_delta_fi < 1.0 && hp.state_matches == hp.samples;
            Some((
                format!(
                    "maks Δ{:.2} • {}/{} ({})",
                    hp.max_delta_fi,
                    hp.state_matches,
                    hp.samples,
                    if ok { "OK" } else { "Cek" }
                ),
                if ok { "tone-ok" } else { "tone-warn" },
            ))
        } else if s.sim.running && s.sim.target == "hils" {
            Some(("menunggu".to_string(), "tone-muted"))
        } else {
            None
        }
    };

    let is_fault = row
        .map(|r| r.state == "FAILSAFE" || r.sensor_fault)
        .unwrap_or(false);

    // FI card (Run 12 P1: calm neutral dashed suspended border during FAILSAFE, no magenta wash)
    let (fi_card_tone, fi_val_class, fi_val_text, fi_show_unit, fi_pct, fi_fill_color, fi_foot_l, fi_foot_r) =
        if let Some(r) = row {
            if is_fault {
                (
                    "suspended",
                    "tone-muted",
                    "TIDAK VALID".to_string(),
                    false,
                    0.0,
                    "var(--text-muted)",
                    "Sensor debu putus",
                    "Kipas 70%",
                )
            } else {
                let (tone, cls, bar_col) = if r.fi >= 150.0 {
                    ("danger", "tone-danger", "var(--danger)")
                } else if r.fi >= 55.0 {
                    ("warn", "tone-warn", "var(--warn)")
                } else {
                    ("ok", "tone-main", "var(--ok)")
                };
                (
                    tone,
                    cls,
                    format!("{:.1}", r.fi),
                    true,
                    ((r.fi / 250.0) * 100.0).clamp(0.0, 100.0),
                    bar_col,
                    "Waspada ≥ 55",
                    "Bahaya ≥ 150",
                )
            }
        } else {
            (
                "idle",
                "tone-muted",
                "--".to_string(),
                true,
                0.0,
                "var(--ok)",
                "Waspada ≥ 55",
                "Bahaya ≥ 150",
            )
        };

    // PM Kalman card (suspended during FAILSAFE without magenta glare)
    let (pm_card_tone, pm_val_class, pm_text, pm_show_unit, pm_frozen, pm_raw_txt, dcdt_txt, vd_txt, v0_txt) =
        if let Some(r) = row {
            if is_fault {
                (
                    Some("suspended"),
                    "tone-muted",
                    "DITANGGUHKAN".to_string(),
                    false,
                    Some(format!("Beku: {:.1} µg/m³", r.pm)),
                    "--".to_string(),
                    "-- (beku)".to_string(),
                    format!("{:.2}", r.v_dust),
                    format!("{:.3}", r.v0),
                )
            } else {
                (
                    None,
                    "tone-accent",
                    format!("{:.1}", r.pm),
                    true,
                    None,
                    format!("{:.1}", r.pm_raw),
                    format!("{:+.2}", r.dcdt),
                    format!("{:.2}", r.v_dust),
                    format!("{:.3}", r.v0),
                )
            }
        } else {
            (
                None,
                "tone-accent",
                "--".to_string(),
                true,
                None,
                "--".to_string(),
                "0.00".to_string(),
                "--".to_string(),
                "--".to_string(),
            )
        };

    // Gas card
    let (ppm_txt, g_txt, vm_txt) = if let Some(r) = row {
        (
            format!("{:.0}", r.gas_ppm),
            format!("{:.2}", r.g),
            format!("{:.2}", r.v_mq),
        )
    } else {
        ("--".into(), "--".into(), "--".into())
    };

    // Forecast AI card
    let (fc_card_tone, fc_val_class, fc_text, fc_show_unit, fc_frozen, eta_txt, mae_txt) =
        if let Some(r) = row {
            let mae_s = r
                .mae
                .map(|m| format!("{m:.2}"))
                .unwrap_or_else(|| "-".into());
            if is_fault {
                let froz = match r.forecast {
                    Some(f) => format!("Beku: {f:.1} µg/m³"),
                    None => "Dijeda (sensor putus)".to_string(),
                };
                (
                    Some("suspended"),
                    "tone-muted",
                    "DITANGGUHKAN".to_string(),
                    false,
                    Some(froz),
                    "— (FAILSAFE)".to_string(),
                    mae_s,
                )
            } else {
                let (fc_s, fc_cls) = match r.forecast {
                    Some(f) if f >= 150.0 => (format!("{f:.1}"), "tone-danger"),
                    Some(f) if f >= 55.0 => (format!("{f:.1}"), "tone-warn"),
                    Some(f) => (format!("{f:.1}"), "tone-violet"),
                    None => ("-".to_string(), "tone-muted"),
                };
                let eta = if r.state == "BAHAYA" {
                    "Tercapai".to_string()
                } else {
                    r.eta_bahaya_s
                        .map(|e| format!("{e:.0} s"))
                        .unwrap_or_else(|| "-".into())
                };
                (None, fc_cls, fc_s, true, None, eta, mae_s)
            }
        } else {
            (
                None,
                "tone-muted",
                "--".to_string(),
                true,
                None,
                "-".to_string(),
                "-".to_string(),
            )
        };

    // Duty Fan card (Run 12 P1: during FAILSAFE use active safety `--accent`, NEVER magenta)
    let (duty_card_tone, duty_val_class, duty_fill_col, duty_txt, fan_ctrl, fan_state) =
        if let Some(r) = row {
            let (ctone, vcls, fcol) = if r.mode == "FORCE_OFF" {
                (Some("warn"), "tone-warn", "var(--warn)")
            } else if r.state == "FAILSAFE" {
                (Some("safety"), "tone-accent", "var(--accent)")
            } else if r.state == "BAHAYA" {
                (Some("danger"), "tone-danger", "var(--danger)")
            } else if r.duty > 0.0 {
                (Some("safety"), "tone-accent", "var(--accent)")
            } else {
                (None, "tone-ok", "var(--ok)")
            };
            let ctrl = if s.sim.ai_preempt_active && r.mode == "FORCE_MAX" {
                "AI PREEMPT"
            } else {
                r.mode.as_str()
            };
            (
                ctone,
                vcls,
                fcol,
                format!("{:.1}", r.duty),
                ctrl.to_string(),
                describe_fan_state(r).to_string(),
            )
        } else {
            (
                None,
                "tone-muted",
                "var(--accent)",
                "--".to_string(),
                "--".to_string(),
                "--".to_string(),
            )
        };
    let duty_pct = row.map(|r| r.duty.clamp(0.0, 100.0)).unwrap_or(0.0);
    drop(s);

    rsx! {
        section { class: "kpi-grid", aria_label: "Indikator Utama Sensor dan Aktuator",
            // 1. State FSM & Mode
            article { class: "kpi-card", id: "card-state", "data-primary": "true", "data-tone": "{state_tone}",
                header {
                    span { "State FSM & Mode" }
                    span { class: "pill-badge", id: "kpi-src-tag", "{src_tag}" }
                }
                div { id: "kpi-state-wrap", class: "kpi-value kpi-state-wrap {state_color_class}",
                    IcoFsm { state: fsm_icon_key }
                    span { id: "kpi-state", class: "kpi-state-text", "{state_text}" }
                }
                footer {
                    span { class: "kpi-atom", "Mode: " strong { id: "kpi-mode", "{mode_text}" } }
                    span { class: "kpi-atom",
                        "t="
                        span { id: "kpi-t", "{t_text}" }
                        if let Some(dur) = t_dur_text {
                            span { id: "kpi-t-dur", "{dur}" }
                        }
                        " (#"
                        span { id: "kpi-seq", "{seq_text}" }
                        ")"
                    }
                    if let Some((ptxt, pcls)) = parity_ui {
                        span {
                            id: "kpi-parity-wrap",
                            class: "kpi-parity-tag kpi-atom",
                            title: "Kriteria paritas OK: |ΔFI| maks < 1,0 dan 100% state cocok",
                            "Paritas: "
                            strong { id: "kpi-parity", class: "{pcls}", "{ptxt}" }
                        }
                    }
                }
            }

            // 2. Fusion Index (FI)
            article { class: "kpi-card", id: "card-fi", "data-primary": "true", "data-tone": "{fi_card_tone}",
                header {
                    span { "Fusion Index (FI)" }
                    IcoPulse {}
                }
                div {
                    div { id: "kpi-fi-wrap", class: "kpi-value {fi_val_class}",
                        span { id: "kpi-fi", "{fi_val_text}" }
                        if fi_show_unit {
                            span { id: "kpi-fi-unit", class: "kpi-unit", "µg/m³" }
                        }
                    }
                    div { class: "kpi-meter", aria_hidden: "true",
                        div {
                            id: "fi-meter-fill",
                            class: "kpi-meter-fill",
                            style: "inline-size: {fi_pct:.1}%; background-color: {fi_fill_color};",
                        }
                        span { class: "fi-tick fi-tick-waspada" }
                        span { class: "fi-tick fi-tick-bahaya" }
                    }
                }
                footer {
                    span { id: "kpi-fi-foot-left", class: "kpi-atom", "{fi_foot_l}" }
                    span { id: "kpi-fi-foot-right", class: "kpi-atom", "{fi_foot_r}" }
                }
            }

            // 3. PM-proxy Kalman
            article {
                class: "kpi-card",
                id: "card-pm",
                "data-tone": pm_card_tone.unwrap_or(""),
                header {
                    span { "PM-proxy Kalman" }
                    IcoCloud {}
                }
                div {
                    div { id: "kpi-pm-wrap", class: "kpi-value {pm_val_class}",
                        span { id: "kpi-pm", "{pm_text}" }
                        if pm_show_unit {
                            span { id: "kpi-pm-unit", class: "kpi-unit", "µg/m³" }
                        }
                    }
                    if let Some(froz) = pm_frozen {
                        div { id: "kpi-pm-frozen", class: "kpi-frozen-note", "{froz}" }
                    }
                }
                footer {
                    span { class: "kpi-atom",
                        button {
                            r#type: "button",
                            class: "kpi-abbr-btn",
                            title: "Mentah: PM setelah filter Median3, sebelum Kalman (µg/m³)",
                            onclick: move |e| open_abbr_help(state, "Mentah", e),
                            "Mentah:"
                        }
                        span { id: "kpi-pmraw", "{pm_raw_txt}" }
                    }
                    span { class: "kpi-atom",
                        button {
                            r#type: "button",
                            class: "kpi-abbr-btn",
                            title: "dC/dt: laju kenaikan konsentrasi asap (µg/m³ per detik)",
                            onclick: move |e| open_abbr_help(state, "dC/dt", e),
                            "dC/dt:"
                        }
                        span { id: "kpi-dcdt", "{dcdt_txt}" }
                    }
                    span { class: "kpi-atom",
                        button {
                            r#type: "button",
                            class: "kpi-abbr-btn",
                            title: "Vd: tegangan keluaran sensor debu Sharp (volt)",
                            onclick: move |e| open_abbr_help(state, "Vd", e),
                            "Vd:"
                        }
                        span { id: "kpi-vd", "{vd_txt}" }
                        "V ("
                        button {
                            r#type: "button",
                            class: "kpi-abbr-btn",
                            title: "V0: tegangan baseline udara bersih sensor debu (volt)",
                            onclick: move |e| open_abbr_help(state, "V0", e),
                            "V0:"
                        }
                        span { id: "kpi-v0", "{v0_txt}" }
                        ")"
                    }
                }
            }

            // 4. Gas MQ-135 (ppm)
            article { class: "kpi-card", id: "card-gas",
                header {
                    span { "Gas MQ-135 (ppm)" }
                    IcoFlask {}
                }
                div { class: "kpi-value tone-teal",
                    span { id: "kpi-ppm", "{ppm_txt}" }
                    span { class: "kpi-unit", "ppm" }
                }
                footer {
                    span { class: "kpi-atom",
                        button {
                            r#type: "button",
                            class: "kpi-abbr-btn",
                            title: "g: skor gas ternormalisasi 0,00–1,00",
                            onclick: move |e| open_abbr_help(state, "Skor g", e),
                            "Skor g:"
                        }
                        span { id: "kpi-g", "{g_txt}" }
                    }
                    span { class: "kpi-atom",
                        button {
                            r#type: "button",
                            class: "kpi-abbr-btn",
                            title: "Vm: tegangan ADC sensor gas MQ-135 (volt)",
                            onclick: move |e| open_abbr_help(state, "Vm", e),
                            "Vm:"
                        }
                        span { id: "kpi-vm", "{vm_txt}" }
                        "V"
                    }
                }
            }

            // 5. Prakiraan PM AI (t+60s)
            article {
                class: "kpi-card",
                id: "card-fc",
                "data-tone": fc_card_tone.unwrap_or(""),
                header {
                    span {
                        "Prakiraan PM AI "
                        span { class: "no-upper", "(t+60s)" }
                    }
                    IcoClock {}
                }
                div {
                    div { id: "kpi-fc-wrap", class: "kpi-value {fc_val_class}",
                        span { id: "kpi-fc", "{fc_text}" }
                        if fc_show_unit {
                            span { id: "kpi-fc-unit", class: "kpi-unit", "µg/m³" }
                        }
                    }
                    if let Some(froz) = fc_frozen {
                        div { id: "kpi-fc-frozen", class: "kpi-frozen-note", "{froz}" }
                    }
                }
                footer {
                    span { class: "kpi-atom",
                        button {
                            r#type: "button",
                            class: "kpi-abbr-btn",
                            title: "ETA: perkiraan waktu tersisa sampai FI mencapai ambang BAHAYA (150)",
                            onclick: move |e| open_abbr_help(state, "ETA", e),
                            "ETA:"
                        }
                        strong { id: "kpi-eta", "{eta_txt}" }
                    }
                    span { class: "kpi-atom",
                        button {
                            r#type: "button",
                            class: "kpi-abbr-btn",
                            title: "MAE: rata-rata galat absolut prakiraan AI terhadap PM aktual (µg/m³)",
                            onclick: move |e| open_abbr_help(state, "MAE", e),
                            "MAE:"
                        }
                        span { id: "kpi-mae", "{mae_txt}" }
                    }
                }
            }

            // 6. Aktuator Kipas PWM
            article {
                class: "kpi-card",
                id: "card-duty",
                "data-tone": duty_card_tone.unwrap_or(""),
                header {
                    span { "Aktuator Kipas PWM" }
                    IcoFan {}
                }
                div {
                    div { id: "kpi-duty-wrap", class: "kpi-value {duty_val_class}",
                        span { id: "kpi-duty", "{duty_txt}" }
                        span { class: "kpi-unit", "%" }
                    }
                    div { class: "kpi-meter", aria_hidden: "true",
                        div {
                            id: "pwm-meter-fill",
                            class: "kpi-meter-fill",
                            style: "inline-size: {duty_pct:.1}%; background-color: {duty_fill_col};",
                        }
                    }
                }
                footer {
                    span { class: "kpi-atom", "Kendali: " strong { id: "kpi-fan-ctrl", "{fan_ctrl}" } }
                    span { "Status: " strong { id: "kpi-fan-state", "{fan_state}" } }
                }
            }
        }
    }
}
