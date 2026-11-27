use crate::actions::{
    apply_preset, export_chart_bundle, persist_prefs, switch_fallback_to_sitl,
    trigger_verified_export,
};
use crate::chart_model::{
    build_chart_models, legend_specs, step_hover_t, x_to_timestamp, ChartPalette,
};
use crate::components::icons::{IcoDownload, IcoFile, IcoSliders};
use crate::state::{
    format_clock, scenario_label, AppState, ConfirmAction, ConfirmModalState, PresetKind, ViewMode,
};
use crate::transport::BackendTarget;
use dioxus::prelude::*;

#[component]
pub fn ChartPanel() -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let target = use_context::<Signal<BackendTarget>>();

    // Track exact CSS pixel dimensions of #main-chart and #sub-chart via ResizeObserver (once on mount)
    use_hook(move || {
        spawn(async move {
            let mut eval = dioxus::document::eval(
                r#"
                (function() {
                    const sendSizes = () => {
                        const m = document.getElementById('main-chart');
                        const s = document.getElementById('sub-chart');
                        if (!m || !s) return;
                        const mr = m.getBoundingClientRect();
                        const sr = s.getBoundingClientRect();
                        if (mr.width > 0 && mr.height > 0 && sr.width > 0 && sr.height > 0) {
                            dioxus.send([Math.round(mr.width), Math.round(mr.height), Math.round(sr.width), Math.round(sr.height)]);
                        }
                    };
                    if ('ResizeObserver' in window) {
                        const ro = new ResizeObserver(sendSizes);
                        const observeBoth = () => {
                            const m = document.getElementById('main-chart');
                            const s = document.getElementById('sub-chart');
                            if (m) ro.observe(m);
                            if (s) ro.observe(s);
                            sendSizes();
                        };
                        observeBoth();
                    } else {
                        window.addEventListener('resize', sendSizes);
                        sendSizes();
                    }
                })()
                "#,
            );
            while let Ok(vals) = eval.recv::<Vec<f64>>().await {
                if vals.len() == 4 {
                    let mut st = state.write();
                    st.main_svg_size = (vals[0].max(260.0), vals[1].max(150.0));
                    st.sub_svg_size = (vals[2].max(260.0), vals[3].max(84.0));
                }
            }
        });
    });

    let s = state.read();
    let src = s.active_source();
    let hist = s.active_history().to_vec();
    let hist_empty = hist.is_empty();
    let allow_pm_true = s.can_show_pm_true();
    let active_layer_count = s.layers.active_count(allow_pm_true);
    let dark = s.is_dark();
    let pal = ChartPalette::for_theme(dark);
    let (mw, mh) = s.main_svg_size;
    let (sw, sh) = s.sub_svg_size;
    let latest_is_fault = s
        .active_latest()
        .map(|r| r.state == "FAILSAFE" || r.sensor_fault)
        .unwrap_or(false);
    let legend = legend_specs(&s.layers, allow_pm_true, latest_is_fault, &pal);

    let (main_m, sub_m) = build_chart_models(
        &hist,
        &s.layers,
        allow_pm_true,
        s.window_sec,
        s.zoom_y,
        dark,
        mw,
        mh,
        sw,
        sh,
        s.hover_t,
        src,
        s.esp.online,
    );

    let src_label = format!(
        "{}{}",
        if src == "esp32" {
            "ESP32 (HILS/Perangkat)"
        } else {
            "SITL (Rust)"
        },
        if s.view_mode == ViewMode::Auto {
            " — mengikuti target"
        } else {
            " — dikunci manual"
        }
    );

    let hils_timeout_shown = s.sim.running
        && s.sim.target == "hils"
        && (s.sim.hils_consec_timeouts > 0 || s.hils_timeout_info.is_some());
    let esp_stale = src == "esp32" && !s.esp.online;
    let show_esp_banner = esp_stale && !hils_timeout_shown;
    let esp_banner_text = if !s.sim.enable_mqtt {
        "Broker MQTT sesi ini tidak aktif — sambungkan broker untuk menerima telemetri ESP32"
            .to_string()
    } else if !s.sim.broker_connected {
        format!(
            "Broker MQTT :{} belum terhubung — pastikan Mosquitto aktif sebelum menguji HILS/ESP32",
            s.sim.broker_port
        )
    } else if let Some(age) = s.esp_stale_age_s() {
        format!(
            "ESP32 TERPUTUS — tidak ada data sejak {age} detik lalu (grafik dibekukan)"
        )
    } else {
        format!(
            "Menunggu ESP32 — belum ada telemetri atau heartbeat di broker :{}",
            s.sim.broker_port
        )
    };

    let view_mode_val = s.view_mode.as_str();
    let preset = s.preset;
    let layers = s.layers.clone();
    let window_val = s.window_sec.to_string();
    let zoom_y = s.zoom_y;
    let layers_open = s.layers_drawer_open;
    let hover_from_touch = s.hover_from_touch && s.hover_t.is_some();
    let run_summary = s.last_run_summary.clone();
    let chart_live_readout = main_m
        .crosshair
        .as_ref()
        .map(|c| c.live_readout.clone())
        .unwrap_or_default();
    let has_right_axis = s.layers.has_right_axis();
    let show_forecast = s.layers.forecast;
    let win_sec = s.window_sec;
    drop(s);

    let mut on_chart_pointer_move = move |coords_x: f64, is_touch: bool| {
        let mut st = state.write();
        let rows = st.active_history().to_vec();
        if rows.is_empty() {
            return;
        }
        let w = st.main_svg_size.0;
        let has_r = st.layers.has_right_axis();
        let fc = st.layers.forecast;
        let ws = st.window_sec;
        if let Some(t) = x_to_timestamp(coords_x, w, has_r, &rows, ws, fc) {
            st.hover_t = Some(t);
            st.hover_from_touch = is_touch;
            st.hover_from_key = false;
        } else if !is_touch {
            st.clear_hover();
        }
    };

    let on_chart_keydown = move |e: KeyboardEvent| {
        let key = e.key();
        if key == Key::ArrowLeft || key == Key::ArrowRight {
            e.prevent_default();
            let delta = if key == Key::ArrowLeft { -1 } else { 1 };
            let mut st = state.write();
            let rows = st.active_history().to_vec();
            if let Some(next_t) = step_hover_t(&rows, st.window_sec, st.hover_t, delta) {
                st.hover_t = Some(next_t);
                st.hover_from_touch = false;
                st.hover_from_key = true;
            }
        } else if key == Key::Escape {
            state.write().clear_hover();
        }
    };

    let on_clear_chart = move |_| {
        let src_s = state.read().active_source().to_string();
        let upper = src_s.to_ascii_uppercase();
        state.write().confirm_modal = Some(ConfirmModalState {
            title: format!("Bersihkan Grafik {upper}?"),
            desc: format!(
                "Seluruh histori kurva dan KPI sumber {upper} akan dikosongkan dari layar saat ini."
            ),
            ok_label: "Ya, Bersihkan".to_string(),
            tone: "danger".to_string(),
            action: ConfirmAction::ClearChart { source: src_s },
        });
    };

    rsx! {
        div { class: "panel-card",
            div { class: "panel-header",
                div {
                    h2 { class: "panel-title", "Grafik Multi-Overlay Real-Time" }
                    div { class: "panel-subtitle-row",
                        p { class: "brand-sub",
                            "Sumber: "
                            strong { id: "src-label", "{src_label}" }
                        }
                    }
                }

                div { class: "chart-actions-wrap desktop-only-controls",
                    select {
                        id: "sel-source",
                        class: "op-select",
                        aria_label: "Sumber Data Grafik",
                        value: "{view_mode_val}",
                        onchange: move |e| {
                            let vm = ViewMode::parse(&e.value());
                            {
                                let mut st = state.write();
                                st.view_mode = vm;
                                if vm == ViewMode::Sitl {
                                    st.last_active_source = "sitl".to_string();
                                } else if vm == ViewMode::Esp32 {
                                    st.last_active_source = "esp32".to_string();
                                }
                                st.clear_hover();
                            }
                            persist_prefs(&state);
                        },
                        option { value: "auto", "Sumber: Ikuti Target" }
                        option { value: "esp32", "Sumber: ESP32" }
                        option { value: "sitl", "Sumber: SITL" }
                    }
                    select {
                        id: "sel-window",
                        class: "op-select",
                        aria_label: "Rentang Waktu Grafik",
                        value: "{window_val}",
                        onchange: move |e| {
                            let parsed = e.value().parse::<u32>().unwrap_or(300);
                            state.write().window_sec = parsed;
                            persist_prefs(&state);
                        },
                        option { value: "120", "120 dtk" }
                        option { value: "300", "300 dtk" }
                        option { value: "900", "900 dtk" }
                        option { value: "0", "Semua" }
                    }
                    button {
                        r#type: "button",
                        class: "chip-btn",
                        id: "btn-export-svg",
                        title: "Unduh grafik vektor SVG",
                        disabled: hist_empty,
                        onclick: move |_| export_chart_bundle(state, false),
                        IcoDownload {}
                        "SVG"
                    }
                    button {
                        r#type: "button",
                        class: "chip-btn",
                        id: "btn-export-png",
                        title: "Unduh gambar grafik lengkap PNG",
                        disabled: hist_empty,
                        onclick: move |_| export_chart_bundle(state, true),
                        IcoDownload {}
                        "PNG"
                    }
                    button {
                        r#type: "button",
                        class: "chip-btn",
                        id: "btn-export-csv",
                        title: "Unduh data histori grafik CSV",
                        disabled: hist_empty,
                        onclick: move |_| {
                            let src_s = state.read().active_source().to_string();
                            trigger_verified_export(
                                state,
                                target,
                                format!("/api/export-csv?source={src_s}"),
                                "karhutla_telemetry.csv",
                                "text/csv; charset=utf-8",
                                format!("Data CSV ({}) diunduh.", src_s.to_ascii_uppercase()),
                            );
                        },
                        IcoFile {}
                        "CSV"
                    }
                }
            }

            // Mobile compact 1-row chart control strip (keeps main chart >=178px!)
            div { class: "mobile-chart-strip",
                select {
                    id: "mob-sel-preset",
                    class: "op-select",
                    aria_label: "Preset Kurva Mobile",
                    value: "{preset.as_str()}",
                    onchange: move |e| {
                        let pk = PresetKind::parse(&e.value());
                        if pk != PresetKind::Custom {
                            apply_preset(state, pk);
                        }
                    },
                    option { value: "demo", "Preset: Demo" }
                    option { value: "dsp", "Preset: DSP" }
                    option { value: "all", "Preset: Semua" }
                    if preset == PresetKind::Custom {
                        option { value: "custom", "Preset: Kustom" }
                    }
                }
                select {
                    id: "mob-sel-source",
                    class: "op-select",
                    aria_label: "Sumber Grafik Mobile",
                    value: "{view_mode_val}",
                    onchange: move |e| {
                        let vm = ViewMode::parse(&e.value());
                        {
                            let mut st = state.write();
                            st.view_mode = vm;
                            if vm == ViewMode::Sitl {
                                st.last_active_source = "sitl".to_string();
                            } else if vm == ViewMode::Esp32 {
                                st.last_active_source = "esp32".to_string();
                            }
                            st.clear_hover();
                        }
                        persist_prefs(&state);
                    },
                    option { value: "auto", "Src: Auto" }
                    option { value: "esp32", "Src: ESP32" }
                    option { value: "sitl", "Src: SITL" }
                }
                button {
                    r#type: "button",
                    class: "chip-btn",
                    id: "mob-btn-layers",
                    aria_expanded: if layers_open { "true" } else { "false" },
                    aria_controls: "layers-drawer",
                    onclick: move |_| {
                        let open = !state.read().layers_drawer_open;
                        state.write().layers_drawer_open = open;
                    },
                    IcoSliders {}
                    "Kurva ({active_layer_count}/9)"
                }
            }

            // Desktop preset bar
            div { class: "preset-bar desktop-only-controls",
                div { role: "toolbar", aria_label: "Tampilan Kurva Grafik", id: "preset-toolbar",
                    span { class: "preset-label", "Tampilan Kurva:" }
                    button {
                        r#type: "button",
                        class: "chip-btn preset-btn",
                        aria_pressed: if preset == PresetKind::Demo { "true" } else { "false" },
                        onclick: move |_| apply_preset(state, PresetKind::Demo),
                        "Demo Sidang "
                        kbd { class: "key-hint", "1" }
                    }
                    button {
                        r#type: "button",
                        class: "chip-btn preset-btn",
                        aria_pressed: if preset == PresetKind::Dsp { "true" } else { "false" },
                        onclick: move |_| apply_preset(state, PresetKind::Dsp),
                        "Validasi DSP "
                        kbd { class: "key-hint", "2" }
                    }
                    button {
                        r#type: "button",
                        class: "chip-btn preset-btn",
                        aria_pressed: if preset == PresetKind::All { "true" } else { "false" },
                        onclick: move |_| apply_preset(state, PresetKind::All),
                        "Semua Kurva "
                        kbd { class: "key-hint", "3" }
                    }
                    if preset == PresetKind::Custom {
                        span { id: "badge-preset-custom", class: "pill-badge", "data-tone": "accent", "Kustom" }
                    }
                }
                div { class: "preset-actions",
                    button {
                        r#type: "button",
                        class: "chip-btn",
                        id: "btn-toggle-layers",
                        aria_expanded: if layers_open { "true" } else { "false" },
                        aria_controls: "layers-drawer",
                        onclick: move |_| {
                            let open = !state.read().layers_drawer_open;
                            state.write().layers_drawer_open = open;
                        },
                        IcoSliders {}
                        span { id: "lbl-toggle-layers", "Atur Kurva ({active_layer_count}/9)" }
                    }
                    button {
                        r#type: "button",
                        class: "chip-btn",
                        id: "btn-sim-clear",
                        "data-variant": "danger",
                        title: "Bersihkan seluruh kurva pada sumber grafik aktif",
                        disabled: hist_empty,
                        onclick: on_clear_chart,
                        "Bersihkan"
                    }
                }
            }

            if layers_open {
                div { id: "layers-drawer", class: "compact-drawer",
                    div { class: "drawer-group-label", "Kurva Telemetri & Skala Sumbu Kanan" }
                    div { role: "toolbar", aria_label: "Filter Kurva Overlay Grafik", id: "overlay-toolbar",
                        div { role: "group", aria_label: "Kurva Utama",
                            LayerBtn { key_name: "fi", label: "Fusion Index (FI)", pressed: layers.fi, disabled: false, on_click: move |_| toggle_layer(state, "fi") }
                            LayerBtn { key_name: "pm", label: "PM-proxy Kalman", pressed: layers.pm, disabled: false, on_click: move |_| toggle_layer(state, "pm") }
                            LayerBtn { key_name: "forecast", label: "Prakiraan AI (+60s)", pressed: layers.forecast, disabled: false, on_click: move |_| toggle_layer(state, "forecast") }
                            LayerBtn { key_name: "thresholds", label: "Ambang 55 / 150", pressed: layers.thresholds, disabled: false, on_click: move |_| toggle_layer(state, "thresholds") }
                        }
                        span { role: "separator", aria_orientation: "vertical" }
                        div { role: "group", aria_label: "Validasi DSP",
                            LayerBtn { key_name: "pm_true", label: "PM Ruang Asap (True)", pressed: layers.pm_true, disabled: !allow_pm_true, on_click: move |_| toggle_layer(state, "pm_true") }
                            LayerBtn { key_name: "pm_raw", label: "PM Mentah (Median)", pressed: layers.pm_raw, disabled: false, on_click: move |_| toggle_layer(state, "pm_raw") }
                        }
                        span { role: "separator", aria_orientation: "vertical" }
                        div { role: "group", aria_label: "Gas dan Tegangan",
                            LayerBtn { key_name: "gas_ppm", label: "Gas MQ-135 (×0.2)", pressed: layers.gas_ppm, disabled: false, on_click: move |_| toggle_layer(state, "gas_ppm") }
                            LayerBtn { key_name: "g_scaled", label: "Skor g (×100)", pressed: layers.g_scaled, disabled: false, on_click: move |_| toggle_layer(state, "g_scaled") }
                            button {
                                r#type: "button",
                                class: "chip-btn layer-btn",
                                "data-layer": "voltages",
                                aria_pressed: if layers.voltages { "true" } else { "false" },
                                onclick: move |_| toggle_layer(state, "voltages"),
                                span { class: "chip-dot chip-dot-vd", aria_hidden: "true" }
                                span { class: "chip-dot chip-dot-vm", aria_hidden: "true" }
                                "Tegangan Vd/Vm (×50)"
                            }
                        }
                    }

                    p { class: "brand-sub",
                        "Faktor skala sumbu kanan: Gas ×0.2 • g ×100 • Vd/Vm ×50."
                    }

                    label { class: "touch-label", r#for: "sw-zoom-y",
                        input {
                            id: "sw-zoom-y",
                            r#type: "checkbox",
                            role: "switch",
                            checked: zoom_y,
                            onchange: move |e| {
                                state.write().zoom_y = e.checked();
                                persist_prefs(&state);
                            },
                        }
                        span { "Skala Y Ketat (Zoom otomatis untuk skenario udara bersih)" }
                    }

                    // Mobile extra actions tucked inside drawer so top controls stay 1-row
                    div { class: "mob-only-lbl preset-actions",
                        select {
                            class: "op-select",
                            aria_label: "Rentang Waktu Grafik Mobile",
                            value: "{window_val}",
                            onchange: move |e| {
                                let parsed = e.value().parse::<u32>().unwrap_or(300);
                                state.write().window_sec = parsed;
                                persist_prefs(&state);
                            },
                            option { value: "120", "120 dtk" }
                            option { value: "300", "300 dtk" }
                            option { value: "900", "900 dtk" }
                            option { value: "0", "Semua" }
                        }
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            disabled: hist_empty,
                            onclick: move |_| export_chart_bundle(state, false),
                            "SVG"
                        }
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            disabled: hist_empty,
                            onclick: move |_| export_chart_bundle(state, true),
                            "PNG"
                        }
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            disabled: hist_empty,
                            onclick: move |_| {
                                let src_s = state.read().active_source().to_string();
                                trigger_verified_export(
                                    state,
                                    target,
                                    format!("/api/export-csv?source={src_s}"),
                                    "karhutla_telemetry.csv",
                                    "text/csv; charset=utf-8",
                                    format!("Data CSV ({}) diunduh.", src_s.to_ascii_uppercase()),
                                );
                            },
                            "CSV"
                        }
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            "data-variant": "danger",
                            disabled: hist_empty,
                            onclick: on_clear_chart,
                            "Bersihkan"
                        }
                    }
                }
            }

            // Active Legend
            div { id: "chart-legend", class: "chart-legend", aria_label: "Legenda Kurva Aktif",
                if legend.is_empty() {
                    span { class: "brand-sub", "Tidak ada kurva aktif — buka Atur Kurva untuk memilih." }
                } else {
                    for item in legend {
                        span {
                            class: "legend-item",
                            "data-dimmed": if item.dimmed { "true" } else { "false" },
                            span {
                                class: "legend-swatch",
                                "data-style": "{item.style}",
                                style: "color: {item.color};",
                            }
                            span { "{item.label}" }
                        }
                    }
                }
            }

            // Main Pure Reactive SVG Overlay Chart
            div {
                id: "main-wrap",
                class: if show_esp_banner { "canvas-wrap is-stale" } else { "canvas-wrap" },
                if show_esp_banner {
                    div { id: "esp-banner", class: "offline-banner",
                        span { id: "esp-banner-text", "{esp_banner_text}" }
                        button {
                            r#type: "button",
                            class: "chip-btn",
                            id: "btn-fallback-sitl",
                            onclick: move |_| switch_fallback_to_sitl(state, target),
                            "Beralih ke SITL"
                        }
                    }
                }
                if hover_from_touch {
                    button {
                        r#type: "button",
                        id: "btn-clear-hover",
                        class: "chip-btn hover-dismiss-btn",
                        onclick: move |_| state.write().clear_hover(),
                        "Tutup Inspeksi"
                    }
                }
                svg {
                    id: "main-chart",
                    class: "chart-stage",
                    view_box: format!("0 0 {:.0} {:.0}", main_m.w, main_m.h),
                    preserve_aspect_ratio: "none",
                    tabindex: "0",
                    role: "img",
                    "aria-label": "Grafik Utama Multi-Overlay (gunakan panah kiri atau kanan untuk inspeksi titik)",
                    "aria-describedby": "chart-sr-summary",
                    onmousemove: move |e| on_chart_pointer_move(e.element_coordinates().x, false),
                    onclick: move |e| on_chart_pointer_move(e.element_coordinates().x, mw < 768.0),
                    onmouseleave: move |_| {
                        if !state.read().hover_from_touch {
                            state.write().clear_hover();
                        }
                    },
                    onkeydown: on_chart_keydown,

                    defs {
                        pattern {
                            id: "fault-hatch-main",
                            width: "7",
                            height: "7",
                            pattern_units: "userSpaceOnUse",
                            line { x1: "0", y1: "7", x2: "7", y2: "0", stroke: "{pal.fault_hatch}", stroke_width: "1.5" }
                            line { x1: "-2", y1: "2", x2: "2", y2: "-2", stroke: "{pal.fault_hatch}", stroke_width: "1.5" }
                            line { x1: "5", y1: "9", x2: "9", y2: "5", stroke: "{pal.fault_hatch}", stroke_width: "1.5" }
                        }
                    }

                    if let Some(ref msg) = main_m.empty_message {
                        text {
                            x: format!("{:.1}", main_m.w * 0.5),
                            y: format!("{:.1}", main_m.h * 0.5),
                            text_anchor: "middle",
                            font_size: "13",
                            font_weight: "600",
                            fill: "{pal.text}",
                            "{msg}"
                        }
                    } else {
                        if let Some(ref hz) = main_m.horizon {
                            rect {
                                x: format!("{:.1}", hz.x),
                                y: format!("{:.1}", hz.y),
                                width: format!("{:.1}", hz.w),
                                height: format!("{:.1}", hz.h),
                                fill: "{hz.fill}",
                            }
                            line {
                                x1: format!("{:.1}", hz.x),
                                y1: format!("{:.1}", hz.y),
                                x2: format!("{:.1}", hz.x),
                                y2: format!("{:.1}", main_m.h - main_m.pad_b),
                                stroke: "{hz.stroke}",
                                stroke_width: "1.25",
                                stroke_dasharray: "4 3",
                            }
                            text {
                                x: format!("{:.1}", hz.label_x),
                                y: format!("{:.1}", hz.y + 14.0),
                                text_anchor: "{hz.label_anchor}",
                                font_family: "var(--font-mono)",
                                font_size: "11.5",
                                font_weight: "700",
                                fill: "{hz.label_color}",
                                "{hz.label}"
                            }
                        }

                        // Horizontal grid + Left/Right Axis Labels
                        for gy in &main_m.grid_y {
                            line {
                                x1: format!("{:.1}", main_m.pad_l),
                                y1: format!("{:.1}", gy.y),
                                x2: format!("{:.1}", main_m.w - main_m.pad_r),
                                y2: format!("{:.1}", gy.y),
                                stroke: "{pal.grid}",
                                stroke_width: "1",
                            }
                            text {
                                x: "6",
                                y: format!("{:.1}", gy.y + 4.0),
                                font_family: "var(--font-mono)",
                                font_size: "11.5",
                                fill: "{pal.text}",
                                "{gy.left_label}"
                            }
                            if !gy.right_spans.is_empty() {
                                text {
                                    x: format!("{:.1}", main_m.w - main_m.pad_r + 4.0),
                                    y: format!("{:.1}", gy.y + 4.0),
                                    font_family: "var(--font-mono)",
                                    font_size: "11",
                                    font_weight: "600",
                                    for (txt, col) in &gy.right_spans {
                                        tspan { fill: "{col}", "{txt}" }
                                    }
                                }
                            }
                        }
                        text {
                            x: "6",
                            y: "15",
                            font_size: "11.5",
                            font_weight: "700",
                            fill: "{pal.text}",
                            "µg/m³ / FI"
                        }
                        if !main_m.right_header_spans.is_empty() {
                            text {
                                x: format!("{:.1}", main_m.w - main_m.pad_r + 4.0),
                                y: "15",
                                font_family: "var(--font-mono)",
                                font_size: "11.5",
                                font_weight: "700",
                                for sp in &main_m.right_header_spans {
                                    tspan { fill: "{sp.color}", "{sp.text}" }
                                }
                            }
                        }

                        // Vertical time grid
                        for gx in &main_m.grid_x {
                            line {
                                x1: format!("{:.1}", gx.x),
                                y1: format!("{:.1}", main_m.pad_t),
                                x2: format!("{:.1}", gx.x),
                                y2: format!("{:.1}", main_m.h - main_m.pad_b),
                                stroke: "{pal.grid}",
                                stroke_width: "1",
                            }
                            if !gx.label.is_empty() {
                                text {
                                    x: format!("{:.1}", gx.label_x),
                                    y: format!("{:.1}", main_m.h - 8.0),
                                    font_family: "var(--font-mono)",
                                    font_size: "11.5",
                                    fill: "{pal.text}",
                                    "{gx.label}"
                                }
                            }
                        }

                        // Threshold dashed lines
                        for thr in &main_m.thresholds {
                            line {
                                x1: format!("{:.1}", main_m.pad_l),
                                y1: format!("{:.1}", thr.y),
                                x2: format!("{:.1}", main_m.w - main_m.pad_r),
                                y2: format!("{:.1}", thr.y),
                                stroke: "{thr.color}",
                                stroke_width: "1.4",
                                stroke_dasharray: "7 5",
                            }
                            text {
                                x: format!("{:.1}", thr.label_x),
                                y: format!("{:.1}", thr.y - 5.0),
                                text_anchor: "{thr.anchor}",
                                font_size: "11.5",
                                font_weight: "700",
                                fill: "{thr.color}",
                                "{thr.label}"
                            }
                        }
                        if let Some((ref zn_txt, zn_x, zn_col)) = main_m.zoom_note {
                            text {
                                x: format!("{zn_x:.1}"),
                                y: format!("{:.1}", main_m.pad_t + 14.0),
                                font_size: "11.5",
                                font_weight: "600",
                                fill: "{zn_col}",
                                "{zn_txt}"
                            }
                        }

                        // Historical FAILSAFE hatched bands + ALWAYS-VISIBLE top FAULT tag
                        for fb in &main_m.fault_bands {
                            rect {
                                x: format!("{:.1}", fb.x),
                                y: format!("{:.1}", main_m.pad_t),
                                width: format!("{:.1}", fb.w),
                                height: format!("{:.1}", main_m.plot_h),
                                fill: "{pal.fault_fill}",
                            }
                            rect {
                                x: format!("{:.1}", fb.x),
                                y: format!("{:.1}", main_m.pad_t),
                                width: format!("{:.1}", fb.w),
                                height: format!("{:.1}", main_m.plot_h),
                                fill: "url(#fault-hatch-main)",
                            }
                            line {
                                x1: format!("{:.1}", fb.x),
                                y1: format!("{:.1}", main_m.pad_t),
                                x2: format!("{:.1}", fb.x),
                                y2: format!("{:.1}", main_m.h - main_m.pad_b),
                                stroke: "{pal.fault}",
                                stroke_width: "1.25",
                                stroke_dasharray: "4 3",
                            }
                            if fb.draw_right_line {
                                line {
                                    x1: format!("{:.1}", fb.x + fb.w),
                                    y1: format!("{:.1}", main_m.pad_t),
                                    x2: format!("{:.1}", fb.x + fb.w),
                                    y2: format!("{:.1}", main_m.h - main_m.pad_b),
                                    stroke: "{pal.fault}",
                                    stroke_width: "1.25",
                                    stroke_dasharray: "4 3",
                                }
                            }
                            rect {
                                x: format!("{:.1}", fb.top_tag_x),
                                y: format!("{:.1}", main_m.pad_t + 2.0),
                                width: format!("{:.1}", fb.top_tag_w),
                                height: "15",
                                rx: "4",
                                fill: "{pal.tip_bg}",
                                stroke: "{pal.fault}",
                                stroke_width: "1",
                            }
                            text {
                                x: format!("{:.1}", fb.top_tag_x + fb.top_tag_w * 0.5),
                                y: format!("{:.1}", main_m.pad_t + 13.0),
                                text_anchor: "middle",
                                font_family: "var(--font-mono)",
                                font_size: "10",
                                font_weight: "700",
                                fill: "{pal.fault}",
                                "{fb.top_tag_label}"
                            }
                            if let Some(in_lbl) = fb.inline_label {
                                rect {
                                    x: format!("{:.1}", fb.inline_label_x),
                                    y: format!("{:.1}", fb.inline_label_y - 11.0),
                                    width: format!("{:.1}", fb.inline_pill_w),
                                    height: "15",
                                    rx: "3",
                                    fill: "{pal.export_bg}",
                                    fill_opacity: "0.94",
                                }
                                text {
                                    x: format!("{:.1}", fb.inline_label_x + 3.0),
                                    y: format!("{:.1}", fb.inline_label_y),
                                    font_family: "var(--font-mono)",
                                    font_size: "11",
                                    font_weight: "700",
                                    fill: "{pal.fault}",
                                    "{in_lbl}"
                                }
                            }
                        }

                        // Data curves
                        for ser in &main_m.series {
                            for pts in &ser.points_list {
                                polyline {
                                    fill: "none",
                                    stroke: "{ser.color}",
                                    stroke_width: format!("{:.2}", ser.stroke_width),
                                    stroke_dasharray: "{ser.dash}",
                                    opacity: format!("{:.2}", ser.opacity),
                                    stroke_linejoin: "round",
                                    stroke_linecap: "round",
                                    points: "{pts}",
                                }
                            }
                        }

                        // Active +60s forecast segment & endpoint dot
                        if let Some(ref tip) = main_m.forecast_tip {
                            line {
                                x1: format!("{:.1}", tip.x0),
                                y1: format!("{:.1}", tip.y0),
                                x2: format!("{:.1}", tip.x1),
                                y2: format!("{:.1}", tip.y1),
                                stroke: "{tip.color}",
                                stroke_width: "2.5",
                                stroke_dasharray: "5 4",
                            }
                            circle {
                                cx: format!("{:.1}", tip.x1),
                                cy: format!("{:.1}", tip.y1),
                                r: "4",
                                fill: "{tip.color}",
                            }
                        }

                        // Synchronized Crosshair + Tooltip
                        if let Some(ref ch) = main_m.crosshair {
                            line {
                                x1: format!("{:.1}", ch.x),
                                y1: format!("{:.1}", main_m.pad_t),
                                x2: format!("{:.1}", ch.x),
                                y2: format!("{:.1}", main_m.h - main_m.pad_b),
                                stroke: "{pal.text}",
                                stroke_width: "1",
                                stroke_dasharray: "3 3",
                            }
                            rect {
                                x: format!("{:.1}", ch.box_x),
                                y: format!("{:.1}", ch.box_y),
                                width: format!("{:.1}", ch.box_w),
                                height: format!("{:.1}", ch.box_h),
                                rx: "6",
                                fill: "{pal.tip_bg}",
                                stroke: "{pal.tip_stroke}",
                                stroke_width: "1",
                            }
                            for (idx, line_txt) in ch.lines.iter().enumerate() {
                                text {
                                    x: format!("{:.1}", ch.box_x + 8.0),
                                    y: format!("{:.1}", ch.box_y + 16.0 + (idx as f64) * 17.0),
                                    font_family: "var(--font-mono)",
                                    font_size: "11.5",
                                    font_weight: if idx == 0 { "700" } else { "500" },
                                    fill: "{pal.tip_fg}",
                                    "{line_txt}"
                                }
                            }
                        }
                    }
                }
                p { id: "chart-sr-summary", class: "sr-only", "{main_m.sr_summary}" }
                div {
                    id: "chart-live-readout",
                    class: "sr-only",
                    role: "status",
                    aria_live: "polite",
                    aria_atomic: "true",
                    "{chart_live_readout}"
                }
            }

            // Sub Pure Reactive SVG Chart (Duty PWM + 4-State FSM Ribbon)
            div { class: "canvas-wrap",
                svg {
                    id: "sub-chart",
                    class: "subchart-stage",
                    view_box: format!("0 0 {:.0} {:.0}", sub_m.w, sub_m.h),
                    preserve_aspect_ratio: "none",
                    tabindex: "0",
                    role: "img",
                    "aria-label": "Sub-Grafik Duty Kipas dan Pita State FSM (gunakan panah kiri atau kanan untuk inspeksi)",
                    "aria-describedby": "subchart-sr-summary",
                    onmousemove: move |e| on_chart_pointer_move(e.element_coordinates().x, false),
                    onclick: move |e| on_chart_pointer_move(e.element_coordinates().x, sw < 768.0),
                    onmouseleave: move |_| {
                        if !state.read().hover_from_touch {
                            state.write().clear_hover();
                        }
                    },
                    onkeydown: move |e| {
                        let key = e.key();
                        if key == Key::ArrowLeft || key == Key::ArrowRight {
                            e.prevent_default();
                            let delta = if key == Key::ArrowLeft { -1 } else { 1 };
                            let mut st = state.write();
                            let rows = st.active_history().to_vec();
                            if let Some(next_t) = step_hover_t(&rows, st.window_sec, st.hover_t, delta) {
                                st.hover_t = Some(next_t);
                                st.hover_from_touch = false;
                                st.hover_from_key = true;
                            }
                        } else if key == Key::Escape {
                            state.write().clear_hover();
                        }
                        let _ = (has_right_axis, show_forecast, win_sec);
                    },

                    defs {
                        pattern {
                            id: "fault-hatch-sub",
                            width: "7",
                            height: "7",
                            pattern_units: "userSpaceOnUse",
                            line { x1: "0", y1: "7", x2: "7", y2: "0", stroke: "{pal.fault_hatch}", stroke_width: "1.4" }
                            line { x1: "-2", y1: "2", x2: "2", y2: "-2", stroke: "{pal.fault_hatch}", stroke_width: "1.4" }
                            line { x1: "5", y1: "9", x2: "9", y2: "5", stroke: "{pal.fault_hatch}", stroke_width: "1.4" }
                        }
                    }

                    if let Some(empty_sub) = sub_m.empty_message {
                        text {
                            x: format!("{:.1}", sub_m.w * 0.5),
                            y: format!("{:.1}", sub_m.h * 0.5 + 4.0),
                            text_anchor: "middle",
                            font_size: "12",
                            font_weight: "600",
                            fill: "{pal.muted}",
                            "{empty_sub}"
                        }
                    } else {
                        // Sub-chart header legend with 4-color FSM swatch (Run 12 Minor Obs #3 fix)
                        rect { x: format!("{:.1}", sub_m.pad_l), y: "8", width: "12", height: "3", fill: "{pal.duty}" }
                        text {
                            x: format!("{:.1}", sub_m.pad_l + 16.0),
                            y: "14",
                            font_size: "11.5",
                            font_weight: "700",
                            fill: "{pal.text}",
                            if sub_m.compact { "Duty %" } else { "Duty Kipas (0–100%)" }
                        }

                        {
                            let fsm_x = sub_m.pad_l + if sub_m.compact { 68.0 } else { 148.0 };
                            let fs_x = fsm_x + if sub_m.compact { 56.0 } else { 112.0 };
                            let fsm_lbl = if sub_m.compact { "FSM" } else { "Pita FSM" };
                            rsx! {
                                rect { x: format!("{fsm_x:.1}"), y: "6", width: "4", height: "7", fill: "{pal.pm_true}" }
                                rect { x: format!("{:.1}", fsm_x + 4.5), y: "6", width: "4", height: "7", fill: "{pal.waspada}" }
                                rect { x: format!("{:.1}", fsm_x + 9.0), y: "6", width: "4", height: "7", fill: "{pal.bahaya}" }
                                rect { x: format!("{:.1}", fsm_x + 13.5), y: "6", width: "4", height: "7", fill: "{pal.fault}" }
                                text {
                                    x: format!("{:.1}", fsm_x + 21.0),
                                    y: "14",
                                    font_size: "11.5",
                                    font_weight: "700",
                                    fill: "{pal.text}",
                                    "{fsm_lbl}"
                                }
                                if fs_x + 110.0 <= sub_m.w - 4.0 {
                                    line {
                                        x1: format!("{fs_x:.1}"),
                                        y1: "9.5",
                                        x2: format!("{:.1}", fs_x + 12.0),
                                        y2: "9.5",
                                        stroke: "{pal.fault}",
                                        stroke_width: "2",
                                        stroke_dasharray: "4 3",
                                    }
                                    text {
                                        x: format!("{:.1}", fs_x + 16.0),
                                        y: "14",
                                        font_size: "11.5",
                                        font_weight: "700",
                                        fill: "{pal.fault}",
                                        "70% FAILSAFE"
                                    }
                                }
                            }
                        }

                        // Duty grid
                        for (y, lbl) in &sub_m.duty_y_ticks {
                            line {
                                x1: format!("{:.1}", sub_m.pad_l),
                                y1: format!("{y:.1}"),
                                x2: format!("{:.1}", sub_m.w - sub_m.pad_r),
                                y2: format!("{y:.1}"),
                                stroke: "{pal.grid}",
                                stroke_width: "1",
                            }
                            text {
                                x: "6",
                                y: format!("{:.1}", y + 4.0),
                                font_family: "var(--font-mono)",
                                font_size: "11",
                                fill: "{pal.text}",
                                "{lbl}"
                            }
                        }

                        line {
                            x1: format!("{:.1}", sub_m.pad_l),
                            y1: format!("{:.1}", sub_m.y70),
                            x2: format!("{:.1}", sub_m.w - sub_m.pad_r),
                            y2: format!("{:.1}", sub_m.y70),
                            stroke: "{pal.fault}",
                            stroke_width: "1.2",
                            stroke_dasharray: "4 4",
                            opacity: "0.7",
                        }
                        line {
                            x1: format!("{:.1}", sub_m.pad_l),
                            y1: format!("{:.1}", sub_m.sep_y),
                            x2: format!("{:.1}", sub_m.w - sub_m.pad_r),
                            y2: format!("{:.1}", sub_m.sep_y),
                            stroke: "{pal.grid}",
                            stroke_width: "1",
                        }
                        text {
                            x: "6",
                            y: format!("{:.1}", sub_m.ribbon_top + sub_m.ribbon_h * 0.5 + 4.0),
                            font_family: "var(--font-mono)",
                            font_size: "11.5",
                            font_weight: "700",
                            fill: "{pal.text}",
                            "FSM"
                        }

                        if !sub_m.duty_points.is_empty() {
                            polyline {
                                fill: "none",
                                stroke: "{pal.duty}",
                                stroke_width: "2.4",
                                points: "{sub_m.duty_points}",
                            }
                        }

                        for seg in &sub_m.ribbon_segments {
                            rect {
                                x: format!("{:.1}", seg.x),
                                y: format!("{:.1}", sub_m.ribbon_top),
                                width: format!("{:.1}", seg.w),
                                height: format!("{:.1}", sub_m.ribbon_h),
                                fill: "{seg.fill}",
                                stroke: "{seg.stroke}",
                                stroke_width: "1.3",
                            }
                            if seg.hatch {
                                rect {
                                    x: format!("{:.1}", seg.x),
                                    y: format!("{:.1}", sub_m.ribbon_top),
                                    width: format!("{:.1}", seg.w),
                                    height: format!("{:.1}", sub_m.ribbon_h),
                                    fill: "url(#fault-hatch-sub)",
                                }
                            }
                            if seg.draw_inner_label {
                                text {
                                    x: format!("{:.1}", seg.text_x),
                                    y: format!("{:.1}", sub_m.ribbon_top + sub_m.ribbon_h * 0.5 + 4.0),
                                    text_anchor: "middle",
                                    font_family: "var(--font-mono)",
                                    font_size: "11",
                                    font_weight: "700",
                                    fill: "{pal.tip_fg}",
                                    "{seg.label}"
                                }
                            }
                        }

                        if let Some((ref t_lbl, tx, tcol)) = sub_m.tail_label {
                            rect {
                                x: format!("{:.1}", tx - 3.0),
                                y: format!("{:.1}", sub_m.ribbon_top + 2.0),
                                width: format!("{:.1}", (t_lbl.chars().count() as f64) * 7.4 + 6.0),
                                height: format!("{:.1}", (sub_m.ribbon_h - 4.0).max(12.0)),
                                rx: "3",
                                fill: "{pal.export_bg}",
                                fill_opacity: "0.92",
                            }
                            text {
                                x: format!("{tx:.1}"),
                                y: format!("{:.1}", sub_m.ribbon_top + sub_m.ribbon_h * 0.5 + 4.0),
                                font_family: "var(--font-mono)",
                                font_size: "11",
                                font_weight: "700",
                                fill: "{tcol}",
                                "{t_lbl}"
                            }
                        }

                        if let Some((hx, hy, hw, hh, hfill, hstroke)) = sub_m.horizon {
                            rect {
                                x: format!("{hx:.1}"),
                                y: format!("{hy:.1}"),
                                width: format!("{hw:.1}"),
                                height: format!("{hh:.1}"),
                                fill: "{hfill}",
                            }
                            line {
                                x1: format!("{hx:.1}"),
                                y1: format!("{hy:.1}"),
                                x2: format!("{hx:.1}"),
                                y2: format!("{:.1}", hy + hh),
                                stroke: "{hstroke}",
                                stroke_width: "1.25",
                                stroke_dasharray: "4 3",
                            }
                        }

                        if let Some(cx) = sub_m.crosshair_x {
                            line {
                                x1: format!("{cx:.1}"),
                                y1: "22",
                                x2: format!("{cx:.1}"),
                                y2: format!("{:.1}", sub_m.h - 8.0),
                                stroke: "{pal.text}",
                                stroke_width: "1",
                                stroke_dasharray: "3 3",
                            }
                        }
                    }
                }
                p { id: "subchart-sr-summary", class: "sr-only", "{sub_m.sr_summary}" }
            }

            // Persistent Run Summary Card
            if let Some(sum) = run_summary {
                {
                    let rs_title = if sum.stopped_early {
                        format!("Ringkasan Simulasi (Dihentikan t={})", format_clock(sum.duration_s))
                    } else {
                        "Ringkasan Simulasi Terakhir".to_string()
                    };
                    let sum_src = sum.source.clone();
                    let bahaya_txt = sum
                        .t_bahaya_s
                        .map(|t| format!("t={t:.1}s"))
                        .unwrap_or_else(|| "Tidak tercapai".to_string());
                    let hp_txt = sum.hils_parity.as_ref().filter(|p| p.samples > 0).map(|hp| {
                        let ok = hp.max_delta_fi < 1.0 && hp.state_matches == hp.samples;
                        format!("ΔFI maks {:.2} ({})", hp.max_delta_fi, if ok { "OK" } else { "Cek" })
                    });
                    rsx! {
                        div { id: "run-summary-card", class: "run-summary", role: "region", aria_labelledby: "rs-title",
                            div { class: "run-summary-head",
                                strong { id: "rs-title", "{rs_title}" }
                                div { class: "run-summary-actions",
                                    button {
                                        r#type: "button",
                                        class: "chip-btn",
                                        onclick: move |_| export_chart_bundle(state, false),
                                        "Unduh SVG"
                                    }
                                    button {
                                        r#type: "button",
                                        class: "chip-btn",
                                        id: "rs-export",
                                        onclick: move |_| export_chart_bundle(state, true),
                                        "Unduh PNG"
                                    }
                                    button {
                                        r#type: "button",
                                        class: "chip-btn",
                                        id: "rs-export-csv",
                                        onclick: move |_| {
                                            let s_src = sum_src.clone();
                                            trigger_verified_export(
                                                state,
                                                target,
                                                format!("/api/export-csv?source={s_src}"),
                                                "karhutla_telemetry.csv",
                                                "text/csv; charset=utf-8",
                                                format!("Data CSV ({}) diunduh.", s_src.to_ascii_uppercase()),
                                            );
                                        },
                                        "Unduh CSV"
                                    }
                                    button {
                                        r#type: "button",
                                        class: "chip-btn",
                                        id: "rs-close",
                                        onclick: move |_| state.write().last_run_summary = None,
                                        "Tutup"
                                    }
                                }
                            }
                            dl { class: "run-summary-grid",
                                div { dt { "Skenario" } dd { "{scenario_label(&sum.scenario)}" } }
                                div { dt { "Target" } dd { "{sum.target.to_ascii_uppercase()}" } }
                                div { dt { "FI puncak" } dd { "{sum.peak_fi:.1} µg/m³" } }
                                div { dt { "Waktu ke BAHAYA" } dd { "{bahaya_txt}" } }
                                div { dt { "AI Preempt" } dd { if sum.ai_preempt_used { "Aktif" } else { "Tidak" } } }
                                if let Some(hpt) = hp_txt {
                                    div { dt { "Paritas HILS" } dd { "{hpt}" } }
                                }
                                div { dt { "Sampel" } dd { "{sum.samples}" } }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn toggle_layer(mut state: Signal<AppState>, key: &str) {
    {
        let mut s = state.write();
        s.layers.toggle(key);
        s.preset = PresetKind::Custom;
    }
    persist_prefs(&state);
}

#[component]
fn LayerBtn(
    key_name: &'static str,
    label: &'static str,
    pressed: bool,
    disabled: bool,
    on_click: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "chip-btn layer-btn",
            "data-layer": "{key_name}",
            aria_pressed: if pressed { "true" } else { "false" },
            disabled: disabled,
            onclick: move |e| on_click.call(e),
            span { class: "chip-dot", aria_hidden: "true" }
            "{label}"
        }
    }
}
