use crate::actions::{
    dispatch_sim, reload_snapshot, send_actuator_cmd, switch_fallback_to_sitl,
};
use crate::components::icons::IcoAlert;
use crate::state::{AppState, CalloutKind};
use crate::transport::BackendTarget;
use dioxus::prelude::*;
use karhutla_protocol::SimControlRequest;

#[component]
pub fn SafetyCallout() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let target = use_context::<Signal<BackendTarget>>();
    let maybe = state.read().compute_callout();

    let Some(callout) = maybe else {
        return rsx! {};
    };

    rsx! {
        div {
            id: "safety-callout",
            class: "safety-callout",
            role: "status",
            "data-tone": "{callout.tone}",
            div { class: "callout-msg",
                IcoAlert {}
                span { id: "safety-callout-text", "{callout.text}" }
            }
            div { id: "safety-callout-actions", class: "callout-actions",
                match callout.kind {
                    CalloutKind::BootError => rsx! {
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            onclick: move |_| reload_snapshot(state, target, false, false),
                            "Coba Muat Ulang"
                        }
                    },
                    CalloutKind::LiveLost => rsx! {
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            onclick: move |_| reload_snapshot(state, target, true, false),
                            "Muat Snapshot"
                        }
                    },
                    CalloutKind::HilsTimeout { can_resume } => rsx! {
                        if can_resume {
                            button {
                                r#type: "button",
                                class: "chip-btn",
                                onclick: move |_| {
                                    state.write().hils_timeout_info = None;
                                    dispatch_sim(
                                        state,
                                        target,
                                        SimControlRequest { action: Some("resume".into()), ..Default::default() },
                                        Some("Melanjutkan HILS...".to_string()),
                                    );
                                },
                                "Lanjutkan HILS"
                            }
                        }
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            onclick: move |_| switch_fallback_to_sitl(state, target),
                            "Beralih ke SITL"
                        }
                    },
                    CalloutKind::Failsafe { forced_off, fault_inject } => rsx! {
                        if forced_off {
                            button {
                                r#type: "button",
                                class: "chip-btn",
                                onclick: move |_| send_actuator_cmd(state, target, "AUTO".to_string()),
                                "Kembalikan ke AUTO"
                            }
                        }
                        if fault_inject {
                            button {
                                r#type: "button",
                                class: "chip-btn",
                                onclick: move |_| {
                                    dispatch_sim(
                                        state,
                                        target,
                                        SimControlRequest { fault_inject: Some(false), ..Default::default() },
                                        Some("Injeksi fault dimatikan.".to_string()),
                                    );
                                },
                                "Matikan Injeksi Fault"
                            }
                        }
                    },
                    CalloutKind::ForceOffHazard => rsx! {
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            onclick: move |_| send_actuator_cmd(state, target, "AUTO".to_string()),
                            "Kembalikan ke AUTO"
                        }
                    },
                    CalloutKind::Bahaya | CalloutKind::AiPreempt => rsx! {},
                }
            }
        }
    }
}
