#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    let _peripherals = embassy_rp::init(Default::default());

    core::future::pending::<()>().await;
}

