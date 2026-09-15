//! A single colour across the whole view.

use luxa_color::Crgb;

use crate::{Ctx, Effect, Params};

/// Fills the view with the primary colour.
///
/// Stateless and time-independent. A white channel in the colour is dropped:
/// how white reaches an RGBW strip is decided at output, not here.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Solid;

impl Solid {
    /// The solid effect.
    pub const fn new() -> Self {
        Self
    }
}

impl Effect for Solid {
    fn render(&mut self, view: &mut [Crgb], _ctx: &Ctx, params: &Params) {
        view.fill(params.colors[0].rgb());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luxa_color::Rgbw;

    fn params(primary: Rgbw) -> Params {
        Params {
            colors: [primary, Rgbw::new(9, 9, 9, 0), Rgbw::BLACK],
            ..Params::DEFAULT
        }
    }

    #[test]
    fn fills_every_pixel_with_the_primary_colour() {
        let mut view = [Crgb::new(0, 0, 0); 5];
        Solid.render(
            &mut view,
            &Ctx::from_millis(0),
            &params(Rgbw::new(255, 160, 0, 0)),
        );
        assert!(view.iter().all(|p| *p == Crgb::new(255, 160, 0)));
    }

    #[test]
    fn does_not_change_over_time() {
        let p = params(Rgbw::new(1, 2, 3, 0));
        let mut a = [Crgb::new(0, 0, 0); 3];
        let mut b = [Crgb::new(0, 0, 0); 3];
        Solid.render(&mut a, &Ctx::from_millis(0), &p);
        Solid.render(&mut b, &Ctx::from_millis(123_456), &p);
        assert_eq!(a, b);
    }

    #[test]
    fn drops_the_white_channel() {
        let mut view = [Crgb::new(0, 0, 0); 1];
        Solid.render(
            &mut view,
            &Ctx::from_millis(0),
            &params(Rgbw::new(0, 0, 0, 200)),
        );
        assert!(view[0].is_black());
    }

    #[test]
    fn empty_view_is_a_no_op() {
        Solid.render(&mut [], &Ctx::from_millis(0), &Params::DEFAULT);
    }
}
