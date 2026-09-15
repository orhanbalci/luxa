//! A segment's settings, as an effect sees them.

use luxa_color::Rgbw;

/// The per-segment settings every effect may read.
///
/// Handed to [`Effect::render`](crate::Effect::render) alongside the
/// [`Ctx`](crate::Ctx): the context says *when*, the params say *how*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Params {
    /// Speed, `0`–`255`.
    pub speed: u8,
    /// Intensity, `0`–`255`.
    pub intensity: u8,
    /// Primary, background and custom colours.
    pub colors: [Rgbw; 3],
}

impl Params {
    /// Middle speed and intensity, every colour black.
    pub const DEFAULT: Self = Self {
        speed: 128,
        intensity: 128,
        colors: [Rgbw::BLACK; 3],
    };
}

impl Default for Params {
    fn default() -> Self {
        Self::DEFAULT
    }
}
