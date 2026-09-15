//! Luxa runtime — the wiring, and only the wiring.
//!
//! This crate is the other half of the IO tier: it owns the executor, the
//! peripherals, the sockets and the task graph. Everything it *does* with
//! them comes from the portable crates, every one of which was tested on a
//! laptop before this file existed.
//!
//! The audit is worth doing by eye. Nowhere below is there a hue, a channel
//! scale, a `0xFF`, or a mention of green-before-red. If one appears, it has
//! escaped from the crate that owns it.
//!
//! # The task graph
//!
//! ```text
//!   /json, /ws ──Command──▶ COMMANDS ──▶ engine ──Snapshot──▶ SNAPSHOTS
//!   (net::web)              (channel)      │                 (watch)      │
//!        ▲                                 │ changes                      ▼
//!        └──── broadcasts ◀── broadcaster ◀┘                        render task
//!                                          segment → output → wire → RMT → strip
//! ```
//!
//! Five independent tasks, two seams, one writer of state.

#![no_std]
#![no_main]
// The web task's future nests the whole router and every handler; computing
// its layout goes deeper than the default limit.
#![recursion_limit = "256"]

mod channels;
mod config;
mod device;
mod discovery;
mod engine;
mod http;
mod net;
mod render;
mod reply;
mod websocket;

use embassy_executor::Spawner;
// Brings in the panic handler and backtrace printing.
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::rmt::{Rmt, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use luxa_driver_esp_rmt::RmtWs2812;

use crate::config::{PROFILE, RMT_CLOCK_MHZ};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // The radio blobs allocate; nothing else in Luxa does.
    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    println!("luxa: booting");

    // --- The strip -------------------------------------------------------
    //
    // Divider of 1 so the RMT ticks at the source clock, which is what the
    // driver converts the profile's nanosecond timings against.
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(RMT_CLOCK_MHZ))
        .expect("RMT clock unreachable")
        .into_async();

    let channel = rmt
        .channel0
        .configure_tx(&TxChannelConfig::default().with_clk_divider(1))
        .expect("RMT channel config rejected")
        .with_pin(peripherals.GPIO8);

    let strip = RmtWs2812::new(channel, Rate::from_mhz(RMT_CLOCK_MHZ), PROFILE.timing)
        .expect("strip timing does not fit the RMT clock");

    // --- Tasks -----------------------------------------------------------
    spawn(&spawner, engine::run(seed() as u32), "engine");
    spawn(&spawner, websocket::broadcaster(), "broadcaster");
    let snapshots = channels::SNAPSHOTS
        .receiver()
        .expect("snapshot observer slot");
    spawn(&spawner, render::run(strip, snapshots), "render");

    // The strip is animating by this point; networking coming up late (or not
    // at all) costs you the controls, not the light.
    let (stack, runner, controller) = net::init(peripherals.WIFI, seed());
    spawn(&spawner, net::connection(controller), "wifi");
    spawn(&spawner, net::net(runner), "net");

    net::wait_for_address(stack).await;
    spawn(&spawner, discovery::advertise(stack), "discovery");

    for id in 0..net::WEB_TASKS {
        spawn(&spawner, net::web(id, stack), "web");
    }
}

/// Spawns a task, treating failure as the boot-time bug it is.
///
/// A task pool is sized at compile time, so exhausting one means the task
/// graph and the pool sizes disagree — not a runtime condition to recover
/// from.
fn spawn(
    spawner: &Spawner,
    token: Result<embassy_executor::SpawnToken<impl Sized>, embassy_executor::SpawnError>,
    name: &str,
) {
    match token {
        Ok(token) => spawner.spawn(token),
        Err(_) => panic!("task pool for {name} is exhausted"),
    }
}

/// A seed from the hardware RNG.
///
/// Used for the TCP/IP stack's port and sequence-number randomisation, and for
/// the engine's random colours and values. The engine's seed is drawn before
/// the radio starts, when the RNG's entropy is weaker — fine for picking a
/// colour, which is all it is used for.
fn seed() -> u64 {
    let mut bytes = [0u8; 8];
    esp_hal::rng::Rng::new().read(&mut bytes);
    u64::from_ne_bytes(bytes)
}
