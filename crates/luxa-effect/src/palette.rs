//! The palettes effects can draw from.
//!
//! Ids and names follow existing LED controllers, so their clients select the
//! palette they mean. Id `0` is no palette at all: effects draw with the
//! segment's colours. Ids `1`–`5` are built for a moment rather than stored —
//! from a changing seed, or from the segment's colours — and `6`–`12` are the
//! classic fixed palettes.

use luxa_color::{
    CLOUD_COLORS, Chsv, Crgb, CrgbPalette16, FOREST_COLORS, LAVA_COLORS, OCEAN_COLORS,
    PARTY_COLORS_GC22, RAINBOW_COLORS_GC22, RAINBOW_STRIPE_COLORS_GC22, Rgbw, fill_gradient_rgb3,
    fill_gradient_rgb4, hsv2rgb_rainbow,
};

/// How long [Random Cycle](PALETTES) shows each palette before moving to the
/// next, in milliseconds.
pub const RANDOM_CYCLE_MS: u32 = 5_000;

/// A palette the catalogue offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Stable id, part of the control API's wire contract.
    pub id: u8,
    /// Display name.
    pub name: &'static str,
    source: Source,
}

/// Where a palette's entries come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// No palette: the segment's colours.
    SegmentColors,
    /// A new random palette each cycle.
    RandomCycle,
    /// The primary colour throughout.
    Primary,
    /// Primary blending into background.
    PrimaryAndBackground,
    /// Custom through background to primary.
    Gradient,
    /// Primary and background in two bands, or three with custom.
    ColorsOnly,
    /// A fixed table.
    Fixed(CrgbPalette16),
}

const fn palette(id: u8, name: &'static str, source: Source) -> Palette {
    Palette { id, name, source }
}

/// Every palette, in id order.
pub const PALETTES: &[Palette] = &[
    palette(0, "Default", Source::SegmentColors),
    palette(1, "* Random Cycle", Source::RandomCycle),
    palette(2, "* Color 1", Source::Primary),
    palette(3, "* Colors 1&2", Source::PrimaryAndBackground),
    palette(4, "* Color Gradient", Source::Gradient),
    palette(5, "* Colors Only", Source::ColorsOnly),
    palette(6, "Party", Source::Fixed(PARTY_COLORS_GC22)),
    palette(7, "Cloud", Source::Fixed(CLOUD_COLORS)),
    palette(8, "Lava", Source::Fixed(LAVA_COLORS)),
    palette(9, "Ocean", Source::Fixed(OCEAN_COLORS)),
    palette(10, "Forest", Source::Fixed(FOREST_COLORS)),
    palette(11, "Rainbow", Source::Fixed(RAINBOW_COLORS_GC22)),
    palette(
        12,
        "Rainbow Bands",
        Source::Fixed(RAINBOW_STRIPE_COLORS_GC22),
    ),
];

impl Palette {
    /// The palette with `id`, if there is one.
    pub const fn from_id(id: u8) -> Option<&'static Palette> {
        let mut i = 0;
        while i < PALETTES.len() {
            if PALETTES[i].id == id {
                return Some(&PALETTES[i]);
            }
            i += 1;
        }
        None
    }

    /// The entries to draw from for a segment with `colors`, or `None` for no
    /// palette.
    ///
    /// `cycle` picks Random Cycle's palette: the same value gives the same
    /// palette, and a new value a new one. Other palettes ignore it.
    pub fn entries(&self, colors: [Rgbw; 3], cycle: u32) -> Option<CrgbPalette16> {
        let [primary, background, custom] = colors.map(Rgbw::rgb);
        let mut entries = [Crgb::new(0, 0, 0); 16];
        match self.source {
            Source::SegmentColors => return None,
            Source::RandomCycle => return Some(random_palette(cycle)),
            Source::Fixed(palette) => return Some(palette),
            Source::Primary => entries.fill(primary),
            Source::PrimaryAndBackground => {
                fill_gradient_rgb4(&mut entries, primary, primary, background, background);
            }
            Source::Gradient => fill_gradient_rgb3(&mut entries, custom, background, primary),
            Source::ColorsOnly => {
                if custom.is_black() {
                    entries[..8].fill(primary);
                    entries[8..].fill(background);
                } else {
                    entries[..5].fill(primary);
                    entries[5..10].fill(background);
                    entries[10..].fill(custom);
                }
            }
        }
        Some(CrgbPalette16(entries))
    }
}

/// A palette of four colours spread around the hue wheel from a random start,
/// each with its own saturation and brightness, blended into each other.
fn random_palette(seed: u32) -> CrgbPalette16 {
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9);
        let mut z = state;
        z = (z ^ (z >> 16)).wrapping_mul(0x21F0_AAAD);
        z = (z ^ (z >> 15)).wrapping_mul(0x735A_2D97);
        z ^ (z >> 15)
    };
    let start = next() as u8;
    let mut stop = |quarter: u8| {
        let r = next();
        let hue = start
            .wrapping_add(quarter.wrapping_mul(64))
            .wrapping_add(r as u8 >> 3);
        let saturation = 192 + (r >> 8) as u8 % 64;
        let value = 128 + (r >> 16) as u8 % 128;
        hsv2rgb_rainbow(Chsv::new(hue, saturation, value))
    };
    let mut entries = [Crgb::new(0, 0, 0); 16];
    let (a, b, c, d) = (stop(0), stop(1), stop(2), stop(3));
    fill_gradient_rgb4(&mut entries, a, b, c, d);
    CrgbPalette16(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgbw = Rgbw::new(255, 0, 0, 0);
    const GREEN: Rgbw = Rgbw::new(0, 255, 0, 0);
    const BLUE: Rgbw = Rgbw::new(0, 0, 255, 0);

    fn entries(id: u8, colors: [Rgbw; 3]) -> [Crgb; 16] {
        Palette::from_id(id).unwrap().entries(colors, 0).unwrap().0
    }

    #[test]
    fn ids_are_unique_in_order_and_found() {
        assert_eq!((PALETTES[0].id, PALETTES[0].name), (0, "Default"));
        for pair in PALETTES.windows(2) {
            assert!(pair[0].id < pair[1].id, "{pair:?}");
        }
        for palette in PALETTES {
            assert_eq!(Palette::from_id(palette.id), Some(palette));
        }
        assert_eq!(Palette::from_id(13), None);
    }

    #[test]
    fn the_default_palette_is_the_segment_colours() {
        let default = Palette::from_id(0).unwrap();
        assert_eq!(default.entries([RED, GREEN, BLUE], 7), None);
    }

    #[test]
    fn colour_palettes_follow_the_segment_colours() {
        assert_eq!(entries(2, [RED, GREEN, BLUE]), [RED.rgb(); 16]);

        let pair = entries(3, [RED, GREEN, BLUE]);
        assert_eq!((pair[0], pair[15]), (RED.rgb(), GREEN.rgb()));

        // The gradient's last step truncates, as FastLED's does: 254, not 255.
        let gradient = entries(4, [RED, GREEN, BLUE]);
        assert_eq!(
            (gradient[0], gradient[15]),
            (BLUE.rgb(), Crgb::new(254, 0, 0))
        );
    }

    #[test]
    fn colors_only_shows_two_bands_or_three() {
        let two = entries(5, [RED, GREEN, Rgbw::BLACK]);
        assert_eq!((two[7], two[8]), (RED.rgb(), GREEN.rgb()));

        let three = entries(5, [RED, GREEN, BLUE]);
        assert_eq!(
            (three[4], three[5], three[9], three[10]),
            (RED.rgb(), GREEN.rgb(), GREEN.rgb(), BLUE.rgb())
        );
    }

    #[test]
    fn fixed_palettes_ignore_the_colours() {
        assert_eq!(entries(8, [RED; 3]), LAVA_COLORS.0);
        assert_eq!(entries(8, [BLUE; 3]), LAVA_COLORS.0);
        assert_eq!(entries(6, [RED; 3]), PARTY_COLORS_GC22.0);
    }

    #[test]
    fn random_cycle_changes_only_with_the_cycle() {
        let random = Palette::from_id(1).unwrap();
        let at = |cycle| random.entries([RED; 3], cycle).unwrap();
        assert_eq!(at(3), at(3));
        assert_ne!(at(3), at(4));
        assert!(at(3).0.iter().any(|entry| !entry.is_black()));
    }
}
