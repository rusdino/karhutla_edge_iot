mod actions;
mod chart_model;
mod components;
mod direct_mqtt;
mod state;
mod transport;

use crate::actions::{
    apply_preset, on_click_start_sim, reload_snapshot, request_actuator_cmd,
    spawn_live_subscriber, toggle_pause_resume,
};
use crate::components::chart_svg::ChartPanel;
use crate::components::control_rail::ControlRail;
use crate::components::kpi_grid::KpiGrid;
use crate::components::mobile_dock::MobileDock;
use crate::components::modals::ModalsAndOverlays;
use crate::components::safety_callout::SafetyCallout;
use crate::components::topbar::Topbar;
use crate::state::{now_ms, AppState, PresetKind, ThemePref};
use crate::transport::{
    apply_dom_theme, load_connection_prefs, load_prefs, sleep_ms, BackendTarget,
};
use dioxus::prelude::*;
use serde::Deserialize;

static KARHUTLA_CSS: Asset = asset!("/assets/karhutla.css");

#[derive(Deserialize)]
struct KeyMsg {
    key: String,
    code: String,
    alt: bool,
    shift: bool,
}

fn main() {
    #[cfg(feature = "desktop")]
    {
        let _host = transport::init_embedded_host();
        let cfg = dioxus::desktop::Config::new().with_window(
            dioxus::desktop::WindowBuilder::new()
                .with_title("Karhutla Edge-IoT — Pusat Telemetri & Kendali")
                .with_inner_size(dioxus::desktop::LogicalSize::new(1280.0, 768.0)),
        );
        LaunchBuilder::desktop().with_cfg(cfg).launch(App);
        return;
    }

    #[cfg(not(feature = "desktop"))]
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let conn_prefs = use_hook(load_connection_prefs);
    let initial_target = use_hook(|| BackendTarget::from_prefs(&conn_prefs));

    let mut state = use_context_provider(|| {
        let mut init = AppState::default();
        init.apply_prefs(load_prefs());
        init.remote_url_input = conn_prefs.remote_url.clone();
        init.mqtt_host_input = conn_prefs.mqtt_host.clone();
        init.mqtt_port_input = conn_prefs.mqtt_port.to_string();
        init.conn_mode = initial_target.mode_key().to_string();
        init.conn_draft_mode = init.conn_mode.clone();
        match &initial_target {
            BackendTarget::Remote(u) if !u.trim().is_empty() => {
                init.using_remote_url = Some(u.trim().to_string());
            }
            BackendTarget::DirectMqtt { host, port } => {
                init.direct_mqtt_endpoint = Some(format!("{host}:{port}"));
            }
            _ => {}
        }
        #[cfg(feature = "desktop")]
        {
            let host = transport::init_embedded_host();
            init.is_embedded_host = true;
            init.embedded_lan_url = host.lan_url.clone();
        }
        Signal::new(init)
    });

    let target = use_context_provider(|| Signal::new(initial_target));
    let mut live_task = use_context_provider(|| {
        Signal::new(None::<dioxus::dioxus_core::Task>)
    });

    // React ONLY when theme preference changes
    let theme_pref = use_memo(move || state.read().theme);
    use_effect(move || {
        let dom_val = match theme_pref() {
            ThemePref::Light => Some("light"),
            ThemePref::Dark => Some("dark"),
            ThemePref::System => None,
        };
        apply_dom_theme(dom_val);
    });

    // Run mount-only background tasks ONCE (never re-triggered by signal reads!)
    use_hook(move || {
        // 1. System dark-mode listener
        spawn(async move {
            let mut eval = dioxus::document::eval(
                r#"
                (function() {
                    const mq = window.matchMedia('(prefers-color-scheme: dark)');
                    dioxus.send(!!mq.matches);
                    if (mq.addEventListener) {
                        mq.addEventListener('change', (e) => dioxus.send(!!e.matches));
                    }
                })()
                "#,
            );
            while let Ok(is_dark) = eval.recv::<bool>().await {
                state.write().system_dark = is_dark;
            }
        });

        // 2. Global keyboard shortcuts listener
        spawn(async move {
            let mut eval = dioxus::document::eval(
                r#"
                (function() {
                    if (window.__karhutlaKeyBound) return;
                    window.__karhutlaKeyBound = true;
                    document.addEventListener('keydown', (e) => {
                        const tag = (e.target && e.target.tagName) || '';
                        if (tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA') {
                            if (e.key !== 'Escape') return;
                        }
                        if (e.ctrlKey || e.metaKey) return;
                        const k = e.key || '';
                        const interested =
                            k === 'Escape' ||
                            k === '?' ||
                            (e.altKey && e.shiftKey && (e.code === 'KeyA' || e.code === 'KeyM')) ||
                            (!e.altKey && (k === 's' || k === 'S' || k === 'p' || k === 'P' || k === ' ' || k === '1' || k === '2' || k === '3'));
                        if (!interested) return;
                        if (k === ' ') {
                            const active = document.activeElement;
                            const aTag = active ? active.tagName : '';
                            if (aTag === 'BUTTON' || aTag === 'A' || aTag === 'SUMMARY') return;
                        }
                        if (k === ' ' || k === '?' || (e.altKey && e.shiftKey)) {
                            e.preventDefault();
                        }
                        dioxus.send({ key: k, code: e.code || '', alt: !!e.altKey, shift: !!e.shiftKey });
                    });
                })()
                "#,
            );
            while let Ok(msg) = eval.recv::<KeyMsg>().await {
                if msg.key == "Escape" {
                    let mut s = state.write();
                    if s.confirm_modal.is_some() {
                        s.confirm_modal = None;
                    } else if s.abbr_popover.is_some() {
                        s.abbr_popover = None;
                    } else if s.conn_modal_open {
                        s.conn_modal_open = false;
                    } else if s.help_open {
                        s.help_open = false;
                    } else if s.mob_tray_open {
                        s.mob_tray_open = false;
                    } else if s.hover_t.is_some() {
                        s.clear_hover();
                    }
                    continue;
                }

                if state.read().confirm_modal.is_some() || state.read().conn_modal_open {
                    continue;
                }

                if state.read().help_open {
                    if msg.key == "?" {
                        state.write().help_open = false;
                    }
                    continue;
                }

                if msg.alt && msg.shift {
                    let can_cmd = state.read().override_routing().can_send;
                    if msg.code == "KeyA" {
                        if can_cmd {
                            request_actuator_cmd(state, target, "AUTO".to_string());
                        }
                    } else if msg.code == "KeyM" && can_cmd {
                        request_actuator_cmd(state, target, "FORCE_MAX".to_string());
                    }
                    continue;
                }

                if msg.key == "?" {
                    state.write().help_open = true;
                    continue;
                }

                if !state.read().single_key_shortcuts {
                    continue;
                }

                match msg.key.as_str() {
                    "s" | "S" => {
                        if !state.read().sim.running {
                            on_click_start_sim(state, target);
                        }
                    }
                    "1" => apply_preset(state, PresetKind::Demo),
                    "2" => apply_preset(state, PresetKind::Dsp),
                    "3" => apply_preset(state, PresetKind::All),
                    "p" | "P" | " " => {
                        if state.read().sim.running {
                            toggle_pause_resume(state, target);
                        }
                    }
                    _ => {}
                }
            }
        });

        // 3. Initial snapshot + live WebSocket/Broadcast subscriber
        let initial_tgt = target.peek().clone();
        reload_snapshot(state, target, false, false);
        let task = spawn_live_subscriber(state, target, initial_tgt);
        live_task.set(Some(task));

        // 4. 1-second ESP32 age ticker + toast TTL sweep
        spawn(async move {
            loop {
                sleep_ms(1000).await;
                let now = now_ms();
                let mut s = state.write();
                s.now_tick_ms = now;
                s.prune_expired_toasts(now);
            }
        });
    });

    rsx! {
        document::Stylesheet { href: KARHUTLA_CSS }
        div {
            class: "app-root",
            a { href: "#control-panel", class: "skip-link", "Lompat ke Panel Kontrol" }
            ModalsAndOverlays {}
            div { class: "page-frame",
                Topbar {}
                SafetyCallout {}
                main { class: "dashboard-shell",
                    KpiGrid {}
                    section { class: "workspace",
                        ChartPanel {}
                        ControlRail {}
                    }
                }
            }
            MobileDock {}
        }
    }
}
