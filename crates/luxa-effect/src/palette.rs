//! The palettes effects can draw from.

/// A palette the catalogue offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Palette {
    /// Stable id, part of the control API's wire contract. `0` is the default
    /// palette: each effect's own choice of colours.
    pub id: u8,
    /// Display name.
    pub name: &'static str,
}

/// Every palette, in id order.
///
/// Only the default palette so far — effects pick their own colours — so
/// selecting any other palette falls back to it.
pub const PALETTES: &[Palette] = &[Palette {
    id: 0,
    name: "Default",
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_palette_is_first() {
        assert_eq!(PALETTES[0].id, 0);
        assert_eq!(PALETTES[0].name, "Default");
    }
}
