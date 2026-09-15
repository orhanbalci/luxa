//! A segment's settings, as an effect sees them.

use luxa_color::{CrgbPalette16, Rgbw};

/// The per-segment settings every effect may read.
///
/// Handed to [`Effect::render`](crate::Effect::render) alongside the
/// [`Ctx`](crate::Ctx): the context says *when*, the params say *how*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// Speed, `0`–`255`.
    pub speed: u8,
    /// Intensity, `0`–`255`.
    pub intensity: u8,
    /// Primary, background and custom colours.
    pub colors: [Rgbw; 3],
    /// The segment's palette, already built from its colours where it depends
    /// on them. Effects that use a palette draw from it in place of the
    /// primary colour; `None` means draw with the segment's colours.
    pub palette: Option<CrgbPalette16>,
}

impl Params {
    /// Middle speed and intensity, every colour black, no palette.
    pub const DEFAULT: Self = Self {
        speed: 128,
        intensity: 128,
        colors: [Rgbw::BLACK; 3],
        palette: None,
    };
}

impl Default for Params {
    fn default() -> Self {
        Self::DEFAULT
    }
}
