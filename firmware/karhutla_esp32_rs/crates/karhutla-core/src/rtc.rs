//! DS3231/DS1307 BCD register helpers and UTC epoch conversion.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub min: u8,
    pub sec: u8,
}

fn bcd_dec(b: u8) -> Option<u8> {
    let (hi, lo) = (b >> 4, b & 0x0F);
    if hi > 9 || lo > 9 {
        None
    } else {
        Some(hi * 10 + lo)
    }
}

fn bcd_enc(v: u8) -> u8 {
    ((v / 10) << 4) | (v % 10)
}

fn is_leap(y: u16) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: u16, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn valid(dt: &RtcDateTime) -> bool {
    dt.month >= 1
        && dt.month <= 12
        && dt.day >= 1
        && dt.day <= days_in_month(dt.year, dt.month)
        && dt.hour < 24
        && dt.min < 60
        && dt.sec < 60
}

/// regs: [sec, min, hour, dow, date, month(+century bit7), year]
pub fn decode_ds3231_regs(regs: &[u8; 7]) -> Option<RtcDateTime> {
    if regs[0] & 0x80 != 0 {
        return None;
    }
    let sec = bcd_dec(regs[0] & 0x7F)?;
    let min = bcd_dec(regs[1] & 0x7F)?;
    let hour = if regs[2] & 0x40 != 0 {
        // 12-hour mode
        let h = bcd_dec(regs[2] & 0x1F)?;
        if h < 1 || h > 12 {
            return None;
        }
        let pm = regs[2] & 0x20 != 0;
        (h % 12) + if pm { 12 } else { 0 }
    } else {
        bcd_dec(regs[2] & 0x3F)?
    };
    let day = bcd_dec(regs[4] & 0x3F)?;
    let month = bcd_dec(regs[5] & 0x1F)?;
    let year = 2000 + bcd_dec(regs[6])? as u16;
    let dt = RtcDateTime { year, month, day, hour, min, sec };
    if valid(&dt) {
        Some(dt)
    } else {
        None
    }
}

fn day_number(y: u16, m: u8, d: u8) -> i64 {
    // days since 1970-01-01 (civil from days algorithm)
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn day_of_week(y: u16, m: u8, d: u8) -> u8 {
    // 1..=7, 1970-01-01 (Thursday) => 4; Monday=1
    let n = day_number(y, m, d);
    ((n + 3).rem_euclid(7) + 1) as u8
}

pub fn encode_ds3231_regs(dt: RtcDateTime) -> [u8; 7] {
    [
        bcd_enc(dt.sec),
        bcd_enc(dt.min),
        bcd_enc(dt.hour),
        day_of_week(dt.year, dt.month, dt.day),
        bcd_enc(dt.day),
        bcd_enc(dt.month),
        bcd_enc((dt.year.saturating_sub(2000) % 100) as u8),
    ]
}

pub fn datetime_to_epoch_utc(dt: RtcDateTime) -> Option<u32> {
    if dt.year < 1970 || !valid(&dt) {
        return None;
    }
    let days = day_number(dt.year, dt.month, dt.day);
    let s = days * 86400 + dt.hour as i64 * 3600 + dt.min as i64 * 60 + dt.sec as i64;
    u32::try_from(s).ok()
}

pub fn epoch_utc_to_datetime(epoch_s: u32) -> RtcDateTime {
    let days = (epoch_s / 86400) as i64;
    let rem = epoch_s % 86400;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    RtcDateTime {
        year: y as u16,
        month: m,
        day: d,
        hour: (rem / 3600) as u8,
        min: ((rem % 3600) / 60) as u8,
        sec: (rem % 60) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(year: u16, month: u8, day: u8, hour: u8, min: u8, sec: u8) -> RtcDateTime {
        RtcDateTime { year, month, day, hour, min, sec }
    }

    #[test]
    fn test_rtc_bcd_and_epoch_roundtrip() {
        let cases = [
            (dt(2026, 11, 26, 10, 15, 30), 1_795_688_130u32),
            (dt(2024, 2, 29, 23, 59, 59), 1_709_251_199),
            (dt(2099, 12, 31, 23, 59, 59), 4102444799),
            (dt(1970, 1, 1, 0, 0, 0), 0),
        ];
        for (d, e) in cases {
            assert_eq!(datetime_to_epoch_utc(d), Some(e), "{:?}", d);
            assert_eq!(epoch_utc_to_datetime(e), d);
        }
        for d in [
            dt(2026, 11, 26, 10, 15, 30),
            dt(2024, 2, 29, 23, 59, 59),
            dt(2099, 12, 31, 23, 59, 59),
        ] {
            let regs = encode_ds3231_regs(d);
            assert_eq!(decode_ds3231_regs(&regs), Some(d));
            let e = datetime_to_epoch_utc(d).unwrap();
            assert_eq!(epoch_utc_to_datetime(e), d);
        }
        // exact BCD bytes + weekday (2026-11-26 is Thursday = 4)
        let r = encode_ds3231_regs(dt(2026, 11, 26, 10, 15, 30));
        assert_eq!(r, [0x30, 0x15, 0x10, 4, 0x26, 0x11, 0x26]);
        // year 2099 -> 0x99
        assert_eq!(encode_ds3231_regs(dt(2099, 1, 1, 0, 0, 0))[6], 0x99);
        // invalid BCD / ranges
        assert_eq!(decode_ds3231_regs(&[0xFF; 7]), None);
        assert_eq!(decode_ds3231_regs(&[0x30, 0x15, 0x10, 4, 0x31, 0x11, 0x26]), None); // Nov 31
        assert_eq!(decode_ds3231_regs(&[0x30, 0x15, 0x10, 4, 0x29, 0x02, 0x25]), None); // 2025 not leap
        assert_eq!(decode_ds3231_regs(&[0x60, 0x15, 0x10, 4, 0x26, 0x11, 0x26]), None);
        // epoch exhaustive-ish sweep roundtrip
        let mut e = 0u32;
        while e < 4_102_444_800 {
            let d = epoch_utc_to_datetime(e);
            assert_eq!(datetime_to_epoch_utc(d), Some(e));
            e += 86_399 * 7 + 1234;
        }
    }

    #[test]
    fn test_rtc_rejects_ds1307_clock_halt_bit() {
        let mut regs = encode_ds3231_regs(dt(2026, 11, 26, 10, 15, 30));
        assert!(decode_ds3231_regs(&regs).is_some());
        regs[0] |= 0x80; // DS1307 Clock Halt (CH) bit set
        assert_eq!(decode_ds3231_regs(&regs), None);
    }
}
