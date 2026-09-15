//! Luxa's color vocabulary.
//!
//! Most of what is here is a re-export of [`color8`], so that the rest of the
//! workspace depends on *one* name for a pixel and can be re-pointed at a
//! different color library by editing a single crate.
//!
//! The original code is two small types, and both placements are deliberate.
//!
//! [`ColorOrder`] is vocabulary — "GRB" is a fact about a strip, meaningful to
//! config, to the UI, and to anyone reading a profile — so it is defined here.
//! *Applying* it is wire framing, so that happens in `luxa-wire`. Effects,
//! canvases and the brightness stage all work in plain RGB and never see a
//! channel permutation.
//!
//! [`Rgbw`] is the colour a user *configures*: four channels, because a white
//! channel chosen for an RGBW strip must survive being stored and reported even
//! while the attached strip cannot show it. Rendering stays in [`Crgb`].
//!
//! [`kelvin_to_rgb`] turns a colour temperature into the RGB a controller shows
//! for it.

#![no_std]
#![forbid(unsafe_code)]

pub use color8::{Chsv, Crgb, HsvHue, hsv2rgb_rainbow, hsv2rgb_spectrum, rgb2hsv_approximate};
pub use color8::{blend, nblend, nscale8, nscale8_video};
pub use color8::{
    CLOUD_COLORS, ColorBlend, CrgbPalette16, FOREST_COLORS, LAVA_COLORS, OCEAN_COLORS,
    PARTY_COLORS_GC22, RAINBOW_COLORS_GC22, RAINBOW_STRIPE_COLORS_GC22, color_from_palette16,
    fill_gradient_rgb3, fill_gradient_rgb4,
};

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

/// A configured colour with a dedicated white channel.
///
/// This is for colours that are *stored and exchanged* — a segment's primary
/// colour, a value in a controller API — not for rendering. Effects and the
/// canvas work in [`Crgb`]; how white is mixed into RGB (or driven on its own
/// LED) is decided at output time for the strip actually attached.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgbw {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// White.
    pub w: u8,
}

impl Rgbw {
    /// Every channel off.
    pub const BLACK: Self = Self::new(0, 0, 0, 0);

    /// A colour from its four channels.
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8, w: u8) -> Self {
        Self { r, g, b, w }
    }

    /// An RGB colour with the white channel off.
    #[inline]
    pub const fn from_rgb(c: Crgb) -> Self {
        Self::new(c.r, c.g, c.b, 0)
    }

    /// The RGB channels, with white dropped.
    #[inline]
    pub const fn rgb(self) -> Crgb {
        Crgb::new(self.r, self.g, self.b)
    }

    /// Unpacks the conventional 32-bit form, `0xWWRRGGBB`.
    ///
    /// White sits in the top byte so that a plain `0xRRGGBB` literal is a
    /// valid colour with white off.
    ///
    /// This is an *integer* packing. It is not the byte order of hex colour
    /// strings in the control API, where eight digits mean `RRGGBBWW`.
    #[inline]
    pub const fn from_u32(packed: u32) -> Self {
        Self::new(
            (packed >> 16) as u8,
            (packed >> 8) as u8,
            packed as u8,
            (packed >> 24) as u8,
        )
    }

    /// Packs into `0xWWRRGGBB`; the inverse of [`from_u32`](Self::from_u32).
    #[inline]
    pub const fn to_u32(self) -> u32 {
        ((self.w as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    /// Whether every channel, white included, is off.
    #[inline]
    pub const fn is_black(self) -> bool {
        self.r == 0 && self.g == 0 && self.b == 0 && self.w == 0
    }
}

impl From<Crgb> for Rgbw {
    fn from(c: Crgb) -> Self {
        Self::from_rgb(c)
    }
}

/// The RGB colour of white light at `kelvin`.
///
/// This is the common curve fit of black-body colour by temperature, with the
/// same single-precision maths and rounding as widely used LED controller
/// firmware, so a temperature produces exactly the colour those controllers
/// report. Outside roughly 1 000–40 000 K the fit is extrapolated; the result
/// is always a valid colour.
#[allow(clippy::excessive_precision)] // the fit's published constants, kept verbatim
pub fn kelvin_to_rgb(kelvin: u16) -> Crgb {
    // NaN and infinities (from ln 0) clamp to 0 like everything else.
    fn channel(v: f32) -> u8 {
        let v = libm::roundf(v);
        if v >= 255.0 {
            255
        } else if v > 0.0 {
            v as u8
        } else {
            0
        }
    }

    let temp = f32::from(kelvin) / 100.0;
    let (r, g, b) = if temp <= 66.0 {
        let g = 99.470_802_586_1 * libm::logf(temp) - 161.119_568_166_1;
        let b = if temp <= 19.0 {
            0.0
        } else {
            138.517_731_223_1 * libm::logf(temp - 10.0) - 305.044_792_730_7
        };
        (255.0, g, b)
    } else {
        let r = 329.698_727_446 * libm::powf(temp - 60.0, -0.133_204_759_2);
        let g = 288.122_169_528_3 * libm::powf(temp - 60.0, -0.075_514_849_2);
        (r, g, 255.0)
    };
    Crgb::new(channel(r), channel(g), channel(b))
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

    #[test]
    fn rgbw_unpacks_white_from_the_top_byte() {
        assert_eq!(
            Rgbw::from_u32(0x10FF_A000),
            Rgbw::new(0xFF, 0xA0, 0x00, 0x10)
        );
        // A plain RGB literal has white off.
        assert_eq!(Rgbw::from_u32(0xFFA000).w, 0);
    }

    #[test]
    fn rgbw_packing_round_trips() {
        for packed in [0, 0xFFA000, 0x10FF_A000, 0xFFFF_FFFF, 0x0102_0304] {
            assert_eq!(Rgbw::from_u32(packed).to_u32(), packed);
        }
    }

    #[test]
    fn rgbw_to_rgb_drops_white_and_back_keeps_it_off() {
        let c = Rgbw::new(1, 2, 3, 200);
        assert_eq!(c.rgb(), Crgb::new(1, 2, 3));
        assert_eq!(Rgbw::from(c.rgb()), Rgbw::new(1, 2, 3, 0));
    }

    #[test]
    fn rgbw_black_needs_white_off_too() {
        assert!(Rgbw::BLACK.is_black());
        assert!(!Rgbw::new(0, 0, 0, 1).is_black());
    }

    #[test]
    fn kelvin_matches_the_reference_values() {
        for (kelvin, rgb) in [
            (1900, (255, 132, 0)),
            (2700, (255, 167, 87)),
            (3000, (255, 177, 110)),
            (4000, (255, 206, 166)),
            (5000, (255, 228, 206)),
            (6500, (255, 254, 250)),
            (10000, (202, 218, 255)),
        ] {
            assert_eq!(
                kelvin_to_rgb(kelvin),
                Crgb::new(rgb.0, rgb.1, rgb.2),
                "{kelvin} K"
            );
        }
    }

    #[test]
    fn kelvin_extremes_stay_in_range() {
        assert_eq!(kelvin_to_rgb(0), Crgb::new(255, 0, 0));
        let hot = kelvin_to_rgb(u16::MAX);
        assert_eq!(hot.b, 255);
    }
}
