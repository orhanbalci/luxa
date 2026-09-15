//! How a segment combines with the segments beneath it.

/// How a segment's pixels combine with what the segments beneath it drew.
///
/// Each mode takes a pixel of the segment — the *top* — and the pixel beneath
/// it, one colour channel at a time unless it says otherwise. The segment's
/// opacity then mixes the result with what was beneath. Numbers follow
/// existing LED controllers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendMode {
    /// `0`: the top covers what is beneath.
    Top = 0,
    /// `1`: what is beneath stays, and the segment does not show.
    Bottom = 1,
    /// `2`: top and beneath add up; where a channel would pass full, the whole
    /// colour is scaled back so its hue holds.
    Add = 2,
    /// `3`: the top is taken away from what is beneath.
    Subtract = 3,
    /// `4`: how far apart the top and what is beneath are.
    Difference = 4,
    /// `5`: halfway between the top and what is beneath.
    Average = 5,
    /// `6`: the top and what is beneath multiplied — darker, and black wherever
    /// either is.
    Multiply = 6,
    /// `7`: what is beneath divided by the top — brighter where the top is
    /// dark, full where it is black.
    Divide = 7,
    /// `8`: the brighter of the two.
    Lighten = 8,
    /// `9`: the darker of the two.
    Darken = 9,
    /// `10`: the inverse of multiplying the inverses — brighter, and unchanged
    /// wherever either is black.
    Screen = 10,
    /// `11`: multiply where what is beneath is dark, screen where it is light.
    Overlay = 11,
    /// `12`: multiply where the top is dark, screen where it is light.
    HardLight = 12,
    /// `13`: a gentler hard light that never reaches full contrast.
    SoftLight = 13,
    /// `14`: what is beneath brightened by the top.
    Dodge = 14,
    /// `15`: what is beneath darkened by the top.
    Burn = 15,
    /// `16`: the top wherever it is lit, and what is beneath wherever it is
    /// black — whole pixels, not channels.
    Stencil = 16,
}

impl BlendMode {
    /// Every blend mode, in number order.
    pub const ALL: [Self; 17] = [
        Self::Top,
        Self::Bottom,
        Self::Add,
        Self::Subtract,
        Self::Difference,
        Self::Average,
        Self::Multiply,
        Self::Divide,
        Self::Lighten,
        Self::Darken,
        Self::Screen,
        Self::Overlay,
        Self::HardLight,
        Self::SoftLight,
        Self::Dodge,
        Self::Burn,
        Self::Stencil,
    ];

    /// The blend mode numbered `number`, or [`Top`](Self::Top) for a number
    /// with none.
    pub const fn from_number(number: u8) -> Self {
        if (number as usize) < Self::ALL.len() {
            Self::ALL[number as usize]
        } else {
            Self::Top
        }
    }

    /// This mode's number.
    pub const fn number(self) -> u8 {
        self as u8
    }
}

impl Default for BlendMode {
    /// [`Top`](Self::Top).
    fn default() -> Self {
        Self::Top
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_round_trip() {
        for (number, mode) in BlendMode::ALL.iter().enumerate() {
            assert_eq!(mode.number() as usize, number);
            assert_eq!(BlendMode::from_number(number as u8), *mode);
        }
    }

    #[test]
    fn a_number_with_no_mode_draws_on_top() {
        assert_eq!(BlendMode::from_number(17), BlendMode::Top);
        assert_eq!(BlendMode::from_number(255), BlendMode::Top);
    }
}
