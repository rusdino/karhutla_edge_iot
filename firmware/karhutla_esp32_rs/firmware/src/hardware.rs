//! Hardware drivers: actuators (LEDC fan, LEDs, buzzer), sensors (riil), DS3231 RTC.
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    i2c::master::I2c,
    ledc::{
        channel::{self, Channel, ChannelHW, ChannelIFace},
        timer::{self, Timer as LedcTimer, TimerIFace},
        Ledc, LowSpeed,
    },
    peripherals::{GPIO18, GPIO23, GPIO25, GPIO26, GPIO27},
    time::Rate,
    Blocking,
};
use karhutla_core::{
    rtc::{datetime_to_epoch_utc, decode_ds3231_regs, encode_ds3231_regs, epoch_utc_to_datetime},
    AlarmState,
};

pub const DS3231_ADDR: u8 = 0x68;
pub const FAN_FREQ_KHZ: u32 = 25;

pub struct Actuators<'d> {
    fan: Channel<'d, LowSpeed>,
    #[cfg(feature = "hils")]
    _ir_led: Output<'d>,
    buzzer: Output<'d>,
    led_g: Output<'d>,
    led_y: Output<'d>,
    led_r: Output<'d>,
}

impl<'d> Actuators<'d> {
    /// Configure LEDC Timer0 for the fan: 25 kHz, 8-bit.
    pub fn configure_timer(timer: &mut LedcTimer<'d, LowSpeed>) {
        let _ = timer.configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(FAN_FREQ_KHZ),
        });
    }

    /// `timer` must already be configured via [`Self::configure_timer`].
    /// Under `riil`, GPIO4 (Sharp IR LED) is owned by `Sensors` instead.
    pub fn new(
        ledc: &Ledc<'d>,
        timer: &'d LedcTimer<'d, LowSpeed>,
        fan_pin: GPIO18<'d>,
        #[cfg(feature = "hils")] ir_led_pin: esp_hal::peripherals::GPIO4<'d>,
        buzzer_pin: GPIO23<'d>,
        g: GPIO25<'d>,
        y: GPIO26<'d>,
        r: GPIO27<'d>,
    ) -> Self {
        let mut fan = ledc.channel::<LowSpeed>(channel::Number::Channel0, fan_pin);
        let _ = fan.configure(channel::config::Config {
            timer,
            duty_pct: 0,
            drive_mode: esp_hal::gpio::DriveMode::PushPull,
        });
        let cfg = OutputConfig::default();
        Self {
            fan,
            #[cfg(feature = "hils")]
            _ir_led: Output::new(ir_led_pin, Level::High, cfg),
            buzzer: Output::new(buzzer_pin, Level::Low, cfg),
            led_g: Output::new(g, Level::High, cfg),
            led_y: Output::new(y, Level::Low, cfg),
            led_r: Output::new(r, Level::Low, cfg),
        }
    }

    pub fn set_fan_duty_pct(&mut self, pct: f32) {
        let hw = ((pct.clamp(0.0, 100.0) * 2.55) + 0.5) as u32;
        self.fan.set_duty_hw(hw);
    }

    pub fn set_leds(&mut self, st: AlarmState) {
        let (g, y, r) = match st {
            AlarmState::Normal => (true, false, false),
            AlarmState::Waspada => (false, true, false),
            AlarmState::Bahaya => (false, false, true),
            AlarmState::Failsafe => (false, true, true),
        };
        self.led_g.set_level(g.into());
        self.led_y.set_level(y.into());
        self.led_r.set_level(r.into());
    }

    pub fn set_buzzer(&mut self, on: bool) {
        self.buzzer.set_level(on.into());
    }
}

#[cfg(feature = "riil")]
#[allow(unused_imports)]
pub use sensors::Sensors;

#[cfg(feature = "riil")]
mod sensors {
    use embassy_time::Timer;
    use esp_hal::{
        analog::adc::{Adc, AdcCalLine, AdcConfig, AdcPin, Attenuation},
        delay::Delay,
        gpio::{Level, Output, OutputConfig},
        peripherals::{ADC1, GPIO34, GPIO35, GPIO4},
        Blocking,
    };

    const DIV_RATIO: f32 = 1.5;
    const DUST_PULSES: usize = 8;
    const MQ_SAMPLES: u32 = 16;

    pub struct Sensors<'d> {
        adc1: Adc<'d, ADC1<'d>, Blocking>,
        pin_dust: AdcPin<GPIO34<'d>, ADC1<'d>, AdcCalLine<ADC1<'d>>>,
        pin_mq: AdcPin<GPIO35<'d>, ADC1<'d>, AdcCalLine<ADC1<'d>>>,
        ir_led: Output<'d>,
        delay: Delay,
    }

    impl<'d> Sensors<'d> {
        pub fn new(adc: ADC1<'d>, dust: GPIO34<'d>, mq: GPIO35<'d>, ir_led: GPIO4<'d>) -> Self {
            let mut cfg = AdcConfig::new();
            let pin_dust = cfg.enable_pin_with_cal::<_, AdcCalLine<ADC1>>(dust, Attenuation::_11dB);
            let pin_mq = cfg.enable_pin_with_cal::<_, AdcCalLine<ADC1>>(mq, Attenuation::_11dB);
            Self {
                adc1: Adc::new(adc, cfg),
                pin_dust,
                pin_mq,
                ir_led: Output::new(ir_led, Level::High, OutputConfig::default()),
                delay: Delay::new(),
            }
        }

        /// Sharp GP2Y1010AU0F: 8 pulses, trimmed mean of middle 6, volts at sensor.
        pub async fn read_dust_volts(&mut self) -> f32 {
            let mut s = [0u16; DUST_PULSES];
            for v in s.iter_mut() {
                self.ir_led.set_low();
                self.delay.delay_micros(280);
                *v = nb::block!(self.adc1.read_oneshot(&mut self.pin_dust)).unwrap_or(0);
                self.delay.delay_micros(40);
                self.ir_led.set_high();
                Timer::after_micros(9680).await;
            }
            for i in 1..DUST_PULSES {
                let mut j = i;
                while j > 0 && s[j - 1] > s[j] {
                    s.swap(j - 1, j);
                    j -= 1;
                }
            }
            let sum: u32 = s[1..DUST_PULSES - 1].iter().map(|&x| x as u32).sum();
            (sum as f32 / (DUST_PULSES - 2) as f32) / 1000.0 * DIV_RATIO
        }

        pub async fn read_mq_volts(&mut self) -> f32 {
            let mut sum = 0u32;
            for _ in 0..MQ_SAMPLES {
                sum += nb::block!(self.adc1.read_oneshot(&mut self.pin_mq)).unwrap_or(0) as u32;
                Timer::after_millis(1).await;
            }
            (sum as f32 / MQ_SAMPLES as f32) / 1000.0 * DIV_RATIO
        }
    }
}

/// DS3231 with software-clock fallback. Never panics on bus errors.
pub struct RtcClock<'d> {
    i2c: I2c<'d, Blocking>,
    hw_present: bool,
    soft_base_epoch: u32,
    soft_base_ms: u64,
    soft_valid: bool,
}

impl<'d> RtcClock<'d> {
    /// `i2c` should be configured for 100 kHz on GPIO21 (SDA) / GPIO22 (SCL).
    pub fn new(i2c: I2c<'d, Blocking>) -> Self {
        let mut s = Self { i2c, hw_present: false, soft_base_epoch: 0, soft_base_ms: 0, soft_valid: false };
        let mut regs = [0u8; 7];
        s.hw_present = s.read_regs(&mut regs);
        if s.hw_present {
            if let Some(e) = decode_ds3231_regs(&regs).and_then(datetime_to_epoch_utc) {
                s.soft_base_epoch = e;
                s.soft_valid = true;
            }
        }
        s
    }

    fn read_regs(&mut self, regs: &mut [u8; 7]) -> bool {
        self.i2c.write_read(DS3231_ADDR, &[0x00], regs).is_ok()
    }

    pub fn is_hw_present(&self) -> bool {
        self.hw_present
    }

    fn soft_now(&self, now_ms: u64) -> u32 {
        self.soft_base_epoch
            .saturating_add((now_ms.saturating_sub(self.soft_base_ms) / 1000) as u32)
    }

    pub fn sync_epoch(&mut self, epoch_s: u32, now_ms: u64) {
        self.soft_base_epoch = epoch_s;
        self.soft_base_ms = now_ms;
        self.soft_valid = true;
        if !self.hw_present {
            return;
        }
        let mut regs = [0u8; 7];
        let hw_epoch = if self.read_regs(&mut regs) {
            decode_ds3231_regs(&regs).and_then(datetime_to_epoch_utc)
        } else {
            None
        };
        let needs_write = match hw_epoch {
            Some(h) => h.abs_diff(epoch_s) > 5,
            None => true,
        };
        if needs_write {
            let r = encode_ds3231_regs(epoch_utc_to_datetime(epoch_s));
            let mut buf = [0u8; 8];
            buf[1..].copy_from_slice(&r);
            let _ = self.i2c.write(DS3231_ADDR, &buf);
        }
    }

    /// Returns (epoch_s, valid).
    pub fn now_epoch(&mut self, now_ms: u64) -> (u32, bool) {
        if self.hw_present {
            let mut regs = [0u8; 7];
            if self.read_regs(&mut regs) {
                if let Some(e) = decode_ds3231_regs(&regs).and_then(datetime_to_epoch_utc) {
                    self.soft_base_epoch = e;
                    self.soft_base_ms = now_ms;
                    self.soft_valid = true;
                    return (e, true);
                }
            }
        }
        (self.soft_now(now_ms), self.soft_valid)
    }
}
