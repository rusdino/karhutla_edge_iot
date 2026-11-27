use crate::actions::{
    direct_stop_sim, dispatch_sim, on_click_start_sim, persist_prefs, request_actuator_cmd,
    sync_and_download_esp_log, toggle_pause_resume, trigger_verified_export,
};
use crate::components::icons::{
    IcoBolt, IcoChevron, IcoGear, IcoPhone, IcoPlay, IcoReset, IcoStopSquare,
};
use crate::state::{
    format_int_id, format_wib_clock, scenario_label, speed_is_preset, AppState, ConfirmAction,
    ConfirmModalState, MAX_LOG_DISPLAY,
};
use crate::transport::BackendTarget;
use dioxus::prelude::*;
use karhutla_protocol::SimControlRequest;

#[component]
pub fn ControlRail() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let target = use_context::<Signal<BackendTarget>>();
    let s = state.read();

    let sim_running = s.sim.running;
    let sim_paused = s.sim.paused;
    let sim_tgt = s.sim_target.clone();
    let scen = s.selected_scenario.clone();
    let speed = s.selected_speed;
    let custom_speed = !speed_is_preset(speed);
    let ai_preempt = s.sim.ai_preempt;
    let fault_inject = s.sim.fault_inject;
    let sim_cfg_expanded = s.sim_config_expanded;
    let sim_busy = s.sim_busy;
    let cmd_busy = s.cmd_busy;
    let is_direct = s.is_direct_mqtt();

    let routing = s.override_routing();
    let cmd_target = s.cmd_target.clone();
    let active_cmd_mode = s.active_latest().map(|r| r.mode.clone());
    let has_live_row = s.active_latest().is_some();

    let sim_active_text = if sim_tgt == "hils" && !s.esp.online {
        format!("HILS • Menunggu ESP32 ({})", scenario_label(&s.sim.scenario))
    } else {
        format!(
            "{} • {} ({:.0}x{})",
            sim_tgt.to_ascii_uppercase(),
            scenario_label(&s.sim.scenario),
            s.sim.speed,
            if sim_paused { ", Jeda" } else { "" }
        )
    };

    // ESP32 Flash Backup & RTC status
    let esp_log = s.esp_log.clone();
    let esp_online = esp_log.online;
    let cap = esp_log.log_cap.filter(|&c| c > 0).unwrap_or(43_008);
    let count = esp_log.log_count.unwrap_or(0);
    let pct = ((count as f64 / cap as f64) * 100.0).round().clamp(0.0, 100.0) as u32;
    let log_full = esp_log.log_full.unwrap_or(false);
    let (rtc_tone, rtc_label) = if !esp_online {
        ("fault", "RTC: ESP32 Offline")
    } else if esp_log.rtc_ok.unwrap_or(false) {
        ("ok", "RTC: Tersinkron")
    } else {
        ("warn", "RTC: Belum Sinkron")
    };
    let rtc_clock_str = format_wib_clock(esp_log.epoch_s);
    let fill_tone = if log_full || pct >= 95 {
        Some("danger")
    } else if pct >= 75 {
        Some("warn")
    } else {
        None
    };
    let usage_txt = format!(
        "{} / {} sampel ({pct}%)",
        format_int_id(count as u64),
        format_int_id(cap as u64)
    );
    let busy_str = esp_log.busy.as_deref();
    let sync_busy = s.esp_sync_busy || busy_str == Some("sync");
    let clear_busy = s.esp_clear_busy || busy_str == Some("clear");
    let can_sync = !is_direct && esp_online && !sync_busy && !clear_busy;
    let can_clear = !is_direct
        && esp_log.can_clear
        && !sync_busy
        && !clear_busy
        && !(count > 0 && !(esp_log.synced && esp_log.downloaded));

    let log_note = if is_direct {
        "Status kapasitas dibaca live dari heartbeat MQTT. Sinkronisasi CSV massal memerlukan Host Lokal / Server LAN (:8080).".to_string()
    } else if sync_busy {
        let p = esp_log
            .progress
            .map(|x| (x * 100.0).round() as u32)
            .unwrap_or(0);
        format!("Menyinkronkan chunk... {p}%")
    } else if clear_busy {
        "Menghapus memori flash ESP32...".to_string()
    } else if let Some(ref err) = esp_log.last_error {
        format!("Galat: {err}")
    } else if esp_log.synced && esp_log.downloaded {
        format!(
            "Siap dikosongkan ({})",
            esp_log.csv_file.as_deref().unwrap_or("CSV terunduh")
        )
    } else if esp_log.synced {
        "Tersinkron — mengunduh CSV...".to_string()
    } else if let Some(ref ls) = esp_log.last_sync {
        format!("Sinkron terakhir: {ls}")
    } else if esp_online {
        "Siap disinkronkan via MQTT".to_string()
    } else {
        "Hubungkan ESP32 untuk menyinkronkan log flash".to_string()
    };

    // MQTT logs
    let log_file = s
        .sim
        .log_file
        .clone()
        .unwrap_or_else(|| "logs/mqtt.log".to_string());
    let mqtt_lines = s.mqtt_logs.clone();
    let total_logs = s.total_logs;
    let badge_log_count = if total_logs > MAX_LOG_DISPLAY {
        format!("{total_logs} ({} tampil)", mqtt_lines.len())
    } else {
        format!("{total_logs} baris")
    };
    let autoscroll = s.autoscroll_mqtt;
    let mqtt_placeholder = if !s.sim.enable_mqtt {
        format!(
            "MQTT broker tidak aktif pada sesi server ini — log trafik :{port} tidak direkam.",
            port = s.sim.broker_port
        )
    } else {
        format!("Belum ada pesan MQTT tercatat pada broker :{}.", s.sim.broker_port)
    };
    let mobile_hint = if s.mobile.ip_is_fallback {
        format!(
            "Hotspot/LAN offline terdeteksi: pastikan host pada file JSON sesuai IP komputer Anda (port {}).",
            s.sim.broker_port
        )
    } else {
        format!(
            "Buka IoT MQTT Panel di HP, pilih Backup/Restore, lalu impor file JSON (host {}, port {}).",
            s.mobile.lan_ip, s.mobile.broker_port
        )
    };
    drop(s);

    let is_pressed = |cmd: &str| {
        routing.can_send
            && has_live_row
            && active_cmd_mode.as_deref() == Some(cmd)
    };

    rsx! {
        aside { id: "control-panel", class: "control-column", tabindex: "-1", aria_label: "Panel Kontrol dan Operasi",
            // 1. SIM PANEL (on TOP of control column)
            section { class: "panel-card", id: "sim-panel", aria_label: "Operasi Simulasi SITL dan HILS",
                div { class: "panel-head-row",
                    div {
                        h2 { class: "panel-title", "Operasi Simulasi SITL / HILS Terpadu" }
                        p { class: "brand-sub", "Jalankan skenario ke SITL Rust atau HILS ESP32" }
                    }
                    if !sim_running {
                        span { id: "badge-sim-state", class: "pill-badge", "Tidak aktif" }
                    } else if sim_paused {
                        span { id: "badge-sim-state", class: "pill-badge", "data-tone": "warn", "Dijeda" }
                    }
                }

                div { role: "toolbar", aria_label: "Kontrol Runner Simulasi", class: "exec-top",
                    button {
                        r#type: "button",
                        class: if !sim_running && !is_direct { "cmd-btn btn-primary" } else { "cmd-btn" },
                        id: "btn-sim-start",
                        disabled: is_direct,
                        aria_busy: if sim_busy { "true" } else { "false" },
                        onclick: move |_| on_click_start_sim(state, target),
                        IcoPlay {}
                        span { id: "lbl-sim-start", if sim_running { "Ulang" } else { "Mulai" } }
                        if !sim_running && !is_direct {
                            kbd { id: "kbd-sim-start", class: "key-hint", "S" }
                        }
                    }
                    button {
                        r#type: "button",
                        class: if sim_running && sim_paused && !is_direct { "cmd-btn btn-primary" } else { "cmd-btn" },
                        id: "btn-sim-pause",
                        disabled: !sim_running || is_direct,
                        aria_busy: if sim_busy { "true" } else { "false" },
                        onclick: move |_| toggle_pause_resume(state, target),
                        span { id: "lbl-sim-pause", if sim_paused { "Lanjut" } else { "Jeda" } }
                        kbd { class: "key-hint", "P" }
                    }
                    // Direct non-modal stop (Run 12 P2 fix)
                    button {
                        r#type: "button",
                        class: "cmd-btn",
                        id: "btn-sim-stop",
                        disabled: !sim_running || is_direct,
                        aria_busy: if sim_busy { "true" } else { "false" },
                        onclick: move |_| direct_stop_sim(state, target),
                        "Berhenti"
                    }
                }

                if is_direct {
                    div { id: "direct-mqtt-sim-note", class: "conn-note-row",
                        span { "Mode Direct MQTT (:6669) terhubung langsung ke broker lapangan. Jalankan simulasi SITL/HILS melalui mode Host Lokal atau Server LAN (:8080)." }
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            onclick: move |_| state.write().open_conn_modal(),
                            "Ubah Koneksi"
                        }
                    }
                }

                if sim_running {
                    div { id: "sim-active-bar", class: "sim-active-bar",
                        span { id: "sim-active-text", "{sim_active_text}" }
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            id: "btn-toggle-sim-cfg",
                            aria_expanded: if sim_cfg_expanded { "true" } else { "false" },
                            onclick: move |_| {
                                let cur = state.read().sim_config_expanded;
                                state.write().sim_config_expanded = !cur;
                            },
                            "Ubah Target/Skenario"
                        }
                    }
                }

                if !sim_running || sim_cfg_expanded {
                    div { id: "sim-config-zone", class: "sim-zone",
                        span { class: "sim-zone-label", "Konfigurasi Simulasi" }
                        div { id: "sim-config-fields", class: "op-list",
                            div { class: "op-row",
                                span { "Target Eksekusi" }
                                div { role: "toolbar", aria_label: "Target Runner Simulasi", id: "target-toolbar",
                                    button {
                                        r#type: "button",
                                        class: "chip-btn tgt-btn",
                                        disabled: is_direct,
                                        aria_pressed: if sim_tgt == "sitl" { "true" } else { "false" },
                                        onclick: move |_| switch_sim_target(state, target, "sitl"),
                                        "SITL (Rust)"
                                    }
                                    button {
                                        r#type: "button",
                                        class: "chip-btn tgt-btn",
                                        disabled: is_direct,
                                        aria_pressed: if sim_tgt == "hils" { "true" } else { "false" },
                                        onclick: move |_| switch_sim_target(state, target, "hils"),
                                        "HILS (ESP32)"
                                    }
                                }
                            }

                            if sim_tgt == "hils" {
                                div { id: "hils-concurrency-hint", class: "hils-hint",
                                    "Pastikan ESP32 terhubung dan jangan menjalankan runner HILS ganda di terminal secara bersamaan."
                                }
                            }

                            div { class: "op-row",
                                label { r#for: "sel-scenario", "Skenario Ruang Asap" }
                                select {
                                    id: "sel-scenario",
                                    class: "op-select",
                                    disabled: is_direct,
                                    value: "{scen}",
                                    onchange: move |e| on_change_scenario(state, target, e.value()),
                                    option { value: "flaming", "Api Cepat (flaming)" }
                                    option { value: "smolder", "Membara Lambat (smolder)" }
                                    option { value: "clean", "Udara Bersih (clean)" }
                                    option { value: "fault", "Putus Sensor (fault)" }
                                }
                            }
                        }
                    }
                }

                div { class: "sim-quick-controls",
                    div { class: "op-row",
                        span { "Kecepatan" }
                        div { role: "toolbar", aria_label: "Kecepatan Simulasi", id: "speed-toolbar",
                            for spd_val in [1.0f64, 5.0, 10.0, 20.0] {
                                button {
                                    r#type: "button",
                                    class: "chip-btn spd-btn",
                                    disabled: is_direct,
                                    aria_pressed: if (speed - spd_val).abs() < 0.05 { "true" } else { "false" },
                                    onclick: move |_| {
                                        state.write().selected_speed = spd_val;
                                        dispatch_sim(
                                            state,
                                            target,
                                            SimControlRequest { speed: Some(spd_val), ..Default::default() },
                                            None,
                                        );
                                    },
                                    "{spd_val:.0}x"
                                }
                            }
                            if custom_speed {
                                span { class: "pill-badge spd-custom-badge", "data-tone": "accent", "{speed:.0}x" }
                            }
                        }
                    }

                    label { class: "op-row touch-label", r#for: "sw-preempt",
                        span { "AI Preempt (FORCE MAX Proaktif Dini)" }
                        input {
                            id: "sw-preempt",
                            r#type: "checkbox",
                            role: "switch",
                            disabled: is_direct,
                            checked: ai_preempt,
                            onchange: move |e| {
                                let v = e.checked();
                                state.write().sim.ai_preempt = v;
                                dispatch_sim(
                                    state,
                                    target,
                                    SimControlRequest { ai_preempt: Some(v), ..Default::default() },
                                    None,
                                );
                            },
                        }
                    }
                }

                details { id: "sim-advanced-drawer", class: "sim-adv-drawer",
                    summary {
                        span { "Injeksi Fault Sensor" }
                        span { class: "summary-trail",
                            span {
                                id: "sim-adv-summary",
                                class: "pill-badge",
                                "data-tone": if fault_inject { "fault" } else { "" },
                                if fault_inject { "Injeksi Fault" } else { "Standar" }
                            }
                            IcoChevron {}
                        }
                    }
                    div { class: "sim-adv-body",
                        label { class: "op-row touch-label", r#for: "sw-fault",
                            span { "Injeksi Putus Kabel Sensor Debu (FAILSAFE) — manual, kapan saja" }
                            input {
                                id: "sw-fault",
                                r#type: "checkbox",
                                role: "switch",
                                disabled: is_direct,
                                checked: fault_inject,
                                onchange: move |e| {
                                    let v = e.checked();
                                    state.write().sim.fault_inject = v;
                                    dispatch_sim(
                                        state,
                                        target,
                                        SimControlRequest { fault_inject: Some(v), ..Default::default() },
                                        None,
                                    );
                                },
                            }
                        }
                    }
                }
            }

            // 2. OVERRIDE PANEL
            section { class: "panel-card", id: "override-panel", aria_label: "Kontrol Override Aktuator",
                div {
                    h2 { class: "panel-title",
                        "Kontrol Override Aktuator "
                        code { class: "topic-tag", "karhutla/cmd" }
                    }
                    div { class: "override-target-wrap",
                        p {
                            id: "override-target-line",
                            class: "brand-sub override-target-line",
                            "data-tone": "{routing.tone}",
                            "{routing.desktop_text}"
                        }
                        if let (Some(fix_tgt), Some(fix_lbl)) = (routing.mismatch_fix_target, routing.mismatch_fix_label) {
                            button {
                                r#type: "button",
                                class: "chip-btn fix-scope-btn",
                                onclick: move |_| {
                                    state.write().cmd_target = fix_tgt.to_string();
                                    persist_prefs(&state);
                                },
                                "{fix_lbl}"
                            }
                        }
                    }
                }

                if routing.show_scope {
                    div { class: "op-row override-scope-row", id: "override-scope-row",
                        label { r#for: "sel-cmd-target", "Sasaran Perintah" }
                        select {
                            id: "sel-cmd-target",
                            class: "op-select",
                            aria_label: "Target Perintah Override",
                            value: "{cmd_target}",
                            onchange: move |e| {
                                state.write().cmd_target = e.value();
                                persist_prefs(&state);
                            },
                            option { value: "auto", "Semua jalur aktif" }
                            option { value: "sitl", "Hanya SITL (Rust)" }
                            option { value: "esp32", "Hanya ESP32 (:6669)" }
                        }
                    }
                }

                div { role: "toolbar", aria_label: "Perintah Override Kipas", class: "cmd-grid", id: "cmd-toolbar",
                    button {
                        r#type: "button",
                        class: "cmd-btn",
                        aria_pressed: if is_pressed("AUTO") { "true" } else { "false" },
                        aria_busy: if cmd_busy { "true" } else { "false" },
                        disabled: !routing.can_send,
                        title: "Kembalikan kendali kipas ke mode otomatis FSM (Alt+⇧A)",
                        onclick: move |_| request_actuator_cmd(state, target, "AUTO".to_string()),
                        IcoGear {}
                        "AUTO "
                        kbd { class: "key-hint", "⌥⇧A" }
                    }
                    button {
                        r#type: "button",
                        class: "cmd-btn",
                        aria_pressed: if is_pressed("FORCE_MAX") { "true" } else { "false" },
                        aria_busy: if cmd_busy { "true" } else { "false" },
                        disabled: !routing.can_send,
                        title: "Paksa kipas 100% secara manual (Alt+⇧M)",
                        onclick: move |_| request_actuator_cmd(state, target, "FORCE_MAX".to_string()),
                        IcoBolt {}
                        "MAX 100% "
                        kbd { class: "key-hint", "⌥⇧M" }
                    }
                    button {
                        r#type: "button",
                        class: "cmd-btn",
                        aria_pressed: if is_pressed("FORCE_OFF") { "true" } else { "false" },
                        aria_busy: if cmd_busy { "true" } else { "false" },
                        disabled: !routing.can_send,
                        title: "Paksa kipas mati 0% (tanpa pintasan demi keselamatan)",
                        onclick: move |_| request_actuator_cmd(state, target, "FORCE_OFF".to_string()),
                        IcoStopSquare {}
                        "OFF 0%"
                    }
                }

                div { class: "reset-zone",
                    span { class: "brand-sub", "Mengatur ulang Kalman, FSM & histori" }
                    button {
                        r#type: "button",
                        class: "cmd-btn",
                        id: "btn-cmd-reset",
                        "data-variant": "danger",
                        aria_busy: if cmd_busy { "true" } else { "false" },
                        disabled: !routing.can_send,
                        onclick: move |_| request_actuator_cmd(state, target, "RESET".to_string()),
                        IcoReset {}
                        "Reset Sistem"
                    }
                }
            }

            // 3. MEMORI BACKUP ESP32 & RTC (collapsible on projector so 1280x768 fits 0 scroll!)
            details {
                id: "esp-log-card",
                class: "panel-card drawer-card",
                open: log_full || sync_busy || clear_busy,
                summary {
                    span { "Memori Backup ESP32 & RTC" }
                    span { class: "summary-trail",
                        if log_full {
                            span { id: "esp-log-full-badge", class: "pill-badge", "data-tone": "danger", "MEMORI PENUH" }
                        } else {
                            span { class: "pill-badge", "data-tone": "{rtc_tone}", "{pct}% ({count})" }
                        }
                        IcoChevron {}
                    }
                }
                div { class: "drawer-body",
                    div { class: "esp-rtc-row",
                        span { id: "esp-rtc-badge", class: "pill-badge", "data-tone": "{rtc_tone}", "{rtc_label}" }
                        span { id: "esp-rtc-clock", class: "esp-rtc-clock", "{rtc_clock_str}" }
                    }
                    div { class: "esp-log-meter",
                        div { class: "esp-log-track",
                            div {
                                id: "esp-log-bar",
                                class: "esp-log-fill",
                                role: "progressbar",
                                aria_label: "Penggunaan Memori Backup ESP32",
                                aria_valuemin: "0",
                                aria_valuemax: "100",
                                aria_valuenow: "{pct}",
                                "data-tone": fill_tone.unwrap_or(""),
                                style: "inline-size: {pct}%;",
                            }
                        }
                        div { class: "esp-log-meta",
                            span { id: "esp-log-usage", "{usage_txt}" }
                            span { id: "esp-log-status-note", class: "brand-sub", "{log_note}" }
                        }
                    }
                    div { class: "esp-log-actions",
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            id: "btn-esp-log-sync",
                            disabled: !can_sync,
                            aria_busy: if sync_busy { "true" } else { "false" },
                            onclick: move |_| sync_and_download_esp_log(state, target),
                            "Sinkronkan & Unduh Log ESP32"
                        }
                        button {
                            r#type: "button",
                            class: "cmd-btn",
                            id: "btn-esp-log-clear",
                            "data-variant": "danger",
                            disabled: !can_clear,
                            aria_busy: if clear_busy { "true" } else { "false" },
                            onclick: move |_| {
                                state.write().confirm_modal = Some(ConfirmModalState {
                                    title: "Kosongkan Memori ESP32?".to_string(),
                                    desc: "Semua rekaman backup di flash ESP32 akan dihapus permanen setelah dipastikan sudah tersinkron dan terunduh. Lanjutkan?".to_string(),
                                    ok_label: "Ya, Kosongkan".to_string(),
                                    tone: "danger".to_string(),
                                    action: ConfirmAction::ClearEspLog,
                                });
                            },
                            "Kosongkan Memori ESP32"
                        }
                    }
                }
            }

            // 4. ALIRAN LOG MQTT :6669
            details { id: "drawer-mqtt", class: "panel-card drawer-card",
                summary {
                    span {
                        "Aliran Log MQTT "
                        code { class: "topic-tag", ":6669" }
                    }
                    span { class: "summary-trail",
                        span { id: "badge-log-count", class: "pill-badge", "{badge_log_count}" }
                        IcoChevron {}
                    }
                }
                div { class: "drawer-body",
                    div { class: "panel-head-row",
                        span { class: "brand-sub",
                            "File log: "
                            code { id: "txt-logfile", class: "topic-tag", "{log_file}" }
                        }
                        div { class: "summary-trail",
                            button {
                                r#type: "button",
                                class: "chip-btn",
                                id: "btn-export-log",
                                title: "Unduh file log trafik MQTT (.log)",
                                onclick: move |_| {
                                    trigger_verified_export(
                                        state,
                                        target,
                                        "/api/export-log".to_string(),
                                        "karhutla_mqtt.log",
                                        "text/plain; charset=utf-8",
                                        "Log trafik MQTT (.log) diunduh.".to_string(),
                                    );
                                },
                                "Unduh Log"
                            }
                            label { class: "touch-label", r#for: "sw-autoscroll",
                                span { "Gulir otomatis" }
                                input {
                                    id: "sw-autoscroll",
                                    r#type: "checkbox",
                                    role: "switch",
                                    checked: autoscroll,
                                    onchange: move |e| {
                                        state.write().autoscroll_mqtt = e.checked();
                                        persist_prefs(&state);
                                    },
                                }
                            }
                        }
                    }
                    div { id: "mqtt-log-box", class: "mqtt-stream", tabindex: "0", aria_label: "Aliran Log MQTT",
                        if mqtt_lines.is_empty() {
                            div { class: "log-line log-empty-note", "{mqtt_placeholder}" }
                        } else {
                            for line in mqtt_lines {
                                div { class: "log-line", "{line}" }
                            }
                        }
                    }
                }
            }

            // 5. KONEKSI APLIKASI HP (IoT MQTT Panel JSON)
            details { id: "drawer-mobile", class: "panel-card drawer-card",
                summary {
                    span { "Koneksi Aplikasi HP (IoT MQTT Panel)" }
                    IcoChevron {}
                }
                div { class: "drawer-body",
                    button {
                        r#type: "button",
                        class: "chip-btn btn-block",
                        id: "btn-export-mobile",
                        onclick: move |_| {
                            trigger_verified_export(
                                state,
                                target,
                                "/api/mobile-config".to_string(),
                                "karhutla_iot_mqtt_panel.json",
                                "application/json; charset=utf-8",
                                "File konfigurasi HP (.json) diunduh.".to_string(),
                            );
                        },
                        IcoPhone {}
                        "Unduh Konfigurasi IoT MQTT Panel (.json)"
                    }
                    p { id: "mobile-config-hint", class: "brand-sub", "{mobile_hint}" }
                }
            }
        }
    }
}

pub fn switch_sim_target(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    new_tgt: &str,
) {
    let cur = state.read().sim_target.clone();
    if new_tgt == cur {
        return;
    }
    let running = state.read().sim.running;
    if running {
        state.write().confirm_modal = Some(ConfirmModalState {
            title: format!("Beralih ke Target {}?", new_tgt.to_ascii_uppercase()),
            desc: format!(
                "Simulasi {} sedang berjalan. Beralih ke {} akan memulai ulang simulasi dari t=0s.",
                cur.to_ascii_uppercase(),
                new_tgt.to_ascii_uppercase()
            ),
            ok_label: format!("Mulai Ulang di {}", new_tgt.to_ascii_uppercase()),
            tone: "warn".to_string(),
            action: ConfirmAction::SwitchTarget {
                target: new_tgt.to_string(),
            },
        });
        return;
    }
    {
        let mut s = state.write();
        s.sim_target = new_tgt.to_string();
    }
    dispatch_sim(
        state,
        target,
        SimControlRequest {
            target: Some(new_tgt.to_string()),
            ..Default::default()
        },
        None,
    );
}

pub fn on_change_scenario(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    new_scen: String,
) {
    let (running, cur_scen, cur_tgt) = {
        let s = state.read();
        (s.sim.running, s.sim.scenario.clone(), s.sim_target.clone())
    };
    if running {
        state.write().confirm_modal = Some(ConfirmModalState {
            title: "Ganti Skenario & Mulai Ulang?".to_string(),
            desc: format!(
                "Simulasi sedang menjalankan \"{}\". Mengganti skenario ke \"{}\" perlu memulai ulang ruang asap dari t=0s.",
                scenario_label(&cur_scen),
                scenario_label(&new_scen)
            ),
            ok_label: "Mulai Ulang dengan Skenario Ini".to_string(),
            tone: "warn".to_string(),
            action: ConfirmAction::SwitchScenario {
                target: cur_tgt,
                scenario: new_scen,
            },
        });
        return;
    }
    state.write().selected_scenario = new_scen.clone();
    dispatch_sim(
        state,
        target,
        SimControlRequest {
            scenario: Some(new_scen),
            ..Default::default()
        },
        None,
    );
}
