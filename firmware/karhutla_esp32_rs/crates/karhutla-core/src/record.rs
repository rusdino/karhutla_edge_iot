//! 64-byte flash BackupRecord, CRC-32/IEEE, Base64 and write-cursor scan.
use crate::{AlarmState, OpMode, Telemetry};

pub const LOG_PART_OFFSET: u32 = 0x15_0000;
pub const LOG_PART_SIZE: u32 = 0x2A_0000;
pub const SECTOR_SIZE: u32 = 4096;
pub const RECORD_SIZE: usize = 64;
pub const LOG_CAPACITY: u32 = 43_008;
pub const FLAG_RTC_VALID: u8 = 0x01;
pub const FLAG_MODE_HILS: u8 = 0x02;

const MAGIC: [u8; 4] = *b"KLOG";
const VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordError {
    BadMagic,
    BadVersion,
    BadField,
    CrcMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackupRecord {
    pub state: AlarmState,
    pub mode: OpMode,
    pub flags: u8,
    pub epoch_s: u32,
    pub uptime_ms: u32,
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
}

pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

pub fn crc32_ieee(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

impl BackupRecord {
    pub fn from_telemetry(
        t: &Telemetry,
        epoch_s: u32,
        uptime_ms: u32,
        rtc_valid: bool,
        is_hils: bool,
    ) -> Self {
        let mut flags = 0;
        if rtc_valid {
            flags |= FLAG_RTC_VALID;
        }
        if is_hils {
            flags |= FLAG_MODE_HILS;
        }
        Self {
            state: t.state,
            mode: t.mode,
            flags,
            epoch_s,
            uptime_ms,
            seq: t.seq,
            v_dust: t.v_dust,
            v_mq: t.v_mq,
            pm_raw: t.pm_raw,
            pm: t.pm,
            dcdt: t.dcdt,
            gas_ppm: t.gas_ppm,
            g: t.g,
            fi: t.fi,
            v0: t.v0,
            duty: t.duty,
        }
    }

    fn floats(&self) -> [f32; 10] {
        [
            self.v_dust, self.v_mq, self.pm_raw, self.pm, self.dcdt, self.gas_ppm, self.g,
            self.fi, self.v0, self.duty,
        ]
    }

    pub fn encode(&self, out: &mut [u8; 64]) {
        out[0..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        out[5] = self.state as u8;
        out[6] = self.mode as u8;
        out[7] = self.flags;
        out[8..12].copy_from_slice(&self.epoch_s.to_le_bytes());
        out[12..16].copy_from_slice(&self.uptime_ms.to_le_bytes());
        out[16..20].copy_from_slice(&self.seq.to_le_bytes());
        for (i, f) in self.floats().iter().enumerate() {
            out[20 + i * 4..24 + i * 4].copy_from_slice(&f.to_le_bytes());
        }
        let crc = crc32_ieee(&out[0..60]);
        out[60..64].copy_from_slice(&crc.to_le_bytes());
    }

    pub fn decode(raw: &[u8; 64]) -> Result<Self, RecordError> {
        if raw[0..4] != MAGIC {
            return Err(RecordError::BadMagic);
        }
        let stored = u32::from_le_bytes([raw[60], raw[61], raw[62], raw[63]]);
        if crc32_ieee(&raw[0..60]) != stored {
            return Err(RecordError::CrcMismatch);
        }
        if raw[4] != VERSION {
            return Err(RecordError::BadVersion);
        }
        let state = match raw[5] {
            0 => AlarmState::Normal,
            1 => AlarmState::Waspada,
            2 => AlarmState::Bahaya,
            3 => AlarmState::Failsafe,
            _ => return Err(RecordError::BadField),
        };
        let mode = match raw[6] {
            0 => OpMode::Auto,
            1 => OpMode::ForceMax,
            2 => OpMode::ForceOff,
            _ => return Err(RecordError::BadField),
        };
        let u = |o: usize| u32::from_le_bytes([raw[o], raw[o + 1], raw[o + 2], raw[o + 3]]);
        let f = |i: usize| f32::from_bits(u(20 + i * 4));
        Ok(Self {
            state,
            mode,
            flags: raw[7],
            epoch_s: u(8),
            uptime_ms: u(12),
            seq: u(16),
            v_dust: f(0),
            v_mq: f(1),
            pm_raw: f(2),
            pm: f(3),
            dcdt: f(4),
            gas_ppm: f(5),
            g: f(6),
            fi: f(7),
            v0: f(8),
            duty: f(9),
        })
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard Base64 with '=' padding. Returns bytes written.
pub fn encode_base64(input: &[u8], out: &mut [u8]) -> Result<usize, ()> {
    let need = input.len().div_ceil(3) * 4;
    if out.len() < need {
        return Err(());
    }
    let mut o = 0;
    for ch in input.chunks(3) {
        let b0 = ch[0] as u32;
        let b1 = *ch.get(1).unwrap_or(&0) as u32;
        let b2 = *ch.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out[o] = B64[(n >> 18) as usize & 63];
        out[o + 1] = B64[(n >> 12) as usize & 63];
        out[o + 2] = if ch.len() > 1 { B64[(n >> 6) as usize & 63] } else { b'=' };
        out[o + 3] = if ch.len() > 2 { B64[n as usize & 63] } else { b'=' };
        o += 4;
    }
    Ok(o)
}

pub fn is_slot_erased(raw: &[u8; 64]) -> bool {
    raw.iter().all(|&b| b == 0xFF)
}

/// Returns the index of the first fully erased slot (= next write slot).
/// Slots are append-only, so non-erased slots form a prefix; a torn slot
/// (partially programmed, bad CRC) counts as used and is skipped, never
/// overwritten. Binary search: <= ~16 reads for 43_008 slots.
pub fn find_write_cursor<E, F>(capacity: u32, mut read_slot: F) -> Result<u32, E>
where
    F: FnMut(u32) -> Result<[u8; 64], E>,
{
    let (mut lo, mut hi) = (0u32, capacity);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if is_slot_erased(&read_slot(mid)?) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BackupRecord {
        BackupRecord {
            state: AlarmState::Bahaya,
            mode: OpMode::ForceMax,
            flags: FLAG_RTC_VALID | FLAG_MODE_HILS,
            epoch_s: 1_795_688_130,
            uptime_ms: 123_456,
            seq: 42,
            v_dust: 0.61,
            v_mq: 1.25,
            pm_raw: 12.5,
            pm: 11.0,
            dcdt: -0.5,
            gas_ppm: 450.0,
            g: 0.0625,
            fi: 77.7,
            v0: 0.6,
            duty: 100.0,
        }
    }

    #[test]
    fn test_record_encode_decode_crc_and_corruption() {
        let r = sample();
        let mut raw = [0u8; 64];
        r.encode(&mut raw);
        assert_eq!(&raw[0..4], b"KLOG");
        assert_eq!(raw[4], 1);
        assert_eq!(raw[5], 2);
        assert_eq!(raw[6], 1);
        assert_eq!(raw[7], 3);
        assert_eq!(BackupRecord::decode(&raw), Ok(r));
        for bit in [0usize, 100, 300, 479, 511] {
            let mut bad = raw;
            bad[bit / 8] ^= 1 << (bit % 8);
            let e = BackupRecord::decode(&bad);
            if bit / 8 < 4 {
                assert_eq!(e, Err(RecordError::BadMagic));
            } else {
                assert_eq!(e, Err(RecordError::CrcMismatch));
            }
        }
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
        let d = b"hello world, karhutla";
        for k in 0..=d.len() {
            assert_eq!(crc32_update(crc32_ieee(&d[..k]), &d[k..]), crc32_ieee(d));
        }
        let t = Telemetry {
            seq: 7, v_dust: 1.0, v_mq: 2.0, pm_raw: 3.0, pm: 4.0, dcdt: 5.0, gas_ppm: 6.0,
            g: 7.0, fi: 8.0, v0: 9.0, duty: 10.0, state: AlarmState::Waspada,
            mode: OpMode::Auto, alarm: 1,
        };
        let br = BackupRecord::from_telemetry(&t, 5, 6, true, false);
        assert_eq!(br.flags, FLAG_RTC_VALID);
        assert_eq!((br.seq, br.epoch_s, br.uptime_ms, br.duty), (7, 5, 6, 10.0));
        assert_eq!(BackupRecord::from_telemetry(&t, 0, 0, false, true).flags, FLAG_MODE_HILS);
    }

    #[test]
    fn test_find_write_cursor_with_torn_slot() {
        let mut good = [0u8; 64];
        sample().encode(&mut good);
        let mut torn = [0xFFu8; 64];
        torn[0..4].copy_from_slice(b"KLOG");
        torn[4] = 1;
        torn[5] = 0x00; // partial program, rest 0xFF, bad CRC
        assert_eq!(BackupRecord::decode(&torn), Err(RecordError::CrcMismatch));
        let run = |valid: u32, torn_at: Option<u32>, cap: u32| {
            find_write_cursor::<(), _>(cap, |i| {
                Ok(if i < valid {
                    good
                } else if Some(i) == torn_at {
                    torn
                } else {
                    [0xFF; 64]
                })
            })
        };
        assert_eq!(run(5, Some(5), 43_008), Ok(6));
        assert_eq!(run(0, None, 43_008), Ok(0));
        assert_eq!(run(0, Some(0), 43_008), Ok(1));
        assert_eq!(run(43_008, None, 43_008), Ok(43_008));
        assert_eq!(run(43_007, Some(43_007), 43_008), Ok(43_008));
        assert_eq!(run(1234, None, 43_008), Ok(1234));
        // read error propagates; bounded read count
        let mut reads = 0;
        let _ = find_write_cursor::<(), _>(43_008, |i| {
            reads += 1;
            Ok(if i < 20_000 { good } else { [0xFF; 64] })
        });
        assert!(reads <= 16);
        assert_eq!(find_write_cursor(10, |_| Err("io")), Err("io"));
        assert_eq!(LOG_CAPACITY * RECORD_SIZE as u32, 2_752_512);
        assert!(LOG_CAPACITY * 64 <= LOG_PART_SIZE);
    }

    #[test]
    fn test_encode_base64_1024_bytes() {
        let mut out = [0u8; 1368];
        assert_eq!(encode_base64(&[0u8; 1024], &mut out), Ok(1368));
        assert_eq!(&out[1364..], b"AA==");
        assert!(out[..1364].iter().all(|&c| c == b'A'));
        let pat: [u8; 1024] = core::array::from_fn(|i| i as u8);
        assert_eq!(encode_base64(&pat, &mut out), Ok(1368));
        assert_eq!(&out[..8], b"AAECAwQF");
        assert!(out.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'+' || *c == b'/' || *c == b'='));
        assert_eq!(encode_base64(&pat, &mut [0u8; 1367]), Err(()));
        let mut s = [0u8; 8];
        let n = encode_base64(b"foobar", &mut s).unwrap();
        assert_eq!(&s[..n], b"Zm9vYmFy");
        let n = encode_base64(b"fo", &mut s).unwrap();
        assert_eq!(&s[..n], b"Zm8=");
    }
}
