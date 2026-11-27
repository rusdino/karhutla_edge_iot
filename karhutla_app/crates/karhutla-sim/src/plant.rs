//! Smoke chamber plant & Sharp/MQ-135 sensor voltage model (ported from `plant.py`).

use karhutla_core::cfg;
use karhutla_protocol::Scenario;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

pub const LAMBDA_LEAK: f64 = 0.002;
pub const LAMBDA_FAN: f64 = 0.03;
pub const FAN_MIN_DUTY: f64 = 0.20;
pub const KGAS: f64 = 3.0;
pub const TAU_GAS: f64 = 25.0;
pub const V0_DRIFT: f64 = 2e-5;
pub const SIM_RS_CLEAN: f64 = 30.0;
pub const NOISE_V_DUST: f64 = 0.02;
pub const NOISE_V_MQ: f64 = 0.005;
pub const SPIKE_P: f64 = 0.02;

pub struct SmokeChamber {
    pub scenario: Scenario,
    pub dt: f64,
    pub c: f64,
    pub gas_ppm: f64,
    r0_true: f64,
    rng: StdRng,
}

impl SmokeChamber {
    pub fn new(scenario: Scenario, seed: u64, dt: f64) -> Self {
        let mq_atm = cfg::MQ_ATM_PPM as f64;
        let mq_para = cfg::MQ_PARA as f64;
        let mq_parb = cfg::MQ_PARB as f64;
        let r0_true = SIM_RS_CLEAN * (mq_atm / mq_para).powf(1.0 / mq_parb);
        Self {
            scenario,
            dt,
            c: 0.0,
            gas_ppm: mq_atm,
            r0_true,
            rng: StdRng::seed_from_u64(seed),
        }
    }

    pub fn source(&self, t: f64) -> f64 {
        match self.scenario {
            Scenario::Smolder | Scenario::Fault => {
                if (60.0..300.0).contains(&t) {
                    1.5 * (t - 60.0) / 240.0
                } else if (300.0..600.0).contains(&t) {
                    1.5
                } else {
                    0.0
                }
            }
            Scenario::Flaming => {
                if (100.0..160.0).contains(&t) {
                    12.0
                } else if (160.0..400.0).contains(&t) {
                    3.0
                } else {
                    0.0
                }
            }
            Scenario::Clean => 0.0,
        }
    }

    pub fn dust_disconnected(&self, t: f64) -> bool {
        self.scenario == Scenario::Fault && (200.0..260.0).contains(&t)
    }

    pub fn advance(&mut self, duty01: f64, t: f64) {
        let flow = if duty01 >= FAN_MIN_DUTY { duty01 } else { 0.0 };
        let lam = LAMBDA_LEAK + LAMBDA_FAN * flow;
        self.c = (self.c + (self.source(t) - lam * self.c) * self.dt).max(0.0);
        let target = cfg::MQ_ATM_PPM as f64 + KGAS * self.c;
        self.gas_ppm += (target - self.gas_ppm) * (1.0 - (-self.dt / TAU_GAS).exp());
    }

    fn gauss(&mut self, sigma: f64) -> f64 {
        let u1: f64 = self.rng.gen_range(1e-12_f64..1.0_f64);
        let u2: f64 = self.rng.gen_range(0.0_f64..1.0_f64);
        let z0 = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        z0 * sigma
    }

    pub fn sensors(&mut self, t: f64) -> (f64, f64) {
        let vd = if self.dust_disconnected(t) {
            self.gauss(0.005).abs()
        } else {
            let mut v = cfg::DUST_V0 as f64
                + V0_DRIFT * t
                + self.c / cfg::DUST_SLOPE as f64
                + self.gauss(NOISE_V_DUST);
            if self.rng.gen::<f64>() < SPIKE_P {
                let sign = if self.rng.gen_bool(0.5) { 1.0 } else { -1.0 };
                v += sign * self.rng.gen_range(0.4..=1.0);
            }
            v
        };
        let rs = self.r0_true
            * (self.gas_ppm / cfg::MQ_PARA as f64).powf(-1.0 / cfg::MQ_PARB as f64);
        let vm = cfg::MQ_VCC as f64 * cfg::MQ_RL_KOHM as f64 / (rs + cfg::MQ_RL_KOHM as f64)
            + self.gauss(NOISE_V_MQ);
        (vd, vm)
    }
}
