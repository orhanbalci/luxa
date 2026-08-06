//! The vocabulary every Luxa ingress speaks.
//!
//! This crate is pure data: it depends on nothing and knows nothing about
//! transports, pixels or executors. An HTTP handler, an MQTT subscriber, a
//! physical button and a scheduler are all just different producers of the
//! same [`Command`], and [`luxa-core`](../luxa_core/index.html) consumes them
//! without ever learning which one sent it.
//!
//! [`Snapshot`] is the other half of the seam: the config-only view of engine
//! state that render-side crates read. It is deliberately the *real* snapshot
//! type rather than a slice-1 stand-in — later slices add fields, not a new
//! concept.

#![no_std]
#![forbid(unsafe_code)]

/// A request to change engine state, independent of how it arrived.
///
/// Commands describe *intent*, not effect: `Brightness(128)` means "the user
/// asked for half brightness", and what that does to pixels is decided
/// downstream. Nothing here is transport-shaped — there is no request id, no
/// socket, no reply channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Turn the whole fixture on or off.
    Power(bool),
    /// Set the global brightness, `0` (off) to `255` (full).
    Brightness(u8),
}

/// The config-only view of engine state, published once per command batch.
///
/// This is what the render path reads. It carries settings, never pixels:
/// the renderer is told *what to be*, and works out the framebuffer itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    /// Whether the fixture is on. When `false`, output is black regardless of
    /// [`brightness`](Self::brightness).
    pub power: bool,
    /// Global brightness, `0`–`255`.
    pub brightness: u8,
}

impl Snapshot {
    /// The state a fresh engine starts in: on, at half brightness.
    pub const DEFAULT: Self = Self {
        power: true,
        brightness: 128,
    };
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::DEFAULT
    }
}
