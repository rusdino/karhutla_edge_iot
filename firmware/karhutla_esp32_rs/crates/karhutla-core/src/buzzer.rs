//! Deterministic buzzer patterns (spec §2.5), phase-based on milliseconds.
use crate::AlarmState;

/// Segment list: (duration_ms, on). Cycle repeats.
fn on_at(segs: &[(u32, bool)], now_ms: u64) -> bool {
    let cycle: u32 = segs.iter().map(|s| s.0).sum();
    let mut t = (now_ms % cycle as u64) as u32;
    for &(d, on) in segs {
        if t < d {
            return on;
        }
        t -= d;
    }
    false
}

pub fn buzzer_on(state: AlarmState, storage_full: bool, now_ms: u64) -> bool {
    if !storage_full {
        match state {
            AlarmState::Normal => false,
            AlarmState::Waspada => on_at(&[(100, true), (1900, false)], now_ms),
            AlarmState::Bahaya => on_at(&[(200, true), (200, false)], now_ms),
            AlarmState::Failsafe => on_at(
                &[(100, true), (100, false), (100, true), (1700, false)],
                now_ms,
            ),
        }
    } else {
        match state {
            AlarmState::Normal => on_at(&[(1500, true), (1500, false)], now_ms),
            AlarmState::Waspada => on_at(
                &[(1500, true), (250, false), (100, true), (1150, false)],
                now_ms,
            ),
            AlarmState::Bahaya => on_at(
                &[
                    (1500, true),
                    (200, false),
                    (150, true),
                    (150, false),
                    (150, true),
                    (150, false),
                    (150, true),
                    (150, false),
                    (150, true),
                    (150, false),
                    (300, false),
                ],
                now_ms,
            ),
            AlarmState::Failsafe => on_at(
                &[
                    (1500, true),
                    (250, false),
                    (100, true),
                    (100, false),
                    (100, true),
                    (950, false),
                ],
                now_ms,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buzzer_all_8_combinations() {
        let ts = [0u64, 50, 150, 300, 1450, 1600, 1780, 1920, 2200, 2600];
        use AlarmState::*;
        let exp: [(AlarmState, bool, [bool; 10]); 8] = [
            (Normal, false, [false; 10]),
            (Waspada, false, [true, true, false, false, false, false, false, false, false, false]),
            (Bahaya, false, [true, true, true, false, false, true, true, false, false, false]),
            (Failsafe, false, [true, true, false, false, false, false, false, false, true, false]),
            (Normal, true, [true, true, true, true, true, false, false, false, false, false]),
            (Waspada, true, [true, true, true, true, true, false, true, false, false, false]),
            (Bahaya, true, [true, true, true, true, true, false, true, false, false, true]),
            (Failsafe, true, [true, true, true, true, true, false, true, false, false, false]),
        ];
        for (s, full, e) in exp.iter() {
            for (i, &t) in ts.iter().enumerate() {
                assert_eq!(buzzer_on(*s, *full, t), e[i], "{:?} full={} t={}", s, full, t);
            }
        }
        // cycle wrap + extra phases
        assert!(buzzer_on(Waspada, false, 2000));
        assert!(buzzer_on(Failsafe, false, 200));
        assert!(!buzzer_on(Failsafe, false, 150));
        assert!(!buzzer_on(Failsafe, false, 300));
        assert!(buzzer_on(Failsafe, true, 1750));
        assert!(!buzzer_on(Failsafe, true, 1900));
        assert!(buzzer_on(Failsafe, true, 2000));
        assert!(buzzer_on(Failsafe, true, 3000));
        assert!(buzzer_on(Bahaya, true, 3200));
        assert!(!buzzer_on(Bahaya, true, 3100));
    }
}
