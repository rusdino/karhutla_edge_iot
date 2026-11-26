#![no_std]
#![no_main]

use esp_backtrace as _;

esp_bootloader_esp_idf::esp_app_desc!();

#[allow(dead_code)]
mod hardware;
#[allow(dead_code)]
mod storage;

#[esp_hal::main]
fn main() -> ! {
    let _p = esp_hal::init(esp_hal::Config::default());
    loop {}
}
