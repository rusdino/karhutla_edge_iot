#![cfg_attr(not(test), no_std)]

use libm::{fabsf, fmaxf, fminf, powf};

#[cfg(test)]
mod golden_vectors;

pub mod cfg {
    // ---- Jaringan ----
    pub const WIFI_SSID: &str = "Hotspot LSKK";
    pub const WIFI_PASS: &str = "stei.lskk";
    pub const MQTT_HOST: &str = "10.8.43.78";
    pub const MQTT_PORT: u16 = 6669;
    pub const MQTT_USER: &str = "";
    pub const MQTT_PASS: &str = "";

    pub const T_TELEM: &str = "karhutla/telemetry";
    pub const T_TELEM_SCALAR: &str = "karhutla/t";
    pub const T_CMD: &str = "karhutla/cmd";
    pub const T_HILS_IN: &str = "karhutla/hils/sensor";
    pub const T_STATUS: &str = "karhutla/status";
    pub const T_HEARTBEAT: &str = "karhutla/hb";

    // ---- Waktu ----
    pub const TS_S: f32 = 1.0;
    pub const HILS_WDT_MS: u32 = 5000;

    // ---- Sharp GP2Y1010AU0F ----
    pub const DUST_V0: f32 = 0.60;
    pub const DUST_SLOPE: f32 = 200.0;
    pub const V_VALID_MIN: f32 = 0.10;
    pub const V_VALID_MAX: f32 = 4.20;
    pub const V0_ADAPT_ALPHA: f32 = 0.002;
    pub const V0_ADAPT_MAXDEV: f32 = 0.15;
    pub const V0_ADAPT_PM_MAX: f32 = 15.0;
    pub const V0_ADAPT_RATE_MAX: f32 = 0.3;
    pub const V0_ADAPT_G_MAX: f32 = 0.10;

    // ---- MQ-135 ----
    pub const MQ_VCC: f32 = 5.0;
    pub const MQ_RL_KOHM: f32 = 10.0;
    pub const MQ_RS_CLEAN_KOHM: f32 = 0.0;
    pub const MQ_CALIB_N: usize = 30;
    pub const MQ_PARA: f32 = 116.6020682;
    pub const MQ_PARB: f32 = 2.769034857;
    pub const MQ_ATM_PPM: f32 = 400.0;
    pub const GAS_FULL_PPM: f32 = 800.0;
    pub const GAS_EMA_ALPHA: f32 = 0.15;

    // ---- Kalman 2-state ----
    pub const KF_Q: f32 = 0.25;
    pub const KF_R: f32 = 36.0;

    // ---- Fusi & State Machine ----
    pub const W_GAS: f32 = 0.30;
    pub const GAS_FULL_PM: f32 = 100.0;
    pub const LEAD_S: f32 = 15.0;
    pub const T_W_IN: f32 = 55.0;
    pub const T_W_OUT: f32 = 42.0;
    pub const T_B_IN: f32 = 150.0;
    pub const T_B_OUT: f32 = 125.0;
    pub const N_ENTER: u8 = 3;
    pub const N_EXIT: u8 = 10;
    pub const IMMEDIATE_MARGIN: f32 = 25.0;

    // ---- Aktuator ----
    pub const DUTY_WASPADA_MIN: f32 = 40.0;
    pub const DUTY_WASPADA_MAX: f32 = 90.0;
    pub const DUTY_FAILSAFE: f32 = 70.0;
    pub const DUTY_SLEW: f32 = 10.0;

    // ---- Failsafe ----
    pub const FS_INVALID_N: u8 = 5;
    pub const FS_RECOVER_N: u8 = 5;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AlarmState {
    Normal = 0,
    Waspada = 1,
    Bahaya = 2,
    Failsafe = 3,
}

impl AlarmState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlarmState::Normal => "NORMAL",
            AlarmState::Waspada => "WASPADA",
            AlarmState::Bahaya => "BAHAYA",
            AlarmState::Failsafe => "FAILSAFE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpMode {
    Auto = 0,
    ForceMax = 1,
    ForceOff = 2,
}

impl OpMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            OpMode::Auto => "AUTO",
            OpMode::ForceMax => "FORCE_MAX",
            OpMode::ForceOff => "FORCE_OFF",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CmdMsg {
    Auto = 0,
    ForceMax = 1,
    ForceOff = 2,
    Reset = 3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Telemetry {
    pub seq: u32,
    pub v_dust: f32,
    pub v_mq: f32,
    pub pm_raw: f32,
    pub pm: f32,
    pub dcdt: f32,
    pub gas_ppm: f32,
    pub g: f32,
    pub fi: f32,
    pub v0: f32,
    pub duty: f32,
    pub state: AlarmState,
    pub mode: OpMode,
    pub alarm: u8,
}

#[inline]
fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

#[inline]
fn r0_from_clean(rs_clean: f32) -> f32 {
    rs_clean * powf(cfg::MQ_ATM_PPM / cfg::MQ_PARA, 1.0 / cfg::MQ_PARB)
}

pub struct Median3 {
    buf: Option<[f32; 3]>,
}

impl Median3 {
    pub fn new() -> Self {
        Self { buf: None }
    }

    pub fn reset(&mut self) {
        self.buf = None;
    }

    pub fn push(&mut self, x: f32) -> f32 {
        let buf = match self.buf {
            Some(b) => [b[1], b[2], x],
            None => [x, x, x],
        };
        self.buf = Some(buf);
        let [a, b, c] = buf;
        a + b + c - fminf(a, fminf(b, c)) - fmaxf(a, fmaxf(b, c))
    }
}

impl Default for Median3 {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Kalman2 {
    pub q: f32,
    pub r: f32,
    pub x0: f32,
    pub x1: f32,
    p00: f32,
    p01: f32,
    p10: f32,
    p11: f32,
    init: bool,
}

impl Kalman2 {
    pub fn new(q: f32, r: f32) -> Self {
        Self {
            q,
            r,
            x0: 0.0,
            x1: 0.0,
            p00: 0.0,
            p01: 0.0,
            p10: 0.0,
            p11: 0.0,
            init: false,
        }
    }

    pub fn reset(&mut self) {
        self.init = false;
    }

    pub fn update(&mut self, z: f32, dt: f32) {
        let q = self.q;
        let r = self.r;
        if !self.init {
            self.x0 = z;
            self.x1 = 0.0;
            self.p00 = r;
            self.p01 = 0.0;
            self.p10 = 0.0;
            self.p11 = 10.0;
            self.init = true;
            return;
        }
        let dt2 = dt * dt;
        let dt3 = dt2 * dt;
        let px0 = self.x0 + dt * self.x1;
        let px1 = self.x1;
        let cap_p00 = self.p00 + dt * (self.p10 + self.p01) + dt2 * self.p11 + q * dt3 / 3.0;
        let cap_p01 = self.p01 + dt * self.p11 + q * dt2 / 2.0;
        let cap_p10 = self.p10 + dt * self.p11 + q * dt2 / 2.0;
        let cap_p11 = self.p11 + q * dt;
        let s = cap_p00 + r;
        let k0 = cap_p00 / s;
        let k1 = cap_p10 / s;
        let y = z - px0;
        self.x0 = px0 + k0 * y;
        self.x1 = px1 + k1 * y;
        let n00 = (1.0 - k0) * cap_p00;
        let n01 = (1.0 - k0) * cap_p01;
        let n10 = cap_p10 - k1 * cap_p00;
        let n11 = cap_p11 - k1 * cap_p01;
        self.p00 = n00;
        self.p11 = n11;
        let p_sym = 0.5 * (n01 + n10);
        self.p01 = p_sym;
        self.p10 = p_sym;
        if self.x0 < 0.0 {
            self.x0 = 0.0;
        }
    }
}

pub struct HysteresisFSM {
    pub state: AlarmState,
    cnt_up: u8,
    cnt_dn: u8,
}

impl HysteresisFSM {
    pub fn new() -> Self {
        Self {
            state: AlarmState::Normal,
            cnt_up: 0,
            cnt_dn: 0,
        }
    }

    pub fn reset(&mut self) {
        self.state = AlarmState::Normal;
        self.cnt_up = 0;
        self.cnt_dn = 0;
    }

    pub fn update(&mut self, fi: f32) -> AlarmState {
        let s = self.state;
        let s_u8 = s as u8;
        let up_th = if s == AlarmState::Normal {
            cfg::T_W_IN
        } else {
            cfg::T_B_IN
        };
        let dn_th = if s == AlarmState::Waspada {
            cfg::T_W_OUT
        } else {
            cfg::T_B_OUT
        };
        let up = s_u8 < (AlarmState::Bahaya as u8) && fi >= up_th;
        let dn = s_u8 > (AlarmState::Normal as u8) && fi <= dn_th;
        self.cnt_up = if up { self.cnt_up.saturating_add(1) } else { 0 };
        self.cnt_dn = if dn { self.cnt_dn.saturating_add(1) } else { 0 };

        let mut new_state = s;
        if s == AlarmState::Normal && fi >= cfg::T_B_IN {
            new_state = AlarmState::Bahaya;
        } else if up && (self.cnt_up >= cfg::N_ENTER || fi >= up_th + cfg::IMMEDIATE_MARGIN) {
            new_state = match s {
                AlarmState::Normal => AlarmState::Waspada,
                AlarmState::Waspada => AlarmState::Bahaya,
                _ => s,
            };
        } else if dn && self.cnt_dn >= cfg::N_EXIT {
            new_state = match s {
                AlarmState::Bahaya => AlarmState::Waspada,
                AlarmState::Waspada => AlarmState::Normal,
                _ => s,
            };
        }

        if new_state != s {
            self.state = new_state;
            self.cnt_up = 0;
            self.cnt_dn = 0;
        }
        self.state
    }
}

impl Default for HysteresisFSM {
    fn default() -> Self {
        Self::new()
    }
}

pub struct FirmwareCore {
    mode: OpMode,
    seq: u32,
    med_d: Median3,
    med_m: Median3,
    kf: Kalman2,
    fsm: HysteresisFSM,
    v0: f32,
    last_vd: f32,
    last_vm: f32,
    pm_raw: f32,
    ppm: f32,
    g: f32,
    fi: f32,
    duty: f32,
    failsafe: bool,
    bad: u8,
    good: u8,
    calib_sum: f32,
    calib_count: usize,
    r0: Option<f32>,
}

impl FirmwareCore {
    pub fn new() -> Self {
        let r0 = if cfg::MQ_RS_CLEAN_KOHM > 0.0 {
            Some(r0_from_clean(cfg::MQ_RS_CLEAN_KOHM))
        } else {
            None
        };
        Self {
            mode: OpMode::Auto,
            seq: 0,
            med_d: Median3::new(),
            med_m: Median3::new(),
            kf: Kalman2::new(cfg::KF_Q, cfg::KF_R),
            fsm: HysteresisFSM::new(),
            v0: cfg::DUST_V0,
            last_vd: cfg::DUST_V0,
            last_vm: 1.0,
            pm_raw: 0.0,
            ppm: cfg::MQ_ATM_PPM,
            g: 0.0,
            fi: 0.0,
            duty: 0.0,
            failsafe: false,
            bad: 0,
            good: 0,
            calib_sum: 0.0,
            calib_count: 0,
            r0,
        }
    }

    pub fn state(&self) -> AlarmState {
        if self.failsafe {
            AlarmState::Failsafe
        } else {
            self.fsm.state
        }
    }

    pub fn mode(&self) -> OpMode {
        self.mode
    }

    fn current_telemetry(&self, v_dust: Option<f32>, v_mq: Option<f32>) -> Telemetry {
        let state = self.state();
        Telemetry {
            seq: self.seq,
            v_dust: v_dust.unwrap_or(self.last_vd),
            v_mq: v_mq.unwrap_or(self.last_vm),
            pm_raw: self.pm_raw,
            pm: self.kf.x0,
            dcdt: self.kf.x1,
            gas_ppm: self.ppm,
            g: self.g,
            fi: self.fi,
            v0: self.v0,
            duty: self.duty,
            state,
            mode: self.mode,
            alarm: state as u8,
        }
    }

    pub fn reset(&mut self) -> Telemetry {
        self.med_d.reset();
        self.med_m.reset();
        self.kf.reset();
        self.kf.x0 = 0.0;
        self.kf.x1 = 0.0;
        self.fsm.reset();
        self.v0 = cfg::DUST_V0;
        self.pm_raw = 0.0;
        self.ppm = cfg::MQ_ATM_PPM;
        self.g = 0.0;
        self.fi = 0.0;
        self.duty = 0.0;
        self.failsafe = false;
        self.bad = 0;
        self.good = 0;
        self.calib_sum = 0.0;
        self.calib_count = 0;
        self.r0 = if cfg::MQ_RS_CLEAN_KOHM > 0.0 {
            Some(r0_from_clean(cfg::MQ_RS_CLEAN_KOHM))
        } else {
            None
        };
        self.mode = OpMode::Auto;
        self.current_telemetry(None, None)
    }

    pub fn apply_command(&mut self, cmd: CmdMsg) -> Telemetry {
        match cmd {
            CmdMsg::Reset => self.reset(),
            CmdMsg::Auto => {
                self.mode = OpMode::Auto;
                self.duty = self.target_duty(self.state());
                self.current_telemetry(None, None)
            }
            CmdMsg::ForceMax => {
                self.mode = OpMode::ForceMax;
                self.duty = self.target_duty(self.state());
                self.current_telemetry(None, None)
            }
            CmdMsg::ForceOff => {
                self.mode = OpMode::ForceOff;
                self.duty = self.target_duty(self.state());
                self.current_telemetry(None, None)
            }
        }
    }

    pub fn force_failsafe(&mut self) -> Telemetry {
        self.bad = cfg::FS_INVALID_N;
        self.good = 0;
        self.enter_failsafe();
        self.current_telemetry(None, None)
    }

    fn enter_failsafe(&mut self) {
        self.failsafe = true;
        self.kf.reset();
        self.med_d.reset();
        self.med_m.reset();
        self.fsm.reset();
    }

    fn gas_update(&mut self, vm: f32) {
        let rs = cfg::MQ_RL_KOHM * (cfg::MQ_VCC - vm) / vm;
        let mut g_t = 0.0_f32;
        match self.r0 {
            None => {
                self.ppm = cfg::MQ_ATM_PPM;
                self.calib_sum += rs;
                self.calib_count += 1;
                if self.calib_count >= cfg::MQ_CALIB_N {
                    self.r0 = Some(r0_from_clean(self.calib_sum / (self.calib_count as f32)));
                }
            }
            Some(r0) => {
                self.ppm = cfg::MQ_PARA * powf(rs / r0, -cfg::MQ_PARB);
                g_t = clamp((self.ppm - cfg::MQ_ATM_PPM) / cfg::GAS_FULL_PPM, 0.0, 1.0);
            }
        }
        self.g += cfg::GAS_EMA_ALPHA * (g_t - self.g);
    }

    fn target_duty(&self, state: AlarmState) -> f32 {
        match self.mode {
            OpMode::ForceMax => 100.0,
            OpMode::ForceOff => 0.0,
            OpMode::Auto => match state {
                AlarmState::Waspada => {
                    let x = clamp((self.fi - cfg::T_W_IN) / (cfg::T_B_IN - cfg::T_W_IN), 0.0, 1.0);
                    cfg::DUTY_WASPADA_MIN + (cfg::DUTY_WASPADA_MAX - cfg::DUTY_WASPADA_MIN) * x
                }
                AlarmState::Bahaya => 100.0,
                AlarmState::Failsafe => cfg::DUTY_FAILSAFE,
                AlarmState::Normal => 0.0,
            },
        }
    }

    pub fn step(&mut self, v_dust: f32, v_mq: f32, seq: u32, dt: f32) -> Telemetry {
        self.seq = seq;
        self.last_vd = v_dust;
        self.last_vm = v_mq;
        let dust_ok = (cfg::V_VALID_MIN..=cfg::V_VALID_MAX).contains(&v_dust);
        let mq_ok = (cfg::V_VALID_MIN..=cfg::V_VALID_MAX).contains(&v_mq);

        if dust_ok {
            self.good = self.good.saturating_add(1);
            self.bad = 0;
        } else {
            self.bad = self.bad.saturating_add(1);
            self.good = 0;
            if self.bad >= cfg::FS_INVALID_N && !self.failsafe {
                self.enter_failsafe();
            }
        }
        if self.failsafe && self.good >= cfg::FS_RECOVER_N {
            self.failsafe = false;
        }

        let mut vd = 0.0_f32;
        if dust_ok {
            vd = self.med_d.push(v_dust);
            self.pm_raw = fmaxf(0.0, (vd - self.v0) * cfg::DUST_SLOPE);
            self.kf.update(self.pm_raw, dt);
        }
        let pm = self.kf.x0;
        let rate = self.kf.x1;

        if mq_ok {
            let vm_med = self.med_m.push(v_mq);
            self.gas_update(vm_med);
        }

        let lead = fmaxf(0.0, rate) * cfg::LEAD_S;
        self.fi = pm + cfg::W_GAS * cfg::GAS_FULL_PM * self.g + lead;

        if dust_ok
            && !self.failsafe
            && self.fsm.state == AlarmState::Normal
            && pm < cfg::V0_ADAPT_PM_MAX
            && fabsf(rate) < cfg::V0_ADAPT_RATE_MAX
            && self.g < cfg::V0_ADAPT_G_MAX
        {
            self.v0 += cfg::V0_ADAPT_ALPHA * (vd - self.v0);
            self.v0 = clamp(
                self.v0,
                cfg::DUST_V0 - cfg::V0_ADAPT_MAXDEV,
                cfg::DUST_V0 + cfg::V0_ADAPT_MAXDEV,
            );
        }

        if !self.failsafe && dust_ok {
            self.fsm.update(self.fi);
        }
        let state = self.state();

        let target = self.target_duty(state);
        if self.mode != OpMode::Auto || matches!(state, AlarmState::Bahaya | AlarmState::Failsafe) {
            self.duty = target;
        } else {
            self.duty += clamp(target - self.duty, -cfg::DUTY_SLEW, cfg::DUTY_SLEW);
        }

        self.current_telemetry(Some(v_dust), Some(v_mq))
    }
}

impl Default for FirmwareCore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_vectors::*;

    #[test]
    fn test_median3_and_kalman2_parity() {
        let mut m = Median3::new();
        let mut kf = Kalman2::new(cfg::KF_Q, cfg::KF_R);
        let samples = [0.60_f32, 0.62, 1.80, 0.65, 0.70, 0.80, 0.90, 1.00, 1.10, 1.20];
        let expected: [(f32, f32, f32); 10] = [
            (0.600000, 0.000000, 0.000000),
            (0.600000, 0.000000, 0.000000),
            (0.620000, 2.058692, 0.731587),
            (0.650000, 6.518440, 2.035146),
            (0.700000, 14.300518, 3.810962),
            (0.700000, 19.006342, 4.053928),
            (0.800000, 30.562066, 5.880317),
            (0.900000, 46.209666, 8.069652),
            (1.000000, 64.340124, 10.200447),
            (1.100000, 84.032872, 12.142811),
        ];
        for (i, &s) in samples.iter().enumerate() {
            let mv = m.push(s);
            let pm_raw = fmaxf(0.0, (mv - cfg::DUST_V0) * cfg::DUST_SLOPE);
            kf.update(pm_raw, cfg::TS_S);
            let (exp_m, exp_x0, exp_x1) = expected[i];
            assert!((mv - exp_m).abs() < 1e-4, "med3 mismatch at {i}: {mv} vs {exp_m}");
            assert!((kf.x0 - exp_x0).abs() < 1e-4, "kf.x0 mismatch at {i}: {} vs {exp_x0}", kf.x0);
            assert!((kf.x1 - exp_x1).abs() < 1e-4, "kf.x1 mismatch at {i}: {} vs {exp_x1}", kf.x1);
        }
    }

    fn assert_scenario_parity(name: &str, inputs: &[(f32, f32)], checkpoints: &[Checkpoint]) {
        let mut fw = FirmwareCore::new();
        let mut cp_idx = 0;
        for (k, &(vd, vm)) in inputs.iter().enumerate() {
            let tel = fw.step(vd, vm, (k + 1) as u32, cfg::TS_S);
            if cp_idx < checkpoints.len() && checkpoints[cp_idx].step == k {
                let cp = &checkpoints[cp_idx];
                assert_eq!(tel.alarm, cp.alarm, "{name} step {k} alarm");
                assert!((tel.pm_raw - cp.pm_raw).abs() < 0.5, "{name} step {k} pm_raw: {} vs {}", tel.pm_raw, cp.pm_raw);
                assert!((tel.pm - cp.pm).abs() < 0.5, "{name} step {k} pm: {} vs {}", tel.pm, cp.pm);
                assert!((tel.dcdt - cp.dcdt).abs() < 0.1, "{name} step {k} dcdt: {} vs {}", tel.dcdt, cp.dcdt);
                assert!((tel.gas_ppm - cp.gas_ppm).abs() < 0.5, "{name} step {k} gas_ppm: {} vs {}", tel.gas_ppm, cp.gas_ppm);
                assert!((tel.g - cp.g).abs() < 1e-3, "{name} step {k} g: {} vs {}", tel.g, cp.g);
                assert!((tel.fi - cp.fi).abs() < 0.5, "{name} step {k} fi: {} vs {}", tel.fi, cp.fi);
                assert!((tel.v0 - cp.v0).abs() < 1e-3, "{name} step {k} v0: {} vs {}", tel.v0, cp.v0);
                assert!((tel.duty - cp.duty).abs() < 0.5, "{name} step {k} duty: {} vs {}", tel.duty, cp.duty);
                cp_idx += 1;
            }
        }
        assert_eq!(cp_idx, checkpoints.len());
    }

    #[test]
    fn test_firmware_core_flaming_parity() {
        assert_scenario_parity("flaming", FLAMING_INPUTS, FLAMING_CHECKPOINTS);
    }

    #[test]
    fn test_firmware_core_smolder_parity() {
        assert_scenario_parity("smolder", SMOLDER_INPUTS, SMOLDER_CHECKPOINTS);
    }

    #[test]
    fn test_firmware_core_clean_parity() {
        assert_scenario_parity("clean", CLEAN_INPUTS, CLEAN_CHECKPOINTS);
    }

    #[test]
    fn test_firmware_core_fault_and_commands() {
        assert_scenario_parity("fault", FAULT_INPUTS, FAULT_CHECKPOINTS);

        let mut fw = FirmwareCore::new();
        assert_eq!(fw.state().as_str(), "NORMAL");
        assert_eq!(fw.mode().as_str(), "AUTO");

        // 5 invalid samples -> Failsafe, duty 70.0
        for i in 1..=5 {
            let tel = fw.step(0.05, 1.0, i, cfg::TS_S);
            if i < 5 {
                assert_eq!(tel.state, AlarmState::Normal);
            } else {
                assert_eq!(tel.state, AlarmState::Failsafe);
                assert_eq!(tel.state.as_str(), "FAILSAFE");
                assert_eq!(tel.alarm, 3);
                assert!((tel.duty - 70.0).abs() < 1e-4);
            }
        }
        // 5 valid samples -> recover to Normal
        for i in 6..=10 {
            let tel = fw.step(0.60, 1.0, i, cfg::TS_S);
            if i < 10 {
                assert_eq!(tel.state, AlarmState::Failsafe);
            } else {
                assert_eq!(tel.state, AlarmState::Normal);
            }
        }
        // ForceMax -> duty 100.0
        let tel_max = fw.apply_command(CmdMsg::ForceMax);
        assert_eq!(tel_max.mode, OpMode::ForceMax);
        assert_eq!(tel_max.mode.as_str(), "FORCE_MAX");
        assert!((tel_max.duty - 100.0).abs() < 1e-4);
        // ForceOff -> duty 0.0
        let tel_off = fw.apply_command(CmdMsg::ForceOff);
        assert_eq!(tel_off.mode, OpMode::ForceOff);
        assert_eq!(tel_off.mode.as_str(), "FORCE_OFF");
        assert!((tel_off.duty - 0.0).abs() < 1e-4);
        // Reset -> Normal & Auto
        let tel_rst = fw.apply_command(CmdMsg::Reset);
        assert_eq!(tel_rst.state, AlarmState::Normal);
        assert_eq!(tel_rst.mode, OpMode::Auto);
        assert!((tel_rst.duty - 0.0).abs() < 1e-4);

        // force_failsafe()
        let tel_fs = fw.force_failsafe();
        assert_eq!(tel_fs.state, AlarmState::Failsafe);
        assert_eq!(fw.state(), AlarmState::Failsafe);
        let tel_auto = fw.apply_command(CmdMsg::Auto);
        assert_eq!(tel_auto.mode, OpMode::Auto);
        assert!((tel_auto.duty - 70.0).abs() < 1e-4);
    }
}
