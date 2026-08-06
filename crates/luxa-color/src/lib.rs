//! Luxa's color vocabulary.
//!
//! Almost everything here is a re-export of [`color8`], so that the rest of
//! the workspace depends on *one* name for a pixel and can be re-pointed at a
//! different color library by editing a single crate.
//!
//! The one piece of original code is [`ColorOrder`], and its placement is a
//! deliberate seam. The *enum* is vocabulary — "GRB" is a fact about a strip,
//! meaningful to config, to the UI, and to anyone reading a profile — so it is
//! defined here. *Applying* it is wire framing, so that happens in
//! `luxa-wire`. Effects, canvases and the brightness stage all work in plain
//! RGB and never see a channel permutation.

#![no_std]
#![forbid(unsafe_code)]

pub use color8::{Chsv, Crgb, HsvHue, hsv2rgb_rainbow, hsv2rgb_spectrum, rgb2hsv_approximate};
pub use color8::{blend, nblend, nscale8, nscale8_video};

/// The order a chipset expects color channels on the wire.
///
/// WS2812/WS2812B strips are [`Grb`](Self::Grb); SK6812 and most APA102 clones
/// are [`Rgb`](Self::Rgb) or [`Bgr`](Self::Bgr). Which one a given strip wants
/// is a property of the hardware you plugged in, so it is a runtime value on
/// the LED profile — never a compile-time constant.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorOrder {
    /// Red, green, blue.
    Rgb,
    /// Green, red, blue — WS2812 and friends.
    #[default]
    Grb,
    /// Blue, green, red.
    Bgr,
    /// Red, blue, green.
    Rbg,
    /// Green, blue, red.
    Gbr,
    /// Blue, red, green.
    Brg,
}

impl ColorOrder {
    /// Permutes a pixel's channels into wire order.
    ///
    /// This is the whole of the conversion; `luxa-wire` calls it per pixel and
    /// nothing else in the workspace calls it at all.
    #[inline]
    pub const fn permute(self, c: Crgb) -> [u8; 3] {
        let (r, g, b) = (c.r, c.g, c.b);
        match self {
            Self::Rgb => [r, g, b],
            Self::Grb => [g, r, b],
            Self::Bgr => [b, g, r],
            Self::Rbg => [r, b, g],
            Self::Gbr => [g, b, r],
            Self::Brg => [b, r, g],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: Crgb = Crgb::new(0x11, 0x22, 0x33);

    #[test]
    fn permutes_each_order() {
        assert_eq!(ColorOrder::Rgb.permute(C), [0x11, 0x22, 0x33]);
        assert_eq!(ColorOrder::Grb.permute(C), [0x22, 0x11, 0x33]);
        assert_eq!(ColorOrder::Bgr.permute(C), [0x33, 0x22, 0x11]);
        assert_eq!(ColorOrder::Rbg.permute(C), [0x11, 0x33, 0x22]);
        assert_eq!(ColorOrder::Gbr.permute(C), [0x22, 0x33, 0x11]);
        assert_eq!(ColorOrder::Brg.permute(C), [0x33, 0x11, 0x22]);
    }

    #[test]
    fn default_is_ws2812_order() {
        assert_eq!(ColorOrder::default(), ColorOrder::Grb);
    }

    /// Every order is a permutation — no channel is dropped or duplicated.
    #[test]
    fn every_order_is_a_permutation() {
        for order in [
            ColorOrder::Rgb,
            ColorOrder::Grb,
            ColorOrder::Bgr,
            ColorOrder::Rbg,
            ColorOrder::Gbr,
            ColorOrder::Brg,
        ] {
            let mut out = order.permute(C);
            out.sort_unstable();
            assert_eq!(out, [0x11, 0x22, 0x33], "{order:?} is not a permutation");
        }
    }
}
