use crate::actions::set_theme;
use crate::components::icons::{IcoFlame, IcoHelp, IcoMonitor, IcoMoon, IcoServer, IcoSun};
use crate::state::{AppState, ThemePref};
use dioxus::prelude::*;

#[component]
pub fn Topbar() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let s = state.read();

    let port = s.sim.broker_port;
    let is_direct = s.is_direct_mqtt();
    let (broker_tone, broker_text) = if is_direct {
        if !s.live_connected && s.had_disconnect {
            ("danger", format!("Direct :{port} Terputus"))
        } else if s.sim.broker_connected {
            ("ok", format!("Direct :{port} Terhubung"))
        } else {
            ("warn", format!("Menghubungkan :{port}"))
        }
    } else if !s.live_connected && s.had_disconnect {
        ("danger", "Koneksi Server Terputus".to_string())
    } else if !s.sim.enable_mqtt {
        ("warn", "MQTT tidak aktif".to_string())
    } else if s.sim.broker_connected {
        ("ok", format!("Broker :{port} Terhubung"))
    } else {
        ("danger", format!("Broker :{port} Terputus"))
    };

    let src = s.active_source();
    let needs_esp =
        src == "esp32" || s.sim_target == "hils" || s.sim.target == "hils";
    let mode_suffix = s
        .esp
        .hw_mode
        .as_deref()
        .map(|m| format!(" ({m})"))
        .unwrap_or_default();
    let (node_tone, node_text) = if s.esp.online {
        (Some("ok"), format!("ESP32: ONLINE{mode_suffix}"))
    } else if let Some(age) = s.esp_stale_age_s() {
        (Some("danger"), format!("ESP32: TERPUTUS ({age} dtk)"))
    } else if needs_esp {
        (Some("warn"), "ESP32: menunggu".to_string())
    } else {
        (None, "ESP32: belum terdeteksi".to_string())
    };

    let theme = s.theme;
    let (host_label, mob_conn_lbl) = if is_direct {
        ("Direct MQTT", "MQTT")
    } else if s.using_remote_url.is_some() || !s.is_embedded_host {
        ("Server LAN", "LAN")
    } else {
        ("Host Lokal", "Lokal")
    };
    drop(s);

    rsx! {
        header { class: "topbar",
            div { class: "brand-group",
                div { class: "brand-badge", aria_hidden: "true", IcoFlame {} }
                div {
                    h1 { class: "brand-title",
                        "Karhutla Edge-IoT"
                        span { class: "brand-title-ext", " — Pusat Telemetri & Kendali" }
                    }
                    p { class: "brand-sub",
                        "Monitor Real-time SITL, HILS & Node Lapangan • Broker Port 6669"
                    }
                }
            }

            div { class: "status-cluster",
                span { id: "badge-broker", class: "pill-badge", "data-tone": "{broker_tone}",
                    span { class: "status-dot", aria_hidden: "true" }
                    span { id: "txt-broker", "{broker_text}" }
                }
                span {
                    id: "badge-node",
                    class: "pill-badge",
                    "data-tone": node_tone.unwrap_or(""),
                    span { class: "status-dot", aria_hidden: "true" }
                    span { id: "txt-node", "{node_text}" }
                }
            }

            div { class: "topbar-actions",
                div { class: "theme-switch", role: "group", aria_label: "Pilih Tema Tampilan",
                    button {
                        r#type: "button",
                        class: "chip-btn theme-btn",
                        aria_pressed: if theme == ThemePref::Light { "true" } else { "false" },
                        aria_label: "Tema Proyektor Terang",
                        title: "Tema Proyektor Terang",
                        onclick: move |_| set_theme(state, ThemePref::Light),
                        IcoSun {}
                        span { class: "topbar-btn-lbl", "Proyektor" }
                    }
                    button {
                        r#type: "button",
                        class: "chip-btn theme-btn",
                        aria_pressed: if theme == ThemePref::Dark { "true" } else { "false" },
                        aria_label: "Tema Gelap Laboratorium",
                        title: "Tema Gelap Laboratorium",
                        onclick: move |_| set_theme(state, ThemePref::Dark),
                        IcoMoon {}
                        span { class: "topbar-btn-lbl", "Gelap" }
                    }
                    button {
                        r#type: "button",
                        class: "chip-btn theme-btn",
                        aria_pressed: if theme == ThemePref::System { "true" } else { "false" },
                        aria_label: "Tema Ikuti Sistem Perangkat",
                        title: "Ikuti tema perangkat",
                        onclick: move |_| set_theme(state, ThemePref::System),
                        IcoMonitor {}
                        span { class: "topbar-btn-lbl", "Sistem" }
                    }
                }

                button {
                    r#type: "button",
                    class: "chip-btn",
                    id: "btn-theme-cycle",
                    aria_label: format!("Ganti tema tampilan (Saat ini: {})", theme.full_label()),
                    title: format!("Ganti tema tampilan (Saat ini: {})", theme.full_label()),
                    onclick: move |_| {
                        let next = state.read().theme.next_cycle();
                        set_theme(state, next);
                    },
                    match theme {
                        ThemePref::Light => rsx! { IcoSun {} },
                        ThemePref::Dark => rsx! { IcoMoon {} },
                        ThemePref::System => rsx! { IcoMonitor {} },
                    }
                    span { id: "lbl-theme-cycle", "{theme.short_label()}" }
                }

                button {
                    r#type: "button",
                    class: "chip-btn",
                    id: "btn-open-conn",
                    aria_label: "Pengaturan Mode Koneksi Server, LAN, & Direct MQTT",
                    title: "Pengaturan Mode Koneksi Server, LAN, & Direct MQTT",
                    onclick: move |_| state.write().open_conn_modal(),
                    IcoServer {}
                    span { class: "topbar-btn-lbl", "{host_label}" }
                    span { class: "mob-only-lbl", "{mob_conn_lbl}" }
                }

                button {
                    r#type: "button",
                    class: "chip-btn",
                    id: "btn-open-help",
                    aria_label: "Buka Glosarium Metrik dan Pintasan Keyboard",
                    title: "Buka Glosarium Metrik dan Pintasan Keyboard",
                    onclick: move |_| state.write().help_open = true,
                    IcoHelp {}
                    span { class: "topbar-btn-lbl", "Glosarium" }
                    span { class: "mob-only-lbl", "Bantuan" }
                    kbd { class: "key-hint", "?" }
                }
            }
        }
    }
}
