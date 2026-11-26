//! 2.625 MiB append-only flash log store (`logs` partition, 43,008 x 64 B).
use embassy_time::Timer;
use esp_storage::{FlashStorage, FlashStorageError};
use karhutla_core::record::{
    crc32_update, encode_base64, find_write_cursor, BackupRecord, LOG_CAPACITY, LOG_PART_OFFSET,
    RECORD_SIZE, SECTOR_SIZE,
};

pub type Flash<'d> = esp_hal::peripherals::FLASH<'d>;

pub const MAX_CHUNK_RECORDS: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogMeta {
    pub count: u32,
    pub cap: u32,
    pub full: bool,
    pub used_bytes: u32,
    pub cap_bytes: u32,
    pub crc32: u32,
}

pub struct FlashLogStore<'d> {
    flash: FlashStorage<'d>,
    count: u32,
    full: bool,
    /// (record count, crc32 over those records)
    cached_crc: Option<(u32, u32)>,
}

impl<'d> FlashLogStore<'d> {
    pub fn new(flash: Flash<'d>) -> Self {
        Self { flash: FlashStorage::new(flash), count: 0, full: false, cached_crc: None }
    }

    fn slot_off(idx: u32) -> u32 {
        LOG_PART_OFFSET + idx * RECORD_SIZE as u32
    }

    pub fn init_scan(&mut self) {
        let flash = &mut self.flash;
        let cur = find_write_cursor(LOG_CAPACITY, |i| {
            let mut raw = [0u8; 64];
            flash.read_nor(Self::slot_off(i), &mut raw)?;
            Ok::<_, FlashStorageError>(raw)
        });
        // On read error, be conservative: treat log as full (never overwrite).
        self.count = cur.unwrap_or(LOG_CAPACITY);
        self.full = self.count >= LOG_CAPACITY;
        self.cached_crc = None;
    }

    pub fn status(&self) -> (u32, bool) {
        (self.count, self.full)
    }

    /// Returns Ok(true) if stored, Ok(false) if the log is full.
    pub fn append(&mut self, rec: &BackupRecord) -> Result<bool, FlashStorageError> {
        if self.count >= LOG_CAPACITY {
            self.full = true;
            return Ok(false);
        }
        let mut raw = [0u8; 64];
        rec.encode(&mut raw);
        self.flash.write_nor(Self::slot_off(self.count), &raw)?;
        let prev = self.count;
        self.count += 1;
        self.full = self.count >= LOG_CAPACITY;
        if let Some((pc, pcrc)) = self.cached_crc {
            if pc == prev {
                self.cached_crc = Some((self.count, crc32_update(pcrc, &raw)));
            }
        }
        Ok(true)
    }

    fn meta(&self, crc: u32) -> LogMeta {
        LogMeta {
            count: self.count,
            cap: LOG_CAPACITY,
            full: self.full,
            used_bytes: self.count * RECORD_SIZE as u32,
            cap_bytes: LOG_CAPACITY * RECORD_SIZE as u32,
            crc32: crc,
        }
    }

    pub async fn compute_meta(&mut self) -> Result<LogMeta, FlashStorageError> {
        if let Some((c, crc)) = self.cached_crc {
            if c == self.count {
                return Ok(self.meta(crc));
            }
        }
        let total = self.count * RECORD_SIZE as u32;
        let mut crc = 0u32;
        let mut buf = [0u8; SECTOR_SIZE as usize];
        let mut off = 0u32;
        let mut sectors = 0u32;
        while off < total {
            let n = core::cmp::min(SECTOR_SIZE, total - off) as usize;
            self.flash.read_nor(LOG_PART_OFFSET + off, &mut buf[..n])?;
            crc = crc32_update(crc, &buf[..n]);
            off += n as u32;
            sectors += 1;
            if sectors % 8 == 0 {
                Timer::after_millis(1).await;
            }
        }
        self.cached_crc = Some((self.count, crc));
        Ok(self.meta(crc))
    }

    /// Reads up to `max_count` (<=16) records from `start`, Base64-encodes them into
    /// `b64_out`. Returns `(start, records_read, b64_len)`.
    pub fn read_chunk_b64(
        &mut self,
        start: u32,
        max_count: u32,
        b64_out: &mut [u8],
    ) -> Result<(u32, u32, usize), FlashStorageError> {
        if start >= self.count {
            return Ok((start, 0, 0));
        }
        let mut n = max_count.min(MAX_CHUNK_RECORDS).min(self.count - start);
        // Base64 expands 3 -> 4 bytes; shrink to fit the output buffer.
        while n > 0 && ((n as usize * RECORD_SIZE + 2) / 3) * 4 > b64_out.len() {
            n -= 1;
        }
        if n == 0 {
            return Ok((start, 0, 0));
        }
        let mut raw = [0u8; MAX_CHUNK_RECORDS as usize * RECORD_SIZE];
        let len = n as usize * RECORD_SIZE;
        self.flash.read_nor(Self::slot_off(start), &mut raw[..len])?;
        let w = encode_base64(&raw[..len], b64_out).unwrap_or(0);
        Ok((start, n, w))
    }

    /// Verifies (count, crc) then erases the used sectors. `Ok(None)` on mismatch.
    pub async fn clear_verified<F>(
        &mut self,
        expected_count: u32,
        expected_crc: u32,
        mut on_yield: F,
    ) -> Result<Option<u32>, FlashStorageError>
    where
        F: FnMut(),
    {
        let meta = self.compute_meta().await?;
        if meta.count != expected_count || meta.crc32 != expected_crc {
            return Ok(None);
        }
        let bytes = self.count * RECORD_SIZE as u32;
        let sectors = (bytes + SECTOR_SIZE - 1) / SECTOR_SIZE;
        self.cached_crc = None;
        let mut done = 0u32;
        while done < sectors {
            let batch = core::cmp::min(4, sectors - done);
            let from = LOG_PART_OFFSET + done * SECTOR_SIZE;
            if let Err(e) = self.flash.erase(from, from + batch * SECTOR_SIZE) {
                self.init_scan();
                return Err(e);
            }
            done += batch;
            on_yield();
            Timer::after_millis(10).await;
        }
        self.count = 0;
        self.full = false;
        self.cached_crc = Some((0, 0));
        Ok(Some(sectors))
    }
}
