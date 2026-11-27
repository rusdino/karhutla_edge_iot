use crate::chart_model::{
    build_chart_models, legend_specs, render_standalone_export_svg, ChartPalette,
};
use crate::direct_mqtt::rows_to_csv_bytes;
use crate::state::{
    format_clock, scenario_label, AppState, ConfirmAction, ConfirmModalState, PresetKind, ViewMode,
};
use crate::transport::{
    apply_dom_theme, clear_esp_logs, control_sim, download_verified_file, fetch_esp_log_status,
    fetch_state, save_connection_prefs, save_prefs, send_cmd, sleep_ms, sync_esp_logs,
    timestamp_suffix, trigger_browser_download, trigger_svg_to_png_export, BackendTarget,
    ConnectionPrefs,
};
use dioxus::prelude::*;
use karhutla_protocol::SimControlRequest;

pub fn persist_prefs(state: &Signal<AppState>) {
    let prefs = state.read().to_prefs();
    save_prefs(&prefs);
}

pub fn set_theme(mut state: Signal<AppState>, theme: crate::state::ThemePref) {
    let dom_val = {
        let mut s = state.write();
        s.theme = theme;
        match theme {
            crate::state::ThemePref::Light => Some("light"),
            crate::state::ThemePref::Dark => Some("dark"),
            crate::state::ThemePref::System => None,
        }
    };
    apply_dom_theme(dom_val);
    persist_prefs(&state);
}

pub fn apply_preset(mut state: Signal<AppState>, preset: PresetKind) {
    {
        let mut s = state.write();
        s.preset = preset;
        s.layers = crate::state::LayerVisibility::for_preset(preset);
    }
    persist_prefs(&state);
}

pub fn reload_snapshot(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    is_reconnect: bool,
    silent: bool,
) {
    let tgt = target.read().clone();
    spawn(async move {
        match fetch_state(&tgt).await {
            Ok(snap) => {
                let mut s = state.write();
                s.apply_snapshot(snap, is_reconnect);
                if is_reconnect && !silent {
                    s.push_toast(
                        "Koneksi live tersambung kembali & data disinkronkan.",
                        "ok",
                        None,
                        None,
                    );
                }
            }
            Err(_) => {
                let mut s = state.write();
                s.boot_error = true;
                if !silent {
                    s.push_toast("Gagal memuat status dari server.", "danger", None, None);
                }
            }
        }
    });
}

pub fn dispatch_sim(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    req: SimControlRequest,
    ok_toast: Option<String>,
) {
    let tgt = target.read().clone();
    state.write().sim_busy = true;
    spawn(async move {
        let res = control_sim(&tgt, req).await;
        let mut needs_reload = false;
        {
            let mut s = state.write();
            s.sim_busy = false;
            match res {
                Ok(resp) => {
                    let was_running = !resp.sim.running;
                    let summary = resp.sim.last_summary.clone();
                    s.apply_sim_status(resp.sim);
                    if was_running {
                        if let Some(sum) = summary {
                            s.last_run_summary = Some(sum);
                        }
                    }
                    if !resp.ok {
                        s.push_toast(
                            resp.reason
                                .unwrap_or_else(|| "Operasi simulasi ditolak.".to_string()),
                            "danger",
                            None,
                            None,
                        );
                    } else if let Some(msg) = ok_toast {
                        s.push_toast(msg, "ok", None, None);
                    }
                }
                Err(reason) => {
                    s.push_toast(
                        format!(
                            "Perintah simulasi gagal diproses ({reason}) — menyinkronkan ulang status."
                        ),
                        "danger",
                        None,
                        None,
                    );
                    needs_reload = true;
                }
            }
        }
        if needs_reload {
            reload_snapshot(state, target, true, true);
        }
    });
}

pub fn launch_sim(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    sim_tgt: String,
    override_scenario: Option<String>,
) {
    let (scenario, ai_preempt, fault_inject, speed, backend) = {
        let mut s = state.write();
        let scenario = override_scenario.unwrap_or_else(|| s.selected_scenario.clone());
        let bucket = if sim_tgt == "hils" { "esp32" } else { "sitl" };
        if bucket == "esp32" {
            s.esp_history.clear();
            s.esp_latest = None;
        } else {
            s.sitl_history.clear();
            s.sitl_latest = None;
        }
        s.clear_hover();
        s.last_run_summary = None;
        s.sim_target = sim_tgt.clone();
        s.selected_scenario = scenario.clone();
        s.last_active_source = bucket.to_string();
        if sim_tgt == "hils" && s.view_mode == ViewMode::Sitl {
            s.view_mode = ViewMode::Esp32;
        } else if sim_tgt == "sitl" && s.view_mode == ViewMode::Esp32 {
            s.view_mode = ViewMode::Sitl;
        }
        s.hils_timeout_info = None;
        s.sim_busy = true;
        (
            scenario,
            s.sim.ai_preempt,
            s.sim.fault_inject,
            s.selected_speed,
            target.read().clone(),
        )
    };
    persist_prefs(&state);

    spawn(async move {
        let req = SimControlRequest {
            action: Some("start".to_string()),
            target: Some(sim_tgt.clone()),
            scenario: Some(scenario.clone()),
            speed: Some(speed),
            ai_preempt: Some(ai_preempt),
            fault_inject: Some(fault_inject),
            ..Default::default()
        };
        let res = control_sim(&backend, req).await;
        let mut s = state.write();
        s.sim_busy = false;
        match res {
            Ok(resp) => {
                s.apply_sim_status(resp.sim);
                if resp.ok {
                    s.push_toast(
                        format!(
                            "Simulasi {} ({}) dimulai.",
                            sim_tgt.to_ascii_uppercase(),
                            scenario_label(&scenario)
                        ),
                        "ok",
                        None,
                        None,
                    );
                } else {
                    s.push_toast(
                        resp.reason
                            .unwrap_or_else(|| "Simulasi gagal dimulai.".to_string()),
                        "danger",
                        None,
                        None,
                    );
                }
            }
            Err(err) => {
                s.push_toast(
                    format!("Gagal memulai simulasi ({err})."),
                    "danger",
                    None,
                    None,
                );
            }
        }
    });
}

pub fn start_target_with_check(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    sim_tgt: String,
    override_scenario: Option<String>,
) {
    let ask_offline = {
        let s = state.read();
        if sim_tgt == "hils" && !s.esp.online {
            let port = s.sim.broker_port;
            let msg = if !s.sim.enable_mqtt {
                "Broker MQTT pada sesi server ini tidak aktif. Jalankan HILS hanya jika ingin menguji mode timeout; tetap mulai?".to_string()
            } else if !s.sim.broker_connected {
                format!("Broker MQTT :{port} belum terhubung. Pastikan Mosquitto aktif sebelum menguji ESP32; tetap mulai?")
            } else {
                format!("Node ESP32 saat ini offline di broker :{port}. Menjalankan HILS tanpa ESP32 akan mengalami timeout dan jeda otomatis setelah 3x gagal. Tetap mulai?")
            };
            Some(ConfirmModalState {
                title: "ESP32 Belum Terdeteksi".to_string(),
                desc: msg,
                ok_label: "Tetap Mulai HILS".to_string(),
                tone: "warn".to_string(),
                action: ConfirmAction::StartHilsOffline {
                    scenario: override_scenario
                        .clone()
                        .unwrap_or_else(|| s.selected_scenario.clone()),
                },
            })
        } else {
            None
        }
    };
    if let Some(modal) = ask_offline {
        state.write().confirm_modal = Some(modal);
    } else {
        launch_sim(state, target, sim_tgt, override_scenario);
    }
}

pub fn on_click_start_sim(mut state: Signal<AppState>, target: Signal<BackendTarget>) {
    let maybe_modal = {
        let s = state.read();
        let tgt = s.sim_target.clone();
        let hist_len = s.active_history().len();
        if s.sim.running && hist_len > 0 {
            Some(ConfirmModalState {
                title: "Mulai Ulang Simulasi?".to_string(),
                desc: format!(
                    "Simulasi {} sedang berjalan ({hist_len} sampel). Mengulang akan menghapus histori kurva berjalan dan memulai dari t=0s.",
                    tgt.to_ascii_uppercase()
                ),
                ok_label: "Ya, Mulai Ulang".to_string(),
                tone: "warn".to_string(),
                action: ConfirmAction::RestartSim {
                    target: tgt,
                    scenario: s.selected_scenario.clone(),
                },
            })
        } else {
            None
        }
    };
    if let Some(modal) = maybe_modal {
        state.write().confirm_modal = Some(modal);
    } else {
        let tgt = state.read().sim_target.clone();
        start_target_with_check(state, target, tgt, None);
    }
}

pub fn toggle_pause_resume(mut state: Signal<AppState>, target: Signal<BackendTarget>) {
    let (running, paused) = {
        let mut s = state.write();
        if !s.sim.running {
            return;
        }
        if s.sim.paused {
            s.hils_timeout_info = None;
        }
        (s.sim.running, s.sim.paused)
    };
    if !running {
        return;
    }
    let req = SimControlRequest {
        action: Some(if paused { "resume" } else { "pause" }.to_string()),
        ..Default::default()
    };
    let msg = if paused {
        "Simulasi dilanjutkan."
    } else {
        "Simulasi dijeda."
    };
    dispatch_sim(state, target, req, Some(msg.to_string()));
}

/// Direct non-modal stop (Run 12 P2 fix): stops simulation cleanly while keeping data + RunSummary.
pub fn direct_stop_sim(state: Signal<AppState>, target: Signal<BackendTarget>) {
    let req = SimControlRequest {
        action: Some("stop".to_string()),
        ..Default::default()
    };
    dispatch_sim(state, target, req, Some("Simulasi dihentikan.".to_string()));
}

pub fn send_actuator_cmd(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    cmd: String,
) {
    let (cmd_target, backend) = {
        let mut s = state.write();
        if s.cmd_busy {
            return;
        }
        s.cmd_busy = true;
        s.pending_esp_cmd = None;
        (s.cmd_target.clone(), target.read().clone())
    };

    spawn(async move {
        let res = send_cmd(&backend, cmd.clone(), Some(cmd_target)).await;
        let mut await_esp_confirm = false;
        {
            let mut s = state.write();
            s.cmd_busy = false;
            match res {
                Ok(resp) => {
                    if !resp.ok {
                        s.push_toast(
                            resp.reason
                                .unwrap_or_else(|| format!("Perintah {cmd} ditolak.")),
                            "danger",
                            None,
                            None,
                        );
                        return;
                    }
                    if cmd == "RESET" {
                        s.clear_hover();
                        s.last_run_summary = None;
                    }
                    let paths = resp.paths.unwrap_or_default();
                    if paths.esp32 {
                        let msg = if paths.sitl {
                            format!("Perintah {cmd} diterapkan ke SITL; dikirim ke ESP32 — menunggu konfirmasi telemetri…")
                        } else {
                            format!("Perintah {cmd} dikirim ke ESP32 — menunggu konfirmasi telemetri…")
                        };
                        s.push_toast(msg, "accent", None, None);
                        s.pending_esp_cmd = Some(cmd.clone());
                        await_esp_confirm = true;
                    } else {
                        s.push_toast(format!("✓ SITL menerapkan {cmd}."), "ok", None, None);
                    }
                }
                Err(err) => {
                    s.push_toast(format!("Gagal mengirim {cmd}: {err}"), "danger", None, None);
                }
            }
        }
        if await_esp_confirm {
            sleep_ms(3200).await;
            let mut s = state.write();
            if s.pending_esp_cmd.as_deref() == Some(cmd.as_str()) {
                s.pending_esp_cmd = None;
                s.push_toast(
                    format!("Belum ada konfirmasi {cmd} dari ESP32 (>3 dtk)."),
                    "warn",
                    Some("Kirim Ulang".to_string()),
                    Some(cmd),
                );
            }
        }
    });
}

pub fn request_actuator_cmd(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    cmd: String,
) {
    if cmd == "FORCE_OFF" {
        let maybe_desc = state.read().multi_target_force_off_desc();
        if let Some(desc) = maybe_desc {
            state.write().confirm_modal = Some(ConfirmModalState {
                title: "Matikan Kipas Saat Kondisi Kritis?".to_string(),
                desc,
                ok_label: "Tetap Matikan (OFF 0%)".to_string(),
                tone: "danger".to_string(),
                action: ConfirmAction::ForceOff,
            });
            return;
        }
    }
    if cmd == "RESET" {
        state.write().confirm_modal = Some(ConfirmModalState {
            title: "Reset Sistem & Filter?".to_string(),
            desc: "Aksi ini mengatur ulang estimator Kalman, state FSM menjadi NORMAL, mode AUTO, serta menghapus histori kurva yang sedang berjalan.".to_string(),
            ok_label: "Ya, Reset Sistem".to_string(),
            tone: "danger".to_string(),
            action: ConfirmAction::Reset,
        });
        return;
    }
    send_actuator_cmd(state, target, cmd);
}

pub fn switch_fallback_to_sitl(mut state: Signal<AppState>, target: Signal<BackendTarget>) {
    let was_running_hils = {
        let mut s = state.write();
        s.hils_timeout_info = None;
        s.sim.running && s.sim.target == "hils"
    };
    if was_running_hils {
        let req = SimControlRequest {
            action: Some("stop".to_string()),
            target: Some("sitl".to_string()),
            ..Default::default()
        };
        dispatch_sim(
            state,
            target,
            req,
            Some("Beralih ke mode & sumber grafik SITL.".to_string()),
        );
    } else {
        state
            .write()
            .push_toast("Beralih ke mode & sumber grafik SITL.", "ok", None, None);
    }
    {
        let mut s = state.write();
        s.sim_target = "sitl".to_string();
        s.view_mode = ViewMode::Sitl;
        s.last_active_source = "sitl".to_string();
    }
    persist_prefs(&state);
}

pub fn trigger_verified_export(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    path: String,
    fallback_name: &'static str,
    mime: &'static str,
    ok_msg: String,
) {
    let backend = target.read().clone();
    if let BackendTarget::DirectMqtt { host, port } = &backend {
        if path.starts_with("/api/export-csv") {
            let s = state.read();
            let rows = if path.contains("source=sitl") {
                &s.sitl_history
            } else if path.contains("source=esp32") {
                &s.esp_history
            } else {
                s.active_history()
            };
            if rows.is_empty() {
                drop(s);
                state.write().push_toast(
                    "Belum ada data telemetri untuk diekspor.",
                    "warn",
                    None,
                    None,
                );
                return;
            }
            let bytes = rows_to_csv_bytes(rows);
            let fname = format!("karhutla_direct_{}.csv", timestamp_suffix());
            drop(s);
            trigger_browser_download(&fname, mime, &bytes);
            state.write().push_toast(ok_msg, "ok", None, None);
            return;
        } else if path == "/api/export-log" {
            let s = state.read();
            if s.mqtt_logs.is_empty() {
                drop(s);
                state.write().push_toast(
                    "Belum ada log trafik MQTT untuk diekspor.",
                    "warn",
                    None,
                    None,
                );
                return;
            }
            let bytes = s.mqtt_logs.join("\n").into_bytes();
            let fname = format!("karhutla_mqtt_direct_{}.log", timestamp_suffix());
            drop(s);
            trigger_browser_download(&fname, mime, &bytes);
            state.write().push_toast(ok_msg, "ok", None, None);
            return;
        } else if path == "/api/mobile-config" {
            let cfg = karhutla_protocol::build_mobile_dashboard_config(host, *port, host);
            let bytes = serde_json::to_vec_pretty(&cfg).unwrap_or_default();
            trigger_browser_download("karhutla_iot_mqtt_panel.json", mime, &bytes);
            state.write().push_toast(ok_msg, "ok", None, None);
            return;
        }
    }

    spawn(async move {
        match download_verified_file(&backend, &path, fallback_name).await {
            Ok((fname, bytes)) => {
                trigger_browser_download(&fname, mime, &bytes);
                state.write().push_toast(ok_msg, "ok", None, None);
            }
            Err(err) => {
                state
                    .write()
                    .push_toast(format!("Ekspor gagal: {err}"), "danger", None, None);
            }
        }
    });
}

pub fn export_chart_bundle(mut state: Signal<AppState>, as_png: bool) {
    let export_data = {
        let s = state.read();
        let rows = s.active_history();
        if rows.is_empty() {
            None
        } else {
            let dark = s.is_dark();
            let pal = ChartPalette::for_theme(dark);
            let allow_pm_true = s.can_show_pm_true();
            let (mw, mh) = s.main_svg_size;
            let (sw, sh) = s.sub_svg_size;
            let (main, sub) = build_chart_models(
                rows,
                &s.layers,
                allow_pm_true,
                s.window_sec,
                s.zoom_y,
                dark,
                mw,
                mh,
                sw,
                sh,
                None,
                s.active_source(),
                s.esp.online,
            );
            let latest_fault = rows
                .last()
                .map(|r| r.state == "FAILSAFE" || r.sensor_fault)
                .unwrap_or(false);
            let legend = legend_specs(&s.layers, allow_pm_true, latest_fault, &pal);
            let src_txt = if s.active_source() == "esp32" {
                "ESP32 (HILS/Perangkat)"
            } else {
                "SITL (Rust)"
            };
            let title = format!("Karhutla Edge-IoT — {src_txt}");
            let dur = rows
                .last()
                .map(|r| format_clock(r.t))
                .unwrap_or_else(|| "0s".into());
            let mut sub_txt = format!("Preset: {} • Durasi: {dur}", s.preset.label());
            let hp = &s.sim.hils_parity;
            if hp.samples > 0 && (s.active_source() == "esp32" || s.sim.target == "hils") {
                sub_txt.push_str(&format!(
                    " • Paritas HILS: maks ΔFI {:.2} ({}/{})",
                    hp.max_delta_fi, hp.state_matches, hp.samples
                ));
            }
            let (svg, w, h) =
                render_standalone_export_svg(&main, &sub, &legend, &pal, &title, &sub_txt);
            Some((s.active_source().to_string(), svg, w, h))
        }
    };

    let Some((src, svg, w, h)) = export_data else {
        state
            .write()
            .push_toast("Belum ada data grafik untuk diekspor.", "warn", None, None);
        return;
    };

    let ts = timestamp_suffix();
    if as_png {
        trigger_svg_to_png_export(&svg, &format!("karhutla_{src}_{ts}.png"), w, h);
        state
            .write()
            .push_toast("Grafik PNG berlegenda lengkap berhasil diunduh.", "ok", None, None);
    } else {
        trigger_browser_download(
            &format!("karhutla_{src}_{ts}.svg"),
            "image/svg+xml;charset=utf-8",
            svg.as_bytes(),
        );
        state
            .write()
            .push_toast("Grafik vektor SVG berhasil diunduh.", "ok", None, None);
    }
}

pub fn sync_and_download_esp_log(mut state: Signal<AppState>, target: Signal<BackendTarget>) {
    let backend = target.read().clone();
    state.write().esp_sync_busy = true;
    spawn(async move {
        let sync_res = sync_esp_logs(&backend).await;
        match sync_res {
            Ok(d) => {
                state.write().esp_log = d.status.clone();
                if !d.ok {
                    state.write().esp_sync_busy = false;
                    state.write().push_toast(
                        d.error
                            .unwrap_or_else(|| "Gagal menyinkronkan log ESP32.".to_string()),
                        "danger",
                        None,
                        None,
                    );
                    return;
                }
                let fallback = d
                    .status
                    .csv_file
                    .as_deref()
                    .unwrap_or("karhutla_esp32_backup.csv");
                match download_verified_file(&backend, "/api/esp-log/download", fallback).await {
                    Ok((fname, bytes)) => {
                        trigger_browser_download(&fname, "text/csv; charset=utf-8", &bytes);
                        state.write().push_toast(
                            "Log ESP32 berhasil disinkronkan dan diunduh.",
                            "ok",
                            None,
                            None,
                        );
                    }
                    Err(e) => {
                        state.write().push_toast(
                            format!("Sinkronisasi berhasil, tetapi gagal mengunduh CSV ({e})."),
                            "danger",
                            None,
                            None,
                        );
                    }
                }
            }
            Err(e) => {
                state.write().push_toast(
                    format!("Gagal menghubungi server saat sinkronisasi log ESP32 ({e})."),
                    "danger",
                    None,
                    None,
                );
            }
        }
        if let Ok(st) = fetch_esp_log_status(&backend).await {
            state.write().esp_log = st;
        }
        state.write().esp_sync_busy = false;
    });
}

pub fn execute_esp_clear(mut state: Signal<AppState>, target: Signal<BackendTarget>) {
    let backend = target.read().clone();
    state.write().esp_clear_busy = true;
    spawn(async move {
        match clear_esp_logs(&backend).await {
            Ok(d) => {
                let mut s = state.write();
                s.esp_log = d.status;
                if d.ok {
                    s.push_toast(
                        "Memori backup ESP32 berhasil dikosongkan.",
                        "ok",
                        None,
                        None,
                    );
                } else {
                    s.push_toast(
                        d.error
                            .unwrap_or_else(|| "Gagal mengosongkan memori ESP32.".to_string()),
                        "danger",
                        None,
                        None,
                    );
                }
            }
            Err(e) => {
                state.write().push_toast(
                    format!("Gagal mengosongkan memori ESP32 ({e})."),
                    "danger",
                    None,
                    None,
                );
            }
        }
        if let Ok(st) = fetch_esp_log_status(&backend).await {
            state.write().esp_log = st;
        }
        state.write().esp_clear_busy = false;
    });
}

pub fn execute_confirmed_action(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    action: ConfirmAction,
) {
    state.write().confirm_modal = None;
    match action {
        ConfirmAction::ForceOff => send_actuator_cmd(state, target, "FORCE_OFF".to_string()),
        ConfirmAction::Reset => send_actuator_cmd(state, target, "RESET".to_string()),
        ConfirmAction::RestartSim {
            target: tgt,
            scenario,
        } => {
            start_target_with_check(state, target, tgt, Some(scenario));
        }
        ConfirmAction::SwitchTarget { target: tgt } => {
            start_target_with_check(state, target, tgt, None);
        }
        ConfirmAction::SwitchScenario {
            target: tgt,
            scenario,
        } => {
            launch_sim(state, target, tgt, Some(scenario));
        }
        ConfirmAction::StartHilsOffline { scenario } => {
            launch_sim(state, target, "hils".to_string(), Some(scenario));
        }
        ConfirmAction::ClearChart { source } => {
            let backend = target.read().clone();
            spawn(async move {
                let req = SimControlRequest {
                    action: Some("clear".to_string()),
                    source: Some(source.clone()),
                    ..Default::default()
                };
                if let Ok(resp) = control_sim(&backend, req).await {
                    let mut s = state.write();
                    s.apply_sim_status(resp.sim);
                    if resp.ok {
                        if source == "esp32" {
                            s.esp_history.clear();
                            s.esp_latest = None;
                        } else {
                            s.sitl_history.clear();
                            s.sitl_latest = None;
                        }
                        s.clear_hover();
                        s.last_run_summary = None;
                        s.push_toast(
                            format!("Grafik {} dibersihkan.", source.to_ascii_uppercase()),
                            "ok",
                            None,
                            None,
                        );
                    }
                }
            });
        }
        ConfirmAction::ClearEspLog => {
            execute_esp_clear(state, target);
        }
    }
}

pub fn switch_backend_target(
    mut state: Signal<AppState>,
    mut target: Signal<BackendTarget>,
    mut live_task: Signal<Option<dioxus::dioxus_core::Task>>,
    new_tgt: BackendTarget,
) {
    if let Some(prev) = live_task.write().take() {
        prev.cancel();
    }
    target.set(new_tgt.clone());
    let toast_msg = {
        let mut s = state.write();
        let port = s.mqtt_port_input.trim().parse::<u16>().unwrap_or(6669);
        let prefs = ConnectionPrefs {
            mode: new_tgt.mode_key().to_string(),
            remote_url: s.remote_url_input.trim().to_string(),
            mqtt_host: s.mqtt_host_input.trim().to_string(),
            mqtt_port: port,
        };
        save_connection_prefs(&prefs);

        s.conn_mode = prefs.mode.clone();
        s.conn_draft_mode = prefs.mode;
        s.conn_modal_open = false;
        s.live_connected = false;
        s.had_disconnect = false;
        s.hils_timeout_info = None;

        match &new_tgt {
            BackendTarget::Embedded => {
                s.using_remote_url = None;
                s.direct_mqtt_endpoint = None;
                "Mode Host Lokal Embedded aktif.".to_string()
            }
            BackendTarget::Remote(u) => {
                let clean = u.trim().to_string();
                s.using_remote_url = if clean.is_empty() {
                    None
                } else {
                    Some(clean.clone())
                };
                s.direct_mqtt_endpoint = None;
                if clean.is_empty() {
                    "Tersambung ke Server LAN asal.".to_string()
                } else {
                    format!("Tersambung ke Server LAN ({clean}).")
                }
            }
            BackendTarget::DirectMqtt { host, port } => {
                let ep = format!("{host}:{port}");
                s.using_remote_url = None;
                s.direct_mqtt_endpoint = Some(ep.clone());
                s.last_run_summary = None;
                if s.cmd_target == "sitl" {
                    s.cmd_target = "auto".to_string();
                }
                format!("Menyambungkan Direct MQTT tcp://{ep}…")
            }
        }
    };
    persist_prefs(&state);
    state.write().push_toast(toast_msg, "accent", None, None);
    reload_snapshot(state, target, false, true);
    let task = spawn_live_subscriber(state, target, new_tgt);
    live_task.set(Some(task));
}

pub fn spawn_live_subscriber(
    mut state: Signal<AppState>,
    target: Signal<BackendTarget>,
    tgt: BackendTarget,
) -> dioxus::dioxus_core::Task {
    spawn(async move {
        crate::transport::run_live_loop(tgt, move |msg| match msg {
            crate::transport::LiveMessage::Connected { is_reconnect } => {
                {
                    let mut s = state.write();
                    s.live_connected = true;
                    s.had_disconnect = false;
                }
                if is_reconnect {
                    reload_snapshot(state, target, true, false);
                }
            }
            crate::transport::LiveMessage::Disconnected => {
                let mut s = state.write();
                s.live_connected = false;
                s.had_disconnect = true;
            }
            crate::transport::LiveMessage::Event(ev) => {
                state.write().apply_event(ev);
            }
        })
        .await;
    })
}
