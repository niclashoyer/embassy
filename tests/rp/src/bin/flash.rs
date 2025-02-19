#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::flash::{ERASE_SIZE, FLASH_BASE};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

const ADDR_OFFSET: u32 = 0x4000;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    info!("Hello World!");

    // add some delay to give an attached debug probe time to parse the
    // defmt RTT header. Reading that header might touch flash memory, which
    // interferes with flash write operations.
    // https://github.com/knurling-rs/defmt/pull/683
    Timer::after(Duration::from_millis(10)).await;

    let mut flash = embassy_rp::flash::Flash::<_, { 2 * 1024 * 1024 }>::new(p.FLASH);

    // Get JEDEC id
    let jedec = defmt::unwrap!(flash.jedec_id());
    info!("jedec id: 0x{:x}", jedec);

    // Get unique id
    let mut uid = [0; 8];
    defmt::unwrap!(flash.unique_id(&mut uid));
    info!("unique id: {:?}", uid);

    let mut buf = [0u8; ERASE_SIZE];
    defmt::unwrap!(flash.read(ADDR_OFFSET, &mut buf));

    info!("Addr of flash block is {:x}", ADDR_OFFSET + FLASH_BASE as u32);
    info!("Contents start with {=[u8]}", buf[0..4]);

    defmt::unwrap!(flash.erase(ADDR_OFFSET, ADDR_OFFSET + ERASE_SIZE as u32));

    defmt::unwrap!(flash.read(ADDR_OFFSET, &mut buf));
    info!("Contents after erase starts with {=[u8]}", buf[0..4]);
    if buf.iter().any(|x| *x != 0xFF) {
        defmt::panic!("unexpected");
    }

    for b in buf.iter_mut() {
        *b = 0xDA;
    }

    defmt::unwrap!(flash.write(ADDR_OFFSET, &mut buf));

    defmt::unwrap!(flash.read(ADDR_OFFSET, &mut buf));
    info!("Contents after write starts with {=[u8]}", buf[0..4]);
    if buf.iter().any(|x| *x != 0xDA) {
        defmt::panic!("unexpected");
    }

    info!("Test OK");
    cortex_m::asm::bkpt();
}
