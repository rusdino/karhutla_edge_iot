#![no_std]
#![no_main]

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use embassy_executor::Spawner;
#[cfg(feature = "riil")]
use embassy_futures::select::{select, Either};
#[cfg(feature = "hils")]
use embassy_futures::select::{select3, Either3};
use embassy_futures::select::{select4, Either4};
use embassy_net::{Runner, StackResources};
use embassy_sync::{
    blocking_mutex::{raw::CriticalSectionRawMutex as CS, Mutex},
    channel::Channel,
    signal::Signal,
};
use embassy_time::{Duration, Instant, Ticker, Timer};
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config as I2cConfig, I2c},
    ledc::{timer, LSGlobalClkSource, Ledc, LowSpeed},
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{
    sta::StationConfig, AuthenticationMethodConfig, Config as WifiConfig, Interface, PowerSaveMode,
    WifiController,
};
use karhutla_core::{
    buzzer::buzzer_on, cfg, record::BackupRecord, AlarmState, CmdMsg, FirmwareCore, Telemetry,
};
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

mod hardware;
mod mqtt;
mod storage;

use hardware::{Actuators, RtcClock};
use storage::{FlashLogStore, LogMeta};

extern crate alloc;

// ---------------------------------------------------------------- shared state
pub enum LogReq {
    Meta,
    Get { start: u32, count: u32 },
    Clear { expected_count: u32, expected_crc: u32 },
}

pub const CHUNK_B64_MAX: usize = 1368;

pub enum LogResp {
    Meta { meta: LogMeta, rtc_ok: bool, epoch_s: u32 },
    Chunk { start: u32, count: u32, total: u32, len: usize, data: [u8; CHUNK_B64_MAX] },
    ClearStarted { sectors: u32 },
    ClearDone { ok: bool, reason: &'static str, erased: u32, count: u32 },
}

pub struct SharedStatus {
    pub log_count: AtomicU32,
    pub log_full: AtomicBool,
    pub rtc_ok: AtomicBool,
    pub epoch_s: AtomicU32,
}

pub static SHARED_STATUS: SharedStatus = SharedStatus {
    log_count: AtomicU32::new(0),
    log_full: AtomicBool::new(false),
    rtc_ok: AtomicBool::new(false),
    epoch_s: AtomicU32::new(0),
};
pub static BUZZER_STATE: AtomicU8 = AtomicU8::new(0);
pub static MQTT_CONNECTED: AtomicBool = AtomicBool::new(false);

pub static TEL_SIG: Signal<CS, Telemetry> = Signal::new();
pub static TIME_SYNC_SIG: Signal<CS, u32> = Signal::new();
pub static CMD_CH: Channel<CS, CmdMsg, 4> = Channel::new();
pub static BACKUP_CH: Channel<CS, BackupRecord, 8> = Channel::new();
pub static LOG_REQ_CH: Channel<CS, LogReq, 4> = Channel::new();
pub static LOG_RESP_CH: Channel<CS, LogResp, 2> = Channel::new();
#[cfg(feature = "hils")]
pub static HILS_CH: Channel<CS, (u32, f32, f32), 4> = Channel::new();

/// Wrapper so the actuators can live behind a critical-section mutex.
/// SAFETY: single-core executor; all access happens inside `critical_section`.
struct ActCell(Actuators<'static>);
unsafe impl Send for ActCell {}

static ACT: Mutex<CS, RefCell<Option<ActCell>>> = Mutex::new(RefCell::new(None));

const IS_HILS: bool = cfg!(feature = "hils");

// ---------------------------------------------------------------------- tasks
#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}

#[embassy_executor::task]
async fn wifi_task(mut controller: WifiController<'static>) {
    let station = StationConfig::default()
        .with_ssid(cfg::WIFI_SSID.try_into().unwrap())
        .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
            cfg::WIFI_PASS.try_into().unwrap(),
        ));
    let _ = controller.set_config(&WifiConfig::Station(station));
    let _ = controller.set_power_saving(PowerSaveMode::None);
    loop {
        match controller.connect_async().await {
            Ok(_) => {
                let _ = controller.wait_for_disconnect_async().await;
            }
            Err(_) => {}
        }
        MQTT_CONNECTED.store(false, Ordering::Relaxed);
        Timer::after_secs(3).await;
    }
}

fn state_from_u8(v: u8) -> AlarmState {
    match v {
        1 => AlarmState::Waspada,
        2 => AlarmState::Bahaya,
        3 => AlarmState::Failsafe,
        _ => AlarmState::Normal,
    }
}

#[embassy_executor::task]
async fn buzzer_task() {
    let mut tick = Ticker::every(Duration::from_millis(20));
    loop {
        tick.next().await;
        let st = state_from_u8(BUZZER_STATE.load(Ordering::Relaxed));
        let full = SHARED_STATUS.log_full.load(Ordering::Relaxed);
        let on = buzzer_on(st, full, Instant::now().as_millis());
        ACT.lock(|c| {
            if let Some(a) = c.borrow_mut().as_mut() {
                a.0.set_buzzer(on);
            }
        });
    }
}

fn actuate(t: &Telemetry) {
    ACT.lock(|c| {
        if let Some(a) = c.borrow_mut().as_mut() {
            a.0.set_fan_duty_pct(t.duty);
            a.0.set_leds(t.state);
        }
    });
    BUZZER_STATE.store(t.state as u8, Ordering::Relaxed);
}

/// Online -> telemetry signal. Offline -> backup record (RIIL: every step;
/// HILS: one-shot FAILSAFE transition record per outage).
fn emit(t: Telemetry, outage_logged: &mut bool) {
    if MQTT_CONNECTED.load(Ordering::Relaxed) {
        *outage_logged = false;
        TEL_SIG.signal(t);
        return;
    }
    let log = if IS_HILS {
        let l = t.state == AlarmState::Failsafe && !*outage_logged;
        if l {
            *outage_logged = true;
        }
        l
    } else {
        true
    };
    if log {
        let rec = BackupRecord::from_telemetry(&t, 0, Instant::now().as_millis() as u32, false, IS_HILS);
        let _ = BACKUP_CH.try_send(rec);
    }
}

#[cfg(feature = "riil")]
#[embassy_executor::task]
async fn control_task(mut sensors: hardware::Sensors<'static>) {
    let mut core = FirmwareCore::new();
    let mut seq = 0u32;
    let mut outage = false;
    let mut tick = Ticker::every(Duration::from_millis((cfg::TS_S * 1000.0) as u64));
    loop {
        match select(tick.next(), CMD_CH.receive()).await {
            Either::First(()) => {
                let vd = sensors.read_dust_volts().await;
                let vm = sensors.read_mq_volts().await;
                seq = seq.wrapping_add(1);
                let t = core.step(vd, vm, seq, cfg::TS_S);
                actuate(&t);
                emit(t, &mut outage);
            }
            Either::Second(cmd) => {
                let t = core.apply_command(cmd);
                actuate(&t);
                emit(t, &mut outage);
            }
        }
    }
}

#[cfg(feature = "hils")]
#[embassy_executor::task]
async fn control_task() {
    let mut core = FirmwareCore::new();
    let mut outage = false;
    let mut active = false;
    let mut last_sample = Instant::now();
    let mut last_wdt = Instant::now();
    let mut last_tel = core.apply_command(CmdMsg::Auto);
    actuate(&last_tel);
    loop {
        match select3(HILS_CH.receive(), CMD_CH.receive(), Timer::after_millis(50)).await {
            Either3::First((seq, vd, vm)) => {
                last_sample = Instant::now();
                active = true;
                last_tel = core.step(vd, vm, seq, cfg::TS_S);
                actuate(&last_tel);
                emit(last_tel, &mut outage);
            }
            Either3::Second(cmd) => {
                if cmd == CmdMsg::Reset {
                    while HILS_CH.try_receive().is_ok() {}
                    active = false;
                }
                last_tel = core.apply_command(cmd);
                actuate(&last_tel);
                emit(last_tel, &mut outage);
            }
            Either3::Third(()) => {
                let now = Instant::now();
                if active
                    && now.duration_since(last_sample) > Duration::from_millis(cfg::HILS_WDT_MS as u64)
                    && now.duration_since(last_wdt) >= Duration::from_millis(1000)
                {
                    last_wdt = now;
                    last_tel = core.force_failsafe();
                    actuate(&last_tel);
                    emit(last_tel, &mut outage);
                }
            }
        }
    }
}

fn refresh_status(store: &FlashLogStore<'_>, rtc: &mut RtcClock<'_>) -> (u32, bool) {
    let (count, full) = store.status();
    SHARED_STATUS.log_count.store(count, Ordering::Relaxed);
    SHARED_STATUS.log_full.store(full, Ordering::Relaxed);
    let (epoch, ok) = rtc.now_epoch(Instant::now().as_millis());
    SHARED_STATUS.rtc_ok.store(ok, Ordering::Relaxed);
    SHARED_STATUS.epoch_s.store(epoch, Ordering::Relaxed);
    (epoch, ok)
}

#[embassy_executor::task]
async fn storage_rtc_task(mut store: FlashLogStore<'static>, mut rtc: RtcClock<'static>) {
    let mut tick = Ticker::every(Duration::from_secs(1));
    refresh_status(&store, &mut rtc);
    loop {
        match select4(BACKUP_CH.receive(), TIME_SYNC_SIG.wait(), LOG_REQ_CH.receive(), tick.next()).await {
            Either4::First(mut rec) => {
                let (epoch, ok) = refresh_status(&store, &mut rtc);
                rec.epoch_s = epoch;
                if ok {
                    rec.flags |= karhutla_core::record::FLAG_RTC_VALID;
                }
                let _ = store.append(&rec);
                refresh_status(&store, &mut rtc);
            }
            Either4::Second(epoch) => {
                rtc.sync_epoch(epoch, Instant::now().as_millis());
                refresh_status(&store, &mut rtc);
            }
            Either4::Third(req) => match req {
                LogReq::Meta => {
                    if let Ok(meta) = store.compute_meta().await {
                        let (epoch_s, rtc_ok) = refresh_status(&store, &mut rtc);
                        let _ = LOG_RESP_CH.try_send(LogResp::Meta { meta, rtc_ok, epoch_s });
                    }
                }
                LogReq::Get { start, count } => {
                    let mut data = [0u8; CHUNK_B64_MAX];
                    if let Ok((s, n, len)) = store.read_chunk_b64(start, count, &mut data) {
                        let total = store.status().0;
                        let _ = LOG_RESP_CH.try_send(LogResp::Chunk { start: s, count: n, total, len, data });
                    }
                }
                LogReq::Clear { expected_count, expected_crc } => {
                    let done = match store.compute_meta().await {
                        Ok(m) if m.count == expected_count && m.crc32 == expected_crc => {
                            let sectors = (m.used_bytes + 4095) / 4096;
                            let _ = LOG_RESP_CH.try_send(LogResp::ClearStarted { sectors });
                            match store.clear_verified(expected_count, expected_crc, || {}).await {
                                Ok(Some(n)) => LogResp::ClearDone { ok: true, reason: "", erased: n, count: 0 },
                                Ok(None) => LogResp::ClearDone { ok: false, reason: "mismatch", erased: 0, count: 0 },
                                Err(_) => LogResp::ClearDone { ok: false, reason: "flash_error", erased: 0, count: 0 },
                            }
                        }
                        Ok(_) => LogResp::ClearDone { ok: false, reason: "mismatch", erased: 0, count: 0 },
                        Err(_) => LogResp::ClearDone { ok: false, reason: "flash_error", erased: 0, count: 0 },
                    };
                    refresh_status(&store, &mut rtc);
                    let _ = LOG_RESP_CH.try_send(done);
                }
            },
            Either4::Fourth(()) => {
                refresh_status(&store, &mut rtc);
            }
        }
    }
}

// ----------------------------------------------------------------------- main
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::_240MHz));
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    // Flash log store (scan before network tasks).
    let mut store = FlashLogStore::new(p.FLASH);
    store.init_scan();

    // RTC (DS3231 on I2C0, GPIO21/22, 100 kHz).
    let i2c = I2c::new(p.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(100)))
        .unwrap()
        .with_sda(p.GPIO21)
        .with_scl(p.GPIO22);
    let rtc = RtcClock::new(i2c);

    // Actuators (LEDC fan + LEDs + buzzer).
    static LEDC: StaticCell<Ledc<'static>> = StaticCell::new();
    static TIMER0: StaticCell<timer::Timer<'static, LowSpeed>> = StaticCell::new();
    let ledc = LEDC.init(Ledc::new(p.LEDC));
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);
    let ledc: &'static Ledc<'static> = ledc;
    let mut t0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    Actuators::configure_timer(&mut t0);
    let t0: &'static timer::Timer<'static, LowSpeed> = TIMER0.init(t0);
    let actuators = Actuators::new(
        ledc,
        t0,
        p.GPIO18,
        #[cfg(feature = "hils")]
        p.GPIO4,
        p.GPIO23,
        p.GPIO25,
        p.GPIO26,
        p.GPIO27,
    );
    ACT.lock(|c| *c.borrow_mut() = Some(ActCell(actuators)));

    // Wi-Fi + network stack.
    let controller = WifiController::new(p.WIFI, Default::default()).unwrap();
    let iface = Interface::station();
    let seed = {
        let r = esp_hal::rng::Rng::new();
        ((r.random() as u64) << 32) | r.random() as u64
    };
    static RES: StaticCell<StackResources<4>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        iface,
        embassy_net::Config::dhcpv4(Default::default()),
        RES.init(StackResources::new()),
        seed,
    );

    #[cfg(feature = "riil")]
    let sensors = hardware::Sensors::new(p.ADC1, p.GPIO34, p.GPIO35, p.GPIO4);

    spawner.spawn(net_task(runner).unwrap());
    spawner.spawn(wifi_task(controller).unwrap());
    spawner.spawn(mqtt::mqtt_task(stack).unwrap());
    spawner.spawn(storage_rtc_task(store, rtc).unwrap());
    #[cfg(feature = "riil")]
    spawner.spawn(control_task(sensors).unwrap());
    #[cfg(feature = "hils")]
    spawner.spawn(control_task().unwrap());
    spawner.spawn(buzzer_task().unwrap());

    loop {
        Timer::after_secs(3600).await;
    }
}
