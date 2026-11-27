use crate::actions::{
    direct_stop_sim, dispatch_sim, on_click_start_sim, persist_prefs, request_actuator_cmd,
    toggle_pause_resume,
};
use crate::components::control_rail::{on_change_scenario, switch_sim_target};
use crate::components::icons::IcoChevron;
use crate::state::{speed_is_preset, AppState};
use crate::transport::BackendTarget;
use dioxus::prelude::*;
use karhutla_protocol::SimControlRequest;

#[component]
pub fn MobileDock() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let target = use_context::<Signal<BackendTarget>>();
    let s = state.read();

    let (context_text, context_tone) = s.mob_dock_context();
    let routing = s.override_routing();
    let tray_open = s.mob_tray_open;
    let sim_running = s.sim.running;
    let sim_paused = s.sim.paused;
    let sim_tgt = s.sim_target.clone();
    let scen = s.selected_scenario.clone();
    let speed = s.selected_speed;
    let custom_speed = !speed_is_preset(speed);
    let ai_preempt = s.sim.ai_preempt;
    let fault_inject = s.sim.fault_inject;
    let cmd_target = s.cmd_target.clone();
    let active_mode = s.active_latest().map(|r| r.mode.clone());
    let has_live = s.active_latest().is_some();
    let is_direct = s.is_direct_mqtt();
    drop(s);

    let play_label = if !sim_running {
        "Mulai"
    } else if sim_paused {
        "Lanjut"
    } else {
        "Jeda"
    };

    let is_pressed = |cmd: &str| {
        routing.can_send && has_live && active_mode.as_deref() == Some(cmd)
    };

    rsx! {
        nav { class: "mobile-dock", aria_label: "Kendali Cepat Mobile",
            div { class: "mob-dock-meta",
                div {
                    id: "mob-dock-context",
                    class: "mob-dock-context",
                    "data-tone": context_tone.unwrap_or(""),
                    "{context_text}"
                }
                div {
                    id: "mob-dock-cmd-status",
                    class: "mob-dock-cmd-status",
                    "data-tone": "{routing.tone}",
                    "{routing.mobile_text}"
                }
            }

            if tray_open {
                div { id: "mob-dock-tray", class: "mob-dock-tray",
                    div { class: "mob-tray-group",
                        span { class: "sim-zone-label", "Simulasi" }
                        div { class: "mob-quick-sim",
                            select {
                                id: "mob-sel-sim-target",
                                class: "op-select",
                                aria_label: "Target Eksekusi Mobile",
                                disabled: is_direct,
                                value: "{sim_tgt}",
                                onchange: move |e| switch_sim_target(state, target, &e.value()),
                                option { value: "sitl", "Target: SITL" }
                                option { value: "hils", "Target: HILS (ESP32)" }
                            }
                            select {
                                id: "mob-sel-scenario",
                                class: "op-select",
                                aria_label: "Skenario Simulasi Mobile",
                                disabled: is_direct,
                                value: "{scen}",
                                onchange: move |e| on_change_scenario(state, target, e.value()),
                                option { value: "flaming", "Api Cepat (flaming)" }
                                option { value: "smolder", "Membara (smolder)" }
                                option { value: "clean", "Udara Bersih (clean)" }
                                option { value: "fault", "Putus Sensor (fault)" }
                            }
                        }

                        div { class: "mob-speed-row",
                            span { "Kecepatan" }
                            div { role: "toolbar", aria_label: "Kecepatan Simulasi Mobile",
                                for spd_val in [1.0f64, 5.0, 10.0, 20.0] {
                                    button {
                                        r#type: "button",
                                        class: "chip-btn spd-btn mob-spd-btn",
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
                                    span { class: "pill-badge", "data-tone": "accent", "{speed:.0}x" }
                                }
                            }
                        }

                        label { class: "mob-fault-row touch-label", r#for: "mob-sw-preempt",
                            span { "AI Preempt (FORCE MAX Dini)" }
                            input {
                                id: "mob-sw-preempt",
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

                    // Run 12 P1: Fault injection clearly separated visually from AI Preempt
                    div { class: "mob-tray-group mob-fault-card",
                        label { class: "mob-fault-row touch-label", r#for: "mob-sw-fault",
                            span { "Injeksi Putus Sensor (FAILSAFE)" }
                            input {
                                id: "mob-sw-fault",
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

                    div { class: "mob-tray-group",
                        span { class: "sim-zone-label", "Override & Aksi" }
                        if routing.show_scope {
                            div { class: "mob-cmd-scope", id: "mob-cmd-scope-row",
                                label { r#for: "mob-sel-cmd-target", "Sasaran Perintah" }
                                select {
                                    id: "mob-sel-cmd-target",
                                    class: "op-select",
                                    aria_label: "Sasaran Perintah Mobile",
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
                        div { class: "mob-tray-actions",
                            button {
                                r#type: "button",
                                class: "cmd-btn mob-cmd-btn",
                                "data-variant": "danger",
                                aria_pressed: if is_pressed("FORCE_OFF") { "true" } else { "false" },
                                disabled: !routing.can_send,
                                onclick: move |_| {
                                    state.write().mob_tray_open = false;
                                    request_actuator_cmd(state, target, "FORCE_OFF".to_string());
                                },
                                "OFF 0%"
                            }
                            button {
                                r#type: "button",
                                class: "cmd-btn",
                                id: "mob-btn-stop",
                                disabled: !sim_running || is_direct,
                                onclick: move |_| {
                                    state.write().mob_tray_open = false;
                                    direct_stop_sim(state, target);
                                },
                                "Berhenti"
                            }
                            button {
                                r#type: "button",
                                class: "cmd-btn",
                                id: "btn-mob-reset",
                                "data-variant": "danger",
                                disabled: !routing.can_send,
                                onclick: move |_| {
                                    state.write().mob_tray_open = false;
                                    request_actuator_cmd(state, target, "RESET".to_string());
                                },
                                "Reset Sistem"
                            }
                        }
                    }
                }
            }

            div { class: "mob-dock-row",
                button {
                    r#type: "button",
                    class: "cmd-btn mob-cmd-btn",
                    aria_pressed: if is_pressed("AUTO") { "true" } else { "false" },
                    disabled: !routing.can_send,
                    onclick: move |_| request_actuator_cmd(state, target, "AUTO".to_string()),
                    "AUTO"
                }
                button {
                    r#type: "button",
                    class: "cmd-btn mob-cmd-btn",
                    aria_pressed: if is_pressed("FORCE_MAX") { "true" } else { "false" },
                    disabled: !routing.can_send,
                    onclick: move |_| request_actuator_cmd(state, target, "FORCE_MAX".to_string()),
                    "MAX 100%"
                }
                button {
                    r#type: "button",
                    class: if (!sim_running || sim_paused) && !is_direct { "cmd-btn btn-primary" } else { "cmd-btn" },
                    id: "mob-btn-playpause",
                    disabled: is_direct,
                    onclick: move |_| {
                        if state.read().sim.running {
                            toggle_pause_resume(state, target);
                        } else {
                            on_click_start_sim(state, target);
                        }
                    },
                    "{play_label}"
                }
                button {
                    r#type: "button",
                    class: "cmd-btn",
                    id: "mob-btn-more",
                    aria_expanded: if tray_open { "true" } else { "false" },
                    aria_controls: "mob-dock-tray",
                    onclick: move |_| {
                        let cur = state.read().mob_tray_open;
                        state.write().mob_tray_open = !cur;
                    },
                    span { "Lainnya" }
                    IcoChevron {}
                }
            }
        }
    }
}
