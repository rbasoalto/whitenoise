#![no_std]
#![no_main]

use core::fmt::Write;
use core::mem;

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_rp::Peri;
use embassy_rp::bind_interrupts;
use embassy_rp::dma;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIN_25, PIO0, USB};
use embassy_rp::pio::{self, Pio};
use embassy_rp::pio_programs::i2s::{PioI2sOut, PioI2sOutProgram};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embassy_usb::UsbDevice;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use static_cell::StaticCell;
use whitenoise::SAMPLE_RATE;
use whitenoise::dsp::{DspChain, Parameters};
use whitenoise::protocol::{self, Command, ResponseBuffer};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

static PARAMETERS: Signal<CriticalSectionRawMutex, Parameters> = Signal::new();
static I2S_PROGRAM: StaticCell<PioI2sOutProgram<'static, PIO0>> = StaticCell::new();

type UsbDriver = Driver<'static, USB>;
type Device = UsbDevice<'static, UsbDriver>;
type Serial = CdcAcmClass<'static, UsbDriver>;
type I2s = PioI2sOut<'static, PIO0, 0>;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());
    spawner.spawn(heartbeat_task(peripherals.PIN_25).unwrap());

    // MAX98357A wiring:
    //   GP0 -> BCLK, GP1 -> LRC/WS, GP2 -> DIN
    //   VBUS -> VIN, GND -> GND
    let Pio {
        mut common, sm0, ..
    } = Pio::new(peripherals.PIO0, Irqs);
    let program = I2S_PROGRAM.init(PioI2sOutProgram::new(&mut common));
    let i2s = PioI2sOut::new(
        &mut common,
        sm0,
        peripherals.DMA_CH0,
        Irqs,
        peripherals.PIN_2,
        peripherals.PIN_0,
        peripherals.PIN_1,
        SAMPLE_RATE,
        16,
        program,
    );
    spawner.spawn(audio_task(i2s).unwrap());

    let driver = Driver::new(peripherals.USB, Irqs);
    let mut usb_config = embassy_usb::Config::new(0x1209, 0x0001);
    usb_config.manufacturer = Some("whitenoise");
    usb_config.product = Some("RP2040 noise machine");
    usb_config.serial_number = Some("0001");
    usb_config.max_power = 100;
    usb_config.max_packet_size_0 = 64;

    static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUFFER: StaticCell<[u8; 64]> = StaticCell::new();
    let mut builder = embassy_usb::Builder::new(
        driver,
        usb_config,
        CONFIG_DESCRIPTOR.init([0; 256]),
        BOS_DESCRIPTOR.init([0; 256]),
        &mut [],
        CONTROL_BUFFER.init([0; 64]),
    );

    static CDC_STATE: StaticCell<State<'static>> = StaticCell::new();
    let mut serial = CdcAcmClass::new(&mut builder, CDC_STATE.init(State::new()), 64);
    let usb = builder.build();
    spawner.spawn(usb_task(usb).unwrap());

    info!("audio and USB control ready");
    control_loop(&mut serial).await;
}

#[embassy_executor::task]
async fn heartbeat_task(pin: Peri<'static, PIN_25>) -> ! {
    let mut led = Output::new(pin, Level::Low);
    loop {
        led.set_high();
        Timer::after_millis(100).await;
        led.set_low();
        Timer::after_millis(900).await;
    }
}

#[embassy_executor::task]
async fn usb_task(mut usb: Device) -> ! {
    usb.run().await
}

#[embassy_executor::task]
async fn audio_task(mut i2s: I2s) -> ! {
    // A repeated 16-bit sample is invariant to left/right slot ordering.
    const FRAMES: usize = 256;
    let mut front = [0_u32; FRAMES];
    let mut back = [0_u32; FRAMES];
    let mut dsp = DspChain::new(SAMPLE_RATE, 0x6d2b_79f5, Parameters::default());

    fill_frames(&mut dsp, &mut front);
    i2s.start();
    loop {
        // Generate into the back buffer while DMA clocks out the front buffer.
        // Once DMA completes, queue the next block within the PIO FIFO's grace
        // period so BCLK and LRC remain continuous.
        let transfer = i2s.write(&front);
        if let Some(parameters) = PARAMETERS.try_take() {
            dsp.set_parameters(parameters);
        }
        fill_frames(&mut dsp, &mut back);
        transfer.await;
        mem::swap(&mut front, &mut back);
    }
}

fn fill_frames(dsp: &mut DspChain, frames: &mut [u32]) {
    for frame in frames {
        let sample = dsp.next_i16() as u16 as u32;
        *frame = sample | (sample << 16);
    }
}

async fn control_loop(serial: &mut Serial) -> ! {
    let mut parameters = Parameters::default();
    loop {
        serial.wait_connection().await;
        info!("USB control connected");
        if control_session(serial, &mut parameters).await.is_err() {
            info!("USB control disconnected");
        }
    }
}

async fn control_session(
    serial: &mut Serial,
    parameters: &mut Parameters,
) -> Result<(), EndpointError> {
    let mut packet = [0_u8; 64];
    let mut line = [0_u8; 64];
    let mut line_len = 0;
    let mut overflowed = false;

    write_packets(serial, b"whitenoise ready; type help\n").await?;
    send_status(serial, *parameters).await?;

    loop {
        let count = serial.read_packet(&mut packet).await?;
        for &byte in &packet[..count] {
            if byte == b'\n' {
                if overflowed {
                    write_packets(serial, b"error: command too long\n").await?;
                } else {
                    handle_line(serial, &line[..line_len], parameters).await?;
                }
                line_len = 0;
                overflowed = false;
            } else if line_len < line.len() {
                line[line_len] = byte;
                line_len += 1;
            } else {
                overflowed = true;
            }
        }
    }
}

async fn handle_line(
    serial: &mut Serial,
    line: &[u8],
    parameters: &mut Parameters,
) -> Result<(), EndpointError> {
    match protocol::parse_line(line) {
        Ok(Command::Help) => write_packets(serial, protocol::HELP.as_bytes()).await,
        Ok(command) => {
            if command.apply(parameters) {
                PARAMETERS.signal(*parameters);
            }
            send_status(serial, *parameters).await
        }
        Err(error) => {
            warn!("bad command: {}", error.message());
            let mut response = ResponseBuffer::<64>::new();
            let _ = writeln!(&mut response, "error: {}", error.message());
            write_packets(serial, response.as_bytes()).await
        }
    }
}

async fn send_status(serial: &mut Serial, parameters: Parameters) -> Result<(), EndpointError> {
    let mut response = ResponseBuffer::<64>::new();
    let _ = protocol::write_parameters(&mut response, parameters);
    write_packets(serial, response.as_bytes()).await
}

async fn write_packets(serial: &mut Serial, bytes: &[u8]) -> Result<(), EndpointError> {
    for packet in bytes.chunks(64) {
        serial.write_packet(packet).await?;
    }
    Ok(())
}
