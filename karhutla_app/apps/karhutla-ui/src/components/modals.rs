use crate::actions::{
    execute_confirmed_action, persist_prefs, send_actuator_cmd, switch_backend_target,
};
use crate::components::icons::IcoChevron;
use crate::state::AppState;
use crate::transport::BackendTarget;
use dioxus::prelude::*;

#[component]
pub fn ModalsAndOverlays() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let target = use_context::<Signal<BackendTarget>>();
    let live_task = use_context::<Signal<Option<dioxus::dioxus_core::Task>>>();
    let s = state.read();

    let confirm_modal = s.confirm_modal.clone();
    let help_open = s.help_open;
    let conn_open = s.conn_modal_open;
    let abbr_pop = s.abbr_popover.clone();
    let toasts = s.toasts.clone();
    let single_key = s.single_key_shortcuts;
    let is_embedded = s.is_embedded_host;
    let embedded_lan = s.embedded_lan_url.clone();
    let conn_draft_mode = s.conn_draft_mode.clone();
    let remote_input = s.remote_url_input.clone();
    let using_remote = s.using_remote_url.clone();
    let mqtt_host_input = s.mqtt_host_input.clone();
    let mqtt_port_input = s.mqtt_port_input.clone();
    let direct_endpoint = s.direct_mqtt_endpoint.clone();
    let safety_announce = s.safety_announce.clone();
    drop(s);

    let last_toast_msg = toasts.last().map(|t| t.message.clone()).unwrap_or_default();

    rsx! {
        div { id: "safety-live", class: "sr-only", role: "alert", aria_live: "assertive", aria_atomic: "true", "{safety_announce}" }
        div { id: "toast-live", class: "sr-only", role: "status", aria_live: "polite", aria_atomic: "true", "{last_toast_msg}" }

        // Toast region
        div { id: "toast-region", class: "toast-region",
            for toast in toasts {
                {
                    let tid = toast.id;
                    let act_lbl = toast.action_label.clone();
                    let act_cmd = toast.action_cmd.clone();
                    rsx! {
                        div { key: "{tid}", class: "toast", "data-tone": "{toast.tone}",
                            span { "{toast.message}" }
                            if let (Some(lbl), Some(cmd)) = (act_lbl, act_cmd) {
                                button {
                                    r#type: "button",
                                    class: "chip-btn",
                                    onclick: move |_| {
                                        state.write().remove_toast(tid);
                                        send_actuator_cmd(state, target, cmd.clone());
                                    },
                                    "{lbl}"
                                }
                            }
                        }
                    }
                }
            }
        }

        // Anchored KPI Abbreviation Explanation Popover
        if let Some(pop) = abbr_pop {
            div {
                id: "abbr-popover",
                class: "abbr-popover",
                role: "note",
                style: "inset-inline-start: {pop.x:.0}px; inset-block-start: {pop.y:.0}px;",
                div { class: "abbr-popover-head",
                    strong { id: "abbr-popover-title", "{pop.title}" }
                    button {
                        r#type: "button",
                        id: "abbr-popover-close",
                        class: "abbr-popover-close",
                        aria_label: "Tutup penjelasan istilah",
                        onclick: move |_| state.write().abbr_popover = None,
                        "✕"
                    }
                }
                p { id: "abbr-popover-body", "{pop.body}" }
            }
        }

        // Confirmation Dialog
        if let Some(modal) = confirm_modal {
            {
                let action_clone = modal.action.clone();
                rsx! {
                    div {
                        class: "modal-backdrop",
                        role: "presentation",
                        onclick: move |_| state.write().confirm_modal = None,
                        div {
                            id: "confirm-dialog",
                            class: "confirm-modal",
                            role: "dialog",
                            aria_modal: "true",
                            aria_labelledby: "confirm-title",
                            aria_describedby: "confirm-desc",
                            onclick: move |e| e.stop_propagation(),
                            h2 { id: "confirm-title", class: "panel-title", "{modal.title}" }
                            p { id: "confirm-desc", class: "brand-sub confirm-desc", "{modal.desc}" }
                            div { class: "modal-actions",
                                button {
                                    r#type: "button",
                                    class: "chip-btn",
                                    id: "confirm-cancel",
                                    onclick: move |_| state.write().confirm_modal = None,
                                    "Batal"
                                }
                                button {
                                    r#type: "button",
                                    class: "cmd-btn",
                                    id: "confirm-ok",
                                    "data-variant": if modal.tone == "danger" { "danger" } else { "" },
                                    onclick: move |_| execute_confirmed_action(state, target, action_clone.clone()),
                                    "{modal.ok_label}"
                                }
                            }
                        }
                    }
                }
            }
        }

        // Glossary & Keyboard Shortcuts Modal (3-accordion layout)
        if help_open {
            div {
                class: "modal-backdrop",
                role: "presentation",
                onclick: move |_| state.write().help_open = false,
                div {
                    id: "help-dialog",
                    class: "confirm-modal help-modal",
                    role: "dialog",
                    aria_modal: "true",
                    aria_labelledby: "help-title",
                    onclick: move |e| e.stop_propagation(),
                    div { class: "help-head",
                        h2 { id: "help-title", class: "panel-title", "Glosarium, Pemecahan Masalah & Pintasan" }
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            id: "help-close",
                            onclick: move |_| state.write().help_open = false,
                            "Tutup"
                        }
                    }

                    div { class: "glossary-list",
                        details { class: "help-sec", "name": "help-sec", open: true,
                            summary {
                                span { "Metrik DSP & Prakiraan AI" }
                                IcoChevron {}
                            }
                            div { class: "help-sec-body",
                                div {
                                    strong { "FI (Fusion Index): " }
                                    "Indeks gabungan dalam µg/m³: "
                                    code { class: "topic-tag", "FI = PM_Kalman + W_gas × GAS_FULL_PM × g + max(0, dC/dt) × t_lead" }
                                    " (W_gas = 0,30; GAS_FULL_PM = 100; t_lead = 15 s). Memicu FSM WASPADA ≥ 55 dan BAHAYA ≥ 150 dengan histeresis pemulihan."
                                }
                                div {
                                    strong { "PM-proxy Kalman, Mentah & dC/dt: " }
                                    "PM tersaring Median3 + Kalman 2-state; "
                                    em { "Mentah" }
                                    " adalah nilai setelah Median3 sebelum Kalman; dC/dt adalah laju kenaikan asap (µg/m³/s); "
                                    code { class: "topic-tag", "Vd" }
                                    " tegangan sensor debu Sharp; "
                                    code { class: "topic-tag", "V0" }
                                    " baseline udara bersih."
                                }
                                div {
                                    strong { "Gas MQ-135 (ppm, Skor g, Vm): " }
                                    "Estimasi gas pembakaran, skor ternormalisasi 0,00–1,00, dan tegangan ADC MQ-135."
                                }
                                div {
                                    strong { "Prakiraan AI (t+60s, ETA, MAE): " }
                                    "Regresi linear 60 detik ke depan; ETA = perkiraan waktu menuju BAHAYA; MAE = galat absolut rata-rata prakiraan. Memicu "
                                    strong { "AI Preempt" }
                                    " (FORCE MAX 100% dini)."
                                }
                            }
                        }

                        details { class: "help-sec", "name": "help-sec",
                            summary {
                                span { "Mode Simulasi, Kendali Override & Alur Demo Sidang" }
                                IcoChevron {}
                            }
                            div { class: "help-sec-body",
                                div {
                                    strong { "State FSM & FAILSAFE: " }
                                    code { class: "topic-tag", "NORMAL" } ", "
                                    code { class: "topic-tag", "WASPADA" } ", "
                                    code { class: "topic-tag", "BAHAYA" } ", dan "
                                    code { class: "topic-tag", "FAILSAFE" }
                                    " (kipas pengaman 70% bila kabel debu terputus, kecuali override manual)."
                                }
                                div {
                                    strong { "Alur Cepat Demo Sidang: " }
                                    "Tekan " code { class: "topic-tag", "S" } " atau Mulai → naikkan kecepatan "
                                    code { class: "topic-tag", "10x/20x" }
                                    " → uji " strong { "AI Preempt" } " atau " strong { "Injeksi Fault" }
                                    " → tekan " strong { "Berhenti" } " (langsung menampilkan Ringkasan Simulasi)."
                                }
                                div {
                                    strong { "SITL vs HILS & Paritas (ΔFI): " }
                                    "SITL dan firmware referensi HILS sama-sama memakai crate "
                                    code { class: "topic-tag", "karhutla-core" }
                                    ". Paritas dinyatakan " strong { "OK" } " bila maks |ΔFI| < 1,0 dan 100% state cocok."
                                }
                                div {
                                    strong { "Sasaran Perintah Override: " }
                                    "Jika muncul peringatan mismatch antara grafik & target override, klik tombol koreksi cepat tepat di sebelah peringatan tersebut."
                                }
                            }
                        }

                        details { class: "help-sec", "name": "help-sec",
                            summary {
                                span { "Koneksi MQTT :6669 & Pemecahan Masalah" }
                                IcoChevron {}
                            }
                            div { class: "help-sec-body",
                                div {
                                    strong { "Skala Sumbu Kanan: " }
                                    code { class: "topic-tag", "Gas ×0.2" } ", "
                                    code { class: "topic-tag", "g ×100" } ", "
                                    code { class: "topic-tag", "Vd/Vm ×50" }
                                    " (sumbu kanan tampil otomatis berwarna per satuan saat kurva diaktifkan)."
                                }
                                div {
                                    strong { "Broker :6669 Terputus: " }
                                    "Pastikan layanan Mosquitto berjalan ("
                                    code { class: "topic-tag", "mosquitto -c mosquitto.conf -d" }
                                    ") dan port "
                                    code { class: "topic-tag", "6669" }
                                    " tidak terblokir firewall. Aplikasi Desktop otomatis menyalakan Mosquitto lokal bila tersedia."
                                }
                            }
                        }
                    }

                    h3 { class: "sim-zone-label help-subhead", "Pintasan Keyboard" }
                    p { class: "brand-sub",
                        "Demi keselamatan, perintah "
                        code { class: "topic-tag", "FORCE OFF (0%)" }
                        " sengaja tidak diberi pintasan keyboard. Tekan "
                        kbd { class: "key-hint", "Esc" }
                        " untuk menutup dialog ini."
                    }

                    label { class: "touch-label shortcut-toggle-row", r#for: "sw-shortcuts",
                        input {
                            id: "sw-shortcuts",
                            r#type: "checkbox",
                            role: "switch",
                            checked: single_key,
                            onchange: move |e| {
                                state.write().single_key_shortcuts = e.checked();
                                persist_prefs(&state);
                            },
                        }
                        span { "Aktifkan pintasan satu tombol (S, P, 1–3, Space)" }
                    }

                    div { class: "shortcut-grid",
                        div { kbd { class: "key-hint", "S" } " Mulai simulasi (saat tidak aktif)" }
                        div { kbd { class: "key-hint", "P" } " / " kbd { class: "key-hint", "Space" } " Jeda atau lanjutkan" }
                        div { kbd { class: "key-hint", "1" } " " kbd { class: "key-hint", "2" } " " kbd { class: "key-hint", "3" } " Pilih preset kurva" }
                        div { kbd { class: "key-hint", "⌥⇧A" } " Mode AUTO • " kbd { class: "key-hint", "⌥⇧M" } " MAX 100%" }
                        div { kbd { class: "key-hint", "←" } " / " kbd { class: "key-hint", "→" } " Geser inspeksi pada grafik" }
                        div { kbd { class: "key-hint", "Esc" } " Lepas crosshair • " kbd { class: "key-hint", "?" } " Bantuan" }
                    }
                }
            }
        }

        // Connection Mode & Server Settings Modal
        if conn_open {
            {
                let is_web_build = cfg!(feature = "web");
                let apply_disabled = is_web_build && conn_draft_mode == "direct";
                rsx! {
                    div {
                        class: "modal-backdrop",
                        role: "presentation",
                        onclick: move |_| state.write().conn_modal_open = false,
                        div {
                            id: "conn-dialog",
                            class: "confirm-modal",
                            role: "dialog",
                            aria_modal: "true",
                            aria_labelledby: "conn-title",
                            onclick: move |e| e.stop_propagation(),
                            h2 { id: "conn-title", class: "panel-title", "Mode Koneksi & Broker MQTT" }
                            div {
                                class: "conn-mode-bar",
                                role: "group",
                                aria_label: "Pilih Mode Koneksi Backend",
                                if is_embedded {
                                    button {
                                        r#type: "button",
                                        id: "btn-mode-embedded",
                                        class: "chip-btn conn-mode-btn",
                                        aria_pressed: if conn_draft_mode == "embedded" { "true" } else { "false" },
                                        onclick: move |_| state.write().conn_draft_mode = "embedded".to_string(),
                                        "Host Lokal"
                                    }
                                }
                                button {
                                    r#type: "button",
                                    id: "btn-mode-remote",
                                    class: "chip-btn conn-mode-btn",
                                    aria_pressed: if conn_draft_mode == "remote" { "true" } else { "false" },
                                    onclick: move |_| state.write().conn_draft_mode = "remote".to_string(),
                                    "Server LAN (:8080)"
                                }
                                button {
                                    r#type: "button",
                                    id: "btn-mode-direct",
                                    class: "chip-btn conn-mode-btn",
                                    aria_pressed: if conn_draft_mode == "direct" { "true" } else { "false" },
                                    onclick: move |_| state.write().conn_draft_mode = "direct".to_string(),
                                    "Direct MQTT (:6669)"
                                }
                            }

                            if conn_draft_mode == "embedded" && is_embedded {
                                p { class: "brand-sub confirm-desc",
                                    "Aplikasi Desktop aktif sebagai Embedded Host. Browser & HP di Wi-Fi/hotspot yang sama dapat membuka alamat LAN berikut:"
                                }
                                div { class: "conn-field-row",
                                    code { class: "topic-tag", "{embedded_lan}" }
                                }
                            } else if conn_draft_mode == "remote" {
                                p { class: "brand-sub confirm-desc",
                                    "Hubungkan ke server Rust LAN (:8080) untuk kontrol penuh simulasi SITL/HILS, sinkronisasi flash ESP32, & aliran WebSocket. Kosongkan untuk memakai origin server saat ini."
                                }
                                div { class: "conn-field-row",
                                    label { r#for: "input-remote-url", class: "sim-zone-label", "URL Server LAN (Opsional)" }
                                    input {
                                        id: "input-remote-url",
                                        class: "conn-input",
                                        r#type: "url",
                                        placeholder: "http://192.168.x.x:8080",
                                        value: "{remote_input}",
                                        oninput: move |e| state.write().remote_url_input = e.value(),
                                    }
                                }
                                if let Some(ref cur_remote) = using_remote {
                                    p { class: "conn-note", "Saat ini tersambung ke Server LAN: {cur_remote}" }
                                }
                            } else {
                                p { class: "brand-sub confirm-desc",
                                    "Hubungkan langsung ke broker Mosquitto (:6669) tanpa perantara host HTTP laptop. Ideal untuk pemantauan lapangan & override cepat."
                                }
                                div { class: "conn-grid-2",
                                    div { class: "conn-field-row",
                                        label { r#for: "input-mqtt-host", class: "sim-zone-label", "Host / IP Broker" }
                                        input {
                                            id: "input-mqtt-host",
                                            class: "conn-input",
                                            r#type: "text",
                                            placeholder: "192.168.4.1",
                                            value: "{mqtt_host_input}",
                                            oninput: move |e| state.write().mqtt_host_input = e.value(),
                                        }
                                    }
                                    div { class: "conn-field-row",
                                        label { r#for: "input-mqtt-port", class: "sim-zone-label", "Port TCP" }
                                        input {
                                            id: "input-mqtt-port",
                                            class: "conn-input",
                                            r#type: "number",
                                            min: "1",
                                            max: "65535",
                                            placeholder: "6669",
                                            value: "{mqtt_port_input}",
                                            oninput: move |e| state.write().mqtt_port_input = e.value(),
                                        }
                                    }
                                }
                                if let Some(ref ep) = direct_endpoint {
                                    p { class: "conn-note", "Endpoint Direct MQTT aktif: tcp://{ep}" }
                                }
                                if is_web_build {
                                    p { class: "conn-note",
                                        "Catatan browser WebAssembly: browser tidak dapat membuka soket TCP :6669 secara langsung. Gunakan mode Server LAN (:8080) di browser, atau gunakan aplikasi Desktop / Android APK untuk Direct MQTT lapangan."
                                    }
                                } else {
                                    p { class: "conn-note",
                                        "Direct MQTT menyubscribe topik karhutla/# dan mengirim perintah karhutla/cmd langsung ke node ESP32. Runner simulasi SITL/HILS & sync flash tetap memerlukan mode Host Lokal / Server LAN."
                                    }
                                }
                            }

                            div { class: "modal-actions",
                                button {
                                    r#type: "button",
                                    class: "chip-btn",
                                    onclick: move |_| state.write().conn_modal_open = false,
                                    "Tutup"
                                }
                                button {
                                    r#type: "button",
                                    id: "btn-conn-apply",
                                    class: "cmd-btn btn-primary",
                                    disabled: apply_disabled,
                                    onclick: move |_| {
                                        let (draft, remote_raw, host_raw, port_raw) = {
                                            let s = state.read();
                                            (
                                                s.conn_draft_mode.clone(),
                                                s.remote_url_input.trim().to_string(),
                                                s.mqtt_host_input.trim().to_string(),
                                                s.mqtt_port_input.trim().to_string(),
                                            )
                                        };
                                        match draft.as_str() {
                                            "embedded" => {
                                                switch_backend_target(state, target, live_task, BackendTarget::Embedded);
                                            }
                                            "direct" => {
                                                if cfg!(feature = "web") {
                                                    return;
                                                }
                                                if host_raw.is_empty() {
                                                    state.write().push_toast(
                                                        "Isi alamat IP/host broker MQTT terlebih dahulu.",
                                                        "danger",
                                                        None,
                                                        None,
                                                    );
                                                    return;
                                                }
                                                let Some(port) = port_raw.parse::<u16>().ok().filter(|&p| p > 0) else {
                                                    state.write().push_toast(
                                                        "Port TCP broker harus angka 1–65535.",
                                                        "danger",
                                                        None,
                                                        None,
                                                    );
                                                    return;
                                                };
                                                switch_backend_target(
                                                    state,
                                                    target,
                                                    live_task,
                                                    BackendTarget::DirectMqtt {
                                                        host: host_raw,
                                                        port,
                                                    },
                                                );
                                            }
                                            _ => {
                                                let normalized = if remote_raw.is_empty()
                                                    || remote_raw.starts_with("http://")
                                                    || remote_raw.starts_with("https://")
                                                {
                                                    remote_raw
                                                } else {
                                                    format!("http://{remote_raw}")
                                                };
                                                state.write().remote_url_input = normalized.clone();
                                                let new_tgt = if normalized.is_empty() {
                                                    #[cfg(feature = "desktop")]
                                                    { BackendTarget::Embedded }
                                                    #[cfg(not(feature = "desktop"))]
                                                    { BackendTarget::Remote(String::new()) }
                                                } else {
                                                    BackendTarget::Remote(normalized)
                                                };
                                                switch_backend_target(state, target, live_task, new_tgt);
                                            }
                                        }
                                    },
                                    "Terapkan Koneksi"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
