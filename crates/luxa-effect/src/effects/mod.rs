//! The effect implementations themselves.
//!
//! [`Solid`] and [`Rainbow`] are drawn here. Most effects come from
//! [`smart_leds_fx`] through [`Stepped`], and adding one of those is a line in
//! the [`EffectKind`](crate::EffectKind) registry's table.

mod rainbow;
mod solid;
mod stepped;

pub use rainbow::Rainbow;
pub use solid::Solid;
pub use stepped::{Controls, Slots, Stepped};
